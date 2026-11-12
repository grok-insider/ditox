//! Where the list opens and how large: where it was left.
//!
//! A Wayland window cannot place itself, nor keep the keyboard when the
//! pointer moves over another window. On Hyprland the program asks the
//! compositor to start it again with a rule that does both, so the window is
//! drawn where it was from its first frame. When the window closes it asks
//! where it ended up and writes that down, measured from the corner of the
//! screen it was on: the next time it opens there on whichever screen has the
//! keyboard. Elsewhere only the size is kept.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

const WAIT: Duration = Duration::from_millis(500);
/// Said to the program started again, with the size to open at.
pub const PLACED: &str = "--placed=";

/// A window on a screen, in points from the screen's top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A screen, in points: where it is among the others, how large, and what
/// its bars keep for themselves on the left, top, right and bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub reserved: [i32; 4],
}

/// One of Hyprland's windows, as far as placing it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    pub at: (i32, i32),
    pub size: (i32, i32),
    pub monitor: i64,
    /// Floating and not fullscreen: where it is, is where it was put.
    pub free: bool,
}

impl Placement {
    /// The placement made to fit the screen: no smaller than `min`, no larger
    /// than what the bars leave, and wholly inside it.
    pub fn inside(self, screen: &Screen, min: (i32, i32)) -> Placement {
        let [left, top, right, bottom] = screen.reserved;
        let room = (
            (screen.width - left - right).max(1),
            (screen.height - top - bottom).max(1),
        );
        let width = self.width.max(min.0).min(room.0);
        let height = self.height.max(min.1).min(room.1);
        Placement {
            x: self.x.clamp(left, left + room.0 - width),
            y: self.y.clamp(top, top + room.1 - height),
            width,
            height,
        }
    }

    fn to_json(self) -> Value {
        json!({ "x": self.x, "y": self.y, "width": self.width, "height": self.height })
    }

    fn from_json(value: &Value) -> Option<Placement> {
        let field = |name: &str| i32::try_from(value.get(name)?.as_i64()?).ok();
        let placement = Placement {
            x: field("x")?,
            y: field("y")?,
            width: field("width")?,
            height: field("height")?,
        };
        (placement.width > 0 && placement.height > 0).then_some(placement)
    }
}

/// `DITOX_STATE`, else `ditox/list.json` under `XDG_STATE_HOME` or `~/.local/state`.
pub fn state_path() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(path) = env("DITOX_STATE") {
        return Some(PathBuf::from(path));
    }
    let base = match env("XDG_STATE_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env("HOME")?).join(".local/state"),
    };
    Some(base.join("ditox/list.json"))
}

pub fn load_from(path: &Path) -> Option<Placement> {
    let text = std::fs::read_to_string(path).ok()?;
    Placement::from_json(&serde_json::from_str(&text).ok()?)
}

/// Writes the placement beside its file first, then in its place.
pub fn save_to(path: &Path, placement: Placement) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let beside = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&beside, format!("{}\n", placement.to_json()))?;
    std::fs::rename(&beside, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&beside);
    })
}

/// Where the list was left, if it was ever written down.
pub fn load() -> Option<Placement> {
    load_from(&state_path()?)
}

/// The screens Hyprland has, each with its id and whether it has the keyboard.
pub fn screens(monitors: &[u8]) -> Vec<(i64, bool, Screen)> {
    let Ok(Value::Array(monitors)) = serde_json::from_slice::<Value>(monitors) else {
        return Vec::new();
    };
    monitors.iter().filter_map(screen).collect()
}

fn screen(monitor: &Value) -> Option<(i64, bool, Screen)> {
    let whole = |name: &str| monitor.get(name)?.as_f64();
    let scale = whole("scale").filter(|scale| *scale > 0.0).unwrap_or(1.0);
    // Hyprland gives a screen's size in pixels and as it is mounted; its
    // place and its bars are already in points, as the windows are.
    let (mut width, mut height) = (whole("width")? / scale, whole("height")? / scale);
    if monitor
        .get("transform")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        % 2
        == 1
    {
        std::mem::swap(&mut width, &mut height);
    }
    let mut reserved = [0; 4];
    if let Some(Value::Array(sides)) = monitor.get("reserved") {
        for (slot, side) in reserved.iter_mut().zip(sides) {
            *slot = side.as_f64().unwrap_or(0.0).max(0.0).round() as i32;
        }
    }
    Some((
        monitor.get("id")?.as_i64()?,
        monitor
            .get("focused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        Screen {
            x: whole("x")?.round() as i32,
            y: whole("y")?.round() as i32,
            width: width.round() as i32,
            height: height.round() as i32,
            reserved,
        },
    ))
}

/// The window of the process `pid` among Hyprland's.
pub fn window_of(clients: &[u8], pid: i64) -> Option<Seen> {
    let clients: Value = serde_json::from_slice(clients).ok()?;
    let client = clients
        .as_array()?
        .iter()
        .find(|client| client.get("pid").and_then(Value::as_i64) == Some(pid))?;
    let pair = |name: &str| {
        let pair = client.get(name)?.as_array()?;
        Some((
            i32::try_from(pair.first()?.as_i64()?).ok()?,
            i32::try_from(pair.get(1)?.as_i64()?).ok()?,
        ))
    };
    let fullscreen = match client.get("fullscreen") {
        Some(Value::Bool(on)) => *on,
        Some(Value::Number(mode)) => mode.as_i64() != Some(0),
        _ => false,
    };
    Some(Seen {
        at: pair("at")?,
        size: pair("size")?,
        monitor: client.get("monitor")?.as_i64()?,
        free: client
            .get("floating")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && !fullscreen,
    })
}

/// Where a window Hyprland shows is, measured on its own screen and kept
/// inside it. A window that is tiled or fullscreen was not put where it is.
pub fn seen_on_its_screen(
    seen: &Seen,
    screens: &[(i64, bool, Screen)],
    min: (i32, i32),
) -> Option<Placement> {
    let (_, _, screen) = screens.iter().find(|(id, _, _)| *id == seen.monitor)?;
    seen.free.then(|| {
        Placement {
            x: seen.at.0 - screen.x,
            y: seen.at.1 - screen.y,
            width: seen.size.0,
            height: seen.size.1,
        }
        .inside(screen, min)
    })
}

/// Asks Hyprland where this window is and writes it down. Called as the
/// window closes, while it is still on screen.
pub fn remember(min: (i32, i32)) {
    let (Some(path), Ok(clients), Ok(monitors)) = (
        state_path(),
        ditox_rpc::hypr::request("j/clients", WAIT),
        ditox_rpc::hypr::request("j/monitors", WAIT),
    ) else {
        return;
    };
    let placement = window_of(&clients, i64::from(std::process::id()))
        .and_then(|seen| seen_on_its_screen(&seen, &screens(&monitors), min));
    if let Some(placement) = placement
        && load_from(&path) != Some(placement)
    {
        let _ = save_to(&path, placement);
    }
}

/// The size the program was told to open at, when it was started again.
pub fn placed_size(args: impl IntoIterator<Item = String>) -> Option<(f32, f32)> {
    let size = args
        .into_iter()
        .find_map(|arg| arg.strip_prefix(PLACED).map(String::from))?;
    let (width, height) = size.split_once('x')?;
    let (width, height) = (width.parse::<u16>().ok()?, height.parse::<u16>().ok()?);
    (width > 0 && height > 0).then_some((f32::from(width), f32::from(height)))
}

/// What Hyprland is asked to run so the window opens as a picker should:
/// this program again, floating, keeping the keyboard while it is on screen
/// whatever the pointer moves over, and at `place` when it has one.
pub fn relaunch_expression(
    program: &str,
    env: &[(String, String)],
    size: (i32, i32),
    place: Option<(i32, i32)>,
) -> String {
    let mut command = String::new();
    if !env.is_empty() {
        command.push_str("env ");
        for (name, value) in env {
            command.push_str(&format!("{} ", shell_quoted(&format!("{name}={value}"))));
        }
    }
    command.push_str(&format!(
        "{} {PLACED}{}x{}",
        shell_quoted(program),
        size.0,
        size.1
    ));
    let place = match place {
        Some((x, y)) => format!(", move = \"{x} {y}\""),
        None => String::new(),
    };
    format!(
        "hl.dsp.exec_cmd(\"{}\", {{ float = true, stay_focused = true{place} }})",
        lua_string_body(&command)
    )
}

/// What Hyprland is asked so that the window at `address` closes, as its
/// close button would. `None` for an address that is not one.
pub fn close_expression(address: &str) -> Option<String> {
    let digits = address.strip_prefix("0x")?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("hl.dsp.window.close({{ window = \"address:{address}\" }})"))
}

/// Asks Hyprland to close the window at `address`, and says whether it took it.
pub fn close(address: &str) -> bool {
    close_expression(address).is_some_and(|expression| {
        ditox_rpc::hypr::request(&format!("/dispatch {expression}"), WAIT)
            .is_ok_and(|reply| reply == b"ok")
    })
}

fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn lua_string_body(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\0'..='\x1f' | '\x7f' => out.push_str(&format!("\\{:03}", ch as u32)),
            _ => out.push(ch),
        }
    }
    out
}

/// The program to start again. A wrapper that sets up its libraries is
/// started instead of the binary it wraps.
pub fn program(current: &Path) -> PathBuf {
    let wrapper = current
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| {
            let inner = name.strip_prefix('.')?.strip_suffix("-wrapped")?;
            Some(current.with_file_name(inner))
        });
    match wrapper {
        Some(wrapper) if wrapper.is_file() => wrapper,
        _ => current.to_path_buf(),
    }
}

/// On Hyprland: asks the compositor to start the program again as a
/// picker, where it was left if it was left anywhere, and says whether it
/// did. Then this process has nothing more to do.
pub fn relaunch(min: (i32, i32), size: (i32, i32)) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    let Ok(monitors) = ditox_rpc::hypr::request("j/monitors", WAIT) else {
        return false;
    };
    let screens = screens(&monitors);
    let Some((_, _, screen)) = screens.iter().find(|(_, focused, _)| *focused) else {
        return false;
    };
    let left = load().map(|left| left.inside(screen, min));
    // What this run was told through its environment goes with it: the
    // compositor starts the program with its own.
    let env: Vec<(String, String)> = std::env::vars()
        .filter(|(name, _)| name.starts_with("DITOX_") || name == "LD_LIBRARY_PATH")
        .collect();
    let expression = relaunch_expression(
        &program(&current).to_string_lossy(),
        &env,
        left.map_or(size, |left| (left.width, left.height)),
        left.map(|left| (left.x, left.y)),
    );
    // An older Hyprland does not take the expression and says so; the
    // window then opens where the compositor puts it.
    ditox_rpc::hypr::request(&format!("/dispatch {expression}"), WAIT)
        .is_ok_and(|reply| reply == b"ok")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Screen = Screen {
        x: 2560,
        y: 0,
        width: 2560,
        height: 1440,
        reserved: [0, 34, 0, 0],
    };
    const MIN: (i32, i32) = (320, 300);

    fn at(x: i32, y: i32, width: i32, height: i32) -> Placement {
        Placement {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_placement_is_kept_whole_inside_what_the_bars_leave() {
        assert_eq!(
            at(40, 800, 420, 560).inside(&SCREEN, MIN),
            at(40, 800, 420, 560)
        );
        assert_eq!(
            at(-50, 2000, 420, 560).inside(&SCREEN, MIN),
            at(0, 880, 420, 560),
            "off the left and the bottom, it comes back to the corner"
        );
        assert_eq!(
            at(2400, 0, 420, 560).inside(&SCREEN, MIN),
            at(2140, 34, 420, 560),
            "off the right and under the bar, it comes back beside them"
        );
        assert_eq!(
            at(10, 50, 100, 5000).inside(&SCREEN, MIN),
            at(10, 34, 320, 1406),
            "no smaller than the least and no taller than the room"
        );
        let small = Screen {
            width: 300,
            height: 200,
            reserved: [0, 0, 0, 0],
            ..SCREEN
        };
        assert_eq!(
            at(500, 500, 420, 560).inside(&small, MIN),
            at(0, 0, 300, 200),
            "a screen smaller than the least still holds all of it"
        );
        let barred = Screen {
            reserved: [100, 0, 60, 40],
            ..SCREEN
        };
        assert_eq!(
            at(0, 1400, 400, 400).inside(&barred, MIN),
            at(100, 1000, 400, 400)
        );
        assert_eq!(
            at(2500, 0, 400, 400).inside(&barred, MIN),
            at(2100, 0, 400, 400)
        );
    }

    #[test]
    fn the_placement_is_written_and_read_back_and_a_bad_file_is_no_placement() {
        let dir = std::env::temp_dir().join(format!("ditox-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state/ditox/list.json");
        assert_eq!(load_from(&path), None);
        save_to(&path, at(12, 880, 500, 526)).unwrap();
        assert_eq!(load_from(&path), Some(at(12, 880, 500, 526)));
        save_to(&path, at(-3, 0, 1, 1)).unwrap();
        assert_eq!(load_from(&path), Some(at(-3, 0, 1, 1)));
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        for bad in [
            "",
            "{",
            "[]",
            r#"{"x": 1, "y": 2, "width": 3}"#,
            r#"{"x": 1, "y": 2, "width": 0, "height": 5}"#,
            r#"{"x": "1", "y": 2, "width": 3, "height": 5}"#,
            r#"{"x": 99999999999, "y": 2, "width": 3, "height": 5}"#,
        ] {
            std::fs::write(&path, bad).unwrap();
            assert_eq!(load_from(&path), None, "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    const MONITORS: &str = r#"[
      {"id": 0, "name": "DP-1", "x": 0, "y": 0, "width": 2560, "height": 1440, "scale": 1.0,
       "transform": 0, "reserved": [0, 34, 0, 0], "focused": false},
      {"id": 1, "name": "DP-2", "x": 2560, "y": 0, "width": 3840, "height": 2160, "scale": 1.5,
       "transform": 0, "reserved": [0, 34, 0, 0], "focused": true},
      {"id": 2, "name": "DP-3", "x": -1080, "y": 0, "width": 1920, "height": 1080, "scale": 1.0,
       "transform": 1, "reserved": [0, 0, 0, 0], "focused": false},
      {"name": "no id"}
    ]"#;

    #[test]
    fn screens_are_read_in_points_and_as_they_are_mounted() {
        let screens = screens(MONITORS.as_bytes());
        assert_eq!(screens.len(), 3);
        assert_eq!(
            screens[0],
            (
                0,
                false,
                Screen {
                    x: 0,
                    y: 0,
                    width: 2560,
                    height: 1440,
                    reserved: [0, 34, 0, 0]
                }
            )
        );
        assert_eq!(
            screens[1],
            (1, true, SCREEN),
            "a dense screen is as large as its points"
        );
        assert_eq!(
            (screens[2].2.width, screens[2].2.height),
            (1080, 1920),
            "a screen on its side"
        );
        assert!(super::screens(b"not json").is_empty());
        assert!(super::screens(b"{}").is_empty());
    }

    const CLIENTS: &str = r#"[
      {"address": "0x1", "pid": 10, "at": [100, 200], "size": [800, 600], "monitor": 0,
       "floating": false, "fullscreen": 0},
      {"address": "0x2", "pid": 77, "at": [2600, 914], "size": [500, 526], "monitor": 1,
       "floating": true, "fullscreen": 0},
      {"address": "0x3", "pid": 78, "at": [0, 0], "size": [2560, 1440], "monitor": 0,
       "floating": true, "fullscreen": 2},
      {"address": "0x4", "pid": 79, "at": [5200, 1500], "size": [420, 560], "monitor": 1,
       "floating": true, "fullscreen": false},
      {"address": "0x5", "pid": 80, "at": [0, 0], "size": [420, 560], "monitor": 9,
       "floating": true, "fullscreen": 0}
    ]"#;

    #[test]
    fn the_window_is_found_by_its_process_and_measured_on_its_own_screen() {
        let screens = screens(MONITORS.as_bytes());
        let seen = |pid| window_of(CLIENTS.as_bytes(), pid);
        assert_eq!(
            seen(77),
            Some(Seen {
                at: (2600, 914),
                size: (500, 526),
                monitor: 1,
                free: true
            })
        );
        assert_eq!(
            seen_on_its_screen(&seen(77).unwrap(), &screens, MIN),
            Some(at(40, 914, 500, 526))
        );
        assert_eq!(
            seen_on_its_screen(&seen(79).unwrap(), &screens, MIN),
            Some(at(2140, 880, 420, 560)),
            "dragged past the corner, it is remembered in the corner"
        );
        assert_eq!(seen(1), None);
        assert!(!seen(10).unwrap().free && !seen(78).unwrap().free);
        for put_there_by_the_compositor in [10, 78] {
            assert_eq!(
                seen_on_its_screen(&seen(put_there_by_the_compositor).unwrap(), &screens, MIN),
                None
            );
        }
        assert_eq!(
            seen_on_its_screen(&seen(80).unwrap(), &screens, MIN),
            None,
            "a screen that is not there"
        );
        assert_eq!(window_of(b"not json", 77), None);
    }

    #[test]
    fn the_size_to_open_at_is_read_from_the_arguments() {
        let args = |args: &[&str]| placed_size(args.iter().map(|arg| arg.to_string()));
        assert_eq!(args(&["--placed=500x526"]), Some((500.0, 526.0)));
        assert_eq!(
            args(&["--scene", "ready", "--placed=320x300"]),
            Some((320.0, 300.0))
        );
        for none in [
            &["options"][..],
            &["--placed="],
            &["--placed=500"],
            &["--placed=0x10"],
            &["--placed=axb"],
            &["--placed=500x526x2"],
            &["--placed=-5x10"],
            &["--placed=99999x10"],
        ] {
            assert_eq!(args(none), None, "{none:?}");
        }
    }

    #[test]
    fn hyprland_is_asked_to_start_the_program_as_a_picker_at_its_place() {
        assert_eq!(
            relaunch_expression(
                "/nix/store/x-ditox/bin/ditox",
                &[],
                (420, 560),
                Some((0, 880))
            ),
            r#"hl.dsp.exec_cmd("'/nix/store/x-ditox/bin/ditox' --placed=420x560", { float = true, stay_focused = true, move = "0 880" })"#
        );
        assert_eq!(
            relaunch_expression("/nix/store/x-ditox/bin/ditox", &[], (420, 560), None),
            r#"hl.dsp.exec_cmd("'/nix/store/x-ditox/bin/ditox' --placed=420x560", { float = true, stay_focused = true })"#,
            "never left anywhere, it opens where the compositor puts it"
        );
        let env = [
            (
                "DITOX_SOCKET".to_string(),
                "/tmp/it's here/rpc.sock".to_string(),
            ),
            (
                "DITOX_CONFIG".to_string(),
                "/tmp/a \"b\"\n.toml".to_string(),
            ),
        ];
        assert_eq!(
            relaunch_expression("/opt/my apps/ditox", &env, (500, 526), Some((-4, 34))),
            concat!(
                r#"hl.dsp.exec_cmd("env 'DITOX_SOCKET=/tmp/it'\\''s here/rpc.sock' "#,
                r#"'DITOX_CONFIG=/tmp/a \"b\"\010.toml' '/opt/my apps/ditox' --placed=500x526", "#,
                r#"{ float = true, stay_focused = true, move = "-4 34" })"#
            )
        );
    }

    #[test]
    fn only_an_address_is_asked_to_close() {
        assert_eq!(
            close_expression("0x60fd2d95f530").as_deref(),
            Some(r#"hl.dsp.window.close({ window = "address:0x60fd2d95f530" })"#)
        );
        for not in ["", "0x", "60fd", "0xzz", "0x1\" }) hl.dsp.exit() --"] {
            assert_eq!(close_expression(not), None, "{not}");
        }
    }

    #[test]
    fn a_wrapper_is_started_instead_of_the_binary_it_wraps() {
        let dir = std::env::temp_dir().join(format!("ditox-program-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wrapped = dir.join(".ditox-wrapped");
        std::fs::write(&wrapped, "").unwrap();
        assert_eq!(program(&wrapped), wrapped, "there is no wrapper beside it");
        std::fs::write(dir.join("ditox"), "").unwrap();
        assert_eq!(program(&wrapped), dir.join("ditox"));
        assert_eq!(program(&dir.join("ditox")), dir.join("ditox"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
