//! The picture on the screen, on Windows.
//!
//! No compositor draws it there, so the program does: a window of its own
//! over everything else, which takes neither the keyboard nor the pointer.
//! The worker does not know the difference. It asks this desk what it asks
//! Hyprland and its plugin, in the same words, and writes the pixels to the
//! file the desk names where the plugin names shared memory. Everything the
//! worker decides (which picture, how large, where, clear of the list) is
//! therefore decided once, for both systems.
//!
//! The desk tells of one screen, the one the list is on, with its corner at
//! zero: places are in that screen's points going out and coming back, and
//! turned into desktop pixels only here.

use serde_json::{Value, json};

/// What the window takes for one picture: a side no screen is wider than,
/// and the bytes of a square of that side.
const SIDE: u32 = 4096;
const BYTES: u64 = 4096 * 4096 * 4;
/// The file the pixels are written to, in the desk's own directory.
#[cfg(windows)]
const ROOM: &str = "room";

/// A window's corner and the size of its content, in desktop pixels.
pub type Frame = (i32, i32, i32, i32);

/// The screen the list is on, as Windows has it: the whole of it and what
/// the taskbar leaves, in desktop pixels, and the pixels of a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub whole: (i32, i32, i32, i32),
    pub work: (i32, i32, i32, i32),
    pub scale: f64,
}

/// A picture the worker asked room for: its pixels, and where on the
/// desktop, in pixels, it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Room {
    pub texture: (u32, u32),
    pub rect: (i32, i32, i32, i32),
}

/// What the plugin's status would say of this window.
pub fn status() -> String {
    json!({
        "capabilities": ["surfaces.rgba.shm"],
        "limits": { "surfaceDimension": SIDE, "surfaceBytesPerSurface": BYTES },
    })
    .to_string()
}

fn points(pixels: i32, scale: f64) -> i32 {
    (f64::from(pixels) / scale).round() as i32
}

fn pixels(points: i32, scale: f64) -> i32 {
    (f64::from(points) * scale).round() as i32
}

/// The screen as Hyprland lists one: its size in pixels, its bars in
/// points, its corner at zero.
pub fn monitors(monitor: &Monitor) -> String {
    let (x, y, width, height) = monitor.whole;
    let (left, top, room_across, room_down) = monitor.work;
    let bar = |pixels: i32| points(pixels.max(0), monitor.scale);
    json!([{
        "id": 0,
        "x": 0,
        "y": 0,
        "width": width,
        "height": height,
        "scale": monitor.scale,
        "transform": 0,
        "reserved": [
            bar(left - x),
            bar(top - y),
            bar(x + width - left - room_across),
            bar(y + height - top - room_down),
        ],
        "focused": true,
    }])
    .to_string()
}

/// The list's window as Hyprland lists a client: in the screen's points.
/// `frame` is its corner and the size of its content, in desktop pixels.
pub fn clients(pid: i64, frame: Option<Frame>, monitor: &Monitor) -> String {
    let Some((x, y, width, height)) = frame else {
        return "[]".into();
    };
    let at = |pixels: i32, corner: i32| points(pixels - corner, monitor.scale);
    json!([{
        "pid": pid,
        "at": [at(x, monitor.whole.0), at(y, monitor.whole.1)],
        "size": [points(width, monitor.scale), points(height, monitor.scale)],
        "monitor": 0,
        "floating": true,
        "fullscreen": 0,
    }])
    .to_string()
}

/// The room a `surface-open-json` request asks for, on `monitor`.
pub fn room(payload: &str, monitor: &Monitor) -> Option<Room> {
    let payload: Value = serde_json::from_str(payload).ok()?;
    let whole = |value: &Value| i32::try_from(value.as_i64()?).ok();
    let side = |name: &str| u32::try_from(payload.get(name)?.as_u64()?).ok();
    let rect = payload.get("rect")?;
    let texture = (side("width")?, side("height")?);
    let (x, y, width, height) = (
        whole(rect.get("x")?)?,
        whole(rect.get("y")?)?,
        whole(rect.get("w")?)?,
        whole(rect.get("h")?)?,
    );
    let fits = (1..=SIDE).contains(&texture.0)
        && (1..=SIDE).contains(&texture.1)
        && width > 0
        && height > 0;
    fits.then(|| Room {
        texture,
        rect: (
            monitor.whole.0 + pixels(x, monitor.scale),
            monitor.whole.1 + pixels(y, monitor.scale),
            pixels(width, monitor.scale).max(1),
            pixels(height, monitor.scale).max(1),
        ),
    })
}

/// The picture as a layered window takes it: as large as it is drawn, blue
/// first, each colour already multiplied by how solid the pixel is.
pub fn layer(texture: (u32, u32), rgba: Vec<u8>, size: (u32, u32)) -> Option<Vec<u8>> {
    let picture = image::RgbaImage::from_raw(texture.0, texture.1, rgba)?;
    let mut drawn = if texture == size {
        picture
    } else {
        image::imageops::resize(
            &picture,
            size.0,
            size.1,
            image::imageops::FilterType::Triangle,
        )
    }
    .into_raw();
    for pixel in drawn.as_chunks_mut::<4>().0 {
        let solid = u32::from(pixel[3]);
        let through = |colour: u8| ((u32::from(colour) * solid + 127) / 255) as u8;
        *pixel = [
            through(pixel[2]),
            through(pixel[1]),
            through(pixel[0]),
            pixel[3],
        ];
    }
    Some(drawn)
}

#[cfg(windows)]
pub use desk::desk;

#[cfg(windows)]
mod desk {
    use std::io;
    use std::path::PathBuf;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use ditox_rpc::win;
    use windows_sys::Win32::Foundation::{HWND, POINT, SIZE};
    use windows_sys::Win32::Graphics::Gdi::{
        AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BLENDFUNCTION, CreateCompatibleDC,
        CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_TOPMOST, MSG,
        PostThreadMessageW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WM_APP, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    use super::{Frame, Monitor, ROOM, Room, clients, layer, monitors, room, status};
    use crate::overlay::Desk;

    const SHOW: u32 = WM_APP + 1;
    const HIDE: u32 = WM_APP + 2;
    const QUIT: u32 = WM_APP + 3;
    /// How long the window's thread may take over one picture.
    const WAIT: Duration = Duration::from_secs(2);

    /// A picture ready to be drawn: where, how large, and its pixels as
    /// `layer` makes them.
    struct Drawing {
        rect: (i32, i32, i32, i32),
        pixels: Vec<u8>,
    }

    /// The thread that owns the picture's window, started with the first
    /// picture. Windows sends a window's messages to the thread that made
    /// it, so that thread does nothing else.
    struct Screen {
        thread: u32,
        frame: Arc<Mutex<Option<Drawing>>>,
        done: Receiver<bool>,
    }

    impl Screen {
        fn start() -> Option<Screen> {
            let frame = Arc::new(Mutex::new(None));
            let (ready, started) = mpsc::channel();
            let (answer, done) = mpsc::channel();
            let shared = frame.clone();
            std::thread::Builder::new()
                .name("ditox-picture".into())
                .spawn(move || serve(shared, ready, answer))
                .ok()?;
            let thread = started.recv_timeout(WAIT).ok()??;
            Some(Screen {
                thread,
                frame,
                done,
            })
        }

        fn ask(&self, message: u32) -> bool {
            // SAFETY: posts a message to a thread id; a thread that is gone
            // makes the call fail and nothing else.
            (unsafe { PostThreadMessageW(self.thread, message, 0, 0) } != 0)
                && self.done.recv_timeout(WAIT).unwrap_or(false)
        }

        fn show(&self, frame: Drawing) -> bool {
            *self.frame.lock().unwrap_or_else(|held| held.into_inner()) = Some(frame);
            self.ask(SHOW)
        }
    }

    impl Drop for Screen {
        fn drop(&mut self) {
            // SAFETY: as in `ask`.
            unsafe { PostThreadMessageW(self.thread, QUIT, 0, 0) };
        }
    }

    fn serve(frame: Arc<Mutex<Option<Drawing>>>, ready: Sender<Option<u32>>, done: Sender<bool>) {
        let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
        // SAFETY: a window of a built-in class with no parent, name, menu,
        // module or creation data; `class` outlives the call. Layered, so
        // its pixels are the ones it is handed; transparent to the pointer;
        // never given the keyboard; kept off the taskbar; over everything.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if window.is_null() {
            let _ = ready.send(None);
            return;
        }
        // SAFETY: no arguments.
        let _ = ready.send(Some(unsafe { GetCurrentThreadId() }));
        // SAFETY: `message` is plain data `GetMessageW` fills, on the
        // thread that owns `window`.
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                match message.message {
                    SHOW => {
                        let frame = frame.lock().unwrap_or_else(|held| held.into_inner()).take();
                        let _ = done.send(frame.is_some_and(|frame| draw(window, &frame)));
                    }
                    HIDE => {
                        ShowWindow(window, SW_HIDE);
                        let _ = done.send(true);
                    }
                    QUIT => break,
                    _ => {
                        DispatchMessageW(&message);
                    }
                }
            }
            DestroyWindow(window);
        }
    }

    /// Hands the window its pixels and place, and shows it without giving
    /// it the keyboard.
    fn draw(window: HWND, frame: &Drawing) -> bool {
        let (x, y, width, height) = frame.rect;
        if frame.pixels.len() != width as usize * height as usize * 4 {
            return false;
        }
        // SAFETY: every handle made here is released before returning. The
        // bitmap is `width` by `height` at four bytes a pixel, top row
        // first, and exactly `frame.pixels` is copied into it.
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let memory = CreateCompatibleDC(screen);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of_val(&info.bmiHeader) as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                memory,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            let mut drawn = false;
            if !bitmap.is_null() && !bits.is_null() {
                std::ptr::copy_nonoverlapping(
                    frame.pixels.as_ptr(),
                    bits.cast::<u8>(),
                    frame.pixels.len(),
                );
                let before = SelectObject(memory, bitmap);
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                drawn = UpdateLayeredWindow(
                    window,
                    screen,
                    &POINT { x, y },
                    &SIZE {
                        cx: width,
                        cy: height,
                    },
                    memory,
                    &POINT { x: 0, y: 0 },
                    0,
                    &blend,
                    ULW_ALPHA,
                ) != 0;
                SelectObject(memory, before);
            }
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(memory);
            ReleaseDC(std::ptr::null_mut(), screen);
            if drawn {
                ShowWindow(window, SW_SHOWNOACTIVATE);
                SetWindowPos(
                    window,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            drawn
        }
    }

    /// What the desk remembers between two requests of the worker.
    struct State {
        dir: PathBuf,
        pid: i64,
        monitor: Option<Monitor>,
        room: Option<Room>,
        screen: Option<Screen>,
    }

    impl State {
        /// The screen the list is on, or the one the pointer is on while
        /// the list has no window yet.
        fn look(&mut self) -> Option<(Monitor, Option<Frame>)> {
            let window = win::own_window();
            let screen = window
                .as_ref()
                .and_then(|window| win::screen_of(&window.address))
                .or_else(win::screen_under_pointer)?;
            let monitor = Monitor {
                whole: screen.whole,
                work: (screen.x, screen.y, screen.width, screen.height),
                scale: screen.scale,
            };
            self.monitor = Some(monitor);
            Some((
                monitor,
                window.and_then(|window| win::frame(&window.address)),
            ))
        }

        fn answer(&mut self, request: &str) -> io::Result<Vec<u8>> {
            const OPEN: &str = "/hyproverlay surface-open-json ";
            let refused = |why: &str| Ok(format!("error: {why}\n").into_bytes());
            if request == "j/hyproverlay status" {
                return Ok(status().into_bytes());
            }
            if request == "j/monitors" {
                return Ok(self
                    .look()
                    .map_or_else(|| "[]".into(), |(monitor, _)| monitors(&monitor))
                    .into_bytes());
            }
            if request == "j/clients" {
                return Ok(self
                    .look()
                    .map_or_else(
                        || "[]".into(),
                        |(monitor, frame)| clients(self.pid, frame, &monitor),
                    )
                    .into_bytes());
            }
            if let Some(payload) = request.strip_prefix(OPEN) {
                let Some(asked) = self.monitor.and_then(|monitor| room(payload, &monitor)) else {
                    return refused("no room for that picture");
                };
                std::fs::create_dir_all(&self.dir)?;
                let bytes = u64::from(asked.texture.0) * u64::from(asked.texture.1) * 4;
                std::fs::File::create(self.dir.join(ROOM))?.set_len(bytes)?;
                self.room = Some(asked);
                return Ok(format!("ok\nshm0=/{ROOM}\n").into_bytes());
            }
            if request.starts_with("/hyproverlay surface-present ") {
                let Some(asked) = self.room else {
                    return refused("nothing was opened");
                };
                let rgba = std::fs::read(self.dir.join(ROOM))?;
                let size = (asked.rect.2 as u32, asked.rect.3 as u32);
                let Some(pixels) = layer(asked.texture, rgba, size) else {
                    return refused("the picture is not as large as it says");
                };
                if self.screen.is_none() {
                    self.screen = Screen::start();
                }
                let frame = Drawing {
                    rect: asked.rect,
                    pixels,
                };
                return match &self.screen {
                    Some(screen) if screen.show(frame) => Ok(b"ok\n".to_vec()),
                    _ => {
                        self.screen = None;
                        refused("Windows did not draw the picture")
                    }
                };
            }
            if request.starts_with("/hyproverlay surface-close ") {
                if let Some(screen) = &self.screen {
                    screen.ask(HIDE);
                }
                self.room = None;
                let _ = std::fs::remove_file(self.dir.join(ROOM));
                let _ = std::fs::remove_dir(&self.dir);
                return Ok(b"ok\n".to_vec());
            }
            Ok(b"unknown request\n".to_vec())
        }
    }

    /// The desk the worker asks on Windows: this program itself.
    pub fn desk() -> Desk {
        let pid = i64::from(std::process::id());
        let dir = std::env::temp_dir().join(format!("ditox-picture-{pid}"));
        let state = Mutex::new(State {
            dir: dir.clone(),
            pid,
            monitor: None,
            room: None,
            screen: None,
        });
        Desk {
            ask: Box::new(move |request| {
                state
                    .lock()
                    .unwrap_or_else(|held| held.into_inner())
                    .answer(request)
            }),
            shm: dir,
            pid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{Rect, frame, limits, open_request};
    use crate::place;

    /// A second screen to the right, at 150 %, with the taskbar at the bottom.
    const MONITOR: Monitor = Monitor {
        whole: (1920, 0, 2880, 1620),
        work: (1920, 0, 2880, 1548),
        scale: 1.5,
    };

    #[test]
    fn the_desk_answers_in_the_plugin_s_words() {
        let limits = limits(status().as_bytes()).unwrap();
        assert_eq!((limits.side, limits.bytes), (SIDE, BYTES));

        let screens = place::screens(monitors(&MONITOR).as_bytes());
        assert_eq!(
            screens,
            [(
                0,
                true,
                place::Screen {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                    reserved: [0, 0, 0, 48],
                }
            )],
            "points, with what the taskbar keeps"
        );

        // The list at 300, 150 pixels into its screen, 630 by 780 pixels.
        let list = clients(77, Some((1920 + 300, 150, 630, 780)), &MONITOR);
        let seen = place::window_of(list.as_bytes(), 77).unwrap();
        assert_eq!((seen.at, seen.size), ((200, 100), (420, 520)));
        assert_eq!(seen.monitor, 0);
        assert!(seen.free);
        assert_eq!(
            place::window_of(clients(77, None, &MONITOR).as_bytes(), 77),
            None
        );
    }

    #[test]
    fn the_room_asked_for_in_points_is_given_in_desktop_pixels() {
        let screen = place::screens(monitors(&MONITOR).as_bytes())[0].2;
        let list = Rect {
            x: 200,
            y: 100,
            width: 420,
            height: 520,
        };
        let rect = frame((600, 400), MONITOR.scale, &screen, Some(list));
        let request = open_request((600, 400), rect);
        let payload = request
            .strip_prefix("/hyproverlay surface-open-json ")
            .unwrap();
        let asked = room(payload, &MONITOR).unwrap();
        assert_eq!(asked.texture, (600, 400));
        assert_eq!(
            asked.rect,
            (
                1920 + (rect.x as f64 * 1.5).round() as i32,
                (rect.y as f64 * 1.5).round() as i32,
                (rect.width as f64 * 1.5).round() as i32,
                (rect.height as f64 * 1.5).round() as i32,
            )
        );
        assert!(
            asked.rect.0 >= 1920 + 300 + 630,
            "beside the list, not over it"
        );
        assert!(asked.rect.1 + asked.rect.3 <= 1548, "above the taskbar");

        for bad in [
            "not json",
            r#"{"width":600,"height":400}"#,
            r#"{"width":0,"height":400,"rect":{"x":0,"y":0,"w":10,"h":10}}"#,
            r#"{"width":9000,"height":400,"rect":{"x":0,"y":0,"w":10,"h":10}}"#,
            r#"{"width":600,"height":400,"rect":{"x":0,"y":0,"w":0,"h":10}}"#,
        ] {
            assert_eq!(room(bad, &MONITOR), None, "{bad}");
        }
    }

    #[test]
    fn a_picture_is_drawn_blue_first_and_dimmed_by_how_solid_it_is() {
        let rgba = vec![
            255, 128, 0, 255, // solid orange
            255, 128, 0, 128, // half of it
            10, 20, 30, 0, // nothing at all
            0, 0, 0, 255, // black
        ];
        assert_eq!(
            layer((2, 2), rgba.clone(), (2, 2)).unwrap(),
            [0, 128, 255, 255, 0, 64, 128, 128, 0, 0, 0, 0, 0, 0, 0, 255]
        );
        let larger = layer((2, 2), rgba, (4, 6)).unwrap();
        assert_eq!(larger.len(), 4 * 6 * 4);
        assert_eq!(layer((2, 2), vec![0; 4], (2, 2)), None, "too few pixels");
    }
}
