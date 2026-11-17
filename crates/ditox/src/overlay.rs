//! A picture shown large on the screen, outside the window.
//!
//! The list is narrow: a picture shown whole inside it is small. The picture
//! is drawn instead in an overlay of the toolkit's: a surface of this
//! program's own, over everything, with nothing around it, that takes
//! neither the keyboard nor the pointer. It goes in the middle of the screen
//! the list is on, or beside the list when the middle is where the list is.
//!
//! A window is not told where it is on the screen, so each system has a
//! module that asks (`hyprland`, `windows`): which screen, how large, what
//! its bars keep, and where the list is. Where nothing can be asked, or the
//! overlay cannot be shown, the window shows the picture inside itself.

#[cfg(any(not(windows), test))]
mod hyprland;
#[cfg(any(windows, test))]
mod windows;

use rune::{App, Cx, DisplayTarget, El, OverlayPlacement, Pixels, SurfaceFit, el};

use crate::place::Screen;

/// Points kept clear between the picture and the edges of the screen.
const MARGIN: i32 = 32;
/// Points kept clear between the picture and the list.
const GAP: i32 = 16;
/// The narrowest room beside the list worth moving the picture into.
const LEAST_BESIDE: i32 = 240;
/// How much larger than its own pixels a picture is drawn, at most.
const UPSCALE: f64 = 1.5;
/// The size a picture is taken to have when its clip does not say.
#[cfg_attr(test, allow(dead_code))]
const UNKNOWN: (u32, u32) = (1600, 1000);

/// A place on the desktop, in points, among all the screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x + self.width
    }

    fn bottom(&self) -> i32 {
        self.y + self.height
    }

    fn touches(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

/// What a system says of the screen the picture goes on.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    /// The screen, in points: its corner among the others and its bars.
    pub screen: Screen,
    /// How many pixels a point of it is.
    pub scale: f64,
    /// The list, in the same points, when it is on that screen.
    pub list: Option<Rect>,
    /// The screen as the toolkit is told it.
    pub display: DisplayTarget,
    /// The point the toolkit measures a place on that screen from.
    pub origin: (i32, i32),
}

/// Where the picture goes, as the toolkit is told: which screen, and the
/// place on it measured from where the toolkit measures.
pub fn placement(size: (u32, u32), look: &Look) -> (DisplayTarget, OverlayPlacement) {
    let rect = frame(size, look.scale, &look.screen, look.list);
    (
        look.display.clone(),
        OverlayPlacement::Rect {
            x: (rect.x - look.origin.0) as f32,
            y: (rect.y - look.origin.1) as f32,
            width: rect.width as f32,
            height: rect.height as f32,
        },
    )
}

/// The longest side, in pixels, a picture needs to be sharp on the screen.
pub fn sharp_side(look: &Look) -> u32 {
    let longest = f64::from(look.screen.width.max(look.screen.height).max(1));
    (longest * look.scale.max(1.0)).round() as u32
}

#[cfg(not(windows))]
fn look() -> Option<Look> {
    hyprland::look()
}

#[cfg(windows)]
fn look() -> Option<Look> {
    windows::look()
}

/// Where a picture of `texture` pixels is drawn on `screen`, whose points
/// are `scale` pixels each: in the middle, no larger than the screen leaves
/// and little larger than its own pixels. When the middle is where the list
/// is, it goes beside the list, on the side with more room.
pub fn frame(texture: (u32, u32), scale: f64, screen: &Screen, window: Option<Rect>) -> Rect {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let largest = (
        f64::from(texture.0.max(1)) / scale * UPSCALE,
        f64::from(texture.1.max(1)) / scale * UPSCALE,
    );
    let [left, top, right, bottom] = screen.reserved;
    let mut free = Rect {
        x: screen.x + left + MARGIN,
        y: screen.y + top + MARGIN,
        width: screen.width - left - right - 2 * MARGIN,
        height: screen.height - top - bottom - 2 * MARGIN,
    };
    if free.width < 1 || free.height < 1 {
        free = Rect {
            x: screen.x,
            y: screen.y,
            width: screen.width.max(1),
            height: screen.height.max(1),
        };
    }
    let within = |room: &Rect| {
        let shrink = (f64::from(room.width) / largest.0)
            .min(f64::from(room.height) / largest.1)
            .min(1.0);
        let width = ((largest.0 * shrink).round() as i32).clamp(1, room.width);
        let height = ((largest.1 * shrink).round() as i32).clamp(1, room.height);
        Rect {
            x: room.x + (room.width - width) / 2,
            y: room.y + (room.height - height) / 2,
            width,
            height,
        }
    };
    let middle = within(&free);
    let Some(window) = window else {
        return middle;
    };
    let kept_clear = Rect {
        x: window.x - GAP,
        y: window.y - GAP,
        width: window.width + 2 * GAP,
        height: window.height + 2 * GAP,
    };
    if !middle.touches(&kept_clear) {
        return middle;
    }
    let before = Rect {
        width: kept_clear.x - free.x,
        ..free
    };
    let after = Rect {
        x: kept_clear.right(),
        width: free.right() - kept_clear.right(),
        ..free
    };
    let beside = if after.width >= before.width {
        after
    } else {
        before
    };
    if beside.width >= LEAST_BESIDE {
        within(&beside)
    } else {
        middle
    }
}

/// What the overlay draws.
#[derive(Default)]
pub struct Shown {
    pixels: Option<Pixels>,
}

/// The picture, whole, over nothing: what it leaves clear stays clear.
fn view(shown: &Shown, _: &mut Cx<Shown>) -> El {
    let whole = el().key("picture").w_full().h_full();
    match &shown.pixels {
        Some(pixels) => whole.pixels_fit(pixels, SurfaceFit::Fit),
        None => whole,
    }
}

thread_local! {
    /// The overlay, on the window's thread, once the program has one.
    static HANDLE: std::cell::RefCell<Option<rune::OverlayHandle<Shown>>> =
        const { std::cell::RefCell::new(None) };
}

/// Gives the program its overlay, hidden. Before the window runs.
pub fn attach<S: 'static>(app: &mut App<S>) {
    let options = rune::OverlayOptions {
        placement: Some(OverlayPlacement::Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }),
        input: rune::OverlayInput::None,
        ..Default::default()
    };
    // Hidden, it holds no surface: a picture as large as a screen is tens
    // of megabytes of one.
    let spec = rune::OverlaySpec::new("ditox-picture", (1.0, 1.0), view)
        .options(options)
        .release_gpu_when_hidden();
    let handle = app.overlay(spec, Shown::default());
    HANDLE.with(|slot| *slot.borrow_mut() = Some(handle));
}

/// Whether a picture can go on the screen: the program has its overlay and
/// the system says where the list is. Pictures are then asked for as large
/// as that screen shows them.
pub fn ready() -> bool {
    if !HANDLE.with(|slot| slot.borrow().is_some()) {
        return false;
    }
    match look() {
        Some(look) => {
            crate::picture::set_screen_side(sharp_side(&look));
            true
        }
        None => false,
    }
}

/// Whether the system refused the overlay after it was asked to show.
pub fn lost() -> bool {
    HANDLE.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|handle| handle.availability() == rune::Availability::Unavailable)
    })
}

/// Puts `pixels`, a picture of `size` pixels, on the screen. Says whether
/// it could: a system that no longer says where the list is cannot.
pub fn show(id: i64, pixels: &Pixels, size: Option<(u32, u32)>) -> bool {
    // A test has no screen: it sees what was asked, and answers for it.
    #[cfg(test)]
    {
        let _ = pixels;
        SENT.with(|sent| sent.borrow_mut().push(Call::Show(id, size)));
        !REFUSING.with(|refusing| refusing.get())
    }
    #[cfg(not(test))]
    {
        let _ = id;
        let Some(look) = look() else {
            return false;
        };
        let (display, place) = placement(size.unwrap_or(UNKNOWN), &look);
        HANDLE.with(|slot| {
            let slot = slot.borrow();
            let Some(handle) = slot.as_ref() else {
                return false;
            };
            let pixels = pixels.clone();
            handle.update(move |shown: &mut Shown, tx| {
                tx.instant();
                shown.pixels = Some(pixels);
            });
            handle.set_display(display);
            handle.set_placement(Some(place));
            handle.show();
            true
        })
    }
}

/// Takes the picture off the screen and lets go of its pixels.
pub fn hide() {
    #[cfg(test)]
    SENT.with(|sent| sent.borrow_mut().push(Call::Hide));
    HANDLE.with(|slot| {
        if let Some(handle) = slot.borrow().as_ref() {
            handle.hide();
            handle.update(|shown: &mut Shown, _| shown.pixels = None);
        }
    });
}

/// What the window asked of the screen, as a test sees it.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Show(i64, Option<(u32, u32)>),
    Hide,
}

#[cfg(test)]
thread_local! {
    static SENT: std::cell::RefCell<Vec<Call>> = const { std::cell::RefCell::new(Vec::new()) };
    static REFUSING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// What the window asked of the screen in this test so far.
#[cfg(test)]
pub fn take_sent() -> Vec<Call> {
    SENT.with(|sent| std::mem::take(&mut *sent.borrow_mut()))
}

/// From here on in this test, the screen cannot be asked.
#[cfg(test)]
pub fn refuse(on: bool) {
    REFUSING.with(|refusing| refusing.set(on));
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

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn the_picture_goes_in_the_middle_of_what_the_screen_leaves() {
        // 965 by 543 pixels, half as large again: 1448 by 815 (rounded).
        assert_eq!(
            frame((965, 543), 1.0, &SCREEN, None),
            rect(2560 + (2560 - 1448) / 2, 66 + (1342 - 815) / 2, 1448, 815)
        );
        let small = frame((200, 100), 1.0, &SCREEN, None);
        assert_eq!((small.width, small.height), (300, 150));
        assert_eq!(small.x + small.width / 2, 2560 + 1280);
        let dense = frame((965, 543), 2.0, &SCREEN, None);
        assert_eq!(
            (dense.width, dense.height),
            (724, 407),
            "on a dense screen a pixel takes less room"
        );
        let tall = frame((300, 1024), 1.0, &SCREEN, None);
        assert_eq!(
            (tall.width, tall.height),
            (393, 1342),
            "no taller than the screen leaves, and still in proportion"
        );
        assert_eq!(tall.y, 66);
        let tiny = Screen {
            width: 40,
            height: 30,
            reserved: [0, 0, 0, 0],
            ..SCREEN
        };
        let cramped = frame((965, 543), 1.0, &tiny, None);
        assert!(cramped.width <= 40 && cramped.height <= 30 && cramped.width >= 1);
        assert_eq!(
            frame((965, 543), 0.0, &SCREEN, None),
            frame((965, 543), 1.0, &SCREEN, None)
        );
    }

    #[test]
    fn the_picture_keeps_clear_of_the_list() {
        let middle = frame((965, 543), 1.0, &SCREEN, None);
        let corner = rect(2560, 880, 420, 560);
        assert!(!middle.touches(&corner));
        assert_eq!(
            frame((965, 543), 1.0, &SCREEN, Some(corner)),
            middle,
            "a list in the corner leaves the middle free"
        );

        let centred = rect(2560 + 1070, 440, 420, 560);
        let beside = frame((965, 543), 1.0, &SCREEN, Some(centred));
        assert!(!beside.touches(&centred));
        assert_eq!(
            beside.x,
            centred.right() + GAP,
            "it fills the room beside it"
        );
        assert_eq!(beside.right(), SCREEN.x + SCREEN.width - MARGIN);
        assert_eq!((beside.width, beside.height), (1022, 575));

        let leaning_right = rect(2560 + 1500, 440, 420, 560);
        let other_side = frame((965, 543), 1.0, &SCREEN, Some(leaning_right));
        assert!(other_side.right() <= leaning_right.x - GAP);
        assert!(other_side.x >= SCREEN.x + MARGIN);

        let nearly_all = rect(2560 + 100, 100, 2360, 1200);
        assert_eq!(
            frame((965, 543), 1.0, &SCREEN, Some(nearly_all)),
            middle,
            "with no room beside the list it stays in the middle"
        );
        let touching = rect(middle.x - 420 - GAP + 1, 600, 420, 300);
        assert_ne!(frame((965, 543), 1.0, &SCREEN, Some(touching)), middle);
        let just_clear = rect(middle.x - 420 - GAP, 600, 420, 300);
        assert_eq!(frame((965, 543), 1.0, &SCREEN, Some(just_clear)), middle);
        let just_above = rect(middle.x, middle.y - 200 - GAP, 420, 200);
        assert_eq!(frame((965, 543), 1.0, &SCREEN, Some(just_above)), middle);
        let just_below = rect(middle.x, middle.bottom() + GAP, 420, 200);
        assert_eq!(frame((965, 543), 1.0, &SCREEN, Some(just_below)), middle);
        let over_the_top = rect(middle.x, middle.y - 200 - GAP + 1, 420, 200);
        assert_ne!(frame((965, 543), 1.0, &SCREEN, Some(over_the_top)), middle);
    }

    #[test]
    fn the_toolkit_is_told_the_place_from_where_it_measures() {
        let look = Look {
            screen: SCREEN,
            scale: 1.0,
            list: Some(rect(2560, 880, 420, 560)),
            display: DisplayTarget::Primary,
            origin: (2560, 34),
        };
        let middle = frame((640, 400), 1.0, &SCREEN, look.list);
        assert_eq!(
            placement((640, 400), &look),
            (
                DisplayTarget::Primary,
                OverlayPlacement::Rect {
                    x: (middle.x - 2560) as f32,
                    y: (middle.y - 34) as f32,
                    width: 960.0,
                    height: 600.0,
                }
            )
        );
        let whole = Look {
            origin: (2560, 0),
            ..look.clone()
        };
        let (_, OverlayPlacement::Rect { y, .. }) = placement((640, 400), &whole) else {
            panic!("a place");
        };
        assert_eq!(y, middle.y as f32, "measured from the corner of the screen");

        // A list in the middle of the screen: the place is beside it.
        let centred = Look {
            list: Some(rect(2560 + 1070, 440, 420, 560)),
            ..look
        };
        let (_, OverlayPlacement::Rect { x, .. }) = placement((640, 400), &centred) else {
            panic!("a place");
        };
        assert!(x >= (1070 + 420 + GAP) as f32, "{x}");
    }

    #[test]
    fn a_picture_is_asked_for_as_large_as_the_screen_shows_it() {
        let look = |width, height, scale| Look {
            screen: Screen {
                width,
                height,
                ..SCREEN
            },
            scale,
            list: None,
            display: DisplayTarget::Primary,
            origin: (0, 0),
        };
        assert_eq!(sharp_side(&look(2560, 1440, 1.0)), 2560);
        assert_eq!(sharp_side(&look(1080, 1920, 1.0)), 1920);
        assert_eq!(sharp_side(&look(1920, 1080, 1.5)), 2880);
        assert_eq!(sharp_side(&look(1920, 1080, 0.0)), 1920);
    }
}
