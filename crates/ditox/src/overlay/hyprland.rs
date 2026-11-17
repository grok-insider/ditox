//! Where the list is, asked of Hyprland.
//!
//! The overlay is given no screen by name: Hyprland then puts it on the
//! screen that has the keyboard, which is where the list opened. It does so
//! again for every picture, because the toolkit makes the surface anew when
//! its place changes. So that is the screen told of each time, and the list
//! is told of only while it is there. A place on it is measured from the
//! corner of what its bars leave.

use std::time::Duration;

use rune::DisplayTarget;
use serde_json::Value;

use super::{Look, Rect};
use crate::place;

#[cfg_attr(test, allow(dead_code))]
const WAIT: Duration = Duration::from_millis(500);

/// The scale of the screen `id`, from Hyprland's list of them.
fn scale_of(monitors: &[u8], id: i64) -> f64 {
    serde_json::from_slice::<Value>(monitors)
        .ok()
        .and_then(|monitors| {
            monitors
                .as_array()?
                .iter()
                .find(|monitor| monitor.get("id").and_then(Value::as_i64) == Some(id))?
                .get("scale")?
                .as_f64()
        })
        .filter(|scale| *scale > 0.0)
        .unwrap_or(1.0)
}

/// The look, from Hyprland's lists of screens and of windows; `pid` is the
/// process whose window the list is.
pub fn look_from(monitors: &[u8], clients: &[u8], pid: i64) -> Option<Look> {
    let screens = place::screens(monitors);
    let (id, _, screen) = screens.iter().find(|(_, focused, _)| *focused)?;
    let list = place::window_of(clients, pid)
        .filter(|window| window.monitor == *id)
        .map(|window| Rect {
            x: window.at.0,
            y: window.at.1,
            width: window.size.0,
            height: window.size.1,
        });
    Some(Look {
        screen: *screen,
        scale: scale_of(monitors, *id),
        list,
        display: DisplayTarget::Primary,
        origin: (screen.x + screen.reserved[0], screen.y + screen.reserved[1]),
    })
}

/// Asks Hyprland. `None` where it does not run or does not answer.
#[cfg_attr(test, allow(dead_code))]
pub fn look() -> Option<Look> {
    let monitors = ditox_rpc::hypr::request("j/monitors", WAIT).ok()?;
    let clients = ditox_rpc::hypr::request("j/clients", WAIT).unwrap_or_default();
    look_from(&monitors, &clients, i64::from(std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place::Screen;

    const MONITORS: &str = r#"[
      {"id": 0, "x": 0, "y": 0, "width": 2560, "height": 1440, "scale": 1.0, "transform": 0,
       "reserved": [0, 34, 0, 0], "focused": false},
      {"id": 1, "x": 2560, "y": 0, "width": 3840, "height": 2160, "scale": 1.5, "transform": 0,
       "reserved": [0, 34, 0, 0], "focused": true}
    ]"#;

    fn clients(monitor: i64) -> String {
        format!(
            r#"[{{"pid": 5, "at": [10, 10], "size": [800, 600], "monitor": 0,
                 "floating": false, "fullscreen": 0}},
                {{"pid": 77, "at": [2560, 914], "size": [420, 526], "monitor": {monitor},
                 "floating": true, "fullscreen": 0}}]"#
        )
    }

    #[test]
    fn the_picture_goes_on_the_screen_with_the_keyboard_measured_from_under_its_bar() {
        let look = look_from(MONITORS.as_bytes(), clients(1).as_bytes(), 77).unwrap();
        assert_eq!(
            look,
            Look {
                screen: Screen {
                    x: 2560,
                    y: 0,
                    width: 2560,
                    height: 1440,
                    reserved: [0, 34, 0, 0],
                },
                scale: 1.5,
                list: Some(Rect {
                    x: 2560,
                    y: 914,
                    width: 420,
                    height: 526,
                }),
                display: DisplayTarget::Primary,
                origin: (2560, 34),
            }
        );
    }

    #[test]
    fn a_list_on_another_screen_or_not_there_is_not_kept_clear_of() {
        let elsewhere = look_from(MONITORS.as_bytes(), clients(0).as_bytes(), 77).unwrap();
        assert_eq!(elsewhere.list, None);
        assert_eq!(
            elsewhere.screen.x, 2560,
            "still the screen with the keyboard"
        );
        let unknown = look_from(MONITORS.as_bytes(), clients(1).as_bytes(), 9).unwrap();
        assert_eq!(unknown.list, None);
        let no_windows = look_from(MONITORS.as_bytes(), b"", 77).unwrap();
        assert_eq!(no_windows.list, None);
    }

    #[test]
    fn with_no_screen_that_has_the_keyboard_there_is_no_look() {
        assert_eq!(look_from(b"[]", b"[]", 77), None);
        assert_eq!(look_from(b"not json", b"[]", 77), None);
        let none = MONITORS.replace("\"focused\": true", "\"focused\": false");
        assert_eq!(look_from(none.as_bytes(), b"[]", 77), None);
    }
}
