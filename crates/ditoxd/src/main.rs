// On Windows the daemon runs without a console window of its own.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::Read;
use std::process::ExitCode;
use std::sync::mpsc::{self, Sender};

use ditox_rpc::{Client, Kind, socket_path};
use ditoxd::app::{App, Desktop, NoDesktop};
use ditoxd::clipboard::Clipboard;
use ditoxd::config::Config;
use ditoxd::error::{Error, Result};
use ditoxd::policy;
use ditoxd::server::{self, Request, Watcher};

#[cfg(not(windows))]
const USAGE: &str = "usage: ditoxd daemon | ditoxd ingest <wake|text|image> | ditoxd --version";
#[cfg(windows)]
const USAGE: &str = "usage: ditoxd daemon | ditoxd install | ditoxd uninstall | ditoxd ingest <wake|text|image> | ditoxd --version";

fn main() -> ExitCode {
    #[cfg(windows)]
    ditox_rpc::win::attach_console();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["daemon" | "run"] => daemon(),
        #[cfg(windows)]
        ["install"] => install(),
        #[cfg(windows)]
        ["uninstall"] => ditoxd::windows::autostart::uninstall(),
        ["ingest", "wake"] => {
            wake();
            Ok(())
        }
        ["ingest", "text"] => {
            ingest(Kind::Text);
            Ok(())
        }
        ["ingest", "image"] => {
            ingest(Kind::Image);
            Ok(())
        }
        ["--version" | "-V"] => {
            println!("ditoxd {}", ditoxd::rpc::VERSION);
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ditoxd: {err}");
            ExitCode::FAILURE
        }
    }
}

/// glibc raises its mmap threshold after a large free and then keeps that much
/// heap for good. One copied screenshot would leave the daemon tens of
/// megabytes heavier for the rest of the session. A fixed threshold hands
/// large buffers back as soon as they are freed.
fn return_large_buffers() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 256 * 1024);
    }
}

fn daemon() -> Result<()> {
    return_large_buffers();
    let socket = socket_path().ok_or_else(|| Error::Config(ditox_rpc::no_socket_path()))?;
    #[cfg(unix)]
    if std::env::var_os("DITOX_SOCKET").is_none_or(|path| path.is_empty())
        && let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR")
    {
        server::check_runtime_dir(std::path::Path::new(&runtime))?;
    }
    let cfg = Config::load()?;
    let clipboard = Clipboard::from_env();
    let mock = clipboard.is_mock();
    let desktop: Box<dyn Desktop> = if mock { Box::new(NoDesktop) } else { desktop() };
    let open_keybind = cfg.open_keybind.clone();
    let app = App::open(cfg, clipboard, desktop)?;
    let _ = app.store.clear_watcher_error();

    let listener = server::listen(&socket)?;
    let (requests, queue) = mpsc::channel();
    let watch = if mock {
        None
    } else {
        Some(watcher(&requests, &open_keybind)?)
    };
    server::accept(listener, requests);
    server::run(app, queue, watch);
    Ok(())
}

#[cfg(not(windows))]
fn desktop() -> Box<dyn Desktop> {
    Box::new(ditoxd::app::Hyprland)
}

#[cfg(windows)]
fn desktop() -> Box<dyn Desktop> {
    Box::new(ditoxd::windows::desktop::Windows)
}

fn own_program() -> Result<std::path::PathBuf> {
    std::env::current_exe().map_err(|err| Error::Io(err.to_string()))
}

/// `wl-paste --watch`, which starts this program again for every change.
#[cfg(not(windows))]
fn watcher(_: &Sender<Request>, _: &str) -> Result<Box<dyn Watcher>> {
    let exe = own_program()?.to_string_lossy().into_owned();
    Ok(Box::new(server::Watch::new(exe)))
}

/// A thread that Windows tells of every change, and of the key that opens
/// the list: the `ditox` beside this program.
#[cfg(windows)]
fn watcher(requests: &Sender<Request>, open_keybind: &str) -> Result<Box<dyn Watcher>> {
    let list = own_program()?.with_file_name("ditox.exe");
    Ok(Box::new(ditoxd::windows::listener::Listener::new(
        requests.clone(),
        list,
        open_keybind,
    )))
}

/// Start the daemon with every session from now on.
#[cfg(windows)]
fn install() -> Result<()> {
    let exe = own_program()?.to_string_lossy().into_owned();
    ditoxd::windows::autostart::install(&exe)?;
    println!("ditoxd starts with the session from now on. Start it now with: ditoxd daemon");
    Ok(())
}

fn sensitive() -> bool {
    policy::offer_is_sensitive(std::env::var("CLIPBOARD_STATE").ok().as_deref())
}

/// What is on stdin, or `None` when it is more than the daemon keeps: a cut
/// image would be stored as a file nothing can open.
fn stdin_bytes() -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let _ = std::io::stdin()
        .lock()
        .take(policy::MAX_IMAGE_READ_BYTES as u64 + 1)
        .read_to_end(&mut bytes);
    (bytes.len() <= policy::MAX_IMAGE_READ_BYTES).then_some(bytes)
}

fn wake() {
    let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
    if let Ok(mut client) = Client::connect() {
        let _ = client.wake(sensitive());
    }
}

fn ingest(kind: Kind) {
    if sensitive() {
        return;
    }
    let Some(bytes) = stdin_bytes() else {
        eprintln!("ditoxd: the input is larger than the daemon keeps");
        return;
    };
    if bytes.is_empty() {
        return;
    }
    if let Ok(mut client) = Client::connect() {
        let _ = client.ingest(kind, &bytes);
    }
}
