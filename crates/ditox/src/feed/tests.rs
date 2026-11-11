//! The window's feed against a real daemon: its store, its socket and its
//! request loop, with a file for a clipboard and no desktop.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use ditox_rpc::{Client, Kind};
use ditoxd::app::{App, NoDesktop};
use ditoxd::clipboard::Clipboard;
use ditoxd::config::Config;
use ditoxd::server;

use super::*;

struct Daemon {
    root: PathBuf,
    socket: PathBuf,
}

impl Daemon {
    fn start(label: &str) -> Daemon {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "ditox-feed-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let daemon = Daemon {
            socket: root.join("run/rpc.sock"),
            root,
        };
        daemon.listen();
        daemon
    }

    fn listen(&self) {
        let listener = server::listen(&self.socket).unwrap();
        let (requests, queue) = mpsc::channel();
        server::accept(listener, requests);
        let (data, clip) = (self.path("data"), self.path("clipboard"));
        std::thread::spawn(move || {
            let mut cfg = Config::defaults(&data);
            cfg.excluded_apps.clear();
            std::fs::create_dir_all(&cfg.image_dir).unwrap();
            let app = App::open(cfg, Clipboard::mock(&clip), Box::new(NoDesktop)).unwrap();
            server::run(app, queue, None);
        });
    }

    fn path(&self, name: &str) -> String {
        self.root.join(name).to_string_lossy().into_owned()
    }

    fn client(&self) -> Client {
        Client::connect_to(&self.socket).unwrap()
    }

    fn session(&self) -> Session {
        Session::new(Some(self.socket.clone()))
    }

    fn clipboard(&self) -> String {
        std::fs::read_to_string(self.path("clipboard")).unwrap_or_default()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    crate::picture::tests::png(width, height)
}

fn lists(events: &[Event]) -> Vec<Vec<String>> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Clips { clips, .. } => {
                Some(clips.iter().map(|clip| clip.content.clone()).collect())
            }
            _ => None,
        })
        .collect()
}

fn turn(session: &mut Session, commands: Vec<Command>) -> Vec<Event> {
    session.turn(commands);
    std::mem::take(&mut session.out)
}

#[test]
fn the_list_arrives_once_and_again_only_when_the_history_changes() {
    let daemon = Daemon::start("pulse");
    let mut client = daemon.client();
    client.ingest(Kind::Text, b"first").unwrap();
    let mut session = daemon.session();

    let events = turn(&mut session, Vec::new());
    assert!(matches!(events[0], Event::Watcher(_)));
    assert_eq!(lists(&events), [["first"]]);
    assert!(
        turn(&mut session, Vec::new()).is_empty(),
        "nothing changed, nothing is said"
    );

    client.ingest(Kind::Text, b"second").unwrap();
    assert_eq!(
        lists(&turn(&mut session, Vec::new())),
        [["second", "first"]]
    );
}

#[test]
fn a_search_is_answered_with_its_own_number_and_only_the_last_one_asked() {
    let daemon = Daemon::start("search");
    let mut client = daemon.client();
    for text in ["cargo test", "cargo build", "nix build"] {
        client.ingest(Kind::Text, text.as_bytes()).unwrap();
    }
    let mut session = daemon.session();
    turn(&mut session, Vec::new());

    let query = |query: &str, generation| Command::Query {
        query: query.into(),
        filter: Filter::All,
        generation,
    };
    let events = turn(
        &mut session,
        vec![query("c", 1), query("ca", 2), query("cargo", 3)],
    );
    assert_eq!(
        lists(&events).len(),
        1,
        "three keystrokes in one batch are one search"
    );
    let Some(Event::Clips {
        generation,
        searching,
        clips,
    }) = events
        .into_iter()
        .find(|event| matches!(event, Event::Clips { .. }))
    else {
        panic!("a list")
    };
    assert_eq!((generation, searching), (3, true));
    assert_eq!(clips.len(), 2);
    assert!(clips.iter().all(|clip| clip.content.contains("cargo")));
}

#[test]
fn the_whole_clip_and_the_picture_come_when_asked_and_a_missing_one_is_said_once() {
    let daemon = Daemon::start("want");
    let mut client = daemon.client();
    let long = "long line of text\n".repeat(200);
    let text = client
        .ingest(Kind::Text, long.as_bytes())
        .unwrap()
        .id
        .unwrap();
    let image = client
        .ingest(Kind::Image, &png(4000, 1000))
        .unwrap()
        .id
        .unwrap();
    let mut session = daemon.session();
    let listed = lists(&turn(&mut session, Vec::new()));
    assert_eq!(
        listed[0][1].chars().count(),
        400,
        "the list brings the start only"
    );

    let want = |id, image| Command::Want(Some(Wanted { id, image }));
    let events = turn(&mut session, vec![want(text, false), want(image, true)]);
    assert!(
        matches!(&events[..], [Event::Picture { id, picture }]
            if *id == image && [(1280, 320), (2048, 512)].contains(&(picture.width, picture.height))),
        "only the last clip asked for is fetched, and its picture comes at preview size"
    );
    assert!(
        turn(&mut session, Vec::new()).is_empty(),
        "and it is fetched once"
    );

    let events = turn(&mut session, vec![want(text, false)]);
    assert!(matches!(&events[..], [Event::Detail(entry)] if entry.content == long));

    let events = turn(&mut session, vec![want(9999, false)]);
    assert!(matches!(
        &events[..],
        [Event::Unavailable(9999, Gone::Missing)]
    ));
    let events = turn(&mut session, vec![want(text, true)]);
    assert!(
        matches!(&events[..], [Event::Unavailable(id, Gone::Missing)] if *id == text),
        "text has no picture"
    );

    let mut broken = png(64, 64);
    broken.truncate(60);
    let broken = client.ingest(Kind::Image, &broken).unwrap().id.unwrap();
    let events = turn(&mut session, vec![want(broken, true)]);
    assert!(
        events.iter().any(
            |event| matches!(event, Event::Unavailable(id, Gone::Unreadable) if *id == broken)
        ),
        "a file that is not a picture is said, not retried"
    );
    assert!(turn(&mut session, Vec::new()).is_empty());
}

#[test]
fn paste_pin_delete_and_pause_reach_the_daemon_and_the_list_follows() {
    let daemon = Daemon::start("act");
    let mut client = daemon.client();
    let keep = client.ingest(Kind::Text, b"keep me").unwrap().id.unwrap();
    let drop = client.ingest(Kind::Text, b"drop me").unwrap().id.unwrap();
    let mut session = daemon.session();
    turn(&mut session, Vec::new());

    let events = turn(
        &mut session,
        vec![Command::Paste {
            id: keep,
            mode: PasteMode::Original,
            target: None,
        }],
    );
    assert!(matches!(events[0], Event::Pasted));
    assert_eq!(daemon.clipboard(), "keep me");
    assert_eq!(
        lists(&events),
        [["keep me", "drop me"]],
        "a pasted clip moves to the front"
    );

    let events = turn(
        &mut session,
        vec![Command::Paste {
            id: 9999,
            mode: PasteMode::Plain,
            target: None,
        }],
    );
    assert!(matches!(&events[0], Event::PasteRefused(reason) if reason == "missing"));

    session.undo_time = Duration::ZERO;
    let events = turn(&mut session, vec![Command::Delete { id: drop }]);
    assert_eq!(lists(&events), [["keep me"]]);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Deleted(id) if *id == drop))
    );
    assert!(matches!(client.get(drop), Err(RpcError::Remote { .. })));

    client
        .call::<serde_json::Value>(
            "entries.sticky",
            serde_json::json!({ "id": keep, "sticky": true }),
        )
        .unwrap();
    turn(&mut session, vec![Command::Pin { id: keep, on: true }]);
    let pinned = client.get(keep).unwrap();
    assert!(pinned.favorite && pinned.sticky);
    turn(
        &mut session,
        vec![Command::Pin {
            id: keep,
            on: false,
        }],
    );
    let unpinned = client.get(keep).unwrap();
    assert!(
        !unpinned.favorite && !unpinned.sticky,
        "unpinning clears both ways of pinning"
    );

    let events = turn(&mut session, vec![Command::Pause(true)]);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Watcher(watcher) if watcher.paused))
    );
    let events = turn(&mut session, vec![Command::Pause(false)]);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Watcher(watcher) if !watcher.paused))
    );
}

#[test]
fn a_daemon_that_is_gone_is_said_once_and_the_list_returns_with_it() {
    let daemon = Daemon::start("offline");
    let mut session = Session::new(Some(Path::new(&daemon.path("nowhere.sock")).to_path_buf()));
    let events = turn(&mut session, Vec::new());
    assert!(matches!(&events[..], [Event::Offline(_)]));
    assert!(
        turn(&mut session, Vec::new()).is_empty(),
        "still gone, said once"
    );

    session.socket = Some(daemon.socket.clone());
    daemon.client().ingest(Kind::Text, b"back").unwrap();
    assert_eq!(lists(&turn(&mut session, Vec::new())), [["back"]]);
}

#[test]
fn a_failure_that_repeats_every_pulse_is_said_once_until_something_works_again() {
    let daemon = Daemon::start("note");
    let mut session = daemon.session();
    for _ in 0..3 {
        session.note_once("database: locked".into());
    }
    session.note_once("database: malformed".into());
    let notes: Vec<String> = std::mem::take(&mut session.out)
        .into_iter()
        .filter_map(|event| match event {
            Event::Note(note) => Some(note),
            _ => None,
        })
        .collect();
    assert_eq!(notes, ["database: locked", "database: malformed"]);

    daemon.client().ingest(Kind::Text, b"works").unwrap();
    turn(&mut session, Vec::new());
    session.note_once("database: malformed".into());
    assert!(
        matches!(&session.out[..], [Event::Note(_)]),
        "after a good list the same failure is news again"
    );
}

#[test]
fn a_deleted_clip_leaves_the_list_at_once_and_the_history_only_when_it_cannot_come_back() {
    let daemon = Daemon::start("undo");
    let mut client = daemon.client();
    let ids: Vec<i64> = ["one", "two", "three"]
        .iter()
        .map(|text| {
            client
                .ingest(Kind::Text, text.as_bytes())
                .unwrap()
                .id
                .unwrap()
        })
        .collect();
    let mut session = daemon.session();
    session.undo_time = Duration::from_secs(3600);
    turn(&mut session, Vec::new());

    let events = turn(&mut session, vec![Command::Delete { id: ids[1] }]);
    assert!(
        lists(&events).is_empty(),
        "the window already took the row off"
    );
    assert_eq!(
        client.get(ids[1]).unwrap().content,
        "two",
        "nothing is deleted yet"
    );

    client.ingest(Kind::Text, b"four").unwrap();
    assert_eq!(
        lists(&turn(&mut session, Vec::new())),
        [["four", "three", "one"]],
        "a list that arrives meanwhile does not bring the row back"
    );

    let events = turn(&mut session, vec![Command::Restore { id: ids[1] }]);
    assert_eq!(lists(&events), [["four", "three", "two", "one"]]);

    let events = turn(
        &mut session,
        vec![
            Command::Delete { id: ids[0] },
            Command::Delete { id: ids[2] },
            Command::Restore { id: ids[2] },
            Command::Finish,
        ],
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Deleted(id) if *id == ids[0])),
        "a closing window deletes what was waiting"
    );
    assert_eq!(lists(&events), [["four", "three", "two"]]);
    assert!(matches!(client.get(ids[0]), Err(RpcError::Remote { .. })));
    assert_eq!(client.get(ids[2]).unwrap().content, "three");
}
