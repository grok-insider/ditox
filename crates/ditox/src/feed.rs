use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ditox_rpc::{
    BLOB_MISSING, Client, ENTRY_NOT_FOUND, Entry, Filter, METHOD_NOT_FOUND, NOT_AN_IMAGE,
    PasteMode, RpcError, WatcherStatus,
};
use rune::Waker;

use crate::picture::{self, Picture};

pub const LIST_LIMIT: u32 = 300;
const PULSE: Duration = Duration::from_millis(250);
/// The wait between turns while rows still wait for their pictures.
const BUSY_PULSE: Duration = Duration::from_millis(8);
const THUMBS_AT_ONCE: usize = 2;
/// How long a deleted clip can still be brought back.
pub const UNDO_TIME: Duration = Duration::from_secs(6);
/// How long a closing window waits for the feed to do what was pending.
const FINISH_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Query {
        query: String,
        filter: Filter,
        generation: u64,
    },
    /// The clip whose whole text or image the window is waiting for, if any.
    Want(Option<Wanted>),
    /// The image clips whose rows are in view and have no small picture yet,
    /// and the pixels each has room for. It replaces the list asked for before.
    Thumbs {
        ids: Vec<i64>,
        fit: (u32, u32),
    },
    Paste {
        id: i64,
        mode: PasteMode,
        target: Option<String>,
    },
    Pin {
        id: i64,
        on: bool,
    },
    /// Take the clip off the list now and delete it once the time to undo has passed.
    Delete {
        id: i64,
    },
    /// Undo a `Delete` that has not happened yet.
    Restore {
        id: i64,
    },
    /// Open a web address with the desktop's handler.
    Open(String),
    /// Open the options window.
    Options,
    Pause(bool),
    /// The window is closing: do now what was waiting.
    Finish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub id: i64,
    pub image: bool,
}

/// Why a clip's content cannot be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gone {
    /// The clip, or the file behind its image, no longer exists.
    Missing,
    /// The file is there and is not a picture the window can decode.
    Unreadable,
}

pub enum Event {
    /// The list that answers the query numbered `generation`.
    Clips {
        generation: u64,
        searching: bool,
        clips: Vec<Entry>,
    },
    Offline(String),
    Watcher(WatcherStatus),
    Detail(Entry),
    Picture {
        id: i64,
        picture: Picture,
    },
    /// The small picture of an image clip's row.
    Thumb {
        id: i64,
        picture: Picture,
    },
    /// That row gets no picture, and asking again will not help.
    NoThumb(i64),
    /// The clip cannot be shown, and asking again will not help.
    Unavailable(i64, Gone),
    /// The time to undo this delete is over; the clip is gone.
    Deleted(i64),
    Note(String),
    Pasted,
    PasteRefused(String),
    /// Whether the compositor can show a picture large on the screen.
    Preview(bool),
}

struct Bridge {
    commands: Sender<Command>,
    inbox: Mutex<Vec<Event>>,
    wake: Mutex<Option<Waker>>,
    finished: (Mutex<bool>, Condvar),
}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

pub fn start() {
    let (commands, queue) = mpsc::channel();
    if BRIDGE
        .set(Bridge {
            commands,
            inbox: Mutex::new(Vec::new()),
            wake: Mutex::new(None),
            finished: (Mutex::new(false), Condvar::new()),
        })
        .is_ok()
    {
        let _ = std::thread::Builder::new()
            .name("ditox-rpc".into())
            .spawn(move || run(queue));
    }
}

pub fn bind(waker: Waker) {
    if let Some(bridge) = BRIDGE.get() {
        *bridge
            .wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(waker.clone());
        waker();
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

/// What the window asked the daemon for in this test so far.
#[cfg(test)]
pub fn take_sent() -> Vec<Command> {
    SENT.with(|sent| std::mem::take(&mut *sent.borrow_mut()))
}

/// Called as the window closes. Waits, briefly, until the feed has done what
/// was still pending: a delete inside its time to undo, a link to open.
pub fn finish() {
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

pub fn drain() -> Vec<Event> {
    match BRIDGE.get() {
        Some(bridge) => std::mem::take(
            &mut *bridge
                .inbox
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        ),
        None => Vec::new(),
    }
}

pub(crate) fn push(event: Event) {
    let Some(bridge) = BRIDGE.get() else { return };
    bridge
        .inbox
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(event);
    let wake = bridge
        .wake
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(wake) = wake {
        wake();
    }
}

enum Fetched {
    Text(Box<Entry>),
    Image(Vec<u8>),
}

fn thumbed(id: i64, picture: Result<Picture, String>) -> Event {
    match picture {
        Ok(picture) => Event::Thumb { id, picture },
        Err(_) => Event::NoThumb(id),
    }
}

fn pictured(id: i64, picture: Result<Picture, String>) -> Event {
    match picture {
        Ok(picture) => Event::Picture { id, picture },
        Err(_) => Event::Unavailable(id, Gone::Unreadable),
    }
}

struct Session {
    client: Option<Client>,
    query: String,
    filter: Filter,
    generation: u64,
    revision: Option<u64>,
    watcher: Option<WatcherStatus>,
    offline: bool,
    wanted: Option<Wanted>,
    last_note: Option<String>,
    /// Clips the window deleted, and when each really goes.
    doomed: Vec<(i64, Instant)>,
    undo_time: Duration,
    /// Where the daemon listens; `None` is the session's own socket.
    socket: Option<PathBuf>,
    /// Where fetched pictures go to be decoded. Without it they are decoded here.
    pictures: Option<Sender<picture::Job>>,
    /// The rows still waiting for their small picture, nearest first.
    thumbs: VecDeque<i64>,
    thumb_fit: (u32, u32),
    thumb_jobs: Option<Sender<picture::ThumbJob>>,
    /// Small pictures fetched and not made yet. Each holds a whole image file.
    thumbs_in_flight: Arc<AtomicUsize>,
    /// What happened, for the window, in order.
    out: Vec<Event>,
}

impl Session {
    fn new(socket: Option<PathBuf>) -> Session {
        Session {
            client: None,
            query: String::new(),
            filter: Filter::All,
            generation: 0,
            revision: None,
            watcher: None,
            offline: false,
            wanted: None,
            last_note: None,
            doomed: Vec::new(),
            undo_time: UNDO_TIME,
            socket,
            pictures: None,
            thumbs: VecDeque::new(),
            thumb_fit: (1, 1),
            thumb_jobs: None,
            thumbs_in_flight: Arc::new(AtomicUsize::new(0)),
            out: Vec::new(),
        }
    }

    /// Everything a batch of commands asks for, then the pulse, then the
    /// clip the window waits for. Several `Want`s in one batch fetch only the
    /// last: holding an arrow key does not download every picture it passes.
    fn turn(&mut self, commands: impl IntoIterator<Item = Command>) {
        for command in commands {
            self.handle(command);
        }
        self.reap(Instant::now());
        self.sync();
        self.fetch_wanted();
        self.fetch_thumb();
    }

    /// Whether there is more to do before the next pulse is due.
    fn busy(&self) -> bool {
        !self.thumbs.is_empty()
    }

    /// Fetches one row's picture and hands it on to be made small. At most
    /// `THUMBS_AT_ONCE` wait to be made: each is a whole image file in memory.
    fn fetch_thumb(&mut self) {
        if self.offline {
            // The window asks again when the list is back.
            self.thumbs.clear();
            return;
        }
        if self.thumbs_in_flight.load(Ordering::Acquire) >= THUMBS_AT_ONCE {
            return;
        }
        let Some(id) = self.thumbs.pop_front() else {
            return;
        };
        match self.call(|client| client.image(id)) {
            Ok(bytes) => match &self.thumb_jobs {
                Some(jobs) => {
                    self.thumbs_in_flight.fetch_add(1, Ordering::AcqRel);
                    let fit = self.thumb_fit;
                    if jobs.send(picture::ThumbJob { id, bytes, fit }).is_err() {
                        self.thumbs_in_flight.fetch_sub(1, Ordering::AcqRel);
                    }
                }
                None => self.out.push(thumbed(
                    id,
                    picture::fit_within(&bytes, self.thumb_fit.0, self.thumb_fit.1),
                )),
            },
            Err(RpcError::Transport(_)) => self.thumbs.clear(),
            Err(_) => self.out.push(Event::NoThumb(id)),
        }
    }

    fn call<T>(
        &mut self,
        body: impl FnOnce(&mut Client) -> Result<T, RpcError>,
    ) -> Result<T, RpcError> {
        if self.client.is_none() {
            self.client = Some(match &self.socket {
                Some(path) => Client::connect_to(path)?,
                None => Client::connect()?,
            });
        }
        let Some(client) = self.client.as_mut() else {
            return Err(RpcError::Transport("not connected".into()));
        };
        let result = body(client);
        if matches!(result, Err(RpcError::Transport(_))) {
            self.client = None;
        }
        result
    }

    fn act(&mut self, body: impl FnOnce(&mut Client) -> Result<(), RpcError>) {
        if let Err(err) = self.call(body) {
            self.out.push(Event::Note(err.to_string()));
        }
        self.revision = None;
    }

    /// Deletes the clips whose time to undo ended by `now`.
    fn reap(&mut self, now: Instant) {
        let due: Vec<i64> = self
            .doomed
            .iter()
            .filter(|(_, at)| *at <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            self.doomed.retain(|(doomed, _)| *doomed != id);
            self.act(|client| client.delete(id));
            self.out.push(Event::Deleted(id));
        }
    }

    /// A failure that repeats every pulse is said once.
    fn note_once(&mut self, message: String) {
        if self.last_note.as_deref() != Some(message.as_str()) {
            self.last_note = Some(message.clone());
            self.out.push(Event::Note(message));
        }
    }

    /// Fetches what the window waits for. A clip that is gone is reported and
    /// dropped; any other failure is tried again on the next pulse.
    fn fetch_wanted(&mut self) {
        let Some(Wanted { id, image }) = self.wanted else {
            return;
        };
        let fetched = if image {
            self.call(|client| client.image(id)).map(Fetched::Image)
        } else {
            self.call(|client| client.get(id))
                .map(|entry| Fetched::Text(Box::new(entry)))
        };
        match fetched {
            Ok(fetched) => {
                self.wanted = None;
                match (fetched, &self.pictures) {
                    (Fetched::Text(entry), _) => self.out.push(Event::Detail(*entry)),
                    (Fetched::Image(bytes), Some(pictures)) => {
                        let _ = pictures.send(picture::Job { id, bytes });
                    }
                    (Fetched::Image(bytes), None) => self
                        .out
                        .push(pictured(id, picture::fit(&bytes, picture::long_side()))),
                }
            }
            Err(RpcError::Remote {
                code: ENTRY_NOT_FOUND | NOT_AN_IMAGE | BLOB_MISSING,
                ..
            }) => {
                self.wanted = None;
                self.out.push(Event::Unavailable(id, Gone::Missing));
            }
            Err(RpcError::Transport(_)) => {}
            Err(err) => self.note_once(err.to_string()),
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Query {
                query,
                filter,
                generation,
            } => {
                self.query = query;
                self.filter = filter;
                self.generation = generation;
                self.revision = None;
            }
            Command::Want(wanted) => self.wanted = wanted,
            Command::Thumbs { ids, fit } => {
                self.thumbs = ids.into();
                self.thumb_fit = fit;
            }
            Command::Paste { id, mode, target } => {
                match self.call(|client| client.paste(id, mode, target.as_deref())) {
                    Ok(result) if result.pasted => self.out.push(Event::Pasted),
                    Ok(result) => self.out.push(Event::PasteRefused(result.reason)),
                    Err(err) => {
                        self.revision = None;
                        self.out.push(Event::PasteRefused(err.to_string()));
                    }
                }
            }
            Command::Pin { id, on: true } => self.act(|client| client.favorite(id, true)),
            Command::Pin { id, on: false } => self.act(|client| {
                client.favorite(id, false)?;
                client.sticky(id, false)
            }),
            Command::Delete { id } => {
                self.doomed.retain(|(doomed, _)| *doomed != id);
                self.doomed.push((id, Instant::now() + self.undo_time));
            }
            Command::Restore { id } => {
                self.doomed.retain(|(doomed, _)| *doomed != id);
                self.revision = None;
            }
            Command::Open(address) => open(&address),
            Command::Options => open_options(),
            Command::Finish => {
                for (_, at) in &mut self.doomed {
                    *at = Instant::now();
                }
            }
            Command::Pause(true) => self.act(|client| client.pause(0)),
            Command::Pause(false) => self.act(|client| client.resume()),
        }
    }

    fn sync(&mut self) {
        let pulse = match self.call(|client| client.pulse()) {
            Ok(pulse) => pulse,
            Err(err) => {
                if !self.offline {
                    self.out.push(Event::Offline(explain(&err)));
                }
                self.offline = true;
                self.revision = None;
                self.watcher = None;
                return;
            }
        };
        self.offline = false;
        if self.watcher.as_ref() != Some(&pulse.watcher) {
            self.watcher = Some(pulse.watcher.clone());
            self.out.push(Event::Watcher(pulse.watcher));
        }
        if self.revision != Some(pulse.revision) {
            let (query, filter) = (self.query.clone(), self.filter);
            match self.call(|client| client.list(&query, filter, LIST_LIMIT, true)) {
                Ok(mut clips) => {
                    clips.retain(|clip| !self.doomed.iter().any(|(doomed, _)| *doomed == clip.id));
                    self.last_note = None;
                    self.revision = Some(pulse.revision);
                    self.out.push(Event::Clips {
                        generation: self.generation,
                        searching: !query.is_empty(),
                        clips,
                    });
                }
                Err(RpcError::Transport(_)) => {}
                Err(err) => self.note_once(err.to_string()),
            }
        }
    }
}

/// Hands a web address to the desktop. Anything else is not opened.
fn open(address: &str) {
    if crate::model::link(address).is_none() {
        return;
    }
    let _ = std::process::Command::new("xdg-open")
        .arg(address)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Starts the options window: this program again, asked for its other window.
fn open_options() {
    let Ok(program) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(program)
        .arg("options")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn explain(err: &RpcError) -> String {
    match err {
        RpcError::Remote { code, .. } if *code == METHOD_NOT_FOUND => {
            "The running ditoxd is older than this window. Restart it: systemctl --user restart ditox".into()
        }
        other => other.to_string(),
    }
}

fn run(queue: Receiver<Command>) {
    let mut session = Session::new(None);
    session.pictures = Some(picture::worker(|id, picture| push(pictured(id, picture))));
    let in_flight = session.thumbs_in_flight.clone();
    session.thumb_jobs = Some(picture::thumb_worker(move |id, picture| {
        in_flight.fetch_sub(1, Ordering::AcqRel);
        push(thumbed(id, picture));
    }));
    let mut commands = Vec::new();
    loop {
        let closing = commands.contains(&Command::Finish);
        session.turn(commands.drain(..));
        for event in session.out.drain(..) {
            push(event);
        }
        if closing {
            finished();
        }
        let wait = if session.busy() { BUSY_PULSE } else { PULSE };
        match queue.recv_timeout(wait) {
            Ok(command) => {
                commands.push(command);
                commands.extend(queue.try_iter());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
