use ditox_rpc::{ConfigView, Entry, Filter, Kind, Settings, Theme, Ui, WatcherStatus};
use rune::Pixels;

use crate::list::{Menu, State};
use crate::model::Link;
use crate::options::{self, Load, Tab};

pub const NOW_MS: i64 = 1_800_000_000_000;

pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("ready", |state| *state = ready()),
    ("light", |state| *state = light(ready())),
    ("one-line", |state| *state = lines(1)),
    ("four-lines", |state| *state = lines(4)),
    ("search", |state| *state = search()),
    ("menu", |state| *state = menu(ready())),
    ("menu-light", |state| *state = light(menu(ready()))),
    ("menu-link", |state| *state = menu(chosen(5))),
    ("menu-no-target", |state| {
        *state = menu(State {
            target: None,
            ..ready()
        })
    }),
    ("peek", |state| *state = peek(3)),
    ("peek-light", |state| *state = light(peek(3))),
    ("peek-image", |state| *state = peek(4)),
    ("peek-link", |state| *state = peek(5)),
    ("odd", |state| *state = odd()),
    ("undo", |state| *state = undo()),
    ("images", |state| *state = images()),
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
    let mut state = State {
        clips: clips(),
        selected: 2,
        link: Link::Ready,
        watcher: Some(watching()),
        target: Some("0xabc".into()),
        now_ms: NOW_MS,
        dark: Some(true),
        ..State::blank()
    };
    state.thumbs.keep(4, storm(160, 100), 0);
    state
}

fn light(state: State) -> State {
    State {
        dark: Some(false),
        ..state
    }
}

fn lines(lines: u32) -> State {
    let mut state = ready();
    state.ui.lines = lines;
    state
}

fn menu(mut state: State) -> State {
    state.menu = Some(Menu { at: 0 });
    state
}

pub fn chosen(id: i64) -> State {
    let mut state = ready();
    state.selected = state
        .clips
        .iter()
        .position(|clip| clip.id == id)
        .unwrap_or(0);
    state
}

fn peek(id: i64) -> State {
    let mut state = chosen(id);
    state.peek = true;
    state.images.keep(4, storm(960, 600), 0);
    state
}

fn odd() -> State {
    let mixed = "\tfn main() {\n\t\tlet greeting = \"\u{4f60}\u{597d}\u{ff0c}\u{4e16}\u{754c} \u{1f44b} caf\u{e9} na\u{ef}ve\";\n\t\tprintln!(\"{greeting}\");\n\t}\n";
    let url = format!(
        "https://example.com/{}?token={}",
        "very/long/path/".repeat(12),
        "a1b2c3d4".repeat(20)
    );
    let rtl = "\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}\u{60c} \u{647}\u{630}\u{627} \u{646}\u{635} \u{639}\u{631}\u{628}\u{64a} \u{637}\u{648}\u{64a}\u{644}";
    let emoji = "\u{1f600}\u{1f680}\u{1f1ea}\u{1f1f8} \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467} done \u{2705}";
    let mut clips = vec![
        text(1, mixed, "zed", 1),
        text(2, &url, "firefox", 2),
        text(3, rtl, "telegram", 3),
        text(4, emoji, "", 4),
        text(5, " \n\t \n", "ghostty", 5),
        text(6, "x", "kitty", 6),
        text(7, &"word ".repeat(400), "obsidian", 7),
        Entry {
            image_width: Some(2400),
            image_height: Some(300),
            ..picture(8, 8)
        },
        Entry {
            image_width: Some(300),
            image_height: Some(2400),
            ..picture(9, 9)
        },
        Entry {
            image_width: Some(16),
            image_height: Some(16),
            ..picture(10, 10)
        },
        picture(11, 11),
    ];
    clips[6].favorite = true;
    clips.rotate_right(5);
    let mut state = State {
        clips,
        selected: 1,
        ..ready()
    };
    state.thumbs.keep(8, storm(480, 60), 0);
    state.thumbs.keep(9, storm(10, 80), 0);
    state.thumbs.keep(10, storm(16, 16), 0);
    state
}

fn undo() -> State {
    let mut state = ready();
    let clip = state.clips.remove(5);
    state.undo.push((clip, 5));
    state
}

fn images() -> State {
    let mut state = ready();
    state.filter = Filter::Images;
    state.clips = (0..7)
        .map(|index| Entry {
            image_width: Some([960, 1920, 640, 3840, 400, 1280, 800][index as usize]),
            image_height: Some([600, 1080, 640, 2160, 900, 720, 200][index as usize]),
            ..picture(40 + index, index * 7)
        })
        .collect();
    for clip in &state.clips {
        let (width, height) = (
            clip.image_width.unwrap_or(1) as f32,
            clip.image_height.unwrap_or(1) as f32,
        );
        let scale = (80.0 / height).min(480.0 / width);
        if clip.id != 44 {
            state.thumbs.keep(
                clip.id,
                storm((width * scale) as u32, (height * scale) as u32),
                0,
            );
        }
    }
    state.selected = 1;
    state
}

fn search() -> State {
    let mut state = ready();
    state.query = rune::TextEdit::new("cargo");
    state.query.focused = true;
    state
        .clips
        .retain(|clip| clip.content.contains("cargo") || clip.content.contains("capture"));
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

pub type OptionsScene = (&'static str, fn(&mut options::State));

pub const OPTIONS_SCENES: &[OptionsScene] = &[
    ("general", |state| *state = options(Tab::General)),
    ("general-light", |state| {
        *state = options(Tab::General);
        state.theme = Theme::Light;
        state.dark = Some(false);
    }),
    ("paste", |state| *state = options(Tab::Paste)),
    ("privacy", |state| *state = options(Tab::Privacy)),
    ("keys", |state| *state = options(Tab::Keys)),
    ("about", |state| *state = options(Tab::About)),
    ("read-only", |state| {
        let mut view = config_view();
        view.writable = false;
        view.path = "/home/friend/.config/ditox/config.toml".into();
        state.show(view);
        state.dark = Some(true);
    }),
    ("refused", |state| {
        *state = options(Tab::Paste);
        state.keybind = rune::TextEdit::new("ctrl+");
        state.error = Some("\"ctrl+\" is not a paste keybind.".into());
    }),
    ("loading", |state| state.dark = Some(true)),
    ("options-offline", |state| {
        state.dark = Some(true);
        state.load = Load::Offline(
            "/run/user/1000/ditox/rpc.sock: No such file or directory (os error 2)".into(),
        );
    }),
];

pub fn config_view() -> ConfigView {
    ConfigView {
        settings: Settings {
            max_entries: 1000,
            delete_after_seconds: 0,
            allow_duplicates: false,
            paste_enabled: true,
            paste_buffer_ms: 120,
            paste_keybind: "ctrl+v".into(),
            terminal_paste_keybind: "ctrl+shift+v".into(),
            open_keybind: if cfg!(windows) { "ctrl+alt+v" } else { "" }.into(),
            terminal_apps: [
                "kitty",
                "ghostty",
                "alacritty",
                "foot",
                "wezterm",
                "konsole",
                "rio",
            ]
            .map(String::from)
            .to_vec(),
            excluded_apps: ["1Password", "Bitwarden", "KeePassXC"]
                .map(String::from)
                .to_vec(),
            excluded_windows: Vec::new(),
            secret_patterns: ["-----BEGIN PRIVATE KEY-----", "ghp_", "AKIA[A-Z0-9]{16}"]
                .map(String::from)
                .to_vec(),
            ui: Ui {
                theme: Theme::Dark,
                lines: 2,
                close_on_blur: true,
            },
        },
        path: "/home/friend/.config/ditox/config.toml".into(),
        writable: true,
        database: "/home/friend/.local/share/ditox/ditox-v2.db".into(),
        version: "0.4.0".into(),
    }
}

pub fn options(tab: Tab) -> options::State {
    let mut state = options::State::blank();
    state.show(config_view());
    state.tab = tab;
    state
}
