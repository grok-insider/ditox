use ditox_rpc::{Entry, Filter, Kind};
use std::sync::Arc;

use rune::{
    Color, Cursor, Cx, El, GrowthSpec, Icon, ScrollSpec, Span, SurfaceFit, col, el, icon, row,
    spacer, text, text_input,
};

use crate::State;
use crate::feed::{Gone, LIST_LIMIT};
use crate::model::{self, FILTERS, Link, ListRow, PREVIEW_CHARS, PREVIEW_LINES, Shape, Tone};
use crate::theme::{self, Palette};

/// The list takes this share of the window, within these widths.
const LIST_SHARE: f32 = 0.39;
const LIST_WIDTH: (f32, f32) = (272.0, 440.0);
const ROW_HEIGHT: f32 = 34.0;
const IMAGE_UPSCALE: f32 = 2.0;
const ORIGIN_WIDTH: f32 = 300.0;
/// In a narrow window the facts keep this much and the origin gives way.
const FACTS_WIDTH: f32 = 150.0;
const CODE_LINE: f32 = 20.0;
const CODE_COLUMN: f32 = 7.8;
const SWATCH: f32 = 72.0;
const MATCH_WEIGHT: f32 = 600.0;
const SCROLL: ScrollSpec = ScrollSpec {
    stick_bottom: false,
};

pub fn view(state: &State, cx: &mut Cx<State>) -> El {
    let p = theme::palette(state.dark.unwrap_or_else(|| cx.system_dark()));
    crate::picture::set_scale(cx.env().scale);
    col()
        .w_full()
        .h_full()
        .fill(p.page)
        // A press on plain content would take the keyboard from the search
        // field. The window has one field; it keeps it.
        .preserve_focus_on_press(true)
        .on_press(|_: &mut State| {})
        .role("main")
        .child(top_bar(state, &p))
        .child(rule(&p))
        .child(body(state, &p))
        .child(rule(&p))
        .child(footer(state, &p))
}

fn rule(p: &Palette) -> El {
    el().w_full().h(1.0).fill(p.separator)
}

fn top_bar(state: &State, p: &Palette) -> El {
    let field = text_input(&state.query, theme::SEARCH)
        .key("search")
        .label("Search clipboard")
        .active_descendant(match state.current() {
            Some(clip) => format!("clip-{}", clip.id),
            None => String::new(),
        })
        .single_line()
        .grow(1.0)
        .min_w(0.0)
        .color(p.ink)
        .placeholder("Search clipboard", p.ink_muted)
        .caret(p.brand_ink, p.brand_ink.alpha(0.3))
        .no_focus_ring()
        .on_edit(|state: &mut State| &mut state.query)
        .on_input_change(|state: &mut State, _| crate::on_query_change(state))
        .on_input_key(crate::on_key);
    row()
        .key("top")
        .w_full()
        .h(52.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(icon(&theme::ICON_SEARCH, 16.0).color(p.ink_muted))
        .child(field)
        .child(if state.query.text.is_empty() {
            el()
        } else {
            icon_button(
                "clear",
                &theme::ICON_CLOSE,
                "Clear the search",
                p.ink_muted,
                p,
            )
            .on_press_tx(crate::press_clear)
        })
        .child(filters(state, p))
}

fn filters(state: &State, p: &Palette) -> El {
    let mut group = row()
        .key("filters")
        .role("tablist")
        .label("Show")
        .h(28.0)
        .padding(2.0)
        .gap(2.0)
        .radius(theme::RADIUS_CONTROL)
        .fill(p.well)
        .center_y();
    for (filter, name) in FILTERS {
        let on = state.filter == filter;
        let mut segment = row()
            .key(format!("filter-{name}"))
            .role("tab")
            .label(name)
            .checked(on)
            .h(24.0)
            .px(10.0)
            .radius(theme::RADIUS_TAG)
            .center()
            .fill(if on { p.segment } else { Color::TRANSPARENT })
            .cursor(Cursor::Pointer)
            .preserve_focus_on_press(true)
            .on_press_tx(move |state: &mut State, tx| crate::press_filter(state, filter, tx))
            .child(
                text(name)
                    .font(theme::CONTROL)
                    .color(if on { p.ink } else { p.ink_muted }),
            );
        if on {
            for shadow in p.raised() {
                segment = segment.box_shadow(shadow);
            }
        } else {
            segment = segment.hover(p.hover);
        }
        group = group.child(segment);
    }
    group
}

fn body(state: &State, p: &Palette) -> El {
    if state.keys_open {
        return keys_sheet(p);
    }
    let query = state.query.text.trim();
    let content = match &state.link {
        Link::Ready if !state.clips.is_empty() => {
            return row()
                .key("body")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .items_stretch()
                .child(list(state, p))
                .child(preview(state, p));
        }
        Link::Ready if query.is_empty() && state.filter == Filter::All => notice(
            "empty",
            "Nothing copied yet",
            "Copy text or an image and it shows up here.",
            None,
            None,
            p,
        ),
        Link::Ready if query.is_empty() => {
            let what = match state.filter {
                Filter::Text => "No text clips",
                Filter::Images => "No images",
                _ => "No pinned clips",
            };
            notice(
                "no-results",
                what,
                "Tab moves to the next filter.",
                None,
                None,
                p,
            )
        }
        Link::Ready => notice(
            "no-results",
            &format!("No clips match \u{201c}{query}\u{201d}"),
            "Try fewer letters. Esc clears the search.",
            None,
            None,
            p,
        ),
        Link::Connecting => notice("connecting", "Connecting to the daemon", "", None, None, p),
        Link::Offline(reason) => notice(
            "offline",
            "The daemon is not running",
            "Start it and this window fills in.",
            Some("systemctl --user start ditox"),
            Some(reason),
            p,
        ),
    };
    col()
        .key("body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(content)
}

fn notice(
    key: &'static str,
    title: &str,
    body: &str,
    command: Option<&str>,
    reading: Option<&str>,
    p: &Palette,
) -> El {
    let mut panel = col()
        .key(key)
        .w_full()
        .max_w(520.0)
        .padding(24.0)
        .gap(8.0)
        .child(text(title).font(theme::HEADING).color(p.ink));
    if !body.is_empty() {
        panel = panel.child(text(body).font(theme::BODY).color(p.ink_muted).wrap());
    }
    if let Some(command) = command {
        panel = panel.child(
            row()
                .mt(4.0)
                .h(28.0)
                .px(10.0)
                .radius(theme::RADIUS_TAG)
                .center_y()
                .self_start()
                .fill(p.well)
                .child(text(command).font(theme::CODE).color(p.ink)),
        );
    }
    if let Some(reading) = reading.filter(|reading| !reading.is_empty()) {
        panel = panel.child(text(reading).font(theme::LABEL).color(p.ink_muted).wrap());
    }
    panel
}

fn list(state: &State, p: &Palette) -> El {
    let p = *p;
    let layout = Arc::new(model::rows(
        &state.clips,
        !state.grouped,
        state.day_start_ms,
    ));
    let (keys, draws) = (layout.clone(), layout.clone());
    let rows = col()
        .key("rows")
        .w_full()
        .px(8.0)
        .py(6.0)
        .growth(GrowthSpec::default())
        .instant_changes()
        .virtual_rows(
            layout.len(),
            ROW_HEIGHT,
            move |state: &State, index| match keys.get(index) {
                Some(ListRow::Section(name)) => format!("section-{name}").into(),
                Some(ListRow::Clip(at)) => {
                    format!("clip-{}", state.clips.get(*at).map_or(0, |clip| clip.id)).into()
                }
                None => format!("row-{index}").into(),
            },
            move |state: &State, index| match draws.get(index) {
                Some(ListRow::Section(name)) => row()
                    .w_full()
                    .h(26.0)
                    .px(10.0)
                    .center_y()
                    .child(text(*name).font(theme::LABEL).color(p.ink_muted)),
                Some(ListRow::Clip(at)) => match state.clips.get(*at) {
                    Some(clip) => clip_row(state, clip, *at == state.selected, &p),
                    None => el(),
                },
                None => el(),
            },
        );
    col()
        .key("list")
        .role("listbox")
        .label("Clipboard history")
        .w_frac(LIST_SHARE)
        .min_w(LIST_WIDTH.0)
        .max_w(LIST_WIDTH.1)
        .min_h(0.0)
        .scroll(SCROLL)
        .scroll_edge_fade(10.0)
        .scroll_margin(8.0)
        .child(rows)
}

/// The mark at the start of a row: what kind of clip it is, or its colour.
fn mark(shape: Shape, on: bool, p: &Palette) -> El {
    let ink = if on { p.brand_ink } else { p.ink_muted };
    let glyph = match shape {
        Shape::Color(rgb) => {
            return row().size(14.0, 14.0).center().child(
                el().size(12.0, 12.0)
                    .radius(theme::RADIUS_SWATCH)
                    .fill(swatch(rgb))
                    .border(1.0, p.border),
            );
        }
        Shape::Image => &theme::ICON_IMAGE,
        Shape::Link => &theme::ICON_LINK,
        Shape::Text => &theme::ICON_TEXT,
    };
    icon(glyph, 14.0).color(ink)
}

fn swatch(rgb: [u8; 3]) -> Color {
    rune::color::hex(&format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
        .unwrap_or(Color::TRANSPARENT)
}

/// The stretches of a row's title to mark: what the search is looking for.
pub fn found(state: &State, title: &str) -> Vec<(usize, usize)> {
    model::matches(title, state.query.text.trim())
}

fn clip_row(state: &State, clip: &Entry, on: bool, p: &Palette) -> El {
    let id = clip.id;
    let title = model::title(clip);
    let spans: Vec<Span> = found(state, &title)
        .into_iter()
        .map(|(start, end)| {
            Span::new(start, end)
                .color(p.brand_ink)
                .weight(MATCH_WEIGHT)
        })
        .collect();
    let mut inner = row()
        .w_full()
        .h(ROW_HEIGHT - 2.0)
        .px(10.0)
        .gap(10.0)
        .radius(theme::RADIUS_CONTROL)
        .center_y()
        .fill(if on { p.selected } else { Color::TRANSPARENT })
        .child(mark(model::shape(clip), on, p))
        .child(
            text(title.clone())
                .spans(spans)
                .font(theme::BODY)
                .color(p.ink)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    if !on {
        inner = inner.hover(p.hover);
    }
    if model::is_pinned(clip) {
        inner = inner.child(icon(&theme::ICON_PIN, 12.0).color(p.ink_muted));
    }
    inner = inner.child(
        text(model::age(state.now_ms, clip.touched_ms()))
            .font(theme::LABEL)
            .color(p.ink_muted),
    );
    row()
        .key(format!("clip-{id}"))
        .role("option")
        .label(if model::is_pinned(clip) {
            format!("{title}, pinned")
        } else {
            title
        })
        .checked(on)
        .w_full()
        .h(ROW_HEIGHT)
        .center_y()
        .cursor(Cursor::Pointer)
        .preserve_focus_on_press(true)
        .on_press_tx(move |state: &mut State, tx| crate::press_clip(state, id, tx))
        .on_double_press(crate::press_paste)
        .child(inner)
}

fn preview(state: &State, p: &Palette) -> El {
    let Some(clip) = state.current() else {
        return col().grow(1.0);
    };
    let text_now = state.full_text();
    let mut cut = false;
    let content = match clip.kind {
        Kind::Image => {
            let mut stage = col()
                .key(format!("preview-image-{}", clip.id))
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .padding(16.0)
                .center();
            if let Some(pixels) = state.images.get(&clip.id) {
                let (width, height) = (
                    clip.image_width.unwrap_or(0) as f32,
                    clip.image_height.unwrap_or(0) as f32,
                );
                let mut picture = el()
                    .key("preview-image")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .radius(theme::RADIUS_TAG)
                    .pixels_fit(pixels, SurfaceFit::Fit);
                if width > 0.0 && height > 0.0 {
                    picture = picture
                        .max_w(width * IMAGE_UPSCALE)
                        .max_h(height * IMAGE_UPSCALE);
                }
                stage = stage.child(picture);
            } else if let Some(why) = state.gone(clip.id) {
                stage = stage.child(plate_note(
                    match why {
                        Gone::Missing => "This image is no longer stored",
                        Gone::Unreadable => "This image cannot be shown",
                    },
                    p,
                ));
            } else if !model::previewable(clip) {
                stage = stage.child(plate_note("Too large to preview", p));
            }
            stage
        }
        Kind::Text => {
            let (shown, more_chars) = model::head(text_now.unwrap_or(&clip.content), PREVIEW_CHARS);
            let (lines, more_lines) = model::indented_lines(shown, PREVIEW_LINES);
            cut = more_chars || more_lines || (text_now.is_none() && !model::is_complete(clip));
            let mut block = col().w_full().padding(16.0);
            if let Shape::Color(rgb) = model::shape(clip) {
                block = block.child(
                    el().key("preview-swatch")
                        .size(SWATCH, SWATCH)
                        .mb(14.0)
                        .radius(theme::RADIUS_CONTROL)
                        .fill(swatch(rgb))
                        .border(1.0, p.border),
                );
            }
            if lines.iter().all(|(_, line)| line.is_empty()) {
                block = block.child(
                    text("Only spaces and line breaks")
                        .key("preview-blank")
                        .font(theme::LABEL)
                        .color(p.ink_muted),
                );
            }
            for (indent, line) in lines {
                block = block.child(if line.is_empty() {
                    el().w_full().h(CODE_LINE)
                } else {
                    row().w_full().pl(indent as f32 * CODE_COLUMN).child(
                        text(line)
                            .font(theme::CODE)
                            .color(p.ink)
                            .grow(1.0)
                            .min_w(0.0)
                            .wrap_anywhere(),
                    )
                });
            }
            col()
                .key(format!("preview-text-{}", clip.id))
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(SCROLL)
                .scroll_edge_fade(10.0)
                .child(block)
        }
    };
    let facts = model::facts(clip, text_now);
    let facts = if cut {
        format!("Start only \u{b7} {facts}")
    } else {
        facts
    };
    let caption = row()
        .w_full()
        .h(36.0)
        .pl(16.0)
        .pr(8.0)
        .gap(12.0)
        .center_y()
        .child(
            text(facts)
                .key("facts")
                .font(theme::LABEL)
                .color(p.ink_muted)
                .shrink()
                .truncate()
                .min_w(FACTS_WIDTH),
        )
        .child(spacer())
        .child(
            text(model::origin(clip, state.now_ms))
                .key("origin")
                .font(theme::LABEL)
                .color(p.ink_muted)
                .max_w(ORIGIN_WIDTH)
                .shrink()
                .truncate(),
        )
        .child(actions(state, clip, p));
    let mut plate = col()
        .key("preview")
        .role("region")
        .label("Preview")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .radius(theme::RADIUS_PLATE)
        .clip_rounded(theme::RADIUS_PLATE)
        .fill(p.card)
        .border(1.0, p.border)
        .child(content)
        .child(caption);
    for shadow in p.raised() {
        plate = plate.box_shadow(shadow);
    }
    col()
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .py(8.0)
        .pr(8.0)
        .child(plate)
}

/// What the pointer can do with the selected clip; each has a key too.
fn actions(state: &State, clip: &Entry, p: &Palette) -> El {
    let pinned = model::is_pinned(clip);
    let mut actions = row().key("actions").gap(2.0).center_y();
    if state.link().is_some() {
        actions = actions.child(
            icon_button(
                "open",
                &theme::ICON_OPEN,
                "Open the link (Ctrl+O)",
                p.ink_muted,
                p,
            )
            .on_press_tx(crate::open_link),
        );
    }
    actions
        .child(
            icon_button(
                "pin",
                &theme::ICON_PIN,
                if pinned {
                    "Unpin (Ctrl+P)"
                } else {
                    "Pin (Ctrl+P)"
                },
                if pinned { p.brand_ink } else { p.ink_muted },
                p,
            )
            .on_press_tx(crate::press_pin),
        )
        .child(
            icon_button(
                "delete",
                &theme::ICON_TRASH,
                "Delete (Ctrl+D)",
                p.ink_muted,
                p,
            )
            .on_press_tx(crate::press_delete),
        )
}

fn icon_button(
    key: &'static str,
    glyph: &'static Icon,
    label: &str,
    ink: Color,
    p: &Palette,
) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .help(label)
        .size(26.0, 26.0)
        .radius(theme::RADIUS_TAG)
        .center()
        .cursor(Cursor::Pointer)
        .hover(p.hover)
        .preserve_focus_on_press(true)
        .child(icon(glyph, 14.0).color(ink))
}

fn plate_note(content: &str, p: &Palette) -> El {
    text(content)
        .key("preview-note")
        .font(theme::LABEL)
        .color(p.ink_muted)
}

fn footer(state: &State, p: &Palette) -> El {
    let capture_error = state
        .watcher
        .as_ref()
        .and_then(|watcher| watcher.last_error.as_deref());
    let left = match (&state.note, capture_error) {
        (Some(note), _) => message(note, p.danger),
        (None, _) if !state.undo.is_empty() => undo(state, p),
        (None, Some(error)) => message(&format!("Last capture failed: {error}"), p.warning),
        (None, None) => hints(state, p),
    };
    let mut bar = row()
        .key("footer")
        .w_full()
        .h(40.0)
        .px(14.0)
        .gap(14.0)
        .center_y()
        .child(left.grow(1.0).min_w(0.0).h_full().clip_rounded(0.0));
    if state.link == Link::Ready {
        bar = bar.child(
            text(model::count_label(state.clips.len(), LIST_LIMIT as usize))
                .key("count")
                .font(theme::LABEL)
                .color(p.ink_muted),
        );
    }
    bar.child(status(state, p))
}

fn message(content: &str, ink: Color) -> El {
    row().key("note").role("alert").gap(8.0).center_y().child(
        text(content)
            .font(theme::META)
            .color(ink)
            .truncate()
            .min_w(0.0),
    )
}

/// What was just deleted, and the way back while there is one.
fn undo(state: &State, p: &Palette) -> El {
    let said = match state.undo.as_slice() {
        [(clip, _)] => format!(
            "Deleted \u{201c}{}\u{201d}",
            model::head(&model::title(clip), 28).0
        ),
        deleted => format!("Deleted {} clips", deleted.len()),
    };
    row()
        .key("undo")
        .role("status")
        .gap(14.0)
        .center_y()
        .child(
            text(said)
                .font(theme::META)
                .color(p.ink)
                .truncate()
                .min_w(0.0),
        )
        .child(
            hint(&["Ctrl", "Z"], "Undo", p)
                .key("undo-button")
                .role("button")
                .label("Undo delete")
                .cursor(Cursor::Pointer)
                .preserve_focus_on_press(true)
                .on_press_tx(crate::press_undo),
        )
        .child(spacer())
}

fn hints(state: &State, p: &Palette) -> El {
    let mut hints = row().key("hints").gap(16.0).center_y();
    if state.keys_open {
        return hints
            .child(hint(&["Any key"], "Back to the clips", p))
            .child(spacer());
    }
    match state.current() {
        Some(clip) => {
            // Without a window to go back to there is nothing to paste into.
            let enter = if state.target.is_some() {
                "Paste"
            } else {
                "Copy"
            };
            let delete: &[&str] = if state.query.text.is_empty() {
                &["Del"]
            } else {
                &["Ctrl", "D"]
            };
            hints = hints
                .child(hint(&["Enter"], enter, p))
                .child(hint(
                    &["Ctrl", "P"],
                    if model::is_pinned(clip) {
                        "Unpin"
                    } else {
                        "Pin"
                    },
                    p,
                ))
                .child(hint(delete, "Delete", p));
        }
        None if state.query.text.is_empty() => hints = hints.child(hint(&["Esc"], "Close", p)),
        None => hints = hints.child(hint(&["Esc"], "Clear the search", p)),
    }
    hints
        .child(
            hint(&["F1"], "Keys", p)
                .key("keys-button")
                .role("button")
                .label("Show the keys")
                .cursor(Cursor::Pointer)
                .preserve_focus_on_press(true)
                .on_press_tx(crate::press_keys),
        )
        .child(spacer())
}

const MOVE_KEYS: [(&[&str], &str); 7] = [
    (&["Up", "Down"], "Move through the clips"),
    (&["PgUp", "PgDn"], "Move eight at a time"),
    (&["Ctrl", "Home"], "First clip"),
    (&["Ctrl", "End"], "Last clip"),
    (&["Tab"], "Next filter; Shift goes back"),
    (&["Ctrl", "1\u{2013}4"], "All, Text, Images, Pinned"),
    (&["Esc"], "Clear the search, then close"),
];

const CLIP_KEYS: [(&[&str], &str); 9] = [
    (&["Enter"], "Paste into the window you came from"),
    (&["Shift", "Enter"], "Paste as plain text"),
    (&["Ctrl", "Enter"], "Copy without pasting"),
    (&["Ctrl", "O"], "Open a link"),
    (&["Ctrl", "P"], "Pin or unpin"),
    (&["Del"], "Delete; Ctrl D while searching"),
    (&["Ctrl", "Z"], "Undo a delete"),
    (&["Ctrl", "Shift", "P"], "Pause or resume capture"),
    (&["F1"], "Show or hide these keys"),
];

/// Every key the window answers to, in place of the list until the next key.
fn keys_sheet(p: &Palette) -> El {
    let column = |name: &str, keys: &[(&[&str], &str)]| {
        let mut column = col()
            .grow(1.0)
            .min_w(0.0)
            .gap(8.0)
            .child(text(name).font(theme::LABEL).color(p.ink_muted).mb(2.0));
        for (caps, action) in keys {
            column = column.child(
                row()
                    .gap(10.0)
                    .center_y()
                    .child(row().w(118.0).child(hint(caps, "", p)))
                    .child(
                        text(*action)
                            .font(theme::META)
                            .color(p.ink)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    ),
            );
        }
        column
    };
    col()
        .key("body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(SCROLL)
        .child(
            col()
                .key("keys")
                .role("region")
                .label("Keys")
                .w_full()
                .padding(24.0)
                .gap(16.0)
                .child(text("Keys").font(theme::HEADING).color(p.ink))
                .child(
                    row()
                        .w_full()
                        .gap(32.0)
                        .items_start()
                        .child(column("Move", &MOVE_KEYS))
                        .child(column("Clip", &CLIP_KEYS)),
                ),
        )
}

fn hint(keys: &[&str], action: &str, p: &Palette) -> El {
    let mut hint = row().gap(4.0).center_y();
    for key in keys {
        let mut cap = row()
            .h(20.0)
            .min_w(20.0)
            .px(5.0)
            .radius(theme::RADIUS_KEY)
            .center()
            .fill(p.well)
            .border(1.0, p.border)
            .child(text(*key).font(theme::KEY).color(p.ink_muted));
        for shadow in p.raised() {
            cap = cap.box_shadow(shadow);
        }
        hint = hint.child(cap);
    }
    if action.is_empty() {
        return hint;
    }
    hint.child(text(action).font(theme::META).color(p.ink_muted).ml(2.0))
}

fn status(state: &State, p: &Palette) -> El {
    let (tone, reading) = model::watcher_reading(&state.link, state.watcher.as_ref());
    let ink = match tone {
        Tone::Good => p.success,
        Tone::Warn => p.warning,
        Tone::Bad => p.danger,
    };
    let action = match state.watcher.as_ref() {
        Some(watcher) if watcher.paused => "Resume capture",
        Some(_) => "Pause capture",
        None => reading,
    };
    row()
        .key("status")
        .role("button")
        .label(action)
        .h(24.0)
        .px(8.0)
        .gap(6.0)
        .radius(theme::RADIUS_TAG)
        .center_y()
        .cursor(Cursor::Pointer)
        .hover(p.hover)
        .preserve_focus_on_press(true)
        .on_press(crate::press_pause)
        .child(el().size(6.0, 6.0).radius(3.0).fill(ink))
        .child(text(reading).font(theme::LABEL).color(p.ink_muted))
}
