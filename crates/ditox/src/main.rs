// On Windows the windows open without a console beside them.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod feed;
mod fixture;
mod list;
mod model;
mod options;
mod overlay;
mod picture;
mod place;
mod startup;
mod theme;

use std::time::Duration;

use rune::{App, AppHandle, AppSpec, WindowOptions};

pub const APP_ID: &str = "dev.ditox";
const LIST_TITLE: &str = "Ditox";
const OPTIONS_TITLE: &str = "Ditox Options";
#[cfg(not(windows))]
const HYPRLAND_WAIT: Duration = Duration::from_millis(500);

struct Window {
    address: String,
    class: String,
    title: String,
}

/// The window that has the keyboard, as Windows has it.
#[cfg(windows)]
fn active_window() -> Option<Window> {
    ditox_rpc::win::foreground().map(|window| Window {
        address: window.address,
        class: window.class,
        title: window.title,
    })
}

/// Whether `window` is one of this program's. Hyprland knows a window by the
/// application id it was given, Windows by the program that owns it.
fn ours(window: &Window) -> bool {
    if cfg!(windows) {
        std::env::current_exe().is_ok_and(|program| {
            program
                .file_stem()
                .is_some_and(|name| name.eq_ignore_ascii_case(&window.class))
        })
    } else {
        window.class == APP_ID
    }
}

/// The window that has the keyboard, from Hyprland's socket, or from
/// `hyprctl` when the socket is not where expected.
#[cfg(not(windows))]
fn active_window() -> Option<Window> {
    let reply = match ditox_rpc::hypr::request("j/activewindow", HYPRLAND_WAIT) {
        Ok(reply) => reply,
        Err(_) => {
            let output = std::process::Command::new("hyprctl")
                .args(["-j", "activewindow"])
                .output()
                .ok()?;
            output.status.success().then_some(output.stdout)?
        }
    };
    let value: serde_json::Value = serde_json::from_slice(&reply).ok()?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(|field| field.as_str())
            .unwrap_or("")
            .to_string()
    };
    Some(Window {
        address: value.get("address")?.as_str()?.to_string(),
        class: field("class"),
        title: field("title"),
    })
}

fn main() {
    #[cfg(windows)]
    {
        ditox_rpc::win::attach_console();
        ditox_rpc::win::know_scaled_screens();
    }
    startup::prepare();
    rune::set_app_id(APP_ID);
    match std::env::args().nth(1).as_deref() {
        Some("options") => options(),
        _ => clips(),
    }
}

/// The options window.
fn options() {
    if !rune::is_headless() {
        options::start();
    }
    let spec = AppSpec {
        scenes: fixture::OPTIONS_SCENES,
        fonts: theme::FONTS,
        families: theme::FAMILIES,
        min_size: Some(options::MIN_SIZE),
        setup: Some(options::setup),
        poll: Some(options::poll),
        on_key: Some(options::on_key),
        ..AppSpec::new(
            OPTIONS_TITLE,
            options::SIZE,
            options::init,
            options::view::view,
        )
    };
    run(App::new(spec).float_on_open().tab_visits_controls(true));
}

/// The clip list.
fn clips() {
    let placed = place::placed_size(std::env::args().skip(1));
    let min = list::least();
    if !rune::is_headless() {
        let window = active_window();
        // Asked for again while it is open, the list goes away: the key that
        // opens it also puts it back. The options window is this program
        // too: nothing is pasted into it.
        if window.as_ref().is_some_and(ours) {
            if let Some(list) = window.filter(|window| window.title == LIST_TITLE) {
                place::close(&list.address);
                return;
            }
            list::came_from(None);
        } else {
            list::came_from(window.map(|window| window.address));
        }
        // The compositor starts the program again as a picker, where it was
        // left, and that run opens the window.
        if placed.is_none() && place::relaunch(min, (list::SIZE.0 as i32, list::SIZE.1 as i32)) {
            return;
        }
        feed::start();
    }
    // Started on its own, the window still opens as large as it was left.
    let size = placed
        .or_else(|| {
            let left = place::load()?;
            Some((left.width.max(min.0) as f32, left.height.max(min.1) as f32))
        })
        .filter(|_| !rune::is_headless())
        .unwrap_or(list::SIZE);
    // A Windows window says where it opens: where it was left, on the screen
    // the pointer is on.
    #[cfg(windows)]
    let (size, position) = match place::opening(min, (list::SIZE.0 as i32, list::SIZE.1 as i32)) {
        Some((at, size)) if !rune::is_headless() => (size, Some(at)),
        _ => (size, None),
    };
    #[cfg(not(windows))]
    let position = None;
    let spec = AppSpec {
        scenes: fixture::SCENES,
        fonts: theme::FONTS,
        families: theme::FAMILIES,
        min_size: Some(list::MIN_SIZE),
        setup: Some(list::setup),
        poll: Some(list::poll),
        on_key: Some(list::on_key),
        ..AppSpec::new(LIST_TITLE, size, list::init, list::view::view)
    };
    let mut app = App::new(spec)
        .window_options(WindowOptions {
            borderless: true,
            position,
            ..Default::default()
        })
        .float_on_open()
        .on_focus(list::on_focus)
        .on_close(list::on_close);
    if !rune::is_headless() {
        overlay::attach(&mut app);
    }
    run(app);
}

fn run<S: 'static>(app: App<S>) {
    if !rune::is_headless() {
        exit_without_a_window(app.handle());
    }
    if let Err(error) = app.try_run() {
        eprintln!("ditox: {error}");
        std::process::exit(1);
    }
}

/// Rune keeps an app alive when its window cannot be created. A picker with
/// no window is a process nobody can see or close, so it leaves instead.
fn exit_without_a_window<S: 'static>(handle: AppHandle<S>) {
    let _ = std::thread::Builder::new()
        .name("ditox-window".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                if let Some(error) = handle.window_error() {
                    startup::retry_with_every_driver();
                    eprintln!("ditox: the window could not open: {error}");
                    std::process::exit(1);
                }
            }
        });
}
