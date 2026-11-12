//! What Windows tells the daemon: the clipboard changed, or the key that
//! opens the list was pressed. One thread with a message-only window hears
//! both, where Linux has `wl-paste --watch` and a key bound in the compositor.

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, RemoveClipboardFormatListener,
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW,
    HWND_MESSAGE, MSG, PostThreadMessageW, WM_APP, WM_CLIPBOARDUPDATE, WM_HOTKEY,
};

use super::format;
use crate::config::Config;
use crate::server::{Request, Watcher};

const OPEN_KEY: i32 = 1;
/// Asks the thread to register the open key again: the setting changed.
const REBIND: u32 = WM_APP + 1;
const START_WAIT: Duration = Duration::from_secs(5);

/// What the main loop and the thread both hold.
#[derive(Default)]
struct Shared {
    /// The keybind that opens the list; empty for none.
    open_keybind: String,
    /// A problem the thread met, until the main loop has told the user.
    problem: Option<String>,
}

struct Running {
    thread: JoinHandle<()>,
    id: u32,
}

pub struct Listener {
    requests: Sender<Request>,
    /// The program a press of the open key starts: the list.
    program: PathBuf,
    shared: Arc<Mutex<Shared>>,
    running: Option<Running>,
    failing: bool,
}

impl Listener {
    pub fn new(requests: Sender<Request>, program: PathBuf, open_keybind: &str) -> Self {
        let shared = Shared {
            open_keybind: open_keybind.to_string(),
            problem: None,
        };
        Listener {
            requests,
            program,
            shared: Arc::new(Mutex::new(shared)),
            running: None,
            failing: false,
        }
    }

    fn start(&self) -> std::result::Result<Running, String> {
        let (ready, started) = mpsc::channel();
        let (requests, program, shared) = (
            self.requests.clone(),
            self.program.clone(),
            self.shared.clone(),
        );
        let thread = std::thread::Builder::new()
            .name("ditoxd-windows".into())
            .spawn(move || listen(requests, program, shared, ready))
            .map_err(|err| format!("the clipboard listener: {err}"))?;
        match started.recv_timeout(START_WAIT) {
            Ok(Ok(id)) => Ok(Running { thread, id }),
            Ok(Err(problem)) => Err(problem),
            Err(_) => Err("the clipboard listener did not start".into()),
        }
    }
}

impl Watcher for Listener {
    fn ensure_running(&mut self) -> std::result::Result<(), Option<String>> {
        if self
            .running
            .as_ref()
            .is_some_and(|running| !running.thread.is_finished())
        {
            self.failing = false;
            return Ok(());
        }
        let stopped = self.running.take().is_some();
        let problem = match self.start() {
            Ok(running) => {
                self.running = Some(running);
                if !stopped {
                    return Ok(());
                }
                "the clipboard listener stopped".to_string()
            }
            Err(problem) => problem,
        };
        let first = !std::mem::replace(&mut self.failing, true);
        Err(first.then_some(problem))
    }

    fn follow(&mut self, cfg: &Config) -> Option<String> {
        let mut shared = self.shared.lock().unwrap_or_else(|held| held.into_inner());
        if shared.open_keybind != cfg.open_keybind {
            shared.open_keybind = cfg.open_keybind.clone();
            if let Some(running) = &self.running {
                // SAFETY: posts a message to a thread id; a thread that is
                // gone makes the call fail and nothing else.
                unsafe { PostThreadMessageW(running.id, REBIND, 0, 0) };
            }
        }
        shared.problem.take()
    }
}

fn listen(
    requests: Sender<Request>,
    program: PathBuf,
    shared: Arc<Mutex<Shared>>,
    ready: Sender<std::result::Result<u32, String>>,
) {
    let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
    // SAFETY: a window of a built-in class, parented to the message-only
    // root, with no name, menu, module or creation data; `class` is a
    // NUL-terminated string that outlives the call.
    let window = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    // SAFETY: `window` is checked before it is listened on.
    if window.is_null() || unsafe { AddClipboardFormatListener(window) } == 0 {
        let _ = ready.send(Err(format!(
            "Windows did not let the daemon listen to the clipboard ({})",
            std::io::Error::last_os_error()
        )));
        return;
    }
    // SAFETY: no arguments.
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    bind_open_key(window, &shared);

    // SAFETY: `message` is plain data `GetMessageW` fills; the loop runs on
    // the thread that owns `window`, and ends when the queue reports an error
    // or a quit.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            match message.message {
                // The clipboard is not looked at here: the program that set
                // it may still be filling it, and cannot while another has
                // it open. The capture reads it once the changes stop.
                WM_CLIPBOARDUPDATE => {
                    if !wake(&requests) {
                        break;
                    }
                }
                WM_HOTKEY if message.wParam == OPEN_KEY as usize => open_list(&program),
                REBIND => bind_open_key(window, &shared),
                _ => {
                    DispatchMessageW(&message);
                }
            }
        }
        UnregisterHotKey(window, OPEN_KEY);
        RemoveClipboardFormatListener(window);
        DestroyWindow(window);
    }
}

/// Asks the main loop for a capture, as `ditoxd ingest wake` does. `false`
/// when the daemon is no longer taking requests.
fn wake(requests: &Sender<Request>) -> bool {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "clipboard.wake",
        "params": {},
    });
    let (reply, _answer) = mpsc::channel();
    requests
        .send(Request {
            body: body.to_string().into_bytes(),
            reply,
        })
        .is_ok()
}

fn bind_open_key(window: HWND, shared: &Mutex<Shared>) {
    let mut shared = shared.lock().unwrap_or_else(|held| held.into_inner());
    // SAFETY: `window` belongs to this thread; a key that was not
    // registered makes the call fail and nothing else.
    unsafe { UnregisterHotKey(window, OPEN_KEY) };
    let keybind = shared.open_keybind.trim().to_string();
    if keybind.is_empty() {
        return;
    }
    let Some(chord) = format::chord(&keybind) else {
        shared.problem = Some(format!("\"{keybind}\" is not a key Windows can listen for"));
        return;
    };
    let modifiers = [
        (chord.ctrl, MOD_CONTROL),
        (chord.shift, MOD_SHIFT),
        (chord.alt, MOD_ALT),
        (chord.win, MOD_WIN),
    ]
    .into_iter()
    .filter_map(|(on, flag)| on.then_some(flag))
    .fold(MOD_NOREPEAT, |all, flag| all | flag);
    // SAFETY: `window` belongs to this thread.
    if unsafe { RegisterHotKey(window, OPEN_KEY, modifiers, u32::from(chord.key)) } == 0 {
        shared.problem = Some(format!(
            "{keybind} could not be made the key that opens Ditox: another program has it"
        ));
    }
}

/// Start the list. The key press lets this thread hand the keyboard on, and
/// the new process is the one it is handed to.
fn open_list(program: &std::path::Path) {
    match std::process::Command::new(program).spawn() {
        // SAFETY: takes a process id; a process that is gone is refused.
        Ok(child) => unsafe {
            AllowSetForegroundWindow(child.id());
        },
        Err(err) => eprintln!("ditoxd: {}: {err}", program.display()),
    }
}
