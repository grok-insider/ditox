//! The options window: what Ditox keeps, how it pastes and what it never
//! saves. It is its own window, opened from the clip list's menu or with
//! `ditox options`, and it talks to the daemon, which owns the config file.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use ditox_rpc::{Client, ConfigView, MAX_LINES, MIN_LINES, RpcError, Settings, Theme, Ui};
use rune::{Key, KeyEvent, TextEdit, Tx, Waker};

pub mod view;

pub const SIZE: (f32, f32) = (560.0, 500.0);
pub const MIN_SIZE: (f32, f32) = (520.0, 420.0);
const DAY: u32 = 86_400;
const RETRY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    General,
    Paste,
    Privacy,
    Keys,
    About,
}

pub const TABS: [(Tab, &str); 5] = [
    (Tab::General, "General"),
    (Tab::Paste, "Paste"),
    (Tab::Privacy, "Privacy"),
    (Tab::Keys, "Keys"),
    (Tab::About, "About"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    Loading,
    Offline(String),
    Ready,
}

pub struct State {
    pub tab: Tab,
    pub load: Load,
    /// The settings as the daemon had them when the window opened.
    pub loaded: Option<ConfigView>,
    pub max_entries: TextEdit,
    pub days: TextEdit,
    pub allow_duplicates: bool,
    pub theme: Theme,
    pub lines: u32,
    pub close_on_blur: bool,
    pub paste_enabled: bool,
    pub buffer_ms: TextEdit,
    pub keybind: TextEdit,
    pub terminal_keybind: TextEdit,
    /// The key that opens the list. Shown where the daemon listens for it.
    pub open_keybind: TextEdit,
    pub terminal_apps: TextEdit,
    pub excluded_apps: TextEdit,
    pub excluded_windows: TextEdit,
    pub secret_patterns: TextEdit,
    /// Why the last save did not happen.
    pub error: Option<String>,
    pub saving: bool,
    pub dark: Option<bool>,
}

impl State {
    pub fn blank() -> State {
        State {
            tab: Tab::General,
            load: Load::Loading,
            loaded: None,
            max_entries: TextEdit::new(""),
            days: TextEdit::new(""),
            allow_duplicates: false,
            theme: Theme::System,
            lines: Ui::default().lines,
            close_on_blur: true,
            paste_enabled: true,
            buffer_ms: TextEdit::new(""),
            keybind: TextEdit::new(""),
            terminal_keybind: TextEdit::new(""),
            open_keybind: TextEdit::new(""),
            terminal_apps: TextEdit::new(""),
            excluded_apps: TextEdit::new(""),
            excluded_windows: TextEdit::new(""),
            secret_patterns: TextEdit::new(""),
            error: None,
            saving: false,
            dark: None,
        }
    }

    /// Fills the fields from what the daemon holds.
    pub fn show(&mut self, view: ConfigView) {
        let settings = &view.settings;
        self.max_entries = TextEdit::new(settings.max_entries.to_string());
        self.days = TextEdit::new(days_of(settings.delete_after_seconds).to_string());
        self.allow_duplicates = settings.allow_duplicates;
        self.theme = settings.ui.theme;
        self.lines = settings.ui.lines;
        self.close_on_blur = settings.ui.close_on_blur;
        self.paste_enabled = settings.paste_enabled;
        self.buffer_ms = TextEdit::new(settings.paste_buffer_ms.to_string());
        self.keybind = TextEdit::new(&settings.paste_keybind);
        self.terminal_keybind = TextEdit::new(&settings.terminal_paste_keybind);
        self.open_keybind = TextEdit::new(&settings.open_keybind);
        self.terminal_apps = TextEdit::new(settings.terminal_apps.join("\n"));
        self.excluded_apps = TextEdit::new(settings.excluded_apps.join("\n"));
        self.excluded_windows = TextEdit::new(settings.excluded_windows.join("\n"));
        self.secret_patterns = TextEdit::new(settings.secret_patterns.join("\n"));
        self.dark = self.theme.dark();
        self.loaded = Some(view);
        self.load = Load::Ready;
    }

    pub fn writable(&self) -> bool {
        self.loaded.as_ref().is_some_and(|view| view.writable)
    }

    /// The settings the fields describe, or what is wrong with one of them.
    pub fn settings(&self) -> Result<Settings, String> {
        let Some(loaded) = &self.loaded else {
            return Err("The settings have not loaded yet.".into());
        };
        let number = |edit: &TextEdit, what: &str| -> Result<u32, String> {
            edit.text
                .trim()
                .parse::<u32>()
                .map_err(|_| format!("{what} takes a whole number."))
        };
        let days = number(&self.days, "Delete clips after")?;
        let lines = |edit: &TextEdit| -> Vec<String> {
            edit.text
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        };
        Ok(Settings {
            max_entries: number(&self.max_entries, "Clips to keep")?,
            // A time the file gave in seconds is kept unless the days changed.
            delete_after_seconds: if days == days_of(loaded.settings.delete_after_seconds) {
                loaded.settings.delete_after_seconds
            } else {
                days.checked_mul(DAY)
                    .ok_or("Delete clips after takes fewer days.")?
            },
            allow_duplicates: self.allow_duplicates,
            paste_enabled: self.paste_enabled,
            paste_buffer_ms: number(&self.buffer_ms, "Wait before pasting")?,
            paste_keybind: self.keybind.text.trim().to_string(),
            terminal_paste_keybind: self.terminal_keybind.text.trim().to_string(),
            open_keybind: self.open_keybind.text.trim().to_string(),
            terminal_apps: lines(&self.terminal_apps),
            excluded_apps: lines(&self.excluded_apps),
            excluded_windows: lines(&self.excluded_windows),
            secret_patterns: lines(&self.secret_patterns),
            ui: Ui {
                theme: self.theme,
                lines: self.lines.clamp(MIN_LINES, MAX_LINES),
                close_on_blur: self.close_on_blur,
            },
        })
    }

    fn save(&mut self) {
        if self.saving || !self.writable() {
            return;
        }
        match self.settings() {
            Ok(settings) => {
                self.error = None;
                self.saving = true;
                send(Ask::Save(Box::new(settings)));
            }
            Err(error) => self.error = Some(error),
        }
    }
}

/// Whole days, rounded up, so a time under a day does not read as "never".
fn days_of(seconds: u32) -> u32 {
    seconds.div_ceil(DAY)
}

pub fn init() -> State {
    State {
        dark: Ui::load().theme.dark(),
        ..State::blank()
    }
}

pub fn setup(_: &mut State, waker: Waker) {
    if let Some(link) = LINK.get() {
        *link
            .wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(waker.clone());
        waker();
    }
}

pub fn poll(state: &mut State, _: f64, tx: &mut Tx) -> bool {
    let told = match LINK.get() {
        Some(link) => std::mem::take(
            &mut *link
                .inbox
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        ),
        None => Vec::new(),
    };
    let changed = !told.is_empty();
    for told in told {
        apply(state, told, tx);
    }
    changed
}

pub fn apply(state: &mut State, told: Told, tx: &mut Tx) {
    tx.instant();
    match told {
        // What the user has typed since is not replaced by a late answer.
        Told::Loaded(view) if state.load != Load::Ready => state.show(*view),
        Told::Loaded(_) => {}
        Told::Offline(reason) if state.load != Load::Ready => state.load = Load::Offline(reason),
        Told::Offline(_) => {}
        Told::Saved => tx.close_window(),
        Told::Refused(reason) => {
            state.saving = false;
            state.error = Some(sentence(&reason));
        }
    }
}

/// The daemon's reason as a sentence of the window.
fn sentence(reason: &str) -> String {
    let mut chars = reason.trim().trim_end_matches('.').chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => "The settings were not saved.".into(),
    }
}

pub fn on_key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    match &event.key {
        Key::Escape => tx.close_window(),
        Key::Command('s') => state.save(),
        Key::Command(digit @ '1'..='5') => {
            state.tab = TABS[*digit as usize - '1' as usize].0;
        }
        Key::PageDown if event.mods.cmd => state.tab = neighbour(state.tab, 1),
        Key::PageUp if event.mods.cmd => state.tab = neighbour(state.tab, TABS.len() - 1),
        _ => return false,
    }
    tx.instant();
    true
}

fn neighbour(tab: Tab, by: usize) -> Tab {
    let at = TABS
        .iter()
        .position(|(other, _)| *other == tab)
        .unwrap_or(0);
    TABS[(at + by) % TABS.len()].0
}

pub fn press_tab(state: &mut State, tab: Tab, tx: &mut Tx) {
    tx.instant();
    state.tab = tab;
}

pub fn press_save(state: &mut State, tx: &mut Tx) {
    tx.instant();
    state.save();
}

pub fn press_cancel(_: &mut State, tx: &mut Tx) {
    tx.close_window();
}

pub fn press_theme(state: &mut State, theme: Theme, tx: &mut Tx) {
    tx.instant();
    state.theme = theme;
    state.dark = theme.dark();
}

pub fn press_lines(state: &mut State, lines: u32, tx: &mut Tx) {
    tx.instant();
    state.lines = lines;
}

pub enum Ask {
    Save(Box<Settings>),
}

pub enum Told {
    Loaded(Box<ConfigView>),
    Offline(String),
    Saved,
    Refused(String),
}

struct Wire {
    asks: Sender<Ask>,
    inbox: Mutex<Vec<Told>>,
    wake: Mutex<Option<Waker>>,
}

static LINK: OnceLock<Wire> = OnceLock::new();

/// Starts the thread that talks to the daemon for this window.
pub fn start() {
    let (asks, queue) = mpsc::channel();
    if LINK
        .set(Wire {
            asks,
            inbox: Mutex::new(Vec::new()),
            wake: Mutex::new(None),
        })
        .is_ok()
    {
        let _ = std::thread::Builder::new()
            .name("ditox-rpc".into())
            .spawn(move || run(queue));
    }
}

fn send(ask: Ask) {
    #[cfg(test)]
    SENT.with(|sent| {
        let Ask::Save(settings) = &ask;
        sent.borrow_mut().push((**settings).clone());
    });
    if let Some(link) = LINK.get() {
        let _ = link.asks.send(ask);
    }
}

#[cfg(test)]
thread_local! {
    static SENT: std::cell::RefCell<Vec<Settings>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The settings the window asked the daemon to save in this test so far.
#[cfg(test)]
pub fn take_sent() -> Vec<Settings> {
    SENT.with(|sent| std::mem::take(&mut *sent.borrow_mut()))
}

fn tell(told: Told) {
    let Some(link) = LINK.get() else { return };
    link.inbox
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(told);
    let wake = link
        .wake
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(wake) = wake {
        wake();
    }
}

fn connect(socket: Option<&Path>) -> Result<Client, RpcError> {
    match socket {
        Some(path) => Client::connect_to(path),
        None => Client::connect(),
    }
}

fn load(socket: Option<&Path>) -> Result<ConfigView, RpcError> {
    connect(socket)?.config()
}

fn save(socket: Option<&Path>, settings: &Settings) -> Result<(), RpcError> {
    connect(socket)?.set_config(settings).map(drop)
}

/// Loads the settings, trying again while the daemon is away, and saves
/// them when asked.
fn run(queue: Receiver<Ask>) {
    let mut loaded = false;
    let mut said_offline = false;
    loop {
        if !loaded {
            match load(None) {
                Ok(view) => {
                    loaded = true;
                    tell(Told::Loaded(Box::new(view)));
                }
                Err(err) if !said_offline => {
                    said_offline = true;
                    tell(Told::Offline(err.to_string()));
                }
                Err(_) => {}
            }
        }
        match queue.recv_timeout(RETRY) {
            Ok(Ask::Save(settings)) => match save(None, &settings) {
                Ok(()) => tell(Told::Saved),
                Err(err) => tell(Told::Refused(err.to_string())),
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests;
