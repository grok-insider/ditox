use ditox_rpc::{Settings, Theme};
use rune::{Env, Key, KeyEvent, Mods, Runtime, Shell, TextEdit};

use super::view::view;
use super::*;
use crate::fixture;

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
    runtime.on_key = Some(on_key);
    runtime.frame(0.0);
    runtime
}

fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    }
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

fn told(runtime: &mut Runtime<State>, told: Told) {
    runtime.update(|state, tx| apply(state, told, tx));
    runtime.frame(0.0);
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

fn closing(runtime: &Runtime<State>) -> bool {
    runtime.shell_request == Some(Shell::Close)
}

fn loaded() -> Settings {
    fixture::config_view().settings
}

#[test]
fn the_fields_show_what_the_daemon_holds_and_say_it_back_unchanged() {
    let state = fixture::options(Tab::General);
    assert_eq!(state.settings().unwrap(), loaded());
    assert_eq!(state.max_entries.text, "1000");
    assert_eq!(state.days.text, "0");
    assert_eq!(state.terminal_apps.text.lines().count(), 7);
    assert_eq!(state.dark, Some(true));

    let runtime = runtime(state);
    for needle in [
        "Clips to keep",
        "Delete clips after",
        "Theme",
        "Lines per clip",
    ] {
        assert!(says(&runtime, needle), "{needle}");
    }
    for element in [
        "tabs",
        "max-entries",
        "days",
        "theme",
        "lines",
        "save",
        "cancel",
    ] {
        assert!(runtime.find(element).is_some(), "{element}");
    }
    assert!(runtime.find("read-only").is_none() && runtime.find("error").is_none());
}

#[test]
fn a_time_the_file_gave_in_seconds_is_kept_until_the_days_change() {
    let mut view = fixture::config_view();
    view.settings.delete_after_seconds = 3_600;
    let mut state = State::blank();
    state.show(view);
    assert_eq!(state.days.text, "1", "an hour is not shown as never");
    assert_eq!(state.settings().unwrap().delete_after_seconds, 3_600);
    state.days = TextEdit::new("2");
    assert_eq!(state.settings().unwrap().delete_after_seconds, 2 * 86_400);
    state.days = TextEdit::new("0");
    assert_eq!(state.settings().unwrap().delete_after_seconds, 0);
    state.days = TextEdit::new("99999");
    assert!(state.settings().is_err());
}

#[test]
fn what_is_changed_is_what_is_saved_and_the_window_closes_when_it_is() {
    let mut state = fixture::options(Tab::General);
    state.max_entries = TextEdit::new(" 250 ");
    state.allow_duplicates = true;
    state.lines = 4;
    state.theme = Theme::Light;
    state.close_on_blur = false;
    state.paste_enabled = false;
    state.buffer_ms = TextEdit::new("80");
    state.keybind = TextEdit::new(" shift+insert ");
    state.terminal_apps = TextEdit::new("kitty\n\n  foot  \n");
    state.excluded_windows = TextEdit::new("Vault");
    state.secret_patterns = TextEdit::new("");
    let mut runtime = runtime(state);
    take_sent();
    key(&mut runtime, Key::Command('s'), ctrl());
    let sent = take_sent();
    assert_eq!(
        sent,
        [Settings {
            max_entries: 250,
            allow_duplicates: true,
            paste_enabled: false,
            paste_buffer_ms: 80,
            paste_keybind: "shift+insert".into(),
            terminal_apps: vec!["kitty".into(), "foot".into()],
            excluded_windows: vec!["Vault".into()],
            secret_patterns: Vec::new(),
            ui: Ui {
                theme: Theme::Light,
                lines: 4,
                close_on_blur: false,
            },
            ..loaded()
        }]
    );
    assert!(runtime.state.saving);
    key(&mut runtime, Key::Command('s'), ctrl());
    press(&mut runtime, "save");
    assert!(take_sent().is_empty(), "one save at a time");

    told(
        &mut runtime,
        Told::Refused("\"ctrl+\" is not a paste keybind".into()),
    );
    assert!(!runtime.state.saving && !closing(&runtime));
    assert!(says(&runtime, "\"ctrl+\" is not a paste keybind."));
    press(&mut runtime, "save");
    assert_eq!(take_sent().len(), 1, "and it can be tried again");
    assert!(runtime.find("error").is_none());
    told(&mut runtime, Told::Saved);
    assert!(closing(&runtime));
}

#[test]
fn a_field_that_is_not_a_number_is_said_and_nothing_is_sent() {
    for (field, said) in [
        ("max", "Clips to keep takes a whole number."),
        ("days", "Delete clips after takes a whole number."),
        ("wait", "Wait before pasting takes a whole number."),
    ] {
        let mut state = fixture::options(Tab::General);
        let edit = match field {
            "max" => &mut state.max_entries,
            "days" => &mut state.days,
            _ => &mut state.buffer_ms,
        };
        *edit = TextEdit::new("many");
        let mut runtime = runtime(state);
        take_sent();
        press(&mut runtime, "save");
        assert!(take_sent().is_empty());
        assert!(!runtime.state.saving);
        assert!(says(&runtime, said), "{said}");
    }
}

#[test]
fn a_file_ditox_cannot_change_is_shown_and_not_saved() {
    let mut view = fixture::config_view();
    view.writable = false;
    let mut state = State::blank();
    state.show(view);
    let mut runtime = runtime(state);
    assert!(runtime.find("read-only").is_some());
    take_sent();
    press(&mut runtime, "save");
    key(&mut runtime, Key::Command('s'), ctrl());
    assert!(take_sent().is_empty());
    assert!(!runtime.state.saving && !closing(&runtime));
    press(&mut runtime, "cancel");
    assert!(closing(&runtime));
}

#[test]
fn the_tabs_answer_to_a_press_and_to_the_keyboard() {
    let mut runtime = runtime(fixture::options(Tab::General));
    press(&mut runtime, "tab-Paste");
    assert_eq!(runtime.state.tab, Tab::Paste);
    for element in [
        "paste-enabled",
        "keybind",
        "terminal-keybind",
        "buffer-ms",
        "terminal-apps",
    ] {
        assert!(runtime.find(element).is_some(), "{element}");
    }
    assert!(runtime.find("max-entries").is_none());

    key(&mut runtime, Key::Command('3'), ctrl());
    assert_eq!(runtime.state.tab, Tab::Privacy);
    for element in ["excluded-apps", "excluded-windows", "secret-patterns"] {
        assert!(runtime.find(element).is_some(), "{element}");
    }
    key(&mut runtime, Key::PageDown, ctrl());
    assert_eq!(runtime.state.tab, Tab::Keys);
    assert!(says(&runtime, "Paste the clip with that number"));
    key(&mut runtime, Key::PageDown, ctrl());
    assert_eq!(runtime.state.tab, Tab::About);
    for needle in [
        "Daemon",
        "/home/friend/.config/ditox/config.toml",
        "ditox-v2.db",
    ] {
        assert!(says(&runtime, needle), "{needle}");
    }
    key(&mut runtime, Key::PageDown, ctrl());
    assert_eq!(runtime.state.tab, Tab::General);
    key(&mut runtime, Key::PageUp, ctrl());
    assert_eq!(runtime.state.tab, Tab::About);

    key(&mut runtime, Key::Escape, Mods::default());
    assert!(closing(&runtime));
}

#[test]
fn the_choices_and_the_boxes_change_what_they_say() {
    let mut runtime = runtime(fixture::options(Tab::General));
    press(&mut runtime, "theme-Light");
    assert_eq!(runtime.state.theme, Theme::Light);
    assert_eq!(
        runtime.state.dark,
        Some(false),
        "the window shows it at once"
    );
    press(&mut runtime, "theme-System");
    assert_eq!(runtime.state.dark, None);
    press(&mut runtime, "lines-5");
    assert_eq!(runtime.state.lines, 5);
    press(&mut runtime, "allow-duplicates");
    assert!(runtime.state.allow_duplicates);
    press(&mut runtime, "close-on-blur");
    assert!(!runtime.state.close_on_blur);
    press(&mut runtime, "close-on-blur");
    assert!(runtime.state.close_on_blur);

    key(&mut runtime, Key::Command('2'), ctrl());
    press(&mut runtime, "paste-enabled");
    assert!(!runtime.state.paste_enabled);

    let settings = runtime.state.settings().unwrap();
    assert_eq!(
        settings.ui,
        Ui {
            theme: Theme::System,
            lines: 5,
            close_on_blur: true
        }
    );
    assert!(settings.allow_duplicates && !settings.paste_enabled);
}

#[test]
fn typing_in_a_field_changes_only_that_field() {
    let mut runtime = runtime(fixture::options(Tab::General));
    press(&mut runtime, "max-entries");
    key(&mut runtime, Key::Text("5".into()), Mods::default());
    assert_eq!(runtime.state.max_entries.text, "10005");
    assert_eq!(runtime.state.days.text, "0");
    key(&mut runtime, Key::Text("s".into()), Mods::default());
    assert_eq!(
        runtime.state.max_entries.text, "10005s",
        "a letter is typed, not a shortcut"
    );
    assert_eq!(runtime.state.tab, Tab::General);
}

#[test]
fn without_a_daemon_the_window_says_so_and_fills_in_when_it_answers() {
    let mut runtime = runtime(State::blank());
    assert!(runtime.find("save").is_some());
    told(&mut runtime, Told::Offline("no socket".into()));
    for needle in [
        "The daemon is not running",
        "systemctl --user start ditox",
        "no socket",
    ] {
        assert!(says(&runtime, needle), "{needle}");
    }
    take_sent();
    press(&mut runtime, "save");
    assert!(take_sent().is_empty());
    key(&mut runtime, Key::Command('4'), ctrl());
    assert!(says(&runtime, "Open the menu"), "the keys need no daemon");

    key(&mut runtime, Key::Command('1'), ctrl());
    told(&mut runtime, Told::Loaded(Box::new(fixture::config_view())));
    assert_eq!(runtime.state.load, Load::Ready);
    assert_eq!(runtime.state.max_entries.text, "1000");

    runtime.state.max_entries = TextEdit::new("7");
    let mut later = fixture::config_view();
    later.settings.max_entries = 3;
    told(&mut runtime, Told::Loaded(Box::new(later)));
    told(&mut runtime, Told::Offline("gone again".into()));
    assert_eq!(
        runtime.state.max_entries.text, "7",
        "what was typed is not replaced by a late answer"
    );
    assert_eq!(runtime.state.load, Load::Ready);
}

#[test]
fn the_daemons_reason_is_said_as_a_sentence() {
    assert_eq!(
        sentence("the history keeps between 1 and 9 clips"),
        "The history keeps between 1 and 9 clips."
    );
    assert_eq!(sentence("Already a sentence."), "Already a sentence.");
    assert_eq!(sentence("  "), "The settings were not saved.");
}

#[test]
fn every_scene_draws_in_both_themes_and_at_the_smallest_size() {
    for (name, scene) in fixture::OPTIONS_SCENES {
        for size in [SIZE, MIN_SIZE] {
            for dark in [true, false] {
                let mut state = State::blank();
                scene(&mut state);
                state.dark = Some(dark);
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
                for key in ["tabs", "tab-About", "footer", "cancel", "save"] {
                    let (x, y) = runtime.find(key).unwrap_or_else(|| panic!("{key} in {at}"));
                    assert!(
                        x >= 0.0 && x <= size.0 && y >= 0.0 && y <= size.1,
                        "{key} is on screen in {at}"
                    );
                }
                let (_, footer) = runtime.find("footer").unwrap();
                assert!(
                    footer > size.1 - 56.0,
                    "the buttons sit at the bottom in {at}"
                );
            }
        }
    }
}

#[test]
fn the_settings_are_read_from_a_daemon_and_saved_through_it() {
    let daemon = crate::feed::tests::Daemon::start("options");
    let socket = Some(daemon.socket.as_path());
    let view = load(socket).unwrap();
    assert!(view.writable);
    assert_eq!(view.settings.max_entries, 1000);

    let mut state = State::blank();
    state.show(view);
    state.max_entries = TextEdit::new("42");
    state.lines = 3;
    state.excluded_apps = TextEdit::new("Signal\nsay \"hi\", twice # really");
    save(socket, &state.settings().unwrap()).unwrap();

    let again = load(socket).unwrap();
    assert_eq!(again.settings, state.settings().unwrap());
    let written = std::fs::read_to_string(&again.path).unwrap();
    assert_eq!(ditox_rpc::Ui::parse(&written).lines, 3);
    assert!(written.contains("max_entries = 42"));

    state.keybind = TextEdit::new("ctrl+");
    let refused = save(socket, &state.settings().unwrap()).unwrap_err();
    assert_eq!(
        sentence(&refused.to_string()),
        "\"ctrl+\" is not a paste keybind."
    );
    assert_eq!(load(socket).unwrap().settings, again.settings);

    let nowhere = daemon.path("nowhere.sock");
    assert!(load(Some(Path::new(&nowhere))).is_err());
}
