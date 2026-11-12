//! The clip list: the one thing Super+V opens. A column of clips, a search
//! field under it, and a menu for everything else.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ditox_rpc::{Entry, Filter, Kind, PasteMode, Ui, WatcherStatus};
use rune::{ImageCache, Key, KeyEvent, Pixels, Spring, TextEdit, Tx, Waker};

use crate::feed;
use crate::model::{self, Link};
use crate::picture;

pub mod view;

pub const SIZE: (f32, f32) = (420.0, 560.0);
pub const MIN_SIZE: (f32, f32) = (320.0, 300.0);
/// How many clips from the top have a number, and a Ctrl+digit that pastes them.
pub const QUICK: usize = 9;
/// The widest a row's small picture is drawn, in points.
pub const THUMB_WIDTH: f32 = 240.0;
const PEEK_SLOTS: usize = 4;
/// Pixels the window keeps for clips looked at whole a moment ago.
const PEEK_BYTES: u64 = 24 * 1024 * 1024;
const THUMB_SLOTS: usize = 96;
const THUMB_BYTES: u64 = 12 * 1024 * 1024;
/// Rows beyond the ones in view whose pictures are fetched ahead.
const THUMB_MARGIN: usize = 6;
/// How long an Enter pressed just before the results arrive stays valid.
const PASTE_WAIT: Duration = Duration::from_millis(1500);

/// Pictures the window holds, oldest first, within a count and a size.
pub struct Images {
    kept: Vec<(i64, Pixels, u64)>,
    slots: usize,
    budget: u64,
}

impl Images {
    pub fn new(slots: usize, budget: u64) -> Images {
        Images {
            kept: Vec::new(),
            slots,
            budget,
        }
    }

    pub fn get(&self, id: i64) -> Option<&Pixels> {
        self.kept
            .iter()
            .find(|(kept, _, _)| *kept == id)
            .map(|(_, pixels, _)| pixels)
    }

    pub fn contains(&self, id: i64) -> bool {
        self.get(id).is_some()
    }

    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.kept.len()
    }

    /// Keeps a picture that takes `bytes` decoded and lets go of the oldest
    /// ones beyond the count and the memory the window allows itself.
    pub fn keep(&mut self, id: i64, pixels: Pixels, bytes: u64) {
        self.kept.retain(|(kept, _, _)| *kept != id);
        self.kept.push((id, pixels, bytes));
        while self.kept.len() > 1
            && (self.kept.len() > self.slots
                || self.kept.iter().map(|(_, _, bytes)| bytes).sum::<u64>() > self.budget)
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

/// Everything the menu can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Put(Put),
    View,
    OpenLink,
    Pin,
    Delete,
    Show(Filter),
    Pause,
    Options,
}

pub struct Item {
    pub act: Act,
    pub label: &'static str,
    pub keys: &'static str,
    /// For the entries that choose one of several: whether this is the one.
    pub chosen: Option<bool>,
    /// The first entry of a group, drawn under a line.
    pub leads: bool,
}

/// The open menu, and the entry the keyboard is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Menu {
    pub at: usize,
}

pub struct State {
    pub ui: Ui,
    pub query: TextEdit,
    pub filter: Filter,
    pub clips: Vec<Entry>,
    pub selected: usize,
    /// The selected clip is shown whole, in place of the list.
    pub peek: bool,
    pub menu: Option<Menu>,
    pub detail: Option<Entry>,
    pub images: Images,
    pub thumbs: Images,
    pub unavailable: Option<(i64, feed::Gone)>,
    pub link: Link,
    pub watcher: Option<WatcherStatus>,
    pub note: Option<String>,
    pub target: Option<String>,
    pub now_ms: i64,
    pub dark: Option<bool>,
    pub live: bool,
    /// Clips deleted a moment ago, newest last, with the row each was on.
    pub undo: Vec<(Entry, usize)>,
    pub(crate) cache: Option<ImageCache>,
    pub(crate) asked: Option<i64>,
    pub(crate) pasting: bool,
    pub(crate) rehome: bool,
    pub(crate) generation: u64,
    /// An Enter or a Ctrl+digit pressed while a search was on its way: what
    /// it asked for, when, and which row if it named one.
    pub(crate) pending_paste: Option<(Put, Instant, Option<usize>)>,
    pub(crate) visible: Range<usize>,
    pub(crate) thumbs_asked: Vec<i64>,
    pub(crate) thumbless: HashSet<i64>,
    pub(crate) had_focus: bool,
}

impl State {
    pub fn blank() -> State {
        let mut query = TextEdit::new("");
        query.focused = true;
        State {
            ui: Ui::default(),
            query,
            filter: Filter::All,
            clips: Vec::new(),
            selected: 0,
            peek: false,
            menu: None,
            detail: None,
            images: Images::new(PEEK_SLOTS, PEEK_BYTES),
            thumbs: Images::new(THUMB_SLOTS, THUMB_BYTES),
            unavailable: None,
            link: Link::Connecting,
            watcher: None,
            note: None,
            target: None,
            now_ms: 0,
            dark: None,
            live: false,
            undo: Vec::new(),
            cache: None,
            asked: None,
            pasting: false,
            rehome: true,
            generation: 0,
            pending_paste: None,
            visible: 0..0,
            thumbs_asked: Vec::new(),
            thumbless: HashSet::new(),
            had_focus: false,
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

    /// The selected clip's web address, when it is one.
    pub fn link(&self) -> Option<&str> {
        let clip = self.current()?;
        model::is_link(clip)
            .then(|| model::link(&clip.content))
            .flatten()
    }

    pub fn paused(&self) -> bool {
        self.watcher.as_ref().is_some_and(|watcher| watcher.paused)
    }

    /// What the menu offers, top to bottom.
    pub fn menu_items(&self) -> Vec<Item> {
        let item = |act, label, keys| Item {
            act,
            label,
            keys,
            chosen: None,
            leads: false,
        };
        let mut items = Vec::new();
        if let Some(clip) = self.current() {
            if self.target.is_some() {
                items.push(item(
                    Act::Put(Put::Paste(PasteMode::Original)),
                    "Paste",
                    "Enter",
                ));
                items.push(item(
                    Act::Put(Put::Paste(PasteMode::Plain)),
                    "Paste as plain text",
                    "Shift+Enter",
                ));
                items.push(item(Act::Put(Put::Copy), "Copy", "Ctrl+Enter"));
            } else {
                items.push(item(Act::Put(Put::Copy), "Copy", "Enter"));
            }
            items.push(item(
                Act::View,
                if self.peek {
                    "Back to the list"
                } else {
                    "View"
                },
                "Tab",
            ));
            if self.link().is_some() {
                items.push(item(Act::OpenLink, "Open link", "Ctrl+O"));
            }
            items.push(item(
                Act::Pin,
                if model::is_pinned(clip) {
                    "Unpin"
                } else {
                    "Pin"
                },
                "Ctrl+P",
            ));
            items.push(item(Act::Delete, "Delete", "Del"));
        }
        for (index, filter) in [Filter::All, Filter::Text, Filter::Images, Filter::Favorites]
            .into_iter()
            .enumerate()
        {
            items.push(Item {
                chosen: Some(self.filter == filter),
                leads: index == 0 && !items.is_empty(),
                ..item(Act::Show(filter), model::filter_name(filter), "")
            });
        }
        let first_of_last_group = items.len();
        if self.watcher.is_some() {
            items.push(item(
                Act::Pause,
                if self.paused() {
                    "Resume capture"
                } else {
                    "Pause capture"
                },
                "Ctrl+Shift+P",
            ));
        }
        items.push(item(Act::Options, "Options\u{2026}", "Ctrl+,"));
        items[first_of_last_group].leads = true;
        items
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
                self.pending_paste = Some((put, Instant::now(), None));
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
            self.peek &= !self.clips.is_empty();
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
        if self.watcher.is_some() {
            feed::send(feed::Command::Pause(!self.paused()));
        }
    }

    fn toggle_peek(&mut self) {
        self.peek = !self.peek && self.current().is_some();
    }

    /// Asks for the small pictures of the image rows in view that have none.
    fn ask_for_thumbs(&mut self) {
        let from = self.visible.start.saturating_sub(THUMB_MARGIN);
        let to = (self.visible.end + THUMB_MARGIN).min(self.clips.len());
        let ids: Vec<i64> = self
            .clips
            .get(from..to)
            .unwrap_or(&[])
            .iter()
            .filter(|clip| {
                model::previewable(clip)
                    && !self.thumbs.contains(clip.id)
                    && !self.thumbless.contains(&clip.id)
            })
            .map(|clip| clip.id)
            .collect();
        if ids != self.thumbs_asked {
            self.thumbs_asked = ids.clone();
            let density = picture::density();
            let fit = (
                THUMB_WIDTH as u32 * density,
                view::thumb_height(self.ui.lines) as u32 * density,
            );
            feed::send(feed::Command::Thumbs { ids, fit });
        }
    }
}

/// A picker answers at once: nothing fades out behind what replaced it, and
/// the selection tint lands within a frame or two.
fn snap(tx: &mut Tx) {
    tx.instant();
    tx.color_motion(Spring::new(0.07, 0.0));
}

static TARGET: OnceLock<Option<String>> = OnceLock::new();

/// Says which window had the keyboard when this one was asked for: where a
/// chosen clip is pasted.
pub fn came_from(window: Option<String>) {
    let _ = TARGET.set(window);
}

pub fn init() -> State {
    let ui = Ui::load();
    State {
        ui,
        dark: ui.theme.dark(),
        target: TARGET.get().cloned().flatten(),
        now_ms: now_ms(),
        live: true,
        ..State::blank()
    }
}

pub fn setup(state: &mut State, waker: Waker) {
    state.cache = Some(ImageCache::new(waker.clone()));
    feed::bind(waker);
}

pub fn poll(state: &mut State, _: f64, tx: &mut Tx) -> bool {
    step(state, feed::drain(), tx)
}

/// The window closes when the keyboard goes to another window, if it had it.
pub fn on_focus(state: &mut State, focused: bool, tx: &mut Tx) {
    if focused {
        state.had_focus = true;
    } else if state.had_focus && state.ui.close_on_blur {
        tx.close_window();
    }
}

/// One turn of the window: the clock, what the daemon said, what to ask next.
pub fn step(state: &mut State, events: Vec<feed::Event>, tx: &mut Tx) -> bool {
    let mut changed = false;
    if state.live {
        let now = now_ms();
        changed = state.peek && now / 60_000 != state.now_ms / 60_000;
        state.now_ms = now;
    }
    for event in events {
        changed = true;
        apply(state, event, tx);
    }
    ask_for_the_selected_clip(state);
    changed
}

/// While a clip is shown whole, asks the daemon for its text or picture when
/// the list only brought its start. Says so once per clip; the feed retries.
pub fn ask_for_the_selected_clip(state: &mut State) {
    let wanted = state
        .current()
        .filter(|_| state.peek)
        .and_then(|clip| match clip.kind {
            _ if state.gone(clip.id).is_some() => None,
            Kind::Image if state.images.contains(clip.id) || !model::previewable(clip) => None,
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
            state.link = Link::Ready;
            state.peek &= !state.clips.is_empty();
            if rehome {
                tx.scroll_to("list", 0.0, 0.0);
            }
            state.ask_for_thumbs();
            if let Some((put, pressed, row)) = state.pending_paste.take()
                && pressed.elapsed() < PASTE_WAIT
                && row.is_none_or(|row| row < state.clips.len())
            {
                state.selected = row.unwrap_or(state.selected);
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
            state.peek = false;
            state.menu = None;
            state.thumbs_asked.clear();
        }
        feed::Event::Watcher(watcher) => state.watcher = Some(watcher),
        feed::Event::Detail(entry) => state.detail = Some(entry),
        feed::Event::Picture { id, picture } => {
            if let Some(pixels) = pixels(state, picture) {
                state.images.keep(id, pixels.0, pixels.1);
            }
        }
        feed::Event::Thumb { id, picture } => {
            if let Some(pixels) = pixels(state, picture) {
                state.thumbs.keep(id, pixels.0, pixels.1);
            }
        }
        feed::Event::NoThumb(id) => {
            state.thumbless.insert(id);
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

/// A decoded picture as something the window can draw, and what it weighs.
fn pixels(state: &State, picture: picture::Picture) -> Option<(Pixels, u64)> {
    let cache = state.cache.as_ref()?;
    // Rune keeps a chain of smaller copies beside the picture: a third more.
    let bytes = picture.rgba.len() as u64 * 4 / 3;
    let image = cache.rgba(picture.width, picture.height, picture.rgba);
    Some((image.pixels().clone(), bytes))
}

/// The rows the list has on screen. It changes nothing the view reads.
pub fn on_visible_rows(state: &mut State, rows: Range<usize>) -> bool {
    state.visible = rows;
    state.ask_for_thumbs();
    false
}

pub fn on_key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if state.menu.is_some() {
        return menu_key(state, event, tx);
    }
    let last = state.clips.len().saturating_sub(1);
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
        Key::Command(digit @ '1'..='9') => {
            let index = *digit as usize - '1' as usize;
            let put = Put::Paste(PasteMode::Original);
            if state.rehome {
                // The number is of the list that is on its way.
                state.put(put);
                if let Some(pending) = state.pending_paste.as_mut() {
                    pending.2 = Some(index);
                }
            } else if index < state.clips.len() {
                state.select(index, tx);
                state.put(put);
            }
        }
        Key::Tab | Key::Function(3) => state.toggle_peek(),
        Key::Escape if state.peek => state.peek = false,
        Key::Escape if state.query.text.is_empty() => tx.close_window(),
        Key::Escape => {
            state.query.clear();
            state.refresh();
        }
        Key::Command('m') | Key::Function(10) => open_menu(state),
        Key::Command(',') => open_options(state, tx),
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

/// While the menu is open the keyboard is the menu's.
fn menu_key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    let count = state.menu_items().len();
    let Some(menu) = state.menu.as_mut() else {
        return false;
    };
    snap(tx);
    match &event.key {
        Key::Down => menu.at = (menu.at + 1) % count,
        Key::Up => menu.at = (menu.at + count - 1) % count,
        Key::Home => menu.at = 0,
        Key::End => menu.at = count - 1,
        Key::Enter => {
            let at = menu.at;
            choose(state, at, tx);
        }
        // A letter puts the menu away and goes on to the search field.
        Key::Text(_) => {
            state.menu = None;
            return false;
        }
        _ => state.menu = None,
    }
    true
}

fn open_menu(state: &mut State) {
    state.menu = Some(Menu { at: 0 });
}

/// Does what the menu entry at `index` says and puts the menu away.
pub fn choose(state: &mut State, index: usize, tx: &mut Tx) {
    state.menu = None;
    let Some(act) = state.menu_items().get(index).map(|item| item.act) else {
        return;
    };
    match act {
        Act::Put(put) => state.put(put),
        Act::View => state.toggle_peek(),
        Act::OpenLink => open_link(state, tx),
        Act::Pin => state.toggle_pin(),
        Act::Delete => state.delete(),
        Act::Show(filter) => state.set_filter(filter),
        Act::Pause => state.toggle_pause(),
        Act::Options => open_options(state, tx),
    }
}

pub fn open_link(state: &mut State, tx: &mut Tx) {
    if let Some(address) = state.link() {
        feed::send(feed::Command::Open(address.to_string()));
        tx.close_window();
    }
}

/// The options are another window; this one makes way for it.
pub fn open_options(_: &mut State, tx: &mut Tx) {
    feed::send(feed::Command::Options);
    tx.close_window();
}

pub fn on_query_change(state: &mut State) {
    state.note = None;
    state.peek = false;
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

/// A right click: the row is selected and the menu opens for it.
pub fn press_clip_menu(state: &mut State, id: i64, tx: &mut Tx) {
    press_clip(state, id, tx);
    open_menu(state);
}

/// A second press on a row pastes it. Not while the list is about to be
/// replaced: the row under the pointer may not be in the next one.
pub fn press_paste(state: &mut State) {
    if !state.rehome {
        state.put(Put::Paste(PasteMode::Original));
    }
}

pub fn press_menu_button(state: &mut State, tx: &mut Tx) {
    snap(tx);
    match state.menu {
        Some(_) => state.menu = None,
        None => open_menu(state),
    }
}

pub fn press_menu_item(state: &mut State, index: usize, tx: &mut Tx) {
    snap(tx);
    choose(state, index, tx);
}

pub fn hover_menu_item(state: &mut State, index: usize) {
    if let Some(menu) = state.menu.as_mut() {
        menu.at = index;
    }
}

pub fn close_menu(state: &mut State, tx: &mut Tx) {
    snap(tx);
    state.menu = None;
}

pub fn press_undo(state: &mut State, tx: &mut Tx) {
    snap(tx);
    state.undo_delete();
}

pub fn press_resume(state: &mut State, tx: &mut Tx) {
    snap(tx);
    if state.paused() {
        state.toggle_pause();
    }
}

pub fn press_show_all(state: &mut State, tx: &mut Tx) {
    snap(tx);
    state.set_filter(Filter::All);
}

pub fn press_back(state: &mut State, tx: &mut Tx) {
    snap(tx);
    state.peek = false;
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
