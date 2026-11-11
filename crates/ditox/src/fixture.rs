use ditox_rpc::{Entry, Filter, Kind, WatcherStatus};
use rune::Pixels;

use crate::State;
use crate::model::Link;

pub const NOW_MS: i64 = 1_800_000_000_000;

pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("ready", |state| *state = ready()),
    ("light", |state| {
        *state = State {
            dark: Some(false),
            ..ready()
        }
    }),
    ("image", |state| *state = image()),
    ("image-light", |state| {
        *state = State {
            dark: Some(false),
            ..image()
        }
    }),
    ("small-image", |state| *state = small_image()),
    ("odd", |state| *state = odd(0)),
    ("odd-url", |state| *state = odd(1)),
    ("odd-rtl", |state| *state = odd(2)),
    ("odd-blank", |state| *state = odd(4)),
    ("odd-app", |state| *state = odd(5)),
    ("search", |state| *state = search()),
    ("link", |state| *state = chosen(5)),
    ("color", |state| *state = chosen(8)),
    ("keys", |state| {
        *state = State {
            keys_open: true,
            ..ready()
        }
    }),
    ("keys-light", |state| {
        *state = State {
            keys_open: true,
            dark: Some(false),
            ..ready()
        }
    }),
    ("undo", |state| *state = undo()),
    ("no-target", |state| {
        *state = State {
            target: None,
            ..ready()
        }
    }),
    ("long", |state| *state = long()),
    ("no-results", |state| *state = no_results()),
    ("empty", |state| {
        *state = State {
            clips: Vec::new(),
            ..ready()
        }
    }),
    ("offline", |state| *state = offline()),
    ("paused", |state| *state = paused()),
    ("error", |state| {
        *state = State {
            note: Some("This clip has no plain text to paste.".into()),
            ..ready()
        }
    }),
];

const SNIPPET: &str = "pub fn capture(&mut self) {\n    match self.capture_current_offer() {\n        Ok(result) => {\n            let _ = self.store.clear_watcher_error();\n            if result.is_some_and(|result| result.stored) {\n                self.touch();\n            }\n        }\n        Err(err) => {\n            let _ = self.store.set_watcher_error(&err.to_string());\n        }\n    }\n}";

fn text(id: i64, content: &str, app: &str, minutes_ago: i64) -> Entry {
    Entry {
        id,
        kind: Kind::Text,
        mime: "text/plain;charset=utf-8".into(),
        content: content.into(),
        preview: content.chars().take(160).collect(),
        hash: String::new(),
        favorite: false,
        created_at_ms: NOW_MS - minutes_ago * 60_000,
        last_used_at_ms: None,
        byte_len: content.len() as i64,
        source_app: Some(app.into()).filter(|app: &String| !app.is_empty()),
        blob_path: None,
        image_width: None,
        image_height: None,
        sticky: false,
    }
}

fn picture(id: i64, minutes_ago: i64) -> Entry {
    Entry {
        kind: Kind::Image,
        mime: "image/png".into(),
        preview: "Image image/png 184320 bytes 960x600".into(),
        byte_len: 184_320,
        image_width: Some(960),
        image_height: Some(600),
        ..text(id, "", "firefox", minutes_ago)
    }
}

pub fn clips() -> Vec<Entry> {
    let mut pinned = vec![
        text(
            1,
            "ssh friend@station.example -p 2222",
            "ghostty",
            3 * 24 * 60,
        ),
        text(
            2,
            "Thanks, I will look at it today and get back to you.",
            "thunderbird",
            9 * 24 * 60,
        ),
    ];
    for clip in &mut pinned {
        clip.favorite = true;
    }
    let recent = vec![
        text(3, SNIPPET, "zed", 0),
        picture(4, 2),
        text(5, "https://ui.fabrials.com/docs/design", "firefox", 6),
        text(6, "cargo test --workspace", "ghostty", 14),
        text(
            7,
            "The daemon owns the clipboard; the window only asks.",
            "obsidian",
            41,
        ),
        text(8, "#8fb8f5", "zed", 95),
        text(9, "nix build .#default -L", "ghostty", 3 * 60),
        text(
            10,
            "SELECT id, kind, preview FROM entries ORDER BY id DESC LIMIT 20;",
            "ghostty",
            5 * 60,
        ),
        text(11, "Calle de la Tormenta 12, 28004 Madrid", "", 26 * 60),
        text(
            12,
            "\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}",
            "telegram",
            2 * 24 * 60,
        ),
    ];
    pinned.into_iter().chain(recent).collect()
}

fn watching() -> WatcherStatus {
    WatcherStatus {
        running: true,
        paused: false,
        backend: "wl-clipboard".into(),
        last_error: None,
    }
}

pub fn ready() -> State {
    State {
        clips: clips(),
        selected: 2,
        link: Link::Ready,
        watcher: Some(watching()),
        target: Some("0xabc".into()),
        now_ms: NOW_MS,
        day_start_ms: Some(NOW_MS - 10 * 3_600_000),
        dark: Some(true),
        ..State::blank()
    }
}

fn image() -> State {
    let mut state = ready();
    state.selected = 3;
    state.images.insert(4, storm(960, 600));
    state
}

fn small_image() -> State {
    let mut state = image();
    state.clips[3].image_width = Some(96);
    state.clips[3].image_height = Some(60);
    state.images.insert(4, storm(96, 60));
    state
}

/// Content that tends to break a layout: no spaces, tabs, long lines, other
/// scripts, emoji, nothing but whitespace.
fn odd(selected: usize) -> State {
    let mixed = "\tfn main() {\n\t\tlet greeting = \"\u{4f60}\u{597d}\u{ff0c}\u{4e16}\u{754c} \u{1f44b} caf\u{e9} na\u{ef}ve\";\n\t\tprintln!(\"{greeting}\");\n\t}\n\n\n    trailing spaces   \n\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30c6}\u{30ad}\u{30b9}\u{30c8}\u{3082}\u{6298}\u{308a}\u{8fd4}\u{3055}\u{308c}\u{307e}\u{3059}\u{3002}\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30c6}\u{30ad}\u{30b9}\u{30c8}\u{3082}\u{6298}\u{308a}\u{8fd4}\u{3055}\u{308c}\u{307e}\u{3059}\u{3002}";
    let url = format!(
        "https://example.com/{}?token={}",
        "very/long/path/".repeat(12),
        "a1b2c3d4".repeat(20)
    );
    let rtl = "\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}\u{60c} \u{647}\u{630}\u{627} \u{646}\u{635} \u{639}\u{631}\u{628}\u{64a} \u{637}\u{648}\u{64a}\u{644} \u{64a}\u{62c}\u{628} \u{623}\u{646} \u{64a}\u{644}\u{62a}\u{641} \u{639}\u{644}\u{649} \u{623}\u{643}\u{62b}\u{631} \u{645}\u{646} \u{633}\u{637}\u{631} \u{648}\u{627}\u{62d}\u{62f} \u{641}\u{64a} \u{627}\u{644}\u{645}\u{639}\u{627}\u{64a}\u{646}\u{629}.\nsecond line in English";
    let emoji = "\u{1f600}\u{1f680}\u{1f1ea}\u{1f1f8} \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467} done \u{2705}";
    let mut clips = vec![
        text(1, mixed, "zed", 1),
        text(2, &url, "firefox", 2),
        text(3, rtl, "telegram", 3),
        text(4, emoji, "", 4),
        text(5, " \n\t \n", "ghostty", 5),
        text(
            6,
            "x",
            "a-very-long-application-class-name.that.keeps.going.Application",
            6,
        ),
        text(7, &"word ".repeat(400), "obsidian", 7),
    ];
    clips[6].favorite = true;
    clips.rotate_right(1);
    let selected = clips
        .iter()
        .position(|clip| clip.id == selected as i64 + 1)
        .unwrap_or(0);
    State {
        clips,
        selected,
        ..ready()
    }
}

/// The usual history with the clip `id` selected.
fn chosen(id: i64) -> State {
    let mut state = ready();
    state.selected = state
        .clips
        .iter()
        .position(|clip| clip.id == id)
        .unwrap_or(0);
    state
}

fn undo() -> State {
    let mut state = ready();
    let clip = state.clips.remove(5);
    state.undo.push((clip, 5));
    state
}

fn search() -> State {
    let mut state = ready();
    state.query = rune::TextEdit::new("cargo");
    state.query.focused = true;
    state
        .clips
        .retain(|clip| clip.content.contains("cargo") || clip.content.contains("capture"));
    state.grouped = false;
    state.selected = 0;
    state
}

fn long() -> State {
    let mut state = ready();
    for index in 0..40 {
        state.clips.push(text(
            100 + index,
            &format!("older clip number {index}"),
            "ghostty",
            3 * 24 * 60 + index,
        ));
    }
    state.selected = 9;
    state
}

fn no_results() -> State {
    let mut state = ready();
    state.query = rune::TextEdit::new("zzzz");
    state.query.focused = true;
    state.clips.clear();
    state.selected = 0;
    state
}

fn offline() -> State {
    State {
        clips: Vec::new(),
        link: Link::Offline(
            "/run/user/1000/ditox/rpc.sock: No such file or directory (os error 2)".into(),
        ),
        watcher: None,
        ..ready()
    }
}

fn paused() -> State {
    State {
        filter: Filter::Favorites,
        selected: 0,
        clips: clips().into_iter().filter(|clip| clip.favorite).collect(),
        watcher: Some(WatcherStatus {
            paused: true,
            ..watching()
        }),
        ..ready()
    }
}

pub fn storm(width: u32, height: u32) -> Pixels {
    const BAYER: [[u8; 8]; 8] = [
        [0, 32, 8, 40, 2, 34, 10, 42],
        [48, 16, 56, 24, 50, 18, 58, 26],
        [12, 44, 4, 36, 14, 46, 6, 38],
        [60, 28, 52, 20, 62, 30, 54, 22],
        [3, 35, 11, 43, 1, 33, 9, 41],
        [51, 19, 59, 27, 49, 17, 57, 25],
        [15, 47, 7, 39, 13, 45, 5, 37],
        [63, 31, 55, 23, 61, 29, 53, 21],
    ];
    const RAMP: [[u8; 3]; 5] = [
        [14, 16, 20],
        [32, 40, 54],
        [58, 82, 124],
        [143, 184, 245],
        [232, 234, 230],
    ];
    const DOT: u32 = 4;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
            let front = ((u * 1.15 - v * 0.55 - 0.18) * 2.2).clamp(0.0, 1.0);
            let swell = (((u * 5.0 + v * 3.0).sin() + (u * 2.0 - v * 6.5).cos()) * 0.07).abs();
            let level = ((front * front + swell * front) * (RAMP.len() - 1) as f32)
                .clamp(0.0, (RAMP.len() - 1) as f32 - 0.001);
            let threshold =
                (f32::from(BAYER[((y / DOT) % 8) as usize][((x / DOT) % 8) as usize]) + 0.5) / 64.0;
            let stop = level.floor() as usize + usize::from(level.fract() > threshold);
            rgba.extend_from_slice(&[RAMP[stop][0], RAMP[stop][1], RAMP[stop][2], 255]);
        }
    }
    let pixels = Pixels::new();
    pixels.set(width, height, rgba);
    pixels
}
