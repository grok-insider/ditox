use std::sync::Arc;
use std::time::Duration;

use ditox_rpc::{PasteMode, WatcherStatus};
use rune::{Env, Key, KeyEvent, Mods, Runtime, Shell, TextEdit, Tx};

use super::view::{row_height, view};
use super::*;
use crate::fixture;
use crate::theme;

fn runtime(state: State) -> Runtime<State> {
    let mut runtime = Runtime::new(
        state,
        view,
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

fn shift() -> Mods {
    Mods {
        shift: true,
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

fn thumbs_asked(sent: &[feed::Command]) -> Vec<Vec<i64>> {
    sent.iter()
        .filter_map(|command| match command {
            feed::Command::Thumbs { ids, .. } => Some(ids.clone()),
            _ => None,
        })
        .collect()
}

/// What was sent, without the questions the window asks on its own.
fn acts(sent: Vec<feed::Command>) -> Vec<feed::Command> {
    sent.into_iter()
        .filter(|command| {
            !matches!(
                command,
                feed::Command::Want(_) | feed::Command::Thumbs { .. }
            )
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

fn height_of(runtime: &Runtime<State>, key: &str) -> f32 {
    runtime
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("{key} is drawn"))
        .rect[3]
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

fn right_press(runtime: &mut Runtime<State>, element: &str) {
    let (x, y) = runtime
        .find(element)
        .unwrap_or_else(|| panic!("{element} is on screen"));
    runtime.secondary_down(x, y);
    runtime.secondary_up(x, y);
    runtime.frame(0.0);
}

fn closing(runtime: &Runtime<State>) -> bool {
    runtime.shell_request == Some(Shell::Close)
}

fn labels(state: &State) -> Vec<&'static str> {
    state.menu_items().iter().map(|item| item.label).collect()
}

/// The menu entry called `label`, by its place in the menu.
fn entry(state: &State, label: &str) -> usize {
    labels(state)
        .iter()
        .position(|other| *other == label)
        .unwrap_or_else(|| panic!("the menu has {label}"))
}

#[test]
fn the_window_is_the_clips_and_a_search_field_and_nothing_else() {
    let mut runtime = runtime(fixture::ready());
    for element in [
        "list",
        "search",
        "menu-button",
        "clip-1",
        "clip-12",
        "thumb-4",
    ] {
        assert!(runtime.find(element).is_some(), "{element}");
    }
    for element in [
        "menu",
        "peek",
        "note",
        "undo",
        "paused",
        "filter",
        "capture-error",
    ] {
        assert!(runtime.find(element).is_none(), "{element}");
    }
    assert!(says(&runtime, "ssh friend@station.example"));
    assert!(says(
        &runtime,
        "pub fn capture(&mut self) {\nmatch self.capture_current_offer() {"
    ));

    let numbers: Vec<String> = texts(&runtime)
        .into_iter()
        .filter(|text| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .collect();
    assert_eq!(numbers, ["1", "2", "3", "4", "5", "6", "7", "8", "9"]);

    assert_eq!(tinted(&mut runtime), [row_y(&runtime, 3)]);
    let brand = theme::palette(true).brand;
    let pin = |id: i64| {
        runtime
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some(&format!("pin-{id}")))
            .map(|element| element.fill)
            .unwrap()
    };
    assert_eq!(pin(1), brand);
    assert_eq!(pin(2), brand);
    assert_eq!(pin(3).a, 0.0);
}

#[test]
fn a_clip_takes_as_many_lines_as_it_has_up_to_the_setting() {
    for (lines, of_the_snippet) in [(1, 1), (2, 2), (4, 4), (5, 5)] {
        let mut state = fixture::ready();
        state.ui.lines = lines;
        let runtime = runtime(state);
        assert_eq!(
            height_of(&runtime, "clip-3"),
            row_height(of_the_snippet),
            "{lines}"
        );
        assert_eq!(height_of(&runtime, "clip-6"), row_height(1), "{lines}");
    }
    let mut state = fixture::ready();
    state.clips[5].content = "a word and then another, ".repeat(12);
    let runtime = runtime(state);
    assert_eq!(
        height_of(&runtime, "clip-6"),
        row_height(2),
        "one long line wraps onto the lines the setting allows"
    );
}

#[test]
fn arrows_move_the_selection_and_stop_at_the_ends() {
    let mut runtime = runtime(fixture::ready());
    key(&mut runtime, Key::Down, Mods::default());
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(4));
    for _ in 0..4 {
        key(&mut runtime, Key::Up, Mods::default());
    }
    assert_eq!(runtime.state.selected, 0);
    key(&mut runtime, Key::PageDown, Mods::default());
    key(&mut runtime, Key::PageDown, Mods::default());
    assert_eq!(runtime.state.selected, 11);
    key(&mut runtime, Key::Home, ctrl());
    assert_eq!(runtime.state.selected, 0);
    key(&mut runtime, Key::End, ctrl());
    assert_eq!(runtime.state.selected, 11);
    key(&mut runtime, Key::PageUp, Mods::default());
    assert_eq!(runtime.state.selected, 3);
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
    let y = row_y(&runtime, id);
    assert!(y > 0.0 && y < SIZE.1 - 44.0, "row at {y}");
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
    let y = row_y(&runtime, first);
    assert!(
        (0.0..row_height(1)).contains(&y),
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
            (0.0..row_height(1)).contains(&y),
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
fn enter_pastes_shift_pastes_plain_and_control_only_copies() {
    for (mods, mode, target) in [
        (Mods::default(), PasteMode::Original, Some("0xabc")),
        (shift(), PasteMode::Plain, Some("0xabc")),
        (ctrl(), PasteMode::Original, None),
    ] {
        let mut runtime = runtime(fixture::ready());
        arrive(&mut runtime, fixture::clips());
        feed::take_sent();
        key(&mut runtime, Key::Enter, mods);
        assert_eq!(
            acts(feed::take_sent()),
            [feed::Command::Paste {
                id: 3,
                mode,
                target: target.map(String::from)
            }]
        );
        key(&mut runtime, Key::Enter, Mods::default());
        assert!(pastes(&feed::take_sent()).is_empty(), "one at a time");

        deliver(
            &mut runtime,
            feed::Event::PasteRefused("no-plain-text".into()),
        );
        assert!(says(&runtime, "This clip has no plain text to paste."));
        assert!(!closing(&runtime));
        key(&mut runtime, Key::Enter, Mods::default());
        assert_eq!(pastes(&feed::take_sent()), [3], "and it can be tried again");
        deliver(&mut runtime, feed::Event::Pasted);
        assert!(closing(&runtime));
    }

    let mut nowhere = runtime(State {
        target: None,
        ..fixture::ready()
    });
    arrive(&mut nowhere, fixture::clips());
    feed::take_sent();
    key(&mut nowhere, Key::Enter, Mods::default());
    assert_eq!(
        acts(feed::take_sent()),
        [feed::Command::Paste {
            id: 3,
            mode: PasteMode::Original,
            target: None
        }]
    );
}

#[test]
fn control_and_a_number_paste_the_clip_with_that_number() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    feed::take_sent();
    key(&mut runtime, Key::Command('5'), ctrl());
    assert_eq!(pastes(&feed::take_sent()), [5]);
    assert_eq!(runtime.state.selected, 4);

    let mut few = super::tests::runtime(fixture::ready());
    arrive(&mut few, many(3));
    feed::take_sent();
    key(&mut few, Key::Command('4'), ctrl());
    assert!(pastes(&feed::take_sent()).is_empty());
    assert_eq!(few.state.selected, 0);
    key(&mut few, Key::Command('3'), ctrl());
    assert_eq!(pastes(&feed::take_sent()), [1002]);
}

#[test]
fn a_number_pressed_before_the_results_arrive_is_a_number_of_the_results() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    on_query_change(&mut runtime.state);
    feed::take_sent();
    key(&mut runtime, Key::Command('3'), ctrl());
    assert!(
        pastes(&feed::take_sent()).is_empty(),
        "nothing is pasted from the list the search replaced"
    );
    assert_eq!(runtime.state.selected, 2, "and the selection waits too");
    arrive(&mut runtime, many(5));
    assert_eq!(pastes(&feed::take_sent()), [1002]);
    assert_eq!(runtime.state.selected, 2);

    let mut short = super::tests::runtime(fixture::ready());
    arrive(&mut short, fixture::clips());
    on_query_change(&mut short.state);
    key(&mut short, Key::Command('7'), ctrl());
    feed::take_sent();
    arrive(&mut short, many(5));
    assert!(
        pastes(&feed::take_sent()).is_empty(),
        "there is no seventh result"
    );
    assert!(!short.state.pasting);
    assert_eq!(
        short.state.current().map(|clip| clip.id),
        Some(1000),
        "and the selection is on a clip that is there"
    );

    let mut pressed = super::tests::runtime(fixture::ready());
    arrive(&mut pressed, fixture::clips());
    on_query_change(&mut pressed.state);
    feed::take_sent();
    press(&mut pressed, "clip-6");
    press(&mut pressed, "clip-6");
    arrive(&mut pressed, many(5));
    assert!(
        pastes(&feed::take_sent()).is_empty(),
        "a second press on a row that is going away is not kept for another"
    );
}

#[test]
fn a_press_selects_a_row_twice_pastes_it_and_the_search_keeps_the_keyboard() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    feed::take_sent();
    press(&mut runtime, "clip-6");
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(6));
    assert!(runtime.state.query.focused);
    assert!(pastes(&feed::take_sent()).is_empty());
    press(&mut runtime, "clip-6");
    assert_eq!(pastes(&feed::take_sent()), [6]);

    let mut empty = super::tests::runtime(State {
        clips: Vec::new(),
        ..fixture::ready()
    });
    press(&mut empty, "empty");
    assert!(empty.state.query.focused);
}

#[test]
fn tab_shows_the_clip_whole_and_only_then_asks_for_the_rest_of_it() {
    let mut state = fixture::ready();
    state.clips[2].byte_len = 50_000;
    let mut runtime = runtime(state);
    feed::take_sent();
    turn(&mut runtime);
    assert!(
        wants(&feed::take_sent()).is_empty(),
        "the list needs no more than it was given"
    );

    key(&mut runtime, Key::Tab, Mods::default());
    turn(&mut runtime);
    assert!(runtime.find("peek").is_some() && runtime.find("list").is_none());
    assert_eq!(
        wants(&feed::take_sent()),
        [Some(feed::Wanted {
            id: 3,
            image: false
        })]
    );
    assert!(says(&runtime, "Start only"));
    assert!(says(&runtime, "zed"));
    let mut whole = runtime.state.clips[2].clone();
    whole.content = "the whole text\n".repeat(4);
    deliver(&mut runtime, feed::Event::Detail(whole));
    assert!(!says(&runtime, "Start only") && says(&runtime, "4 lines"));
    assert_eq!(
        wants(&feed::take_sent()),
        [None],
        "it has what it waited for"
    );

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
        "it is not asked for again"
    );

    key(&mut runtime, Key::Down, Mods::default());
    turn(&mut runtime);
    assert!(
        wants(&feed::take_sent()).is_empty(),
        "a whole clip needs nothing more"
    );
    assert!(says(&runtime, "Link"));

    key(&mut runtime, Key::Tab, Mods::default());
    assert!(runtime.find("list").is_some() && runtime.find("peek").is_none());
    key(&mut runtime, Key::Function(3), Mods::default());
    assert!(runtime.find("peek").is_some());
    press(&mut runtime, "back");
    assert!(runtime.find("list").is_some());
    assert!(runtime.state.query.focused);
}

/// The clips of the fixture with a second picture after the first.
fn two_shots() -> State {
    let mut state = fixture::ready();
    let mut second = state.clips[3].clone();
    second.id = 40;
    state.clips.insert(4, second);
    state.selected = 3;
    state
}

fn shot(width: u32, height: u32) -> picture::Picture {
    picture::Picture {
        width,
        height,
        rgba: vec![7; (width * height * 4) as usize],
    }
}

fn arrives(runtime: &mut Runtime<State>, id: i64) {
    deliver(
        runtime,
        feed::Event::Picture {
            id,
            picture: shot(8, 6),
        },
    );
}

#[test]
fn where_the_screen_can_show_it_a_picture_goes_there_and_the_list_stays() {
    let mut runtime = runtime(two_shots());
    deliver(&mut runtime, feed::Event::Preview(true));
    feed::take_sent();
    overlay::take_sent();

    key(&mut runtime, Key::Tab, Mods::default());
    turn(&mut runtime);
    assert!(runtime.find("list").is_some() && runtime.find("peek").is_none());
    assert_eq!(
        wants(&feed::take_sent()),
        [Some(feed::Wanted { id: 4, image: true })]
    );
    assert!(overlay::take_sent().is_empty(), "nothing to show yet");
    assert!(labels(&runtime.state).contains(&"Hide the picture"));

    arrives(&mut runtime, 4);
    assert_eq!(
        overlay::take_sent(),
        [overlay::Command::Load {
            id: 4,
            picture: shot(8, 6),
            show: true
        }]
    );
    assert!(
        !runtime.state.images.contains(4),
        "the window keeps no copy of what the screen shows"
    );
    assert_eq!(wants(&feed::take_sent()), [None]);

    key(&mut runtime, Key::Down, Mods::default());
    turn(&mut runtime);
    assert_eq!(
        wants(&feed::take_sent()),
        [Some(feed::Wanted {
            id: 40,
            image: true
        })]
    );
    assert!(
        overlay::take_sent().is_empty(),
        "the picture before stays up until the next one is here"
    );
    arrives(&mut runtime, 40);
    assert!(matches!(
        overlay::take_sent()[..],
        [overlay::Command::Load {
            id: 40,
            show: true,
            ..
        }]
    ));

    key(&mut runtime, Key::Up, Mods::default());
    turn(&mut runtime);
    assert_eq!(overlay::take_sent(), [overlay::Command::Show(4)]);
    assert!(
        wants(&feed::take_sent()).iter().all(Option::is_none),
        "a picture kept there is not fetched again"
    );

    // A text clip is read in the window, and the picture comes down.
    key(&mut runtime, Key::Up, Mods::default());
    turn(&mut runtime);
    assert_eq!(overlay::take_sent(), [overlay::Command::Hide]);
    assert!(runtime.find("peek").is_some() && runtime.find("list").is_none());
    key(&mut runtime, Key::Down, Mods::default());
    turn(&mut runtime);
    assert_eq!(overlay::take_sent(), [overlay::Command::Show(4)]);
    assert!(runtime.find("list").is_some());

    key(&mut runtime, Key::Escape, Mods::default());
    turn(&mut runtime);
    assert_eq!(overlay::take_sent(), [overlay::Command::Hide]);
    assert!(!runtime.state.peek && !closing(&runtime));
    turn(&mut runtime);
    assert!(overlay::take_sent().is_empty(), "it is taken down once");

    key(&mut runtime, Key::Tab, Mods::default());
    turn(&mut runtime);
    assert_eq!(overlay::take_sent(), [overlay::Command::Show(4)]);
    key(&mut runtime, Key::Text("z".into()), Mods::default());
    turn(&mut runtime);
    assert_eq!(
        overlay::take_sent(),
        [overlay::Command::Hide],
        "a search puts the picture away"
    );
}

#[test]
fn the_picture_follows_each_key_and_press_without_waiting_for_a_turn() {
    let mut runtime = runtime(two_shots());
    deliver(&mut runtime, feed::Event::Preview(true));
    key(&mut runtime, Key::Tab, Mods::default());
    arrives(&mut runtime, 4);
    // From here the window is given no turn of its own.
    runtime.poll = None;
    feed::take_sent();
    overlay::take_sent();

    key(&mut runtime, Key::Escape, Mods::default());
    assert_eq!(overlay::take_sent(), [overlay::Command::Hide]);
    key(&mut runtime, Key::Tab, Mods::default());
    assert_eq!(overlay::take_sent(), [overlay::Command::Show(4)]);
    key(&mut runtime, Key::Down, Mods::default());
    assert_eq!(
        wants(&feed::take_sent()),
        [Some(feed::Wanted {
            id: 40,
            image: true
        })]
    );
    key(&mut runtime, Key::Up, Mods::default());
    key(&mut runtime, Key::Text("z".into()), Mods::default());
    assert_eq!(overlay::take_sent(), [overlay::Command::Hide]);

    let mut runtime = runtime_with_picture();
    press(&mut runtime, "clip-2");
    assert_eq!(overlay::take_sent(), [overlay::Command::Hide]);
    assert!(
        runtime.find("peek").is_some(),
        "a text clip is read in the window"
    );
    press(&mut runtime, "back");
    press(&mut runtime, "clip-4");
    assert!(overlay::take_sent().is_empty());
    key(&mut runtime, Key::Command('m'), ctrl());
    let view = labels(&runtime.state)
        .iter()
        .position(|label| *label == "View")
        .unwrap();
    overlay::take_sent();
    press(&mut runtime, &format!("menu-{view}"));
    assert_eq!(overlay::take_sent(), [overlay::Command::Show(4)]);
}

/// A window with the first picture up on the screen and no turn of its own.
fn runtime_with_picture() -> Runtime<State> {
    let mut runtime = runtime(two_shots());
    deliver(&mut runtime, feed::Event::Preview(true));
    key(&mut runtime, Key::Tab, Mods::default());
    arrives(&mut runtime, 4);
    runtime.poll = None;
    feed::take_sent();
    overlay::take_sent();
    runtime
}

#[test]
fn a_picture_that_arrives_late_is_kept_and_not_shown() {
    let mut runtime = runtime(two_shots());
    deliver(&mut runtime, feed::Event::Preview(true));
    key(&mut runtime, Key::Tab, Mods::default());
    turn(&mut runtime);
    key(&mut runtime, Key::Down, Mods::default());
    turn(&mut runtime);
    overlay::take_sent();
    arrives(&mut runtime, 4);
    assert!(matches!(
        overlay::take_sent()[..],
        [overlay::Command::Load {
            id: 4,
            show: false,
            ..
        }]
    ));
    assert_eq!(runtime.state.preview.showing, None);

    // More pictures than are kept there: the oldest is asked for again.
    for id in [40, 41, 42] {
        arrives(&mut runtime, id);
    }
    assert_eq!(runtime.state.preview.held, [40, 41, 42]);
    feed::take_sent();
    key(&mut runtime, Key::Up, Mods::default());
    turn(&mut runtime);
    assert_eq!(
        wants(&feed::take_sent()),
        [Some(feed::Wanted { id: 4, image: true })]
    );
}

#[test]
fn what_the_screen_cannot_show_the_window_does() {
    let mut state = two_shots();
    state.clips[3].image_width = Some(20_000);
    state.clips[3].image_height = Some(20_000);
    let mut huge = runtime(state);
    deliver(&mut huge, feed::Event::Preview(true));
    overlay::take_sent();
    key(&mut huge, Key::Tab, Mods::default());
    turn(&mut huge);
    assert!(says(&huge, "Too large to show") && huge.find("peek").is_some());
    assert!(overlay::take_sent().is_empty());
    assert!(labels(&huge.state).contains(&"Back to the list"));

    let mut gone = runtime(two_shots());
    deliver(&mut gone, feed::Event::Preview(true));
    key(&mut gone, Key::Tab, Mods::default());
    turn(&mut gone);
    arrives(&mut gone, 4);
    overlay::take_sent();
    deliver(&mut gone, feed::Event::Unavailable(4, feed::Gone::Missing));
    assert_eq!(overlay::take_sent(), [overlay::Command::Hide]);
    assert!(says(&gone, "This image is no longer stored"));

    // The compositor stops showing pictures: the window asks for the
    // picture again and shows it itself.
    let mut state = two_shots();
    state.cache = Some(ImageCache::new(Arc::new(|| {})));
    let mut lost = runtime(state);
    deliver(&mut lost, feed::Event::Preview(true));
    key(&mut lost, Key::Tab, Mods::default());
    turn(&mut lost);
    arrives(&mut lost, 4);
    feed::take_sent();
    overlay::take_sent();
    deliver(&mut lost, feed::Event::Preview(false));
    assert_eq!(
        wants(&feed::take_sent()),
        [Some(feed::Wanted { id: 4, image: true })]
    );
    assert!(lost.find("peek").is_some() && lost.find("peek-image").is_none());
    assert_eq!(lost.state.preview, Preview::default());
    arrives(&mut lost, 4);
    assert!(lost.find("peek-image").is_some());
    assert!(
        overlay::take_sent().is_empty(),
        "nothing more is sent to a screen that shows nothing"
    );
}

#[test]
fn closing_the_window_takes_the_picture_down() {
    let mut state = two_shots();
    overlay::take_sent();
    on_close(&mut state, &mut Tx::default());
    assert_eq!(overlay::take_sent(), [overlay::Command::Finish]);
}

#[test]
fn there_is_no_clip_to_see_whole_when_the_list_is_empty_or_its_last_clip_goes() {
    let mut empty = runtime(State {
        clips: Vec::new(),
        ..fixture::ready()
    });
    key(&mut empty, Key::Tab, Mods::default());
    assert!(!empty.state.peek);
    assert!(empty.find("empty").is_some());

    let mut one = runtime(State {
        clips: many(1),
        selected: 0,
        ..fixture::ready()
    });
    key(&mut one, Key::Tab, Mods::default());
    assert!(one.find("peek").is_some());
    key(&mut one, Key::Command('d'), ctrl());
    assert!(!one.state.peek, "the deleted clip is not left on show");
    assert!(one.find("peek").is_none() && one.find("undo").is_some());
}

#[test]
fn a_rows_picture_fits_two_lines_or_three_and_is_never_made_larger() {
    let shot = |width, height| Entry {
        image_width: Some(width),
        image_height: Some(height),
        ..fixture::clips().remove(3)
    };
    assert_eq!(view::thumb_size(&shot(4000, 1000), 2), Some((160.0, 40.0)));
    assert_eq!(view::thumb_size(&shot(4000, 1000), 1), Some((160.0, 40.0)));
    assert_eq!(view::thumb_size(&shot(4000, 1000), 3), Some((240.0, 60.0)));
    assert_eq!(view::thumb_size(&shot(4000, 1000), 5), Some((240.0, 60.0)));
    assert_eq!(view::thumb_size(&shot(4000, 100), 2), Some((240.0, 6.0)));
    assert_eq!(view::thumb_size(&shot(300, 900), 3), Some((20.0, 60.0)));
    assert_eq!(view::thumb_size(&shot(16, 12), 2), Some((16.0, 12.0)));
    assert_eq!(view::thumb_size(&shot(20_000, 20_000), 2), None);
    assert_eq!(view::thumb_size(&shot(0, 10), 2), None);
    let mut unknown = shot(1, 1);
    unknown.image_width = None;
    assert_eq!(view::thumb_size(&unknown, 2), None);
    assert_eq!(view::thumb_size(&fixture::clips()[0], 2), None);
    assert_eq!(view::thumb_height(1), 40.0);
    assert_eq!(view::thumb_height(9), 60.0);
}

#[test]
fn a_picture_shows_whole_and_one_too_large_is_described_and_never_fetched() {
    let mut state = fixture::ready();
    state.selected = 3;
    state.peek = true;
    state.images.keep(4, fixture::storm(64, 40), 0);
    let shown = runtime(state);
    assert!(shown.find("peek-image").is_some());
    assert!(says(&shown, "PNG \u{b7} 960 \u{d7} 600 \u{b7} 184.3 kB"));

    let mut state = fixture::ready();
    state.selected = 3;
    state.peek = true;
    state.clips[3].image_width = Some(20_000);
    state.clips[3].image_height = Some(20_000);
    feed::take_sent();
    let mut huge = runtime(state);
    turn(&mut huge);
    let sent = feed::take_sent();
    assert!(wants(&sent).is_empty());
    assert!(thumbs_asked(&sent).iter().all(|ids| !ids.contains(&4)));
    assert!(says(&huge, "Too large to show"));
    assert!(huge.find("thumb-4").is_none());
    key(&mut huge, Key::Tab, Mods::default());
    assert!(says(&huge, "Image, 20000 \u{d7} 20000"));
}

#[test]
fn the_image_rows_in_view_ask_for_their_pictures_once() {
    let mut state = fixture::ready();
    state.thumbs = Images::new(THUMB_SLOTS, THUMB_BYTES);
    state.cache = Some(ImageCache::new(Arc::new(|| {})));
    let shots: Vec<Entry> = (0..40)
        .map(|index| Entry {
            id: 500 + index,
            ..state.clips[3].clone()
        })
        .collect();
    state.clips = shots.clone();
    state.selected = 0;
    feed::take_sent();
    let mut runtime = runtime(state);
    turn(&mut runtime);
    let asked = feed::take_sent();
    let ids = thumbs_asked(&asked);
    assert_eq!(ids.len(), 1, "{ids:?}");
    let in_view = (SIZE.1 - 44.0) / row_height(2);
    assert!(
        ids[0].len() >= in_view as usize && ids[0].len() <= in_view as usize + 2 + THUMB_MARGIN,
        "the rows in view and a few past them, not all forty: {}",
        ids[0].len()
    );
    assert_eq!(ids[0][0], 500);
    assert!(asked.contains(&feed::Command::Thumbs {
        ids: ids[0].clone(),
        fit: (240, 40)
    }));
    assert!(runtime.find("thumb-500").is_some(), "its place is kept");

    let picture = || picture::Picture {
        width: 2,
        height: 2,
        rgba: vec![255; 16],
    };
    deliver(
        &mut runtime,
        feed::Event::Thumb {
            id: 500,
            picture: picture(),
        },
    );
    deliver(&mut runtime, feed::Event::NoThumb(501));
    assert!(runtime.state.thumbs.contains(500));
    assert!(
        thumbs_asked(&feed::take_sent()).is_empty(),
        "a picture arriving asks for nothing"
    );

    arrive(&mut runtime, shots.clone());
    let again = thumbs_asked(&feed::take_sent());
    assert_eq!(again.len(), 1);
    assert!(!again[0].contains(&500) && !again[0].contains(&501));
    assert_eq!(again[0][0], 502);
    arrive(&mut runtime, shots.clone());
    assert!(
        thumbs_asked(&feed::take_sent()).is_empty(),
        "the same rows are not asked for twice"
    );

    for _ in 0..3 {
        key(&mut runtime, Key::PageDown, Mods::default());
    }
    turn(&mut runtime);
    let further = thumbs_asked(&feed::take_sent());
    assert!(
        further.last().is_some_and(|ids| ids.contains(&524)),
        "{further:?}"
    );

    let mut state = fixture::ready();
    state.thumbs = Images::new(THUMB_SLOTS, THUMB_BYTES);
    state.ui.lines = 3;
    state.clips = shots;
    feed::take_sent();
    let mut dense = Runtime::new(
        state,
        view,
        SIZE,
        Env {
            system_dark: true,
            scale: 2.0,
        },
    );
    for _ in 0..3 {
        dense.frame(1.0 / 60.0);
    }
    let asked = feed::take_sent();
    let fits: Vec<(u32, u32)> = asked
        .iter()
        .filter_map(|command| match command {
            feed::Command::Thumbs { fit, .. } => Some(*fit),
            _ => None,
        })
        .collect();
    assert_eq!(
        fits,
        [(480, 120)],
        "a dense screen and three lines get the pixels they will draw"
    );
}

#[test]
fn the_window_lets_go_of_the_oldest_pictures() {
    let mut images = Images::new(PEEK_SLOTS, PEEK_BYTES);
    for id in 0..PEEK_SLOTS as i64 + 3 {
        images.keep(id, fixture::storm(2, 2), 1024);
    }
    assert_eq!(images.count(), PEEK_SLOTS);
    assert!(!images.contains(2) && images.contains(3));

    images.keep(100, fixture::storm(2, 2), PEEK_BYTES - 2048);
    assert_eq!(
        images.count(),
        3,
        "a heavy picture pushes out all but what fits beside it"
    );
    images.keep(101, fixture::storm(2, 2), PEEK_BYTES * 2);
    assert_eq!(
        images.count(),
        1,
        "the picture on screen stays, whatever it weighs"
    );
    assert!(images.contains(101));
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
fn the_menu_opens_from_its_button_a_key_and_a_right_click() {
    let mut runtime = runtime(fixture::ready());
    press(&mut runtime, "menu-button");
    assert!(runtime.find("menu").is_some());
    assert!(runtime.state.query.focused);
    press(&mut runtime, "menu-button");
    assert!(runtime.find("menu").is_none());

    key(&mut runtime, Key::Command('m'), ctrl());
    assert!(runtime.find("menu").is_some());
    key(&mut runtime, Key::Escape, Mods::default());
    assert!(runtime.find("menu").is_none());
    assert!(
        !closing(&runtime),
        "Escape put the menu away, not the window"
    );

    right_press(&mut runtime, "clip-6");
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(6));
    assert!(runtime.find("menu").is_some());
    press(&mut runtime, "menu-away");
    assert!(runtime.find("menu").is_none());
    assert_eq!(runtime.state.current().map(|clip| clip.id), Some(6));

    key(&mut runtime, Key::Function(10), Mods::default());
    let (x, y) = runtime.find("menu").unwrap();
    assert!(
        x > SIZE.0 / 2.0 && y < SIZE.1 - 44.0,
        "over its button: {x}, {y}"
    );
}

#[test]
fn the_menu_follows_the_arrows_and_enter_does_what_the_entry_says() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    feed::take_sent();
    key(&mut runtime, Key::Command('m'), ctrl());
    key(&mut runtime, Key::Down, Mods::default());
    key(&mut runtime, Key::Down, Mods::default());
    assert_eq!(runtime.state.menu, Some(Menu { at: 2 }));
    assert_eq!(runtime.state.selected, 2, "the list does not move under it");
    key(&mut runtime, Key::Enter, Mods::default());
    assert_eq!(runtime.state.menu, None);
    assert_eq!(
        acts(feed::take_sent()),
        [feed::Command::Paste {
            id: 3,
            mode: PasteMode::Original,
            target: None
        }],
        "the third entry is Copy"
    );

    let mut runtime = super::tests::runtime(fixture::ready());
    feed::take_sent();
    key(&mut runtime, Key::Command('m'), ctrl());
    key(&mut runtime, Key::Up, Mods::default());
    assert_eq!(
        runtime.state.menu.map(|menu| menu.at),
        Some(labels(&runtime.state).len() - 1),
        "Up from the first entry goes to the last"
    );
    key(&mut runtime, Key::Down, Mods::default());
    assert_eq!(runtime.state.menu, Some(Menu { at: 0 }));
    key(&mut runtime, Key::End, Mods::default());
    key(&mut runtime, Key::Enter, Mods::default());
    assert_eq!(acts(feed::take_sent()), [feed::Command::Options]);
    assert!(closing(&runtime));
}

#[test]
fn a_letter_puts_the_menu_away_and_goes_to_the_search() {
    let mut runtime = runtime(fixture::ready());
    key(&mut runtime, Key::Command('m'), ctrl());
    key(&mut runtime, Key::Text("c".into()), Mods::default());
    assert_eq!(runtime.state.menu, None);
    assert_eq!(runtime.state.query.text, "c");

    key(&mut runtime, Key::Command('m'), ctrl());
    feed::take_sent();
    key(&mut runtime, Key::Command('d'), ctrl());
    assert_eq!(runtime.state.menu, None);
    assert!(
        acts(feed::take_sent()).is_empty(),
        "a shortcut pressed over the menu only closes it"
    );
}

#[test]
fn the_menu_offers_what_fits_the_clip_and_the_moment() {
    let ready = fixture::ready();
    assert_eq!(
        labels(&ready),
        [
            "Paste",
            "Paste as plain text",
            "Copy",
            "View",
            "Pin",
            "Delete",
            "All clips",
            "Text",
            "Images",
            "Pinned",
            "Pause capture",
            "Options\u{2026}"
        ]
    );
    let leading: Vec<&str> = ready
        .menu_items()
        .iter()
        .filter(|item| item.leads)
        .map(|item| item.label)
        .collect();
    assert_eq!(leading, ["All clips", "Pause capture"]);
    let chosen: Vec<&str> = ready
        .menu_items()
        .iter()
        .filter(|item| item.chosen == Some(true))
        .map(|item| item.label)
        .collect();
    assert_eq!(chosen, ["All clips"]);

    let link = fixture::chosen(5);
    assert_eq!(labels(&link)[4], "Open link");
    let pinned = fixture::chosen(1);
    assert!(labels(&pinned).contains(&"Unpin") && !labels(&pinned).contains(&"Pin"));

    let nowhere = State {
        target: None,
        ..fixture::ready()
    };
    assert_eq!(labels(&nowhere)[..2], ["Copy", "View"]);
    assert!(
        !labels(&nowhere)
            .iter()
            .any(|label| label.starts_with("Paste"))
    );

    let mut paused = fixture::ready();
    paused.watcher = Some(WatcherStatus {
        paused: true,
        ..paused.watcher.clone().unwrap()
    });
    assert!(labels(&paused).contains(&"Resume capture"));

    let mut peeking = fixture::ready();
    peeking.peek = true;
    assert!(labels(&peeking).contains(&"Back to the list"));

    let empty = State {
        clips: Vec::new(),
        watcher: None,
        ..fixture::ready()
    };
    assert_eq!(
        labels(&empty),
        ["All clips", "Text", "Images", "Pinned", "Options\u{2026}"]
    );
    let leading: Vec<&str> = empty
        .menu_items()
        .iter()
        .filter(|item| item.leads)
        .map(|item| item.label)
        .collect();
    assert_eq!(leading, ["Options\u{2026}"]);
}

#[test]
fn every_entry_of_the_menu_does_its_one_thing() {
    let run = |state: State, label: &str| {
        let mut runtime = runtime(state);
        arrive(&mut runtime, fixture::clips());
        feed::take_sent();
        press(&mut runtime, "menu-button");
        let at = entry(&runtime.state, label);
        press(&mut runtime, &format!("menu-{at}"));
        assert_eq!(runtime.state.menu, None, "{label}");
        let sent = acts(feed::take_sent());
        (runtime, sent)
    };
    let paste = |mode, target: Option<&str>| feed::Command::Paste {
        id: 3,
        mode,
        target: target.map(String::from),
    };
    assert_eq!(
        run(fixture::ready(), "Paste").1,
        [paste(PasteMode::Original, Some("0xabc"))]
    );
    assert_eq!(
        run(fixture::ready(), "Paste as plain text").1,
        [paste(PasteMode::Plain, Some("0xabc"))]
    );
    assert_eq!(
        run(fixture::ready(), "Copy").1,
        [paste(PasteMode::Original, None)]
    );
    assert!(run(fixture::ready(), "View").0.find("peek").is_some());
    assert_eq!(
        run(fixture::ready(), "Pin").1,
        [feed::Command::Pin { id: 3, on: true }]
    );
    let (deleted, sent) = run(fixture::ready(), "Delete");
    assert_eq!(sent, [feed::Command::Delete { id: 3 }]);
    assert_eq!(deleted.state.clips.len(), 11);
    assert_eq!(
        run(fixture::ready(), "Pause capture").1,
        [feed::Command::Pause(true)]
    );
    let (options, sent) = run(fixture::ready(), "Options\u{2026}");
    assert_eq!(sent, [feed::Command::Options]);
    assert!(closing(&options));
}

#[test]
fn a_filter_narrows_the_list_and_its_tag_takes_it_back() {
    let mut runtime = runtime(fixture::ready());
    arrive(&mut runtime, fixture::clips());
    feed::take_sent();
    press(&mut runtime, "menu-button");
    let images = entry(&runtime.state, "Images");
    press(&mut runtime, &format!("menu-{images}"));
    let generation = runtime.state.generation;
    assert_eq!(
        acts(feed::take_sent()),
        [feed::Command::Query {
            query: String::new(),
            filter: Filter::Images,
            generation
        }]
    );
    assert!(runtime.find("filter").is_some());
    assert!(says(&runtime, "Images"));
    let chosen: Vec<&str> = runtime
        .state
        .menu_items()
        .iter()
        .filter(|item| item.chosen == Some(true))
        .map(|item| item.label)
        .collect();
    assert_eq!(chosen, ["Images"]);

    press(&mut runtime, "menu-button");
    press(&mut runtime, &format!("menu-{images}"));
    assert!(
        acts(feed::take_sent()).is_empty(),
        "the filter already on asks for nothing"
    );

    press(&mut runtime, "filter");
    assert_eq!(runtime.state.filter, Filter::All);
    assert!(runtime.find("filter").is_none());
    assert!(matches!(
        acts(feed::take_sent())[..],
        [feed::Command::Query {
            filter: Filter::All,
            ..
        }]
    ));
    assert!(runtime.state.query.focused);
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
    assert_eq!(
        acts(feed::take_sent()),
        [
            feed::Command::Pin { id: 3, on: true },
            feed::Command::Delete { id: 3 }
        ]
    );

    let mut both = fixture::chosen(1);
    both.clips[0].sticky = true;
    let mut runtime = super::tests::runtime(both);
    feed::take_sent();
    key(&mut runtime, Key::Command('p'), ctrl());
    assert!(!runtime.state.clips[0].favorite && !runtime.state.clips[0].sticky);
    assert_eq!(
        acts(feed::take_sent()),
        [feed::Command::Pin { id: 1, on: false }]
    );
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
    assert!(runtime.find("undo-button").is_some());

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
    assert!(runtime.find("undo").is_none());
    assert_eq!(
        acts(feed::take_sent()),
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
    assert!(runtime.find("undo").is_some());
    deliver(&mut runtime, feed::Event::Deleted(4));
    assert!(
        runtime.find("undo").is_none(),
        "the way back goes when the clip is gone"
    );
    assert!(runtime.state.undo.is_empty());
}

#[test]
fn delete_edits_the_search_while_there_is_one() {
    let mut runtime = runtime(fixture::ready());
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
}

#[test]
fn a_link_opens_and_other_text_does_not() {
    let mut runtime = runtime(fixture::ready());
    feed::take_sent();
    key(&mut runtime, Key::Command('o'), ctrl());
    assert!(acts(feed::take_sent()).is_empty());
    assert!(!closing(&runtime));

    press(&mut runtime, "clip-5");
    feed::take_sent();
    key(&mut runtime, Key::Command('o'), ctrl());
    assert_eq!(
        acts(feed::take_sent()),
        [feed::Command::Open(
            "https://ui.fabrials.com/docs/design".into()
        )]
    );
    assert!(closing(&runtime));

    let mut state = fixture::ready();
    state.selected = 4;
    state.clips[4].byte_len = 9000;
    let mut cut = super::tests::runtime(state);
    feed::take_sent();
    key(&mut cut, Key::Command('o'), ctrl());
    assert!(
        acts(feed::take_sent()).is_empty(),
        "an address seen only in part is not opened"
    );
    assert!(!labels(&cut.state).contains(&"Open link"));
}

#[test]
fn control_comma_opens_the_options_and_this_window_makes_way() {
    let mut runtime = runtime(fixture::ready());
    feed::take_sent();
    key(&mut runtime, Key::Command(','), ctrl());
    assert_eq!(acts(feed::take_sent()), [feed::Command::Options]);
    assert!(closing(&runtime));
}

#[test]
fn typing_a_letter_that_is_also_a_shortcut_only_types() {
    let mut runtime = runtime(fixture::ready());
    feed::take_sent();
    for letter in ["d", "p", "m", "o", "z", "1"] {
        key(&mut runtime, Key::Text(letter.into()), Mods::default());
    }
    assert_eq!(runtime.state.clips.len(), 12);
    assert!(!runtime.state.clips[2].favorite);
    assert_eq!(runtime.state.menu, None);
    assert_eq!(runtime.state.query.text, "dpmoz1");
    assert!(
        acts(feed::take_sent())
            .iter()
            .all(|command| matches!(command, feed::Command::Query { .. }))
    );
}

#[test]
fn escape_leaves_the_clip_then_the_search_then_the_window() {
    let mut runtime = runtime(fixture::ready());
    runtime.state.query = TextEdit::new("cargo");
    runtime.state.query.focused = true;
    runtime.state.peek = true;
    key(&mut runtime, Key::Escape, Mods::default());
    assert!(!runtime.state.peek);
    assert_eq!(runtime.state.query.text, "cargo");
    key(&mut runtime, Key::Escape, Mods::default());
    assert_eq!(runtime.state.query.text, "");
    assert!(!closing(&runtime));
    key(&mut runtime, Key::Escape, Mods::default());
    assert!(closing(&runtime));
}

#[test]
fn typing_a_search_goes_back_to_the_list() {
    let mut runtime = runtime(fixture::ready());
    key(&mut runtime, Key::Tab, Mods::default());
    assert!(runtime.find("peek").is_some());
    key(&mut runtime, Key::Text("c".into()), Mods::default());
    assert!(runtime.find("peek").is_none());
}

#[test]
fn a_search_marks_what_it_found_in_the_rows() {
    let mut state = fixture::ready();
    state.query = TextEdit::new("CARGO work");
    assert_eq!(
        view::found(&state, "cargo test --workspace"),
        [(0, 5), (13, 17)]
    );
    state.query = TextEdit::new("  ");
    assert!(view::found(&state, "cargo test --workspace").is_empty());
}

#[test]
fn the_line_above_the_search_is_there_only_while_it_has_something_to_say() {
    let noted = runtime(State {
        note: Some("That clip is gone.".into()),
        ..fixture::ready()
    });
    assert!(noted.find("note").is_some() && says(&noted, "That clip is gone."));

    let mut failing = fixture::ready();
    failing.watcher = Some(WatcherStatus {
        running: true,
        paused: false,
        backend: "wl-clipboard".into(),
        last_error: Some("clipboard: exit 1".into()),
    });
    let failing = runtime(failing);
    assert!(says(&failing, "Last capture failed: clipboard: exit 1"));

    let mut paused = fixture::ready();
    paused.watcher = Some(WatcherStatus {
        paused: true,
        ..paused.watcher.clone().unwrap()
    });
    let mut paused = runtime(paused);
    assert!(says(&paused, "Capture is paused"));
    feed::take_sent();
    press(&mut paused, "resume");
    assert_eq!(acts(feed::take_sent()), [feed::Command::Pause(false)]);
    key(
        &mut paused,
        Key::Command('p'),
        Mods {
            shift: true,
            ..ctrl()
        },
    );
    assert_eq!(acts(feed::take_sent()), [feed::Command::Pause(false)]);

    let mut watching = runtime(fixture::ready());
    feed::take_sent();
    key(
        &mut watching,
        Key::Command('p'),
        Mods {
            shift: true,
            ..ctrl()
        },
    );
    assert_eq!(acts(feed::take_sent()), [feed::Command::Pause(true)]);
    assert!(!watching.state.clips[2].favorite, "and it did not pin");
}

#[test]
fn every_empty_state_says_what_happened() {
    let empty = runtime(State {
        clips: Vec::new(),
        ..fixture::ready()
    });
    assert!(says(&empty, "Nothing copied yet"));

    let mut searching = State {
        clips: Vec::new(),
        ..fixture::ready()
    };
    searching.query = TextEdit::new("zzzz");
    let searching = runtime(searching);
    assert!(says(&searching, "Nothing matches \u{201c}zzzz\u{201d}"));

    for (filter, said) in [
        (Filter::Images, "No images"),
        (Filter::Text, "No text clips"),
        (Filter::Favorites, "No pinned clips"),
    ] {
        let filtered = runtime(State {
            clips: Vec::new(),
            filter,
            ..fixture::ready()
        });
        assert!(says(&filtered, said), "{said}");
    }

    let mut offline = runtime(fixture::ready());
    key(&mut offline, Key::Tab, Mods::default());
    key(&mut offline, Key::Command('m'), ctrl());
    deliver(&mut offline, feed::Event::Offline("no socket".into()));
    for needle in [
        "The daemon is not running",
        "systemctl --user start ditox",
        "no socket",
    ] {
        assert!(says(&offline, needle), "{needle}");
    }
    assert!(offline.find("peek").is_none() && offline.find("menu").is_none());
    assert!(
        !labels(&offline.state)
            .iter()
            .any(|label| label.contains("capture"))
    );

    let connecting = runtime(State::blank());
    assert!(
        texts(&connecting)
            .iter()
            .all(|text| text.is_empty() || text == "Search")
    );
}

#[test]
fn the_window_closes_when_the_keyboard_goes_elsewhere_if_it_had_it() {
    let mut runtime = runtime(fixture::ready());
    runtime.update(|state, tx| on_focus(state, false, tx));
    assert!(
        !closing(&runtime),
        "a window that never had the keyboard stays"
    );
    runtime.update(|state, tx| on_focus(state, true, tx));
    assert!(!closing(&runtime));
    runtime.update(|state, tx| on_focus(state, false, tx));
    assert!(closing(&runtime));

    let mut state = fixture::ready();
    state.ui.close_on_blur = false;
    let mut staying = super::tests::runtime(state);
    staying.update(|state, tx| on_focus(state, true, tx));
    staying.update(|state, tx| on_focus(state, false, tx));
    assert!(!closing(&staying));
}

#[test]
fn a_press_outside_the_window_puts_it_away_and_one_inside_does_not() {
    for (x, y) in [
        (SIZE.0 / 2.0, 20.0),
        (SIZE.0 / 2.0, SIZE.1 - 150.0),
        (SIZE.0 - 20.0, SIZE.1 - 20.0),
        (1.0, 1.0),
        (SIZE.0 - 1.0, SIZE.1 - 1.0),
    ] {
        let mut runtime = runtime(fixture::ready());
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(!closing(&runtime), "inside, at {x}, {y}");
        assert!(runtime.state.query.focused);
    }
    let mut empty = runtime(State {
        clips: Vec::new(),
        ..fixture::ready()
    });
    for (x, y) in [
        (SIZE.0 / 2.0, 100.0),
        (SIZE.0 / 2.0, 1.0),
        (1.0, SIZE.1 / 2.0),
        (SIZE.0 - 1.0, SIZE.1 / 2.0),
        (SIZE.0 / 2.0, SIZE.1 - 1.0),
    ] {
        empty.pointer_down(x, y);
        empty.pointer_up(x, y);
        empty.frame(0.0);
        assert!(
            !closing(&empty),
            "the empty part of the window is still the window, at {x}, {y}"
        );
    }

    for (x, y) in [
        (-40.0, 200.0),
        (SIZE.0 + 300.0, 200.0),
        (200.0, -1.0),
        (200.0, SIZE.1 + 900.0),
        (-2000.0, -1500.0),
    ] {
        let mut runtime = runtime(fixture::ready());
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(closing(&runtime), "outside, at {x}, {y}");
    }
}

#[test]
fn every_scene_draws_in_both_themes_and_at_the_smallest_size() {
    for (name, scene) in fixture::SCENES {
        for size in [SIZE, MIN_SIZE] {
            for dark in [true, false] {
                let mut state = State::blank();
                scene(&mut state);
                state.dark = Some(dark);
                let listed = !state.clips.is_empty() && !state.peek;
                let menu = state.menu.is_some();
                let mut runtime = Runtime::new(
                    state,
                    view,
                    size,
                    Env {
                        system_dark: dark,
                        scale: 2.0,
                    },
                );
                runtime.frame(0.0);
                let at = format!("{name} at {size:?}, dark {dark}");
                for key in ["search", "menu-button"] {
                    let (x, y) = runtime.find(key).unwrap_or_else(|| panic!("{key} in {at}"));
                    assert!(
                        x >= 0.0 && x <= size.0 && y > size.1 - 44.0 && y <= size.1,
                        "{key} sits at the bottom in {at}"
                    );
                }
                assert_eq!(runtime.find("list").is_some(), listed, "{at}");
                if menu {
                    let sheet = runtime
                        .elements()
                        .into_iter()
                        .find(|element| element.key.as_deref() == Some("menu"))
                        .unwrap_or_else(|| panic!("menu in {at}"));
                    let [x, y, width, height] = sheet.rect;
                    assert!(
                        x >= 0.0 && y >= 0.0 && x + width <= size.0 && y + height <= size.1,
                        "the menu is inside the window in {at}: {:?}",
                        sheet.rect
                    );
                }
            }
        }
    }
}
