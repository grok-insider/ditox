//! A picture shown large on the screen, outside the window.
//!
//! The list is narrow: a picture shown whole inside it is small. Where
//! Hyprland runs the hypr-overlay plugin, the compositor draws the picture
//! itself, over everything and taking no keyboard or pointer, in the middle
//! of the screen the list is on. The pixels go through shared memory the
//! plugin makes; the plugin says how large they may be.
//!
//! One thread does the asking, so the window never waits for the
//! compositor. Without the plugin, or when it refuses, the window is told
//! and shows the picture inside itself as before.
//!
//! Windows has no compositor to ask: there the program draws the picture in
//! a window of its own that nothing can click or give the keyboard to, and
//! the same worker asks it in the plugin's words (`windows`).

#[cfg(any(windows, test))]
mod windows;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};

use crate::feed;
use crate::picture::Picture;
use crate::place::{self, Screen};

/// The plugin's name for the picture. One at a time.
const SURFACE: &str = "ditox.preview";
#[cfg(not(windows))]
const WAIT: Duration = Duration::from_millis(500);
/// How long a closing window waits for the picture to be taken down.
const FINISH_WAIT: Duration = Duration::from_secs(1);
/// Pictures kept ready to show again, each already as small as the plugin takes.
pub const HELD: usize = 3;
/// Points kept clear between the picture and the edges of the screen.
const MARGIN: i32 = 32;
/// Points kept clear between the picture and the list.
const GAP: i32 = 16;
/// The narrowest room beside the list worth moving the picture into.
const LEAST_BESIDE: i32 = 240;
/// How much larger than its own pixels a picture is drawn, at most.
const UPSCALE: f64 = 1.5;

/// What the plugin takes for one picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most pixels on a side.
    pub side: u32,
    /// The most bytes, four to a pixel.
    pub bytes: u64,
}

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

/// What the plugin's status says it takes, if it draws pictures at all.
pub fn limits(status: &[u8]) -> Option<Limits> {
    let status: Value = serde_json::from_slice(status).ok()?;
    let draws = status
        .get("capabilities")?
        .as_array()?
        .iter()
        .any(|capability| capability.as_str() == Some("surfaces.rgba.shm"));
    let limit = |name: &str| status.get("limits")?.get(name)?.as_u64();
    let limits = Limits {
        side: u32::try_from(limit("surfaceDimension")?).ok()?,
        bytes: limit("surfaceBytesPerSurface")?,
    };
    (draws && limits.side > 0 && limits.bytes >= 4).then_some(limits)
}

/// The size a picture of `width` by `height` is sent at: its own, or made
/// smaller, in proportion, until the plugin takes it.
pub fn texture_size((width, height): (u32, u32), limits: Limits) -> (u32, u32) {
    let (wide, tall) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let side = f64::from(limits.side.max(1));
    let pixels = (limits.bytes / 4).max(1) as f64;
    let shrink = (side / wide)
        .min(side / tall)
        .min((pixels / (wide * tall)).sqrt())
        .min(1.0);
    let fit = |length: f64| ((length * shrink).floor() as u32).clamp(1, limits.side.max(1));
    (fit(wide), fit(tall))
}

/// The picture as the plugin takes it.
pub fn fitted(picture: Picture, limits: Limits) -> Result<Picture, String> {
    let (width, height) = texture_size((picture.width, picture.height), limits);
    if (width, height) == (picture.width, picture.height) {
        return Ok(picture);
    }
    let image = image::RgbaImage::from_raw(picture.width, picture.height, picture.rgba)
        .ok_or("the picture is not as large as it says")?;
    let small = image::imageops::resize(
        &image,
        width,
        height,
        image::imageops::FilterType::CatmullRom,
    );
    Ok(Picture {
        width,
        height,
        rgba: small.into_raw(),
    })
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

/// What the plugin is asked so that it makes room for the picture.
pub fn open_request(size: (u32, u32), rect: Rect) -> String {
    let payload = json!({
        "id": SURFACE,
        "width": size.0,
        "height": size.1,
        "format": "rgba8",
        "buffers": 1,
        "clickThrough": true,
        "rect": { "x": rect.x, "y": rect.y, "w": rect.width, "h": rect.height },
    });
    format!("/hyproverlay surface-open-json {payload}")
}

/// The name of the shared memory the plugin made, from its answer.
pub fn opened(reply: &[u8]) -> Result<String, String> {
    let reply = String::from_utf8_lossy(reply);
    let mut lines = reply.lines();
    if lines.next().map(str::trim) != Some("ok") {
        return Err(reply.trim().to_string());
    }
    let name = lines
        .find_map(|line| line.strip_prefix("shm0="))
        .map(|name| name.trim().trim_start_matches('/'))
        .filter(|name| !name.is_empty() && !name.contains('/') && !name.starts_with('.'))
        .ok_or("the plugin named no shared memory")?;
    Ok(name.to_string())
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// A picture to keep ready, and whether to show it now.
    Load {
        id: i64,
        picture: Picture,
        show: bool,
    },
    /// Show a picture loaded before.
    Show(i64),
    Hide,
    /// The window is closing: take the picture down and say when it is.
    Finish,
}

/// One request to the compositor and its answer.
pub type Ask = Box<dyn Fn(&str) -> io::Result<Vec<u8>> + Send>;

/// The compositor and its shared memory, as the worker reaches them.
pub struct Desk {
    pub ask: Ask,
    /// Where the shared memory the plugin names is found.
    pub shm: PathBuf,
    /// This process, to find its window among the compositor's.
    pub pid: i64,
}

pub struct Worker {
    desk: Desk,
    limits: Option<Limits>,
    held: Vec<(i64, Picture)>,
    shown: Option<i64>,
    up: bool,
    seq: u64,
}

impl Worker {
    pub fn new(desk: Desk) -> Worker {
        Worker {
            desk,
            limits: None,
            held: Vec::new(),
            shown: None,
            up: false,
            seq: 0,
        }
    }

    /// Asks the plugin what it takes. A picture left up by a run that died
    /// is taken down. Says whether pictures can be shown.
    pub fn probe(&mut self) -> bool {
        self.limits = (self.desk.ask)("j/hyproverlay status")
            .ok()
            .and_then(|status| limits(&status));
        if self.limits.is_some() {
            let _ = (self.desk.ask)(&format!("/hyproverlay surface-close {SURFACE}"));
        }
        self.limits.is_some()
    }

    /// Gives up on the plugin: the picture comes down and nothing more is shown.
    pub fn give_up(&mut self) {
        self.hide();
        self.limits = None;
        self.held.clear();
    }

    /// What a batch of commands comes to. Only the last word on what is
    /// shown is acted on: holding an arrow key does not draw every picture.
    pub fn turn(&mut self, commands: impl IntoIterator<Item = Command>) -> Result<(), String> {
        let Some(limits) = self.limits else {
            self.hide();
            return Ok(());
        };
        let mut target = None;
        let mut reloaded = false;
        for command in commands {
            match command {
                Command::Load { id, picture, show } => {
                    let picture = fitted(picture, limits)?;
                    self.held.retain(|(held, _)| *held != id);
                    self.held.push((id, picture));
                    if self.held.len() > HELD {
                        self.held.remove(0);
                    }
                    reloaded |= self.shown == Some(id);
                    if show {
                        target = Some(Some(id));
                    }
                }
                Command::Show(id) => target = Some(Some(id)),
                Command::Hide | Command::Finish => target = Some(None),
            }
        }
        // The picture on show was replaced: it is drawn again.
        let target = target.or(reloaded.then_some(self.shown));
        match target {
            Some(Some(id)) if self.shown != Some(id) || reloaded => self.show(id),
            Some(None) => {
                self.hide();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn show(&mut self, id: i64) -> Result<(), String> {
        let Some(limits) = self.limits else {
            return Err("the plugin is not there".into());
        };
        let (_, picture) = self
            .held
            .iter()
            .find(|(held, _)| *held == id)
            .ok_or("the picture was not kept")?;
        if u64::from(picture.width) * u64::from(picture.height) * 4 > limits.bytes {
            return Err("the picture does not fit what the plugin takes".into());
        }
        let ask = |request: &str| (self.desk.ask)(request).map_err(|err| err.to_string());
        let monitors = ask("j/monitors")?;
        let screens = place::screens(&monitors);
        let window = ask("j/clients")
            .ok()
            .and_then(|clients| place::window_of(&clients, self.desk.pid));
        let screen = window
            .and_then(|window| screens.iter().find(|(id, _, _)| *id == window.monitor))
            .or_else(|| screens.iter().find(|(_, focused, _)| *focused))
            .ok_or("no screen to show the picture on")?;
        let rect = frame(
            (picture.width, picture.height),
            scale_of(&monitors, screen.0),
            &screen.2,
            window.map(|window| Rect {
                x: window.at.0,
                y: window.at.1,
                width: window.size.0,
                height: window.size.1,
            }),
        );
        let name = opened(&ask(&open_request((picture.width, picture.height), rect))?)?;
        self.up = true;
        self.shown = None;
        write_pixels(&self.desk.shm.join(name), &picture.rgba).map_err(|err| err.to_string())?;
        self.seq += 1;
        let presented = ask(&format!(
            "/hyproverlay surface-present {SURFACE} 0 {}",
            self.seq
        ))?;
        if presented.trim_ascii() != b"ok" {
            return Err(String::from_utf8_lossy(&presented).trim().to_string());
        }
        self.shown = Some(id);
        Ok(())
    }

    fn hide(&mut self) {
        if self.up {
            let _ = (self.desk.ask)(&format!("/hyproverlay surface-close {SURFACE}"));
        }
        self.up = false;
        self.shown = None;
    }
}

/// Writes the pixels over the shared memory the plugin made, which is
/// exactly as large as they are.
fn write_pixels(path: &Path, rgba: &[u8]) -> io::Result<()> {
    let mut memory = std::fs::OpenOptions::new().write(true).open(path)?;
    if memory.metadata()?.len() != rgba.len() as u64 {
        return Err(io::Error::other(
            "the shared memory is not as large as the picture",
        ));
    }
    memory.write_all(rgba)
}

struct Bridge {
    commands: Sender<Command>,
    finished: (Mutex<bool>, Condvar),
}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

/// Starts the thread that talks to the plugin. It tells the window, through
/// the feed, whether pictures can be shown on the screen.
pub fn start() {
    let (commands, queue) = mpsc::channel();
    if BRIDGE
        .set(Bridge {
            commands,
            finished: (Mutex::new(false), Condvar::new()),
        })
        .is_ok()
    {
        let _ = std::thread::Builder::new()
            .name("ditox-overlay".into())
            .spawn(move || run(queue, desk()));
    }
}

/// What the plugin is asked for `request`, in order; the answer to the last
/// is the answer. The plugin puts a new picture in the place of the one
/// before without clearing where that one was drawn: what the new one does
/// not cover stayed on the screen, picture over picture. Closing the one
/// before first clears it.
#[cfg_attr(windows, allow(dead_code))]
fn plugin_requests(request: &str) -> Vec<String> {
    let mut requests = Vec::new();
    if request.starts_with("/hyproverlay surface-open-json ") {
        requests.push(format!("/hyproverlay surface-close {SURFACE}"));
    }
    requests.push(request.to_string());
    requests
}

/// Hyprland and the shared memory its plugin makes.
#[cfg(not(windows))]
fn desk() -> Desk {
    Desk {
        ask: Box::new(|request| {
            let mut answer = Ok(Vec::new());
            for request in plugin_requests(request) {
                answer = ditox_rpc::hypr::request(&request, WAIT);
            }
            answer
        }),
        shm: PathBuf::from("/dev/shm"),
        pid: i64::from(std::process::id()),
    }
}

/// This program's own picture window, answering as the plugin does.
#[cfg(windows)]
fn desk() -> Desk {
    windows::desk()
}

fn run(queue: Receiver<Command>, desk: Desk) {
    let mut worker = Worker::new(desk);
    feed::push(feed::Event::Preview(worker.probe()));
    while let Ok(first) = queue.recv() {
        let mut batch = vec![first];
        batch.extend(queue.try_iter());
        let closing = batch.contains(&Command::Finish);
        if worker.turn(batch).is_err() {
            worker.give_up();
            feed::push(feed::Event::Preview(false));
        }
        if closing {
            finished();
        }
    }
}

pub fn send(command: Command) {
    #[cfg(test)]
    SENT.with(|sent| sent.borrow_mut().push(command.clone()));
    if let Some(bridge) = BRIDGE.get() {
        let _ = bridge.commands.send(command);
    }
}

#[cfg(test)]
thread_local! {
    static SENT: std::cell::RefCell<Vec<Command>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// What the window asked to be shown on the screen in this test so far.
#[cfg(test)]
pub fn take_sent() -> Vec<Command> {
    SENT.with(|sent| std::mem::take(&mut *sent.borrow_mut()))
}

/// Called as the window closes. Waits, briefly, until the picture is down:
/// nothing else would ever take it off the screen.
pub fn finish() {
    #[cfg(test)]
    SENT.with(|sent| sent.borrow_mut().push(Command::Finish));
    let Some(bridge) = BRIDGE.get() else { return };
    if bridge.commands.send(Command::Finish).is_err() {
        return;
    }
    let (done, signal) = &bridge.finished;
    let guard = done.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = signal.wait_timeout_while(guard, FINISH_WAIT, |done| !*done);
}

fn finished() {
    if let Some(bridge) = BRIDGE.get() {
        let (done, signal) = &bridge.finished;
        *done.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        signal.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    const PLUGIN: Limits = Limits {
        side: 1024,
        bytes: 2 * 1024 * 1024,
    };

    const SCREEN: Screen = Screen {
        x: 2560,
        y: 0,
        width: 2560,
        height: 1440,
        reserved: [0, 34, 0, 0],
    };

    const STATUS: &str = r#"{"capabilities":["cards.basic","surfaces.rgba.shm"],
        "limits":{"surfaceDimension":1024,"surfaceBytesPerSurface":2097152,"surfaceBytesTotal":8388608},
        "surfaces":{"bytes":0,"count":0}}"#;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn picture(width: u32, height: u32) -> Picture {
        let rgba = (0..width * height)
            .flat_map(|pixel| {
                let (x, y) = (pixel % width, pixel / width);
                [(x % 251) as u8, (y % 241) as u8, 90, 255]
            })
            .collect();
        Picture {
            width,
            height,
            rgba,
        }
    }

    #[test]
    fn the_plugin_says_what_it_takes_or_that_it_draws_no_pictures() {
        assert_eq!(limits(STATUS.as_bytes()), Some(PLUGIN));
        for not in [
            "",
            "not json",
            "{}",
            r#"{"capabilities":["cards.basic"],"limits":{"surfaceDimension":1024,"surfaceBytesPerSurface":2097152}}"#,
            r#"{"capabilities":["surfaces.rgba.shm"],"limits":{"surfaceDimension":1024}}"#,
            r#"{"capabilities":["surfaces.rgba.shm"],"limits":{"surfaceDimension":0,"surfaceBytesPerSurface":2097152}}"#,
            r#"{"capabilities":["surfaces.rgba.shm"],"limits":{"surfaceDimension":1024,"surfaceBytesPerSurface":3}}"#,
            r#"{"capabilities":"surfaces.rgba.shm","limits":{"surfaceDimension":1024,"surfaceBytesPerSurface":2097152}}"#,
            "unknown request",
        ] {
            assert_eq!(limits(not.as_bytes()), None, "{not}");
        }
    }

    #[test]
    fn a_picture_is_sent_at_its_own_size_or_as_large_as_the_plugin_takes() {
        assert_eq!(texture_size((640, 400), PLUGIN), (640, 400));
        assert_eq!(texture_size((724, 724), PLUGIN), (724, 724));
        for (size, sent) in [
            ((1280, 720), (965, 543)),
            ((3840, 2160), (965, 543)),
            ((720, 1280), (543, 965)),
            ((1280, 1280), (724, 724)),
            ((4000, 100), (1024, 25)),
            ((100, 4000), (25, 1024)),
            ((1, 1), (1, 1)),
            ((0, 0), (1, 1)),
            ((1_000_000, 1), (1024, 1)),
        ] {
            let (width, height) = texture_size(size, PLUGIN);
            assert_eq!((width, height), sent, "{size:?}");
            assert!(width <= PLUGIN.side && height <= PLUGIN.side);
            assert!(u64::from(width) * u64::from(height) * 4 <= PLUGIN.bytes);
        }
        let roomy = Limits {
            side: 4096,
            bytes: 64 * 1024 * 1024,
        };
        assert_eq!(texture_size((3840, 2160), roomy), (3840, 2160));
        assert_eq!(texture_size((8192, 4096), roomy), (4096, 2048));
    }

    #[test]
    fn a_picture_made_smaller_keeps_its_look_and_one_that_fits_is_not_touched() {
        let small = picture(320, 200);
        assert_eq!(fitted(small.clone(), PLUGIN), Ok(small));
        let large = fitted(picture(1280, 720), PLUGIN).unwrap();
        assert_eq!((large.width, large.height), (965, 543));
        assert_eq!(large.rgba.len(), 965 * 543 * 4);
        assert!(large.rgba.chunks(4).all(|pixel| pixel[3] == 255));
        assert!(
            large.rgba.chunks(4).all(|pixel| pixel[2].abs_diff(90) <= 2),
            "a flat channel stays flat"
        );
        let lying = Picture {
            width: 2000,
            height: 2000,
            rgba: vec![0; 16],
        };
        assert!(fitted(lying, PLUGIN).is_err());
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
    fn the_plugin_is_asked_for_room_and_names_the_memory() {
        assert_eq!(
            open_request((965, 543), rect(3116, 329, 1448, 815)),
            concat!(
                r#"/hyproverlay surface-open-json {"buffers":1,"clickThrough":true,"format":"rgba8","#,
                r#""height":543,"id":"ditox.preview","rect":{"h":815,"w":1448,"x":3116,"y":329},"width":965}"#
            )
        );
        let reply = "ok\nid=ditox.preview\nwidth=965\nheight=543\nstride=3860\nbuffers=1\n\
                     transport=shm+cpu\nshm0=/hyproverlay-ditox.preview-0\nnote=map shm\n";
        assert_eq!(
            opened(reply.as_bytes()).as_deref(),
            Ok("hyproverlay-ditox.preview-0")
        );
        assert_eq!(
            opened(b"error: too many surfaces\n"),
            Err("error: too many surfaces".into())
        );
        for bad in [
            "ok\nid=x\n",
            "ok\nshm0=\n",
            "ok\nshm0=/a/b\n",
            "ok\nshm0=/../etc\n",
            "",
            "unknown request",
        ] {
            assert!(opened(bad.as_bytes()).is_err(), "{bad:?}");
        }
    }

    /// A compositor that answers from a script and notes what it was asked,
    /// with a directory standing in for its shared memory.
    struct Fake {
        asked: Arc<Mutex<Vec<String>>>,
        dir: PathBuf,
        /// The process the worker takes itself for; 77 has the list's window.
        pid: i64,
        /// Bytes the shared memory is made larger than it was asked for.
        spare: usize,
    }

    const MONITORS: &str = r#"[
      {"id": 0, "x": 0, "y": 0, "width": 2560, "height": 1440, "scale": 1.0, "transform": 0,
       "reserved": [0, 34, 0, 0], "focused": false},
      {"id": 1, "x": 2560, "y": 0, "width": 2560, "height": 1440, "scale": 1.0, "transform": 0,
       "reserved": [0, 34, 0, 0], "focused": false},
      {"id": 2, "x": 5120, "y": 0, "width": 1920, "height": 1080, "scale": 1.0, "transform": 0,
       "reserved": [0, 0, 0, 0], "focused": true}
    ]"#;

    impl Fake {
        fn new(name: &str) -> Fake {
            let dir =
                std::env::temp_dir().join(format!("ditox-overlay-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Fake {
                asked: Arc::new(Mutex::new(Vec::new())),
                dir,
                pid: 77,
                spare: 0,
            }
        }

        /// A worker on this compositor. `refuse` is the start of a request
        /// the compositor answers with an error.
        fn worker(&self, status: &'static str, refuse: Option<&'static str>) -> Worker {
            let (asked, dir, spare) = (self.asked.clone(), self.dir.clone(), self.spare);
            Worker::new(Desk {
                ask: Box::new(move |request| {
                    asked.lock().unwrap().push(request.to_string());
                    if refuse.is_some_and(|refuse| request.starts_with(refuse)) {
                        return Ok(b"error: refused\n".to_vec());
                    }
                    Ok(match request {
                        "j/hyproverlay status" => status.as_bytes().to_vec(),
                        "j/monitors" => MONITORS.as_bytes().to_vec(),
                        "j/clients" => br#"[{"pid": 77, "at": [2560, 914], "size": [420, 526],
                            "monitor": 1, "floating": true, "fullscreen": 0}]"#
                            .to_vec(),
                        open if open.starts_with("/hyproverlay surface-open-json ") => {
                            let payload: Value = serde_json::from_str(
                                open.trim_start_matches("/hyproverlay surface-open-json "),
                            )
                            .unwrap();
                            let bytes = payload["width"].as_u64().unwrap()
                                * payload["height"].as_u64().unwrap()
                                * 4;
                            std::fs::write(dir.join("room"), vec![0; bytes as usize + spare])
                                .unwrap();
                            b"ok\nid=ditox.preview\nshm0=/room\n".to_vec()
                        }
                        _ => b"ok\n".to_vec(),
                    })
                }),
                shm: self.dir.clone(),
                pid: self.pid,
            })
        }

        fn asked(&self) -> Vec<String> {
            std::mem::take(&mut *self.asked.lock().unwrap())
        }

        fn room(&self) -> Vec<u8> {
            std::fs::read(self.dir.join("room")).unwrap_or_default()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn load(id: i64, picture: Picture, show: bool) -> Command {
        Command::Load { id, picture, show }
    }

    #[test]
    fn a_picture_is_opened_written_and_presented_on_the_screen_of_the_list() {
        let desk = Fake::new("show");
        let mut worker = desk.worker(STATUS, None);
        assert!(worker.probe());
        assert_eq!(
            desk.asked(),
            [
                "j/hyproverlay status",
                "/hyproverlay surface-close ditox.preview"
            ],
            "a picture left up by a run that died comes down"
        );

        let shot = picture(640, 400);
        worker.turn([load(4, shot.clone(), true)]).unwrap();
        let asked = desk.asked();
        assert_eq!(asked[..2], ["j/monitors", "j/clients"]);
        let open: Value =
            serde_json::from_str(asked[2].trim_start_matches("/hyproverlay surface-open-json "))
                .unwrap();
        assert_eq!(
            (open["width"].as_u64(), open["height"].as_u64()),
            (Some(640), Some(400))
        );
        // The list is on the second screen, in its corner: the middle of that screen.
        assert_eq!(open["rect"]["w"], 960);
        assert_eq!(open["rect"]["x"], 2560 + (2560 - 960) / 2);
        assert_eq!(asked[3], "/hyproverlay surface-present ditox.preview 0 1");
        assert_eq!(asked.len(), 4);
        assert_eq!(
            desk.room(),
            shot.rgba,
            "the pixels are in the memory before it is presented"
        );

        worker.turn([Command::Show(4)]).unwrap();
        assert!(desk.asked().is_empty(), "what is up is not drawn again");
        worker.turn([Command::Hide]).unwrap();
        assert_eq!(desk.asked(), ["/hyproverlay surface-close ditox.preview"]);
        worker.turn([Command::Hide, Command::Finish]).unwrap();
        assert!(desk.asked().is_empty(), "what is down is not closed again");

        worker.turn([Command::Show(4)]).unwrap();
        let again = desk.asked();
        assert_eq!(
            again.last().unwrap(),
            "/hyproverlay surface-present ditox.preview 0 2"
        );
        assert_eq!(desk.room(), shot.rgba);
    }

    #[test]
    fn only_the_last_word_of_a_batch_is_drawn_and_a_few_pictures_are_kept() {
        let desk = Fake::new("batch");
        let mut worker = desk.worker(STATUS, None);
        worker.probe();
        desk.asked();
        let (one, two) = (picture(64, 40), picture(48, 48));
        worker
            .turn([
                load(1, one.clone(), true),
                load(2, two.clone(), true),
                Command::Show(1),
                Command::Show(2),
            ])
            .unwrap();
        let asked = desk.asked();
        assert_eq!(
            asked
                .iter()
                .filter(|request| request.contains("surface-open-json"))
                .count(),
            1
        );
        assert_eq!(desk.room(), two.rgba);

        worker.turn([load(3, picture(8, 8), false)]).unwrap();
        assert!(desk.asked().is_empty(), "a picture only kept is not drawn");
        assert_eq!(desk.room(), two.rgba);

        worker.turn([load(2, one.clone(), false)]).unwrap();
        assert_eq!(
            desk.room(),
            one.rgba,
            "the picture on show is drawn again when it changes"
        );
        desk.asked();

        worker.turn([Command::Show(1), Command::Hide]).unwrap();
        assert_eq!(desk.asked(), ["/hyproverlay surface-close ditox.preview"]);

        // Four loaded: the first is let go.
        worker.turn([load(4, picture(8, 8), false)]).unwrap();
        assert_eq!(
            worker.turn([Command::Show(1)]),
            Err("the picture was not kept".into())
        );
        worker.turn([Command::Show(3)]).unwrap();
    }

    #[test]
    fn without_the_plugin_nothing_is_asked_and_a_refusal_takes_the_picture_down() {
        let desk = Fake::new("none");
        let mut absent = desk.worker("unknown request", None);
        assert!(!absent.probe() && absent.limits.is_none());
        assert_eq!(desk.asked(), ["j/hyproverlay status"]);
        absent
            .turn([load(1, picture(8, 8), true), Command::Hide])
            .unwrap();
        assert!(desk.asked().is_empty());

        for refused in [
            "/hyproverlay surface-open-json",
            "/hyproverlay surface-present",
        ] {
            let desk = Fake::new("refused");
            let mut worker = desk.worker(STATUS, Some(refused));
            assert!(worker.probe());
            desk.asked();
            assert!(
                worker.turn([load(1, picture(8, 8), true)]).is_err(),
                "{refused}"
            );
            worker.give_up();
            let asked = desk.asked();
            assert_eq!(
                asked.last().map(String::as_str)
                    == Some("/hyproverlay surface-close ditox.preview"),
                refused.ends_with("present"),
                "what was opened is closed: {asked:?}"
            );
            assert!(worker.limits.is_none());
            worker.turn([Command::Show(1)]).unwrap();
            assert!(desk.asked().is_empty(), "nothing more is asked of it");
        }

        let desk = Fake::new("short");
        let mut worker = desk.worker(STATUS, None);
        worker.probe();
        let lying = Picture {
            width: 8,
            height: 8,
            rgba: vec![0; 12],
        };
        assert!(worker.turn([load(1, lying, true)]).is_err());
        assert_eq!(desk.room(), vec![0; 256], "nothing is written over it");

        let mut desk = Fake::new("spare");
        desk.spare = 4;
        let mut worker = desk.worker(STATUS, None);
        worker.probe();
        assert!(
            worker.turn([load(1, picture(8, 8), true)]).is_err(),
            "memory of another size is not this picture's"
        );
        assert_eq!(desk.room(), vec![0; 260]);
    }

    #[test]
    fn the_picture_before_is_closed_before_the_plugin_is_asked_for_the_next() {
        let open = open_request((965, 543), rect(3116, 329, 1448, 815));
        assert_eq!(
            plugin_requests(&open),
            ["/hyproverlay surface-close ditox.preview", open.as_str()]
        );
        for other in [
            "j/hyproverlay status",
            "j/monitors",
            "j/clients",
            "/hyproverlay surface-present ditox.preview 0 3",
            "/hyproverlay surface-close ditox.preview",
        ] {
            assert_eq!(plugin_requests(other), [other]);
        }
    }

    #[test]
    fn with_no_window_of_its_own_the_picture_goes_to_the_screen_with_the_keyboard() {
        let mut desk = Fake::new("focused");
        desk.pid = 5;
        let mut worker = desk.worker(STATUS, None);
        worker.probe();
        desk.asked();
        worker.turn([load(1, picture(640, 400), true)]).unwrap();
        let open: Value = serde_json::from_str(
            desk.asked()[2].trim_start_matches("/hyproverlay surface-open-json "),
        )
        .unwrap();
        assert_eq!(open["rect"]["x"], 5120 + (1920 - 960) / 2);
        assert_eq!(open["rect"]["y"], (1080 - 600) / 2);
    }
}
