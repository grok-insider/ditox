mod feed;
mod fixture;
mod model;
mod picture;
mod startup;
mod theme;
mod ui;

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ditox_rpc::{Entry, Filter, Kind, PasteMode, WatcherStatus};
use model::Link;
use rune::{
    App, AppHandle, AppSpec, ImageCache, Key, KeyEvent, Pixels, Spring, TextEdit, Tx, Waker,
    WindowOptions,
};

pub const APP_ID: &str = "dev.ditox";
const SIZE: (f32, f32) = (780.0, 500.0);
const IMAGE_SLOTS: usize = 4;
/// Pixels the window keeps for clips looked at a moment ago, already at preview size.
const IMAGE_BYTES: u64 = 24 * 1024 * 1024;
/// How long an Enter pressed just before the results arrive stays valid.
const PASTE_WAIT: Duration = Duration::from_millis(1500);
const HYPRLAND_WAIT: Duration = Duration::from_millis(500);

/// The pictures of the clips looked at last, oldest first.
#[derive(Default)]
pub struct Images {
    kept: Vec<(i64, Pixels, u64)>,
}

impl Images {
    pub fn get(&self, id: &i64) -> Option<&Pixels> {
        self.kept
            .iter()
            .find(|(kept, _, _)| kept == id)
            .map(|(_, pixels, _)| pixels)
    }

    pub fn contains_key(&self, id: &i64) -> bool {
        self.get(id).is_some()
    }

    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.kept.len()
    }

    pub fn insert(&mut self, id: i64, pixels: Pixels) {
        self.keep(id, pixels, 0);
    }

    /// Keeps a picture that takes `bytes` decoded and lets go of the oldest
    /// ones beyond the count and the memory the window allows itself.
    pub fn keep(&mut self, id: i64, pixels: Pixels, bytes: u64) {
        self.kept.retain(|(kept, _, _)| *kept != id);
        self.kept.push((id, pixels, bytes));
        while self.kept.len() > 1
            && (self.kept.len() > IMAGE_SLOTS
                || self.kept.iter().map(|(_, _, bytes)| bytes).sum::<u64>() > IMAGE_BYTES)
        {
            self.kept.remove(0);
        }
    }
}

/// What Enter asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Put {
    Paste(PasteMode),
    /// On the clipboard only, with no paste after it.
    Copy,
}

pub struct State {
    pub query: TextEdit,
    pub filter: Filter,
    pub clips: Vec<Entry>,
    pub selected: usize,
    pub detail: Option<Entry>,
    pub images: Images,
    pub unavailable: Option<(i64, feed::Gone)>,
    pub link: Link,
    pub watcher: Option<WatcherStatus>,
    pub note: Option<String>,
    pub target: Option<String>,
    pub now_ms: i64,
    pub dark: Option<bool>,
    pub live: bool,
    cache: Option<ImageCache>,
    asked: Option<i64>,
    pasting: bool,
    rehome: bool,
    generation: u64,
    pending_paste: Option<(Put, Instant)>,
    pub grouped: bool,
    /// When today began, for the day headings. `None` keeps one "Recent" group.
    pub day_start_ms: Option<i64>,
    /// Clips deleted a moment ago, newest last, with the row each was on.
    pub undo: Vec<(Entry, usize)>,
    pub keys_open: bool,
}

static TARGET: OnceLock<Option<String>> = OnceLock::new();

impl State {
    pub fn blank() -> State {
        let mut query = TextEdit::new("");
        query.focused = true;
        State {
            query,
            filter: Filter::All,
            clips: Vec::new(),
            selected: 0,
            detail: None,
            images: Images::default(),
            unavailable: None,
            link: Link::Connecting,
            watcher: None,
            note: None,
            target: None,
            now_ms: 0,
            dark: None,
            live: false,
            cache: None,
            asked: None,
            pasting: false,
            rehome: true,
            generation: 0,
            pending_paste: None,
            grouped: true,
            day_start_ms: None,
            undo: Vec::new(),
            keys_open: false,
        }
    }

    pub fn current(&self) -> Option<&Entry> {
        self.clips.get(self.selected)
    }

    /// Why the clip `id` cannot be shown, if the daemon said so.
    pub fn gone(&self, id: i64) -> Option<feed::Gone> {
        self.unavailable
            .filter(|(gone, _)| *gone == id)
            .map(|(_, why)| why)
    }

    pub fn full_text(&self) -> Option<&str> {
        let clip = self.current()?;
        if model::is_complete(clip) {
            return Some(&clip.content);
        }
        self.detail
            .as_ref()
            .filter(|detail| detail.id == clip.id)
            .map(|detail| detail.content.as_str())
    }

    /// Ask for the list again. What is on screen stays until the answer comes.
    fn refresh(&mut self) {
        self.generation += 1;
        self.rehome = true;
        self.pending_paste = None;
        feed::send(feed::Command::Query {
            query: self.query.text.trim().to_string(),
            filter: self.filter,
            generation: self.generation,
        });
    }

    fn set_filter(&mut self, filter: Filter) {
        if self.filter != filter {
            self.filter = filter;
            self.refresh();
        }
    }

    fn select(&mut self, index: usize, tx: &mut Tx) {
        self.selected = index.min(self.clips.len().saturating_sub(1));
        self.note = None;
        self.pending_paste = None;
        match self.current() {
            Some(_) if self.selected == 0 => tx.scroll_to("list", 0.0, 0.0),
            Some(clip) => tx.scroll_into_view(format!("clip-{}", clip.id)),
            None => {}
        }
    }

    /// Puts the selected clip on the clipboard and, unless only a copy was
    /// asked for, pastes it into the window the user came from.
    fn put(&mut self, put: Put) {
        if self.pasting {
            return;
        }
        if self.rehome {
            // The list on screen is about to be replaced. With a daemon that
            // answers, Enter means the first of the new results; without one,
            // it means nothing later.
            if self.link == Link::Ready {
                self.pending_paste = Some((put, Instant::now()));
            }
            return;
        }
        if let Some(id) = self.current().map(|clip| clip.id) {
            self.pasting = true;
            self.note = None;
            let (mode, target) = match put {
                Put::Paste(mode) => (mode, self.target.clone()),
                Put::Copy => (PasteMode::Original, None),
            };
            feed::send(feed::Command::Paste { id, mode, target });
        }
    }

    /// The selected clip's web address, when it is one.
    pub fn link(&self) -> Option<&str> {
        let clip = self.current()?;
        (model::shape(clip) == model::Shape::Link)
            .then(|| model::link(&clip.content))
            .flatten()
    }

    fn toggle_pin(&mut self) {
        if let Some(clip) = self.clips.get_mut(self.selected) {
            let on = !model::is_pinned(clip);
            clip.favorite = on;
            clip.sticky &= on;
            feed::send(feed::Command::Pin { id: clip.id, on });
        }
    }

    fn delete(&mut self) {
        if self.selected < self.clips.len() {
            let clip = self.clips.remove(self.selected);
            feed::send(feed::Command::Delete { id: clip.id });
            self.undo.push((clip, self.selected));
            self.selected = self.selected.min(self.clips.len().saturating_sub(1));
            self.note = None;
        }
    }

    /// Brings back the clip deleted last, where it was.
    fn undo_delete(&mut self) {
        if let Some((clip, index)) = self.undo.pop() {
            feed::send(feed::Command::Restore { id: clip.id });
            let index = index.min(self.clips.len());
            self.clips.insert(index, clip);
            self.selected = index;
        }
    }

    fn toggle_pause(&mut self) {
        if let Some(watcher) = &self.watcher {
            feed::send(feed::Command::Pause(!watcher.paused));
        }
    }
}

/// A picker answers at once: nothing fades out behind what replaced it, and
/// the selection tint lands within a frame or two.
fn snap(tx: &mut Tx) {
    tx.instant();
    tx.color_motion(Spring::new(0.07, 0.0));
}

fn init() -> State {
    State {
        target: TARGET.get().cloned().flatten(),
        now_ms: now_ms(),
        day_start_ms: day_start_ms(now_ms()),
        live: true,
        ..State::blank()
    }
}

fn setup(state: &mut State, waker: Waker) {
    state.cache = Some(ImageCache::new(waker.clone()));
    feed::bind(waker);
}

fn poll(state: &mut State, _: f64, tx: &mut Tx) -> bool {
    step(state, feed::drain(), tx)
}

/// One turn of the window: the clock, what the daemon said, what to ask next.
pub fn step(state: &mut State, events: Vec<feed::Event>, tx: &mut Tx) -> bool {
    let mut changed = false;
    if state.live {
        let now = now_ms();
        changed = now / 60_000 != state.now_ms / 60_000;
        state.now_ms = now;
        if changed {
            state.day_start_ms = day_start_ms(now);
        }
    }
    for event in events {
        changed = true;
        apply(state, event, tx);
    }
    ask_for_the_selected_clip(state);
    changed
}

/// Asks the daemon for the selected clip's whole text or picture when the
/// list only brought its start. Says so once per clip; the feed retries.
pub fn ask_for_the_selected_clip(state: &mut State) {
    let wanted = state.current().and_then(|clip| match clip.kind {
        _ if state.gone(clip.id).is_some() => None,
        Kind::Image if state.images.contains_key(&clip.id) || !model::previewable(clip) => None,
        Kind::Image => Some(feed::Wanted {
            id: clip.id,
            image: true,
        }),
        Kind::Text if state.full_text().is_some() => None,
        Kind::Text => Some(feed::Wanted {
            id: clip.id,
            image: false,
        }),
    });
    let wanted_id = wanted.map(|wanted| wanted.id);
    if wanted_id != state.asked {
        state.asked = wanted_id;
        feed::send(feed::Command::Want(wanted));
    }
}

pub fn apply(state: &mut State, event: feed::Event, tx: &mut Tx) {
    snap(tx);
    match event {
        feed::Event::Clips { generation, .. } if generation != state.generation => {}
        feed::Event::Clips {
            searching, clips, ..
        } => {
            let rehome = std::mem::take(&mut state.rehome);
            let previous = state.current().map(|clip| clip.id).filter(|_| !rehome);
            state.selected = model::keep_selection(&clips, previous, searching);
            state.clips = clips;
            state.grouped = !searching;
            state.link = Link::Ready;
            if rehome {
                tx.scroll_to("list", 0.0, 0.0);
            }
            if let Some((put, pressed)) = state.pending_paste.take()
                && pressed.elapsed() < PASTE_WAIT
            {
                state.put(put);
            }
        }
        feed::Event::Offline(message) => {
            state.link = Link::Offline(message);
            state.clips.clear();
            state.selected = 0;
            state.rehome = true;
            state.pending_paste = None;
            state.watcher = None;
            state.pasting = false;
        }
        feed::Event::Watcher(watcher) => state.watcher = Some(watcher),
        feed::Event::Detail(entry) => state.detail = Some(entry),
        feed::Event::Picture { id, picture } => {
            if let Some(cache) = &state.cache {
                // Rune keeps a chain of smaller copies beside the picture: a third more.
                let bytes = picture.rgba.len() as u64 * 4 / 3;
                let image = cache.rgba(picture.width, picture.height, picture.rgba);
                state.images.keep(id, image.pixels().clone(), bytes);
            }
        }
        feed::Event::Unavailable(id, why) => state.unavailable = Some((id, why)),
        feed::Event::Deleted(id) => state.undo.retain(|(clip, _)| clip.id != id),
        feed::Event::Note(message) => state.note = Some(message),
        feed::Event::Pasted => tx.close_window(),
        feed::Event::PasteRefused(reason) => {
            state.pasting = false;
            state.note = Some(model::refusal(&reason));
        }
    }
}

pub fn on_key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    let last = state.clips.len().saturating_sub(1);
    if state.keys_open {
        // The sheet covers the list: the next key puts it away, and only
        // the keys that leave or close it do more than that.
        state.keys_open = false;
        snap(tx);
        return !matches!(event.key, Key::Text(_));
    }
    match &event.key {
        Key::Down => state.select(model::step(state.clips.len(), state.selected, 1), tx),
        Key::Up => state.select(model::step(state.clips.len(), state.selected, -1), tx),
        Key::PageDown => state.select(model::step(state.clips.len(), state.selected, 8), tx),
        Key::PageUp => state.select(model::step(state.clips.len(), state.selected, -8), tx),
        Key::Home if event.mods.cmd => state.select(0, tx),
        Key::End if event.mods.cmd => state.select(last, tx),
        Key::Enter if event.mods.cmd => state.put(Put::Copy),
        Key::Enter if event.mods.shift => state.put(Put::Paste(PasteMode::Plain)),
        Key::Enter => state.put(Put::Paste(PasteMode::Original)),
        Key::Tab => state.set_filter(model::cycle_filter(state.filter, !event.mods.shift)),
        Key::Command(digit @ '1'..='4') => {
            let filter = model::FILTERS[*digit as usize - '1' as usize].0;
            state.set_filter(filter);
        }
        Key::Escape if state.query.text.is_empty() => tx.close_window(),
        Key::Escape => {
            state.query.clear();
            state.refresh();
        }
        Key::Function(1) | Key::Command('/') => state.keys_open = true,
        Key::Command('p') if event.mods.shift => state.toggle_pause(),
        Key::Command('p') => state.toggle_pin(),
        Key::Command('d') => state.delete(),
        Key::Delete if state.query.text.is_empty() => state.delete(),
        Key::Command('z') if !state.undo.is_empty() => state.undo_delete(),
        Key::Command('o') => open_link(state, tx),
        _ => return false,
    }
    snap(tx);
    true
}

pub fn open_link(state: &mut State, tx: &mut Tx) {
    if let Some(address) = state.link() {
        feed::send(feed::Command::Open(address.to_string()));
        tx.close_window();
    }
}

pub fn on_query_change(state: &mut State) {
    state.note = None;
    state.refresh();
}

pub fn press_clip(state: &mut State, id: i64, tx: &mut Tx) {
    snap(tx);
    state.pending_paste = None;
    if let Some(index) = state.clips.iter().position(|clip| clip.id == id) {
        state.selected = index;
        state.note = None;
    }
}

pub fn press_filter(state: &mut State, filter: Filter, tx: &mut Tx) {
    snap(tx);
    state.set_filter(filter);
}

pub fn press_paste(state: &mut State) {
    state.put(Put::Paste(PasteMode::Original));
}

pub fn press_pause(state: &mut State) {
    state.toggle_pause();
}

pub fn press_pin(state: &mut State, tx: &mut Tx) {
    snap(tx);
    state.toggle_pin();
}

pub fn press_delete(state: &mut State, tx: &mut Tx) {
    snap(tx);
    state.delete();
}

pub fn press_undo(state: &mut State, tx: &mut Tx) {
    snap(tx);
    state.undo_delete();
}

pub fn press_clear(state: &mut State, tx: &mut Tx) {
    snap(tx);
    if !state.query.text.is_empty() {
        state.query.clear();
        state.refresh();
    }
}

pub fn press_keys(state: &mut State, tx: &mut Tx) {
    snap(tx);
    state.keys_open = !state.keys_open;
}

/// When the day `now_ms` falls on began, by the clock on the wall.
fn day_start_ms(now_ms: i64) -> Option<i64> {
    let seconds = libc::time_t::try_from(now_ms / 1000).ok()?;
    // SAFETY: `localtime_r` fills the `tm` it is given and keeps nothing.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&seconds, &mut tm).is_null() {
            return None;
        }
        tm
    };
    let into_day = i64::from(tm.tm_hour) * 3600 + i64::from(tm.tm_min) * 60 + i64::from(tm.tm_sec);
    Some((now_ms / 1000 - into_day) * 1000)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// The address and class of the window that has the keyboard, from
/// Hyprland's socket, or from `hyprctl` when the socket is not where expected.
fn active_window() -> Option<(String, String)> {
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
    let address = value.get("address")?.as_str()?.to_string();
    let class = value
        .get("class")
        .and_then(|class| class.as_str())
        .unwrap_or("")
        .to_string();
    Some((address, class))
}

fn main() {
    startup::prepare();
    if !rune::is_headless() {
        let window = active_window();
        if window.as_ref().is_some_and(|(_, class)| class == APP_ID) {
            return;
        }
        let _ = TARGET.set(window.map(|(address, _)| address));
        feed::start();
    }
    rune::set_app_id(APP_ID);
    let spec = AppSpec {
        scenes: fixture::SCENES,
        fonts: theme::FONTS,
        families: theme::FAMILIES,
        min_size: Some((560.0, 380.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new("Ditox", SIZE, init, ui::view)
    };
    let app = App::new(spec)
        .window_options(WindowOptions {
            borderless: true,
            ..Default::default()
        })
        .float_on_open()
        .on_close(|_: &mut State, _: &mut Tx| feed::finish());
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
fn exit_without_a_window(handle: AppHandle<State>) {
    let _ = std::thread::Builder::new()
        .name("ditox-window".into())
        .spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if let Some(error) = handle.window_error() {
                    startup::retry_with_every_driver();
                    eprintln!("ditox: the window could not open: {error}");
                    std::process::exit(1);
                }
            }
        });
}

#[cfg(test)]
mod tests;
