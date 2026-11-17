//! Where the list is, asked of Windows.
//!
//! The overlay is told its monitor by Windows' own handle for it. A place
//! on it is in that monitor's points, measured from its corner, taskbar
//! included: the corner is zero here, and desktop pixels are turned into
//! points only here.

use rune::{DisplayTarget, NativeDisplayId};

use super::{Look, Rect};
use crate::place::Screen;

/// A window's corner and the size of its content, in desktop pixels.
pub type Frame = (i32, i32, i32, i32);

/// The screen the list is on, as Windows has it: its handle, the whole of
/// it and what the taskbar leaves, in desktop pixels, and the pixels of a
/// point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub handle: isize,
    pub whole: (i32, i32, i32, i32),
    pub work: (i32, i32, i32, i32),
    pub scale: f64,
}

fn points(pixels: i32, scale: f64) -> i32 {
    (f64::from(pixels) / scale).round() as i32
}

/// The look, from the monitor and the list's frame on it, if it has one.
pub fn look_from(monitor: &Monitor, frame: Option<Frame>) -> Look {
    let scale = if monitor.scale > 0.0 {
        monitor.scale
    } else {
        1.0
    };
    let (x, y, width, height) = monitor.whole;
    let (left, top, across, down) = monitor.work;
    let bar = |pixels: i32| points(pixels.max(0), scale);
    Look {
        screen: Screen {
            x: 0,
            y: 0,
            width: points(width, scale),
            height: points(height, scale),
            reserved: [
                bar(left - x),
                bar(top - y),
                bar(x + width - left - across),
                bar(y + height - top - down),
            ],
        },
        scale,
        list: frame.map(|(at_x, at_y, wide, tall)| Rect {
            x: points(at_x - x, scale),
            y: points(at_y - y, scale),
            width: points(wide, scale),
            height: points(tall, scale),
        }),
        display: DisplayTarget::Native(NativeDisplayId::Windows(monitor.handle)),
        origin: (0, 0),
    }
}

/// Asks Windows: the screen the list is on, or the one the pointer is on
/// while the list has no window yet.
#[cfg(windows)]
pub fn look() -> Option<Look> {
    use ditox_rpc::win;

    // The overlay is a window of this program too, and the topmost one.
    let window = win::own_window_titled(crate::LIST_TITLE);
    let screen = window
        .as_ref()
        .and_then(|window| win::screen_of(&window.address))
        .or_else(win::screen_under_pointer)?;
    let monitor = Monitor {
        handle: screen.monitor,
        whole: screen.whole,
        work: (screen.x, screen.y, screen.width, screen.height),
        scale: screen.scale,
    };
    Some(look_from(
        &monitor,
        window.and_then(|window| win::frame(&window.address)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{frame, placement};
    use rune::OverlayPlacement;

    /// A second screen to the right, at 150 %, with the taskbar at the bottom.
    const MONITOR: Monitor = Monitor {
        handle: 0x10043,
        whole: (1920, 0, 2880, 1620),
        work: (1920, 0, 2880, 1548),
        scale: 1.5,
    };

    #[test]
    fn the_screen_is_told_in_its_own_points_with_its_corner_at_zero() {
        let look = look_from(&MONITOR, Some((1920 + 30, 900, 630, 840)));
        assert_eq!(
            look.screen,
            Screen {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                reserved: [0, 0, 0, 48],
            }
        );
        assert_eq!(look.scale, 1.5);
        assert_eq!(
            look.list,
            Some(Rect {
                x: 20,
                y: 600,
                width: 420,
                height: 560,
            })
        );
        assert_eq!(
            look.display,
            DisplayTarget::Native(NativeDisplayId::Windows(0x10043))
        );
        assert_eq!(look.origin, (0, 0));

        let barred = Monitor {
            whole: (0, 0, 1920, 1080),
            work: (60, 40, 1800, 1000),
            scale: 1.0,
            handle: 1,
        };
        assert_eq!(look_from(&barred, None).screen.reserved, [60, 40, 60, 40]);
        assert_eq!(look_from(&barred, None).list, None);
        let unscaled = Monitor {
            scale: 0.0,
            ..barred
        };
        assert_eq!(look_from(&unscaled, None).scale, 1.0);
    }

    #[test]
    fn the_picture_is_placed_on_that_screen_clear_of_the_taskbar_and_the_list() {
        let look = look_from(&MONITOR, Some((1920 + 30, 900, 630, 840)));
        let rect = frame((640, 400), 1.5, &look.screen, look.list);
        assert!(
            rect.y + rect.height <= 1080 - 48,
            "above the taskbar: {rect:?}"
        );
        assert!(rect.x >= 0 && rect.x + rect.width <= 1920);
        let (_, place) = placement((640, 400), &look);
        assert_eq!(
            place,
            OverlayPlacement::Rect {
                x: rect.x as f32,
                y: rect.y as f32,
                width: rect.width as f32,
                height: rect.height as f32,
            },
            "measured from the corner of the monitor"
        );
        // A list in the middle of the screen: the picture goes beside it.
        let centred = look_from(&MONITOR, Some((1920 + 1125, 390, 630, 840)));
        let beside = frame((1600, 1000), 1.5, &centred.screen, centred.list);
        let list = centred.list.unwrap();
        assert!(
            beside.x >= list.x + list.width || beside.x + beside.width <= list.x,
            "{beside:?}"
        );
    }
}
