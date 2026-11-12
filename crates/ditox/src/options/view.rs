use ditox_rpc::{MAX_LINES, MIN_LINES, Theme};
use rune::{
    Color, Cursor, Cx, El, ScrollSpec, TextEdit, col, el, icon, row, spacer, text, text_input,
};

use super::{Load, State, TABS, Tab};
use crate::theme::{self, Palette};

const SCROLL: ScrollSpec = ScrollSpec {
    stick_bottom: false,
};
const FIELD_HEIGHT: f32 = 30.0;
const NUMBER_WIDTH: f32 = 84.0;
const SHORTCUT_WIDTH: f32 = 168.0;
const LIST_LINE: f32 = 20.0;

/// The keys of the clip list, for the Keys tab.
const MOVE_KEYS: [(&str, &str); 6] = [
    ("Up / Down", "Move through the clips"),
    ("PgUp / PgDn", "Move eight at a time"),
    ("Ctrl+Home / End", "First and last clip"),
    ("Tab", "See the clip whole, and back"),
    ("Ctrl+M", "Open the menu"),
    ("Esc", "Clear the search, then close"),
];

const CLIP_KEYS: [(&str, &str); 9] = [
    ("Enter", "Paste into the window you came from"),
    ("Shift+Enter", "Paste as plain text"),
    ("Ctrl+Enter", "Copy without pasting"),
    ("Ctrl+1 \u{2026} 9", "Paste the clip with that number"),
    ("Ctrl+O", "Open a link"),
    ("Ctrl+P", "Pin or unpin"),
    ("Del", "Delete; Ctrl+D while searching"),
    ("Ctrl+Z", "Undo a delete"),
    ("Ctrl+Shift+P", "Pause or resume capture"),
];

pub fn view(state: &State, cx: &mut Cx<State>) -> El {
    let p = theme::palette(state.dark.unwrap_or_else(|| cx.system_dark()));
    col()
        .w_full()
        .h_full()
        .fill(p.page)
        .role("main")
        .child(tabs(state, &p))
        .child(rule(&p))
        .child(
            col()
                .key("page")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(SCROLL)
                .child(page(state, &p)),
        )
        .child(rule(&p))
        .child(footer(state, &p))
}

fn rule(p: &Palette) -> El {
    el().w_full().h(1.0).fill(p.separator)
}

fn tabs(state: &State, p: &Palette) -> El {
    let mut strip = row()
        .key("tabs")
        .role("tablist")
        .label("Options")
        .w_full()
        .h(44.0)
        .px(12.0)
        .gap(2.0)
        .focus_group();
    for (tab, name) in TABS {
        let on = state.tab == tab;
        strip = strip.child(
            col()
                .key(format!("tab-{name}"))
                .role("tab")
                .label(name)
                .checked(on)
                .focusable(true)
                .focus_selected(on)
                .h_full()
                .px(12.0)
                .center()
                .cursor(Cursor::Pointer)
                .on_press_tx(move |state: &mut State, tx| super::press_tab(state, tab, tx))
                .child(
                    text(name)
                        .font(theme::CONTROL)
                        .color(if on { p.ink } else { p.ink_muted }),
                )
                .child(
                    el().pin(Some(8.0), None, Some(8.0), Some(0.0))
                        .h(2.0)
                        .radius(1.0)
                        .fill(if on { p.brand } else { Color::TRANSPARENT }),
                ),
        );
    }
    strip
}

fn page(state: &State, p: &Palette) -> El {
    let page = col().w_full().padding(20.0).gap(16.0);
    match &state.load {
        Load::Ready => {}
        Load::Loading => return page,
        Load::Offline(reason) if state.tab != Tab::Keys => {
            return page
                .key("offline")
                .gap(6.0)
                .child(
                    text("The daemon is not running")
                        .font(theme::STRONG)
                        .color(p.ink),
                )
                .child(
                    text("Its settings show here once it is. Start it with:")
                        .font(theme::META)
                        .color(p.ink_muted)
                        .wrap(),
                )
                .child(
                    row()
                        .mt(4.0)
                        .h(28.0)
                        .px(10.0)
                        .radius(theme::RADIUS_TAG)
                        .center_y()
                        .self_start()
                        .fill(p.well)
                        .child(
                            text(crate::model::START_DAEMON)
                                .font(theme::CODE)
                                .color(p.ink),
                        ),
                )
                .child(
                    text(reason.clone())
                        .font(theme::LABEL)
                        .color(p.ink_muted)
                        .wrap_anywhere(),
                );
        }
        Load::Offline(_) => {}
    }
    let page = if state.load == Load::Ready && !state.writable() && state.tab != Tab::Keys {
        page.child(
            text("The settings file was put there by a system configuration, so Ditox cannot change it. What you see is what it says.")
                .key("read-only")
                .font(theme::META)
                .color(p.warning)
                .wrap(),
        )
    } else {
        page
    };
    match state.tab {
        Tab::General => opening_key(page, state, p)
            .child(setting(
                "Clips to keep",
                "The oldest go first. Pinned clips stay.",
                number(
                    "max-entries",
                    &state.max_entries,
                    "Clips to keep",
                    |state| &mut state.max_entries,
                    p,
                ),
                p,
            ))
            .child(setting(
                "Delete clips after",
                "In days. 0 keeps them until the history is full.",
                number(
                    "days",
                    &state.days,
                    "Delete clips after, in days",
                    |state| &mut state.days,
                    p,
                ),
                p,
            ))
            .child(check(
                "allow-duplicates",
                "Keep a clip again each time it is copied",
                state.allow_duplicates,
                |state| state.allow_duplicates = !state.allow_duplicates,
                p,
            ))
            .child(rule(p))
            .child(setting(
                "Theme",
                "",
                segments(
                    "theme",
                    "Theme",
                    [Theme::System, Theme::Dark, Theme::Light].map(|theme| {
                        (
                            match theme {
                                Theme::System => "System",
                                Theme::Dark => "Dark",
                                Theme::Light => "Light",
                            }
                            .to_string(),
                            state.theme == theme,
                            Pick::Theme(theme),
                        )
                    }),
                    p,
                ),
                p,
            ))
            .child(setting(
                "Lines per clip",
                "How much of each clip the list shows.",
                segments(
                    "lines",
                    "Lines per clip",
                    (MIN_LINES..=MAX_LINES)
                        .map(|lines| (lines.to_string(), state.lines == lines, Pick::Lines(lines))),
                    p,
                ),
                p,
            ))
            .child(check(
                "close-on-blur",
                "Close the list when another window takes the keyboard",
                state.close_on_blur,
                |state| state.close_on_blur = !state.close_on_blur,
                p,
            )),
        Tab::Paste => page
            .child(check(
                "paste-enabled",
                "Paste into the window I came from",
                state.paste_enabled,
                |state| state.paste_enabled = !state.paste_enabled,
                p,
            ))
            .child(setting(
                "Shortcut",
                "What Ditox presses to paste.",
                field(
                    "keybind",
                    &state.keybind,
                    SHORTCUT_WIDTH,
                    "Paste shortcut",
                    |state| &mut state.keybind,
                    p,
                ),
                p,
            ))
            .child(setting(
                "Shortcut in terminals",
                "",
                field(
                    "terminal-keybind",
                    &state.terminal_keybind,
                    SHORTCUT_WIDTH,
                    "Paste shortcut in terminals",
                    |state| &mut state.terminal_keybind,
                    p,
                ),
                p,
            ))
            .child(setting(
                "Wait before pasting",
                "In milliseconds, for the window to take the keyboard back.",
                number(
                    "buffer-ms",
                    &state.buffer_ms,
                    "Wait before pasting, in milliseconds",
                    |state| &mut state.buffer_ms,
                    p,
                ),
                p,
            ))
            .child(list(
                "terminal-apps",
                "Terminals",
                "Window classes that get the terminal shortcut, one per line.",
                &state.terminal_apps,
                |state| &mut state.terminal_apps,
                6,
                p,
            )),
        Tab::Privacy => page
            .child(list(
                "excluded-apps",
                "Never save what is copied in these apps",
                "Window classes, one per line.",
                &state.excluded_apps,
                |state| &mut state.excluded_apps,
                3,
                p,
            ))
            .child(list(
                "excluded-windows",
                "Nor in windows with these titles",
                "Part of a title, one per line.",
                &state.excluded_windows,
                |state| &mut state.excluded_windows,
                2,
                p,
            ))
            .child(list(
                "secret-patterns",
                "Nor text that contains any of these",
                "One per line. A pattern like AKIA[A-Z0-9]{16} is a start and a run of characters.",
                &state.secret_patterns,
                |state| &mut state.secret_patterns,
                4,
                p,
            )),
        Tab::Keys => {
            page.child(keys("In the list", &MOVE_KEYS, p))
                .child(keys("On a clip", &CLIP_KEYS, p))
        }
        Tab::About => {
            let Some(loaded) = &state.loaded else {
                return page;
            };
            page.gap(12.0)
                .child(text("Ditox").font(theme::HEADING).color(p.ink))
                .child(fact("Version", env!("CARGO_PKG_VERSION"), p))
                .child(fact("Daemon", &loaded.version, p))
                .child(fact("Settings", &loaded.path, p))
                .child(fact("History", &loaded.database, p))
        }
    }
}

/// A label, the line under it, and the control at the end of the row.
/// The key that opens the list, first on its page. Only where the daemon is
/// what listens for it: on Linux the compositor binds a key to `ditox`.
fn opening_key(page: El, state: &State, p: &Palette) -> El {
    if !cfg!(windows) {
        return page;
    }
    page.child(setting(
        "Key that opens Ditox",
        "Pressed anywhere. Empty for none.",
        field(
            "open-keybind",
            &state.open_keybind,
            SHORTCUT_WIDTH,
            "Key that opens Ditox",
            |state| &mut state.open_keybind,
            p,
        ),
        p,
    ))
    .child(rule(p))
}

fn setting(label: &'static str, hint: &'static str, control: El, p: &Palette) -> El {
    let mut words = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(label).font(theme::BODY).color(p.ink));
    if !hint.is_empty() {
        words = words.child(text(hint).font(theme::META).color(p.ink_muted).wrap());
    }
    row()
        .w_full()
        .gap(16.0)
        .center_y()
        .child(words)
        .child(control)
}

fn input(
    key: &'static str,
    edit: &TextEdit,
    label: &'static str,
    access: fn(&mut State) -> &mut TextEdit,
    p: &Palette,
) -> El {
    text_input(edit, theme::CODE)
        .key(key)
        .label(label)
        .grow(1.0)
        .min_w(0.0)
        .color(p.ink)
        .caret(p.brand_ink, p.brand_ink.alpha(0.3))
        .no_focus_ring()
        .on_edit(access)
        .on_input_key(super::on_key)
}

fn boxed(key: &'static str, p: &Palette) -> El {
    row()
        .key(format!("{key}-box"))
        .radius(theme::RADIUS_TAG)
        .fill(p.well)
        .border(1.0, p.border)
        .focus_within_ring(1.5, -1.0, p.brand)
}

fn field(
    key: &'static str,
    edit: &TextEdit,
    width: f32,
    label: &'static str,
    access: fn(&mut State) -> &mut TextEdit,
    p: &Palette,
) -> El {
    boxed(key, p)
        .w(width)
        .h(FIELD_HEIGHT)
        .px(9.0)
        .center_y()
        .child(input(key, edit, label, access, p).single_line())
}

fn number(
    key: &'static str,
    edit: &TextEdit,
    label: &'static str,
    access: fn(&mut State) -> &mut TextEdit,
    p: &Palette,
) -> El {
    field(key, edit, NUMBER_WIDTH, label, access, p)
}

/// A list of entries, one per line, that scrolls past `lines` of them.
fn list(
    key: &'static str,
    label: &'static str,
    hint: &'static str,
    edit: &TextEdit,
    access: fn(&mut State) -> &mut TextEdit,
    lines: usize,
    p: &Palette,
) -> El {
    col()
        .w_full()
        .gap(6.0)
        .child(text(label).font(theme::BODY).color(p.ink))
        .child(
            boxed(key, p).w_full().px(9.0).py(6.0).child(
                col()
                    .key(format!("{key}-scroll"))
                    .w_full()
                    .h(lines as f32 * LIST_LINE)
                    .scroll(SCROLL)
                    .child(input(key, edit, label, access, p).w_full()),
            ),
        )
        .child(text(hint).font(theme::META).color(p.ink_muted).wrap())
}

fn check(
    key: &'static str,
    label: &'static str,
    on: bool,
    flip: fn(&mut State),
    p: &Palette,
) -> El {
    let mark = row()
        .size(16.0, 16.0)
        .radius(theme::RADIUS_CHECK)
        .center()
        .fill(if on { p.brand } else { p.well })
        .border(1.0, if on { p.brand } else { p.border });
    row()
        .key(key)
        .role("checkbox")
        .label(label)
        .checked(on)
        .focusable(true)
        .self_start()
        .gap(10.0)
        .center_y()
        .cursor(Cursor::Pointer)
        .on_press_tx(move |state: &mut State, tx| {
            tx.instant();
            flip(state);
        })
        .child(if on {
            mark.child(icon(&theme::ICON_CHECK, 12.0).color(p.on_brand))
        } else {
            mark
        })
        .child(text(label).font(theme::BODY).color(p.ink))
}

#[derive(Clone, Copy)]
enum Pick {
    Theme(Theme),
    Lines(u32),
}

/// One of a few choices, side by side.
fn segments(
    key: &'static str,
    label: &'static str,
    choices: impl IntoIterator<Item = (String, bool, Pick)>,
    p: &Palette,
) -> El {
    let mut group = row()
        .key(key)
        .role("radiogroup")
        .label(label)
        .h(FIELD_HEIGHT)
        .padding(2.0)
        .gap(2.0)
        .radius(theme::RADIUS_TAG)
        .fill(p.well)
        .border(1.0, p.border)
        .center_y()
        .focus_group();
    for (name, on, pick) in choices {
        group = group.child(
            row()
                .key(format!("{key}-{name}"))
                .role("radio")
                .label(name.clone())
                .checked(on)
                .focusable(true)
                .focus_selected(on)
                .h_full()
                .min_w(30.0)
                .px(10.0)
                .radius(theme::RADIUS_CHECK)
                .center()
                .fill(if on { p.selected } else { Color::TRANSPARENT })
                .cursor(Cursor::Pointer)
                .on_press_tx(move |state: &mut State, tx| match pick {
                    Pick::Theme(theme) => super::press_theme(state, theme, tx),
                    Pick::Lines(lines) => super::press_lines(state, lines, tx),
                })
                .child(
                    text(name)
                        .font(theme::CONTROL)
                        .color(if on { p.ink } else { p.ink_muted }),
                ),
        );
    }
    group
}

fn keys(name: &'static str, keys: &[(&'static str, &'static str)], p: &Palette) -> El {
    let mut block = col()
        .w_full()
        .gap(7.0)
        .child(text(name).font(theme::LABEL).color(p.ink_muted).mb(2.0));
    for (chord, action) in keys {
        block = block.child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    row()
                        .w(150.0)
                        .child(text(*chord).font(theme::CODE).color(p.ink)),
                )
                .child(
                    text(*action)
                        .font(theme::BODY)
                        .color(p.ink_muted)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                ),
        );
    }
    block
}

fn fact(name: &'static str, value: &str, p: &Palette) -> El {
    row()
        .w_full()
        .gap(12.0)
        .items_start()
        .child(
            row()
                .w(90.0)
                .child(text(name).font(theme::BODY).color(p.ink_muted)),
        )
        .child(
            text(value.to_string())
                .key(format!("fact-{name}"))
                .font(theme::CODE)
                .color(p.ink)
                .selectable(true)
                .grow(1.0)
                .min_w(0.0)
                .wrap_anywhere(),
        )
}

fn footer(state: &State, p: &Palette) -> El {
    let can_save = state.writable() && !state.saving;
    let button = |key: &'static str, label: &'static str, primary: bool| {
        row()
            .key(key)
            .role("button")
            .label(label)
            .focusable(true)
            .h(32.0)
            .min_w(84.0)
            .px(14.0)
            .radius(theme::RADIUS_CONTROL)
            .center()
            .cursor(Cursor::Pointer)
            .fill(if primary { p.brand } else { p.well })
            .border(1.0, if primary { p.brand } else { p.border })
            .child(
                text(label)
                    .font(theme::CONTROL)
                    .color(if primary { p.on_brand } else { p.ink }),
            )
    };
    let mut bar = row()
        .key("footer")
        .w_full()
        .h(56.0)
        .px(16.0)
        .gap(10.0)
        .center_y();
    bar = match &state.error {
        Some(error) => bar.child(
            text(error.clone())
                .key("error")
                .role("alert")
                .font(theme::META)
                .color(p.danger)
                .grow(1.0)
                .min_w(0.0)
                .max_lines(2),
        ),
        None => bar.child(spacer()),
    };
    bar = bar.child(button("cancel", "Cancel", false).on_press_tx(super::press_cancel));
    bar.child(if can_save {
        button("save", "Save", true)
            .default_button()
            .on_press_tx(super::press_save)
    } else {
        button("save", "Save", true).disabled(true).opacity(0.45)
    })
}
