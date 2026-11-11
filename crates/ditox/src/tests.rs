use rune::{Env, Key, KeyEvent, Mods, Runtime, Tx};
use std::time::Duration;

use super::*;

fn runtime(state: State) -> Runtime<State> {
    let mut runtime = Runtime::new(
        state,
        ui::view,
        SIZE,
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    runtime
}

thread_local! {
    static ARRIVING: std::cell::RefCell<Vec<feed::Event>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Hands the window an event through its real turn (`step`), then lets it settle.
fn deliver(runtime: &mut Runtime<State>, event: feed::Event) {
    ARRIVING.with(|arriving| arriving.borrow_mut().push(event));
    turn(runtime);
}

fn turn(runtime: &mut Runtime<State>) {
    runtime.poll = Some(|state: &mut State, _, tx: &mut Tx| {
        let events = ARRIVING.with(|arriving| std::mem::take(&mut *arriving.borrow_mut()));
        step(state, events, tx)
    });
    for _ in 0..30 {
        runtime.frame(1.0 / 60.0);
    }
}

fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    }
}

fn pastes(sent: &[feed::Command]) -> Vec<i64> {
    sent.iter()
        .filter_map(|command| match command {
            feed::Command::Paste { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

fn wants(sent: &[feed::Command]) -> Vec<Option<feed::Wanted>> {
    sent.iter()
        .filter_map(|command| match command {
            feed::Command::Want(wanted) => Some(*wanted),
            _ => None,
        })
        .collect()
}

fn clips_event(runtime: &Runtime<State>, clips: Vec<Entry>) -> feed::Event {
    feed::Event::Clips {
        generation: runtime.state.generation,
        searching: !runtime.state.query.text.trim().is_empty(),
        clips,
    }
}

fn arrive(runtime: &mut Runtime<State>, clips: Vec<Entry>) {
    let event = clips_event(runtime, clips);
    deliver(runtime, event);
}

fn many(count: i64) -> Vec<Entry> {
    let template = fixture::clips().remove(5);
    (0..count)
        .map(|index| Entry {
            id: 1000 + index,
            ..template.clone()
        })
        .collect()
}

/// The vertical centre of every row drawn with the selection tint.
fn tinted(runtime: &mut Runtime<State>) -> Vec<f32> {
    for _ in 0..30 {
        runtime.frame(1.0 / 60.0);
    }
    let tint = theme::palette(true).selected;
    let close = |a: f32, b: f32| (a - b).abs() < 0.0005;
    runtime
        .elements()
        .into_iter()
        .filter(|element| {
            close(element.fill.r, tint.r)
                && close(element.fill.g, tint.g)
                && close(element.fill.b, tint.b)
                && close(element.fill.a, tint.a)
        })
        .map(|element| element.rect[1] + element.rect[3] / 2.0)
        .collect()
}

fn row_y(runtime: &Runtime<State>, id: i64) -> f32 {
    runtime
        .find(&format!("clip-{id}"))
        .unwrap_or_else(|| panic!("clip {id} is drawn"))
        .1
}

fn texts(runtime: &Runtime<State>) -> Vec<String> {
    runtime
        .elements()
        .into_iter()
        .filter_map(|element| element.text)
        .collect()
}

fn says(runtime: &Runtime<State>, needle: &str) -> bool {
    texts(runtime).iter().any(|line| line.contains(needle))
}

fn key(runtime: &mut Runtime<State>, key: Key, mods: Mods) {
    runtime.key(KeyEvent { key, mods });
    runtime.frame(0.0);
}

fn press(runtime: &mut Runtime<State>, element: &str) {
    let (x, y) = runtime
        .find(element)
        .unwrap_or_else(|| panic!("{element} is on screen"));
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(0.0);
}

#[test]
fn the_history_shows_pinned_then_recent_with_the_selected_clip_previewed() {
    let runtime = runtime(fixture::ready());
    for needle in [
        "Pinned",
        "Today",
        "Yesterday",
        "Last 7 days",
        "ssh friend@station.example",
        "pub fn capture(&mut self)",
        "zed",
        "13 lines",
        "12 clips",
        "Watching",
    ] {
        assert!(says(&runtime, needle), "{needle}");
    }
    for element in [
        "search", "filters", "list", "preview", "footer", "status", "clip-3",
    ] {
        assert!(runtime.find(element).is_some(), "{element}");
    }
    for hint in ["Paste", "Pin", "Delete", "Keys"] {
        assert!(says(&runtime, hint), "{hint}");
    }
    assert!(!says(&runtime, "Unpin"));
    assert!(!says(&runtime, "Recent"));
}

#[test]
fn arrows_move_the_selection_and_stop_at_the_ends() {
    let mut runtime = runtime(fixture::ready());
    key(&mut runtime, Key::Down, Mods::default());
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(4));
    key(&mut runtime, Key::Up, Mods::default());
    key(&mut runtime, Key::Up, Mods::default());
    key(&mut runtime, Key::Up, Mods::default());
    key(&mut runtime, Key::Up, Mods::default());
    assert_eq!(runtime.state.selected, 0);
    assert!(says(&runtime, "Unpin"));
    key(&mut runtime, Key::PageDown, Mods::default());
    key(&mut runtime, Key::PageDown, Mods::default());
    assert_eq!(runtime.state.selected, 11);
}

#[test]
fn moving_past_the_fold_scrolls_the_selected_row_into_view() {
    let mut state = fixture::ready();
    for index in 0..40 {
        let mut clip = state.clips[5].clone();
        clip.id = 100 + index;
        state.clips.push(clip);
    }
    let mut runtime = runtime(state);
    assert!(
        runtime.find("clip-130").is_none(),
        "rows below the fold are not built"
    );
    for _ in 0..5 {
        key(&mut runtime, Key::PageDown, Mods::default());
        runtime.frame(0.5);
    }
    let id = runtime.state.current().map(|clip| clip.id).unwrap();
    assert_eq!(runtime.state.selected, 42);
    for _ in 0..60 {
        runtime.frame(1.0 / 60.0);
    }
    let (_, y) = runtime
        .find(&format!("clip-{id}"))
        .expect("the selected row is drawn");
    assert!(y > 52.0 && y < SIZE.1 - 40.0, "row at {y}");
}

#[test]
fn new_results_start_at_the_top_with_the_best_match_selected() {
    let mut state = fixture::ready();
    state.clips.extend(many(120));
    let mut runtime = runtime(state);
    for _ in 0..6 {
        key(&mut runtime, Key::PageDown, Mods::default());
    }
    for _ in 0..30 {
        runtime.frame(1.0 / 60.0);
    }
    assert!(
        runtime.find("clip-1").is_none(),
        "the list is scrolled away from its start"
    );

    runtime.state.query = TextEdit::new("cargo");
    runtime.state.query.focused = true;
    on_query_change(&mut runtime.state);
    let results: Vec<Entry> = many(120).into_iter().rev().take(81).collect();
    let first = results[0].id;
    arrive(&mut runtime, results);

    assert_eq!(runtime.state.selected, 0);
    let (_, y) = runtime
        .find(&format!("clip-{first}"))
        .expect("the best match is drawn");
    assert!(
        (52.0..100.0).contains(&y),
        "the best match is the first row; it is at {y}"
    );
}

#[test]
fn results_that_reorder_the_same_clips_still_start_at_the_top() {
    let mut runtime = runtime(State {
        clips: many(120),
        selected: 0,
        ..fixture::ready()
    });
    arrive(&mut runtime, many(120));
    for _ in 0..3 {
        key(&mut runtime, Key::PageDown, Mods::default());
    }
    for step in 0..4 {
        runtime.state.query = TextEdit::new("cargo");
        runtime.state.query.focused = true;
        on_query_change(&mut runtime.state);
        let mut results = many(120);
        results.rotate_left(37 + step * 11);
        results.truncate(100 - step * 6);
        let first = results[0].id;
        arrive(&mut runtime, results);
        let y = row_y(&runtime, first);
        assert!(
            (52.0..100.0).contains(&y),
            "step {step}: the best match is at {y}"
        );
        assert_eq!(tinted(&mut runtime), [y], "step {step}");
    }
}

#[test]
fn an_answer_to_an_older_search_is_dropped() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    runtime.state.query = TextEdit::new("car");
    on_query_change(&mut runtime.state);
    let older = clips_event(&runtime, many(40));
    runtime.state.query = TextEdit::new("cargo");
    on_query_change(&mut runtime.state);

    deliver(&mut runtime, older);
    assert_eq!(
        runtime.state.clips.len(),
        12,
        "the list waits for the answer to the last search"
    );

    let mut newest = many(40);
    newest.rotate_left(7);
    let first = newest[0].id;
    arrive(&mut runtime, newest);
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(first));
    assert_eq!(tinted(&mut runtime), [row_y(&runtime, first)]);
}

#[test]
fn enter_before_the_results_arrive_pastes_the_best_match_once_they_do() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    runtime.state.query = TextEdit::new("cargo");
    runtime.state.query.focused = true;
    on_query_change(&mut runtime.state);
    feed::take_sent();
    key(&mut runtime, Key::Enter, Mods::default());
    assert!(
        pastes(&feed::take_sent()).is_empty(),
        "nothing is pasted from the list the search replaced"
    );
    let results = many(5);
    let best = results[0].id;
    arrive(&mut runtime, results);
    assert_eq!(pastes(&feed::take_sent()), [best]);
    assert!(runtime.state.pasting);
}

#[test]
fn enter_without_a_daemon_is_not_remembered_for_when_it_comes_back() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    deliver(&mut runtime, feed::Event::Offline("no socket".into()));
    feed::take_sent();
    key(&mut runtime, Key::Enter, Mods::default());
    arrive(&mut runtime, fixture::clips());
    assert!(pastes(&feed::take_sent()).is_empty());
    assert!(!runtime.state.pasting);

    key(&mut runtime, Key::Enter, Mods::default());
    assert_eq!(
        pastes(&feed::take_sent()),
        [3],
        "Enter now pastes the selected clip"
    );
}

#[test]
fn an_enter_waiting_for_results_is_dropped_when_the_selection_moves_or_time_passes() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    on_query_change(&mut runtime.state);
    key(&mut runtime, Key::Enter, Mods::default());
    key(&mut runtime, Key::Down, Mods::default());
    arrive(&mut runtime, many(5));
    assert!(pastes(&feed::take_sent()).is_empty());

    on_query_change(&mut runtime.state);
    key(&mut runtime, Key::Enter, Mods::default());
    std::thread::sleep(PASTE_WAIT + Duration::from_millis(50));
    arrive(&mut runtime, many(5));
    assert!(pastes(&feed::take_sent()).is_empty());
}

#[test]
fn a_press_on_the_preview_or_a_notice_leaves_the_search_focused() {
    let mut runtime = runtime(fixture::ready());
    press(&mut runtime, "preview");
    press(&mut runtime, "footer");
    assert!(runtime.state.query.focused);

    let mut empty = super::tests::runtime(State {
        clips: Vec::new(),
        ..fixture::ready()
    });
    press(&mut empty, "empty");
    assert!(empty.state.query.focused);
}

#[test]
fn a_cut_clip_is_asked_for_once_and_says_it_is_only_the_start_until_it_arrives() {
    let mut state = fixture::ready();
    state.clips[2].byte_len = 50_000;
    let mut runtime = runtime(state);
    feed::take_sent();
    turn(&mut runtime);
    turn(&mut runtime);
    assert_eq!(
        wants(&feed::take_sent()),
        [Some(feed::Wanted {
            id: 3,
            image: false
        })]
    );
    assert!(says(&runtime, "Start only"));

    key(&mut runtime, Key::Down, Mods::default());
    turn(&mut runtime);
    assert_eq!(
        wants(&feed::take_sent()),
        [Some(feed::Wanted { id: 4, image: true })]
    );

    deliver(
        &mut runtime,
        feed::Event::Unavailable(4, feed::Gone::Missing),
    );
    assert_eq!(wants(&feed::take_sent()), [None]);
    assert!(says(&runtime, "This image is no longer stored"));
    deliver(
        &mut runtime,
        feed::Event::Unavailable(4, feed::Gone::Unreadable),
    );
    assert!(says(&runtime, "This image cannot be shown"));
    assert!(
        wants(&feed::take_sent()).is_empty(),
        "and it is not asked for again"
    );

    key(&mut runtime, Key::Down, Mods::default());
    turn(&mut runtime);
    assert!(
        wants(&feed::take_sent()).is_empty(),
        "a whole clip needs nothing more"
    );
}

#[test]
fn a_picture_too_large_to_draw_is_described_and_never_fetched() {
    let mut state = fixture::ready();
    state.selected = 3;
    state.clips[3].image_width = Some(20_000);
    state.clips[3].image_height = Some(20_000);
    let mut runtime = runtime(state);
    feed::take_sent();
    turn(&mut runtime);
    assert!(wants(&feed::take_sent()).is_empty());
    assert!(says(&runtime, "Too large to preview"));
    assert!(says(&runtime, "20000 \u{d7} 20000"));
}

#[test]
fn the_window_lets_go_of_the_oldest_pictures() {
    let mut images = Images::default();
    for id in 0..IMAGE_SLOTS as i64 + 3 {
        images.keep(id, fixture::storm(2, 2), 1024);
    }
    assert_eq!(images.count(), IMAGE_SLOTS);
    assert!(!images.contains_key(&2) && images.contains_key(&3));

    images.keep(100, fixture::storm(2, 2), IMAGE_BYTES - 2048);
    assert_eq!(
        images.count(),
        3,
        "a heavy picture pushes out all but what fits beside it"
    );
    images.keep(101, fixture::storm(2, 2), IMAGE_BYTES * 2);
    assert_eq!(
        images.count(),
        1,
        "the picture on screen stays, whatever it weighs"
    );
    assert!(images.contains_key(&101));
}

#[test]
fn a_new_copy_keeps_the_selection_on_its_clip() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    key(&mut runtime, Key::Down, Mods::default());
    key(&mut runtime, Key::Down, Mods::default());
    let chosen = runtime.state.current().map(|clip| clip.id);
    let mut clips = fixture::clips();
    clips.insert(2, many(1).remove(0));
    arrive(&mut runtime, clips);
    assert_eq!(runtime.state.current().map(|clip| clip.id), chosen);
    assert_eq!(runtime.state.selected, 5);
}

#[test]
fn the_tint_is_on_the_selected_row_and_follows_it() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    assert_eq!(tinted(&mut runtime), [row_y(&runtime, 3)]);
    key(&mut runtime, Key::Down, Mods::default());
    assert_eq!(tinted(&mut runtime), [row_y(&runtime, 4)]);
    press(&mut runtime, "clip-7");
    assert_eq!(tinted(&mut runtime), [row_y(&runtime, 7)]);

    let mut reordered = fixture::clips();
    reordered[2..].reverse();
    arrive(&mut runtime, reordered);
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(7));
    assert_eq!(tinted(&mut runtime), [row_y(&runtime, 7)]);
}

#[test]
fn going_back_to_the_first_clip_shows_the_heading_above_it() {
    let mut state = fixture::ready();
    state.clips.extend(many(60));
    let mut runtime = runtime(state);
    for _ in 0..4 {
        key(&mut runtime, Key::PageDown, Mods::default());
    }
    turn(&mut runtime);
    key(&mut runtime, Key::Home, ctrl());
    turn(&mut runtime);
    assert_eq!(runtime.state.selected, 0);
    let (_, heading) = runtime
        .find("section-Pinned")
        .expect("the heading of the first group is back");
    assert!(
        (60.0..90.0).contains(&heading),
        "the heading is inside the list, not scrolled under the search bar: {heading}"
    );
    assert!(heading < row_y(&runtime, 1));
}

#[test]
fn tab_and_a_press_change_the_filter() {
    let mut runtime = runtime(fixture::ready());
    key(&mut runtime, Key::Tab, Mods::default());
    assert_eq!(runtime.state.filter, Filter::Text);
    key(
        &mut runtime,
        Key::Tab,
        Mods {
            shift: true,
            ..Mods::default()
        },
    );
    assert_eq!(runtime.state.filter, Filter::All);
    press(&mut runtime, "filter-Images");
    assert_eq!(runtime.state.filter, Filter::Images);
    assert!(runtime.state.query.focused);
}

#[test]
fn a_press_selects_a_row_and_keeps_the_search_focused() {
    let mut runtime = runtime(fixture::ready());
    press(&mut runtime, "clip-6");
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(6));
    assert!(runtime.state.query.focused);
    assert!(says(&runtime, "cargo test --workspace"));
}

#[test]
fn pin_and_delete_act_on_the_selected_clip() {
    let mut runtime = runtime(fixture::ready());
    feed::take_sent();
    key(&mut runtime, Key::Command('p'), ctrl());
    assert!(runtime.state.clips[2].favorite);
    key(&mut runtime, Key::Command('d'), ctrl());
    assert_eq!(runtime.state.clips.len(), 11);
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(4));
    let sent = feed::take_sent();
    assert!(sent.contains(&feed::Command::Pin { id: 3, on: true }));
    assert!(sent.contains(&feed::Command::Delete { id: 3 }));
}

#[test]
fn a_deleted_clip_comes_back_where_it_was_until_the_daemon_says_it_is_gone() {
    let mut runtime = runtime(fixture::ready());
    feed::take_sent();
    key(&mut runtime, Key::Down, Mods::default());
    key(&mut runtime, Key::Delete, Mods::default());
    key(&mut runtime, Key::Command('d'), ctrl());
    assert_eq!(runtime.state.clips.len(), 10);
    assert!(says(&runtime, "Deleted 2 clips"));
    assert!(says(&runtime, "Undo"));

    key(&mut runtime, Key::Command('z'), ctrl());
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(5));
    assert_eq!(runtime.state.selected, 3);
    assert!(says(
        &runtime,
        "Deleted \u{201c}Image, 960 \u{d7} 600\u{201d}"
    ));
    press(&mut runtime, "undo-button");
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(4));
    let ids: Vec<i64> = runtime.state.clips.iter().map(|clip| clip.id).collect();
    assert_eq!(
        ids,
        fixture::clips()
            .iter()
            .map(|clip| clip.id)
            .collect::<Vec<_>>()
    );
    assert!(!says(&runtime, "Undo"));
    assert_eq!(
        feed::take_sent()
            .into_iter()
            .filter(|command| !matches!(command, feed::Command::Want(_)))
            .collect::<Vec<_>>(),
        [
            feed::Command::Delete { id: 4 },
            feed::Command::Delete { id: 5 },
            feed::Command::Restore { id: 5 },
            feed::Command::Restore { id: 4 },
        ]
    );
    key(&mut runtime, Key::Text("a".into()), Mods::default());
    key(&mut runtime, Key::Text("b".into()), Mods::default());
    key(&mut runtime, Key::Command('z'), ctrl());
    assert_ne!(
        runtime.state.query.text, "ab",
        "with no delete to undo, the key undoes typing in the search"
    );
    assert_eq!(runtime.state.clips.len(), 12);
    key(&mut runtime, Key::Escape, Mods::default());

    key(&mut runtime, Key::Delete, Mods::default());
    assert!(says(&runtime, "Undo"));
    deliver(&mut runtime, feed::Event::Deleted(4));
    assert!(
        !says(&runtime, "Undo"),
        "the way back goes when the clip is gone"
    );
    assert!(runtime.state.undo.is_empty());
}

#[test]
fn delete_edits_the_search_while_there_is_one() {
    let mut runtime = runtime(fixture::ready());
    assert!(texts(&runtime).iter().any(|text| text == "Del"));
    key(&mut runtime, Key::Text("c".into()), Mods::default());
    key(&mut runtime, Key::Text("a".into()), Mods::default());
    key(&mut runtime, Key::Left, Mods::default());
    feed::take_sent();
    key(&mut runtime, Key::Delete, Mods::default());
    assert_eq!(
        runtime.state.query.text, "c",
        "Delete took the letter after the caret"
    );
    assert_eq!(runtime.state.clips.len(), 12);
    assert!(
        !texts(&runtime).iter().any(|text| text == "Del"),
        "the hint names the key that deletes while there is a search"
    );
}

#[test]
fn control_enter_copies_without_a_paste_and_enter_says_copy_when_there_is_nowhere_to_paste() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    feed::take_sent();
    key(&mut runtime, Key::Enter, ctrl());
    assert_eq!(
        feed::take_sent(),
        [feed::Command::Paste {
            id: 3,
            mode: PasteMode::Original,
            target: None
        }]
    );
    key(&mut runtime, Key::Enter, Mods::default());
    assert!(pastes(&feed::take_sent()).is_empty(), "one at a time");

    let mut nowhere = super::tests::runtime(State {
        target: None,
        ..fixture::ready()
    });
    arrive(&mut nowhere, fixture::clips());
    feed::take_sent();
    assert!(says(&nowhere, "Copy") && !says(&nowhere, "Paste"));
    key(&mut nowhere, Key::Enter, Mods::default());
    assert_eq!(
        feed::take_sent(),
        [feed::Command::Paste {
            id: 3,
            mode: PasteMode::Original,
            target: None
        }]
    );
}

#[test]
fn a_link_opens_and_other_text_does_not() {
    let mut runtime = runtime(fixture::ready());
    feed::take_sent();
    key(&mut runtime, Key::Command('o'), ctrl());
    assert!(feed::take_sent().is_empty());
    assert!(runtime.find("open").is_none());

    press(&mut runtime, "clip-5");
    assert!(runtime.find("open").is_some());
    feed::take_sent();
    key(&mut runtime, Key::Command('o'), ctrl());
    assert_eq!(
        feed::take_sent(),
        [feed::Command::Open(
            "https://ui.fabrials.com/docs/design".into()
        )]
    );

    let mut state = fixture::ready();
    state.selected = 4;
    state.clips[4].byte_len = 9000;
    let mut cut = super::tests::runtime(state);
    feed::take_sent();
    key(&mut cut, Key::Command('o'), ctrl());
    assert!(
        !feed::take_sent()
            .iter()
            .any(|command| matches!(command, feed::Command::Open(_))),
        "an address seen only in part is not opened"
    );
    assert!(cut.find("open").is_none());
}

#[test]
fn the_pointer_can_pin_delete_and_clear_the_search() {
    let mut runtime = runtime(fixture::ready());
    feed::take_sent();
    press(&mut runtime, "pin");
    assert!(runtime.state.clips[2].favorite);
    press(&mut runtime, "delete");
    assert_eq!(runtime.state.clips.len(), 11);
    assert!(runtime.state.query.focused);
    assert!(runtime.find("clear").is_none());

    key(&mut runtime, Key::Text("c".into()), Mods::default());
    press(&mut runtime, "clear");
    assert_eq!(runtime.state.query.text, "");
    assert!(runtime.state.query.focused);
}

#[test]
fn f1_shows_the_keys_and_the_next_key_puts_them_away() {
    let mut runtime = runtime(fixture::ready());
    key(&mut runtime, Key::Function(1), Mods::default());
    assert!(runtime.find("keys").is_some());
    assert!(runtime.find("list").is_none());
    for needle in [
        "Copy without pasting",
        "Undo a delete",
        "Pause or resume capture",
    ] {
        assert!(says(&runtime, needle), "{needle}");
    }
    feed::take_sent();
    key(&mut runtime, Key::Enter, Mods::default());
    assert!(runtime.find("keys").is_none());
    assert!(
        pastes(&feed::take_sent()).is_empty(),
        "the key that closes the sheet does nothing else"
    );

    press(&mut runtime, "keys-button");
    assert!(runtime.find("keys").is_some());
    key(&mut runtime, Key::Text("c".into()), Mods::default());
    assert!(runtime.find("keys").is_none());
    assert_eq!(
        runtime.state.query.text, "c",
        "typing goes on to the search"
    );
}

#[test]
fn control_and_a_digit_choose_a_filter() {
    let mut runtime = runtime(fixture::ready());
    key(&mut runtime, Key::Command('3'), ctrl());
    assert_eq!(runtime.state.filter, Filter::Images);
    key(&mut runtime, Key::Command('1'), ctrl());
    assert_eq!(runtime.state.filter, Filter::All);
}

#[test]
fn a_row_shows_what_its_clip_is_and_a_search_marks_what_it_found() {
    let mut runtime = runtime(fixture::ready());
    let marked = |runtime: &Runtime<State>| {
        runtime
            .state
            .clips
            .iter()
            .map(model::title)
            .filter(|title| !ui::found(&runtime.state, title).is_empty())
            .collect::<Vec<_>>()
    };
    assert!(marked(&runtime).is_empty());
    press(&mut runtime, "clip-8");
    assert!(runtime.find("preview-swatch").is_some());

    runtime.state.query = TextEdit::new("cargo");
    runtime.state.query.focused = true;
    on_query_change(&mut runtime.state);
    let results: Vec<Entry> = fixture::clips()
        .into_iter()
        .filter(|clip| clip.content.contains("cargo"))
        .collect();
    arrive(&mut runtime, results);
    assert_eq!(marked(&runtime), ["cargo test --workspace"]);
}

#[test]
fn a_clip_pinned_either_way_shows_its_pin_and_unpins() {
    let mut state = fixture::ready();
    state.clips[0].favorite = false;
    state.clips[0].sticky = true;
    state.selected = 0;
    let mut runtime = runtime(state);
    assert!(says(&runtime, "Unpin"));
    feed::take_sent();
    key(&mut runtime, Key::Command('p'), ctrl());
    assert!(!model::is_pinned(&runtime.state.clips[0]));
    assert_eq!(feed::take_sent(), [feed::Command::Pin { id: 1, on: false }]);
}

#[test]
fn typing_a_letter_that_is_also_a_shortcut_only_types() {
    let mut runtime = runtime(fixture::ready());
    feed::take_sent();
    key(&mut runtime, Key::Text("d".into()), Mods::default());
    key(&mut runtime, Key::Text("p".into()), Mods::default());
    assert_eq!(runtime.state.clips.len(), 12);
    assert!(!runtime.state.clips[2].favorite);
    assert_eq!(runtime.state.query.text, "dp");
    assert!(feed::take_sent().iter().all(|command| matches!(
        command,
        feed::Command::Query { .. } | feed::Command::Want(_)
    )));
}

#[test]
fn escape_clears_the_search_before_it_closes() {
    let mut runtime = runtime(fixture::ready());
    runtime.state.query = TextEdit::new("cargo");
    runtime.state.query.focused = true;
    key(&mut runtime, Key::Escape, Mods::default());
    assert_eq!(runtime.state.query.text, "");
}

#[test]
fn an_image_clip_shows_its_picture_and_its_size() {
    let mut state = fixture::ready();
    state.selected = 3;
    state.images.insert(4, fixture::storm(64, 40));
    let runtime = runtime(state);
    assert!(runtime.find("preview-image").is_some());
    assert!(says(&runtime, "960 \u{d7} 600"));
    assert!(says(&runtime, "184.3 kB"));
}

#[test]
fn a_cut_clip_waits_for_its_text_and_a_long_one_says_it_is_cut() {
    let mut state = fixture::ready();
    state.clips[2].byte_len = 50_000;
    assert_eq!(state.full_text(), None);
    let mut detail = state.clips[2].clone();
    detail.content = "a line of text\n".repeat(3_500);
    state.detail = Some(detail);
    assert_eq!(state.full_text().map(str::len), Some(52_500));
    let runtime = runtime(state);
    assert!(says(&runtime, "Start only"));
    let (_, caption) = runtime.find("facts").expect("the caption is drawn");
    assert!(
        caption < SIZE.1 - 40.0,
        "the long text scrolls inside the plate; caption at {caption}"
    );
}

#[test]
fn every_empty_state_says_what_happened_and_what_to_do() {
    let empty = runtime(State {
        clips: Vec::new(),
        ..fixture::ready()
    });
    assert!(says(&empty, "Nothing copied yet"));
    assert!(says(&empty, "Esc"));

    let mut searching = State {
        clips: Vec::new(),
        ..fixture::ready()
    };
    searching.query = TextEdit::new("zzzz");
    let searching = runtime(searching);
    assert!(says(&searching, "No clips match \u{201c}zzzz\u{201d}"));
    assert!(says(&searching, "Clear the search"));

    let filtered = runtime(State {
        clips: Vec::new(),
        filter: Filter::Images,
        ..fixture::ready()
    });
    assert!(says(&filtered, "No images"));

    let offline = runtime(State {
        clips: Vec::new(),
        link: Link::Offline("no socket".into()),
        watcher: None,
        ..fixture::ready()
    });
    for needle in [
        "The daemon is not running",
        "systemctl --user start ditox",
        "no socket",
        "Daemon offline",
    ] {
        assert!(says(&offline, needle), "{needle}");
    }
}

#[test]
fn a_note_and_a_failed_capture_replace_the_hints() {
    let noted = runtime(State {
        note: Some("That clip is gone.".into()),
        ..fixture::ready()
    });
    assert!(says(&noted, "That clip is gone."));
    assert!(!says(&noted, "Keys"));

    let mut failing = fixture::ready();
    failing.watcher = Some(WatcherStatus {
        running: true,
        paused: false,
        backend: "wl-clipboard".into(),
        last_error: Some("clipboard: exit 1".into()),
    });
    let failing = runtime(failing);
    assert!(says(&failing, "Last capture failed: clipboard: exit 1"));
}

#[test]
fn every_scene_draws_in_both_themes_and_at_the_smallest_size() {
    for (name, scene) in fixture::SCENES {
        for size in [SIZE, (560.0, 380.0)] {
            for dark in [true, false] {
                let mut state = State::blank();
                scene(&mut state);
                state.dark = Some(dark);
                let listed = !state.clips.is_empty() && !state.keys_open;
                let mut runtime = Runtime::new(
                    state,
                    ui::view,
                    size,
                    Env {
                        system_dark: dark,
                        scale: 2.0,
                    },
                );
                runtime.frame(0.0);
                let at = format!("{name} at {size:?}, dark {dark}");
                for key in ["search", "filters", "footer", "status"] {
                    let (x, y) = runtime.find(key).unwrap_or_else(|| panic!("{key} in {at}"));
                    assert!(
                        x >= 0.0 && x <= size.0 && y >= 0.0 && y <= size.1,
                        "{key} is on screen in {at}"
                    );
                }
                assert_eq!(runtime.find("list").is_some(), listed, "{at}");
                assert_eq!(runtime.find("preview").is_some(), listed, "{at}");
                let (_, footer) = runtime.find("footer").unwrap();
                assert!(
                    footer > size.1 - 40.0,
                    "the footer sits at the bottom in {at}"
                );
            }
        }
    }
}
