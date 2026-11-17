//! The Windows desktop, asked directly: which window has the keyboard, whose
//! it is, where it sits. Both the daemon and the window ask, as both ask
//! Hyprland on Linux.
//!
//! A window's address is its handle in hexadecimal, as Hyprland's is. Its
//! class is the name of the program that owns it, without `.exe`: what a
//! person would write in a list of applications.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HWND, INVALID_HANDLE_VALUE, LPARAM, POINT, RECT,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MonitorFromWindow,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    SetStdHandle,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, EnumWindows, GetClientRect, GetCursorPos,
    GetForegroundWindow, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed, PostMessageW,
    SetForegroundWindow, WM_CLOSE,
};

/// A top-level window as the desktop has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub address: String,
    pub class: String,
    pub title: String,
    pub pid: u32,
}

/// A monitor: the part of it windows may cover, in desktop pixels, and how
/// many pixels a point is there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
    /// The whole monitor, taskbar included: its corner and its size.
    pub whole: (i32, i32, i32, i32),
    /// Windows' handle for the monitor, as a number.
    pub monitor: isize,
}

/// A program built for the Windows desktop starts without a console. Started
/// from one, it borrows it, so that `--version` and errors can be read.
/// Output the starter redirected is left where it was sent.
pub fn attach_console() {
    // SAFETY: the handles are only compared, and `name` is a NUL-terminated
    // string that outlives the call. The console handle is kept open for the
    // life of the process on purpose: it is its output.
    unsafe {
        let missing = |which| {
            let handle = GetStdHandle(which);
            handle.is_null() || handle == INVALID_HANDLE_VALUE
        };
        if (!missing(STD_OUTPUT_HANDLE) && !missing(STD_ERROR_HANDLE))
            || AttachConsole(ATTACH_PARENT_PROCESS) == 0
        {
            return;
        }
        let name: Vec<u16> = "CONOUT$".encode_utf16().chain([0]).collect();
        let console = CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if console.is_null() || console == INVALID_HANDLE_VALUE {
            return;
        }
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            if missing(which) {
                SetStdHandle(which, console);
            }
        }
    }
}

/// Ask Windows for true pixels from here on. Until a program says it knows
/// about scaled screens, Windows answers it as if every screen were at
/// 100 %: sizes come back divided by the scale and the scale as 1. The
/// window's toolkit says so when it starts, which is after the list has
/// worked out where to open. Call first in `main`.
pub fn know_scaled_screens() {
    // SAFETY: takes a constant. A program that already said so is refused
    // and nothing else happens.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// Let another program take the keyboard from this one: the daemon, which
/// gives it to the window a clip is pasted into.
pub fn allow_focus_change() {
    // SAFETY: takes a process id or this constant for any process.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

/// A handle written as an address.
pub fn address(window: HWND) -> String {
    format!("{:#x}", window as usize)
}

/// The handle an address names, when a window still answers to it.
pub fn handle(address: &str) -> Option<HWND> {
    let digits = address.trim().strip_prefix("0x")?;
    let window = usize::from_str_radix(digits, 16).ok()? as HWND;
    // SAFETY: `IsWindow` takes any value and only says whether it is a window.
    (!window.is_null() && unsafe { IsWindow(window) } != 0).then_some(window)
}

/// The window that has the keyboard.
pub fn foreground() -> Option<Window> {
    // SAFETY: no arguments; a null handle means no window has the keyboard.
    describe(unsafe { GetForegroundWindow() })
}

/// The window at `address`.
pub fn window(address: &str) -> Option<Window> {
    describe(handle(address)?)
}

/// The class of the window at `address`: the program that owns it.
pub fn class_of(address: &str) -> Option<String> {
    window(address)
        .map(|window| window.class)
        .filter(|class| !class.is_empty())
}

/// Whether the window at `address` has the keyboard.
pub fn is_foreground(address: &str) -> bool {
    // SAFETY: no arguments.
    handle(address).is_some_and(|window| window == unsafe { GetForegroundWindow() })
}

/// Give the keyboard to the window at `address`. Windows lets a program do
/// that only while it has the keyboard itself or was just handed it.
pub fn focus(address: &str) -> bool {
    let Some(window) = handle(address) else {
        return false;
    };
    // SAFETY: `window` was a window a moment ago; a stale handle is refused.
    unsafe { SetForegroundWindow(window) != 0 }
}

/// Ask the window at `address` to close, as its close button would.
pub fn close(address: &str) -> bool {
    let Some(window) = handle(address) else {
        return false;
    };
    // SAFETY: posting a message to a handle that went stale fails cleanly.
    unsafe { PostMessageW(window, WM_CLOSE, 0, 0) != 0 }
}

/// This process's visible top-level window.
pub fn own_window() -> Option<Window> {
    own_window_where(None)
}

/// This process's visible top-level window called `title`, when it has
/// more than one.
pub fn own_window_titled(title: &str) -> Option<Window> {
    own_window_where(Some(title))
}

fn own_window_where(titled: Option<&str>) -> Option<Window> {
    struct Search<'a> {
        pid: u32,
        titled: Option<&'a str>,
        found: HWND,
    }
    unsafe extern "system" fn visit(window: HWND, search: LPARAM) -> i32 {
        // SAFETY: `search` is the `Search` the caller below lends for the
        // length of `EnumWindows`, and `window` is a live handle it passes.
        unsafe {
            let search = &mut *(search as *mut Search);
            let mut pid = 0;
            GetWindowThreadProcessId(window, &mut pid);
            if pid == search.pid
                && IsWindowVisible(window) != 0
                && search.titled.is_none_or(|wanted| title(window) == wanted)
            {
                search.found = window;
                return 0;
            }
        }
        1
    }
    let mut search = Search {
        pid: std::process::id(),
        titled,
        found: std::ptr::null_mut(),
    };
    // SAFETY: `visit` only runs during this call, with `search` alive.
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    describe(search.found)
}

/// Where the window at `address` is, in desktop pixels: the corner of the
/// window, which is what a window is placed by, and the size of its content,
/// which is what it is sized by. `None` for a window that is minimized or
/// maximized, whose place is not one it was put at.
pub fn frame(address: &str) -> Option<(i32, i32, i32, i32)> {
    let window = handle(address)?;
    let empty = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let (mut outer, mut content) = (empty, empty);
    // SAFETY: `window` is a handle and each rectangle a place to write four
    // numbers.
    unsafe {
        if IsIconic(window) != 0
            || IsZoomed(window) != 0
            || GetWindowRect(window, &mut outer) == 0
            || GetClientRect(window, &mut content) == 0
        {
            return None;
        }
    }
    Some((outer.left, outer.top, content.right, content.bottom))
}

/// The screen the window at `address` is on.
pub fn screen_of(address: &str) -> Option<Screen> {
    // SAFETY: a stale handle gives the nearest monitor, which is still one.
    screen(unsafe { MonitorFromWindow(handle(address)?, MONITOR_DEFAULTTONEAREST) })
}

/// The screen the pointer is on: where a person is looking.
pub fn screen_under_pointer() -> Option<Screen> {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a place to write two numbers; any point has a
    // nearest monitor.
    unsafe {
        if GetCursorPos(&mut point) == 0 {
            return None;
        }
        screen(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST))
    }
}

fn screen(monitor: *mut c_void) -> Option<Screen> {
    if monitor.is_null() {
        return None;
    }
    // SAFETY: `info` is all-zero plain data with its size filled in, as
    // `GetMonitorInfoW` asks, and the two numbers are places to write.
    unsafe {
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let (mut across, mut down) = (96u32, 96u32);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut across, &mut down);
        let (work, whole) = (info.rcWork, info.rcMonitor);
        Some(Screen {
            x: work.left,
            y: work.top,
            width: work.right - work.left,
            height: work.bottom - work.top,
            scale: f64::from(across.max(1)) / 96.0,
            whole: (
                whole.left,
                whole.top,
                whole.right - whole.left,
                whole.bottom - whole.top,
            ),
            monitor: monitor as isize,
        })
    }
}

fn describe(window: HWND) -> Option<Window> {
    if window.is_null() {
        return None;
    }
    let mut pid = 0;
    // SAFETY: `window` is a handle and `pid` a place to write one number.
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    Some(Window {
        address: address(window),
        class: program(pid).unwrap_or_default(),
        title: title(window),
        pid,
    })
}

fn title(window: HWND) -> String {
    // SAFETY: the buffer is as long as the length it is given.
    unsafe {
        let length = GetWindowTextLengthW(window).max(0) as usize;
        let mut text = vec![0u16; length + 1];
        let written = GetWindowTextW(window, text.as_mut_ptr(), text.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&text[..written.min(length)])
    }
}

/// The name of the program process `pid` runs, without its directory or `.exe`.
fn program(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed before returning, and the buffer is as
    // long as the length it is given.
    let path = unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut path = vec![0u16; 1024];
        let mut length = path.len() as u32;
        let named = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if named == 0 {
            return None;
        }
        String::from_utf16_lossy(&path[..length as usize])
    };
    Some(program_name(&path))
}

/// `C:\Program Files\Mozilla Firefox\firefox.exe` is `firefox`.
pub fn program_name(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match file.rsplit_once('.') {
        Some((stem, extension)) if extension.eq_ignore_ascii_case("exe") && !stem.is_empty() => {
            stem.to_string()
        }
        _ => file.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_named_without_its_directory_or_exe() {
        assert_eq!(
            program_name("C:\\Program Files\\Mozilla Firefox\\firefox.exe"),
            "firefox"
        );
        assert_eq!(
            program_name("C:/tools/WindowsTerminal.EXE"),
            "WindowsTerminal"
        );
        assert_eq!(program_name("notes.v2.exe"), "notes.v2");
        assert_eq!(program_name("system"), "system");
        assert_eq!(program_name(".exe"), ".exe");
    }

    #[test]
    fn an_address_is_the_handle_in_hexadecimal() {
        assert_eq!(address(0x1a2b as HWND), "0x1a2b");
        assert_eq!(handle("0x0"), None);
        assert_eq!(handle("nonsense"), None);
        assert_eq!(handle("1a2b"), None, "an address starts with 0x");
    }
}
