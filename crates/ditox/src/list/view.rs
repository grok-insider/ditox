use ditox_rpc::{Entry, Filter, Kind};
use rune::el::{Align, Side};
use rune::{
    Color, Cursor, Cx, El, GrowthSpec, ScrollSpec, Span, SurfaceFit, col, el, icon, row, spacer,
    text, text_input,
};

use super::{Menu, QUICK, State, THUMB_WIDTH};
use crate::feed::Gone;
use crate::model::{self, Link, PREVIEW_CHARS, PREVIEW_LINES};
use crate::theme::{self, Palette};

/// How far around the window a press still reaches it, in points.
const AWAY: f32 = 20_000.0;
const GUTTER: f32 = 28.0;
const ROW_PAD: f32 = 7.0;
const BAR_HEIGHT: f32 = 44.0;
const STRIP_HEIGHT: f32 = 30.0;
const MENU_WIDTH: f32 = 264.0;
const MENU_ROW: f32 = 28.0;
const IMAGE_UPSCALE: f32 = 2.0;
const CODE_LINE: f32 = 20.0;
const CODE_COLUMN: f32 = 7.8;
const MATCH_WEIGHT: f32 = 600.0;
const SCROLL: ScrollSpec = ScrollSpec {
    stick_bottom: false,
};

/// How tall a row is when its clip takes `lines` lines.
pub fn row_height(lines: u32) -> f32 {
    lines.max(1) as f32 * theme::ROW_LINE + 2.0 * ROW_PAD
}

/// How tall a row's small picture is, in points: two lines of text, or three
/// when the list is set to show that many.
pub fn thumb_height(lines: u32) -> f32 {
    lines.clamp(2, 3) as f32 * theme::ROW_LINE
}

/// The size a clip's small picture is drawn at, in whole points, when the
/// clip says how large it is.
pub fn thumb_size(clip: &Entry, lines: u32) -> Option<(f32, f32)> {
    let (width, height) = (clip.image_width? as f32, clip.image_height? as f32);
    if !model::previewable(clip) || width < 1.0 || height < 1.0 {
        return None;
    }
    let scale = (thumb_height(lines) / height)
        .min(THUMB_WIDTH / width)
        .min(1.0);
    Some((
        (width * scale).round().max(1.0),
        (height * scale).round().max(1.0),
    ))
}

pub fn view(state: &State, cx: &mut Cx<State>) -> El {
    let p = theme::palette(state.dark.unwrap_or_else(|| cx.system_dark()));
    crate::picture::set_scale(cx.env().scale);
    let mut root = col()
        .w_full()
        .h_full()
        .fill(p.page)
        // A press on plain content would take the keyboard from the search
        // field. The window has one field; it keeps it.
        .preserve_focus_on_press(true)
        .on_press(|_: &mut State| {})
        .role("main")
        // A press that lands outside the window is a press on something
        // else: Hyprland hands this window every press while it keeps the
        // keyboard. It puts the window away, as a click elsewhere does.
        .child(elsewhere("left", Some(-AWAY), Some(-AWAY), None, Some(-AWAY)).w(AWAY))
        .child(elsewhere("right", None, Some(-AWAY), Some(-AWAY), Some(-AWAY)).w(AWAY))
        .child(elsewhere("above", Some(0.0), Some(-AWAY), Some(0.0), None).h(AWAY))
        .child(elsewhere("below", Some(0.0), None, Some(0.0), Some(-AWAY)).h(AWAY))
        .child(behind_the_menu(state, body(state, &p)));
    if let Some(strip) = strip(state, &p) {
        root = root.child(behind_the_menu(state, strip));
    }
    root = root.child(rule(&p)).child(search_bar(state, &p));
    if let Some(open) = state.menu {
        root = root
            .child(
                el().key("menu-away")
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .preserve_focus_on_press(true)
                    .on_press_tx(super::close_menu)
                    .on_secondary_press(|state: &mut State, _, tx| super::close_menu(state, tx)),
            )
            .child(menu(state, open, &p));
    }
    root
}

/// While the menu is open, what lies under it answers nothing: no row
/// lights up as the pointer passes over the menu, as if it went through it.
/// A hover tint goes to every element the pointer is inside, drawn over or
/// not, so being covered is not enough.
fn behind_the_menu(state: &State, content: El) -> El {
    if state.menu.is_some() {
        content.inert()
    } else {
        content
    }
}

fn rule(p: &Palette) -> El {
    el().w_full().h(1.0).fill(p.separator)
}

/// One of the four stretches around the window where a press is a press
/// on something else.
fn elsewhere(
    side: &'static str,
    left: Option<f32>,
    top: Option<f32>,
    right: Option<f32>,
    bottom: Option<f32>,
) -> El {
    el().key(format!("elsewhere-{side}"))
        .pin(left, top, right, bottom)
        .on_press_tx(super::press_elsewhere)
}

fn body(state: &State, p: &Palette) -> El {
    // A picture the screen shows large leaves the list where it is.
    if state.peek
        && state.on_screen().is_none()
        && let Some(clip) = state.current()
    {
        return peek(state, clip, p);
    }
    let query = state.query.text.trim();
    let content = match &state.link {
        Link::Ready if !state.clips.is_empty() => return list(state, p),
        Link::Ready if query.is_empty() && state.filter == Filter::All => notice(
            "empty",
            "Nothing copied yet",
            "Copy text or an image and it shows up here.",
            None,
            None,
            p,
        ),
        Link::Ready if query.is_empty() => notice(
            "no-results",
            match state.filter {
                Filter::Text => "No text clips",
                Filter::Images => "No images",
                _ => "No pinned clips",
            },
            "",
            None,
            None,
            p,
        ),
        Link::Ready => notice(
            "no-results",
            &format!("Nothing matches \u{201c}{query}\u{201d}"),
            "",
            None,
            None,
            p,
        ),
        Link::Connecting => el(),
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
        .center()
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
        .max_w(340.0)
        .px(24.0)
        .gap(6.0)
        .items_center()
        .child(
            text(title)
                .font(theme::STRONG)
                .color(p.ink)
                .align(Align::Center)
                .wrap(),
        );
    if !body.is_empty() {
        panel = panel.child(
            text(body)
                .font(theme::META)
                .color(p.ink_muted)
                .align(Align::Center)
                .wrap(),
        );
    }
    if let Some(command) = command {
        panel = panel.child(
            row()
                .mt(6.0)
                .h(28.0)
                .px(10.0)
                .radius(theme::RADIUS_TAG)
                .center_y()
                .fill(p.well)
                .child(text(command).font(theme::CODE).color(p.ink)),
        );
    }
    if let Some(reading) = reading.filter(|reading| !reading.is_empty()) {
        panel = panel.child(
            text(reading)
                .font(theme::LABEL)
                .color(p.ink_muted)
                .align(Align::Center)
                .wrap_anywhere(),
        );
    }
    panel
}

fn list(state: &State, p: &Palette) -> El {
    let p = *p;
    let rows = col()
        .key("rows")
        .w_full()
        .growth(GrowthSpec::default())
        .instant_changes()
        .virtual_rows(
            state.clips.len(),
            row_height(1),
            |state: &State, index| {
                format!("clip-{}", state.clips.get(index).map_or(0, |clip| clip.id)).into()
            },
            move |state: &State, index| match state.clips.get(index) {
                Some(clip) => clip_row(state, clip, index, &p),
                None => el(),
            },
        )
        .on_visible_rows(super::on_visible_rows);
    col()
        .key("list")
        .role("listbox")
        .label("Clipboard history")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(SCROLL)
        .child(rows)
}

/// The stretches of a row's text to mark: what the search is looking for.
pub fn found(state: &State, shown: &str) -> Vec<(usize, usize)> {
    model::matches(shown, state.query.text.trim())
}

fn clip_row(state: &State, clip: &Entry, index: usize, p: &Palette) -> El {
    let id = clip.id;
    let on = index == state.selected;
    let pinned = model::is_pinned(clip);
    let shown = model::row_text(clip, state.ui.lines as usize);
    let number = if index < QUICK {
        (index + 1).to_string()
    } else {
        String::new()
    };
    let content = match thumb_size(clip, state.ui.lines) {
        Some((width, height)) => {
            let picture = el()
                .key(format!("thumb-{id}"))
                .size(width, height)
                .radius(theme::RADIUS_THUMB)
                .clip_rounded(theme::RADIUS_THUMB);
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(10.0)
                .center_y()
                .child(match state.thumbs.get(id) {
                    Some(pixels) => picture.pixels_fit(pixels, SurfaceFit::Fit),
                    None => picture.fill(p.well),
                })
                .child(
                    text(shown.strip_prefix("Image, ").unwrap_or(&shown).to_string())
                        .font(theme::LABEL)
                        .color(p.ink_muted)
                        .truncate()
                        .min_w(0.0),
                )
        }
        None => {
            let spans: Vec<Span> = found(state, &shown)
                .into_iter()
                .map(|(start, end)| {
                    Span::new(start, end)
                        .color(p.brand_ink)
                        .weight(MATCH_WEIGHT)
                })
                .collect();
            text(shown.clone())
                .spans(spans)
                .font(theme::BODY)
                .color(p.ink)
                .grow(1.0)
                .min_w(0.0)
                .wrap_anywhere()
                .max_lines(state.ui.lines.max(1) as usize)
        }
    };
    let mut line = row()
        .key(format!("clip-{id}"))
        .role("option")
        .label(if pinned {
            format!("{shown}, pinned")
        } else {
            shown
        })
        .checked(on)
        .w_full()
        .min_h(row_height(1))
        .pr(12.0)
        .items_start()
        .fill(if on {
            p.selected
        } else if index % 2 == 1 {
            p.stripe
        } else {
            Color::TRANSPARENT
        })
        .cursor(Cursor::Pointer)
        .preserve_focus_on_press(true)
        .on_press_tx(move |state: &mut State, tx| super::press_clip(state, id, tx))
        .on_double_press(super::press_paste)
        .on_secondary_press(move |state: &mut State, _, tx| super::press_clip_menu(state, id, tx))
        .child(
            el().key(format!("pin-{id}"))
                .w(2.0)
                .self_stretch()
                .fill(if pinned { p.brand } else { Color::TRANSPARENT }),
        )
        .child(
            row().w(GUTTER).pt(ROW_PAD + 2.0).justify_center().child(
                text(number)
                    .font(theme::LABEL)
                    .color(if on { p.brand_ink } else { p.ink_muted }),
            ),
        )
        .child(row().grow(1.0).min_w(0.0).py(ROW_PAD).child(content));
    if !on {
        line = line.hover(p.hover);
    }
    line
}

/// The selected clip whole, where the list was.
fn peek(state: &State, clip: &Entry, p: &Palette) -> El {
    let text_now = state.full_text();
    let mut cut = false;
    let content = match clip.kind {
        Kind::Image => {
            let mut stage = col()
                .key(format!("peek-image-{}", clip.id))
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .padding(12.0)
                .center();
            if let Some(pixels) = state.images.get(clip.id) {
                let (width, height) = (
                    clip.image_width.unwrap_or(0) as f32,
                    clip.image_height.unwrap_or(0) as f32,
                );
                let mut picture = el()
                    .key("peek-image")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .pixels_fit(pixels, SurfaceFit::Fit);
                if width > 0.0 && height > 0.0 {
                    picture = picture
                        .max_w(width * IMAGE_UPSCALE)
                        .max_h(height * IMAGE_UPSCALE);
                }
                stage = stage.child(picture);
            } else if let Some(why) = state.gone(clip.id) {
                stage = stage.child(peek_note(
                    match why {
                        Gone::Missing => "This image is no longer stored",
                        Gone::Unreadable => "This image cannot be shown",
                    },
                    p,
                ));
            } else if !model::previewable(clip) {
                stage = stage.child(peek_note("Too large to show", p));
            }
            stage
        }
        Kind::Text => {
            let (shown, more_chars) = model::head(text_now.unwrap_or(&clip.content), PREVIEW_CHARS);
            let (lines, more_lines) = model::indented_lines(shown, PREVIEW_LINES);
            cut = more_chars || more_lines || (text_now.is_none() && !model::is_complete(clip));
            let mut block = col().w_full().px(14.0).py(12.0);
            if lines.iter().all(|(_, line)| line.is_empty()) {
                block = block.child(peek_note("Only spaces and line breaks", p));
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
                .key(format!("peek-text-{}", clip.id))
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(SCROLL)
                .child(block)
        }
    };
    let caption = model::caption(clip, text_now, state.now_ms);
    col()
        .key("peek")
        .role("region")
        .label("Clip")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(content)
        .child(
            row()
                .w_full()
                .h(STRIP_HEIGHT)
                .px(14.0)
                .gap(12.0)
                .center_y()
                .child(
                    text(if cut {
                        format!("Start only \u{b7} {caption}")
                    } else {
                        caption
                    })
                    .key("caption")
                    .font(theme::LABEL)
                    .color(p.ink_muted)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
                )
                .child(
                    link_button("back", "Back", "Back to the list (Tab)", p)
                        .on_press_tx(super::press_back),
                ),
        )
}

fn peek_note(content: &str, p: &Palette) -> El {
    text(content)
        .key("peek-note")
        .font(theme::LABEL)
        .color(p.ink_muted)
}

/// A line above the search field, there only while it has something to say.
fn strip(state: &State, p: &Palette) -> Option<El> {
    let capture_error = state
        .watcher
        .as_ref()
        .and_then(|watcher| watcher.last_error.as_deref());
    let said = |key: &'static str, role: &'static str, content: String, ink: Color| {
        row()
            .key(key)
            .role(role)
            .w_full()
            .h(STRIP_HEIGHT)
            .px(14.0)
            .gap(12.0)
            .center_y()
            .child(
                text(content)
                    .font(theme::META)
                    .color(ink)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
            )
    };
    let strip = if let Some(note) = &state.note {
        said("note", "alert", note.clone(), p.danger)
    } else if let Some(deleted) = state.undo.last() {
        let content = match state.undo.len() {
            1 => format!(
                "Deleted \u{201c}{}\u{201d}",
                model::head(&model::row_text(&deleted.0, 1), 32).0
            ),
            count => format!("Deleted {count} clips"),
        };
        said("undo", "status", content, p.ink).child(
            link_button("undo-button", "Undo", "Undo the delete (Ctrl+Z)", p)
                .on_press_tx(super::press_undo),
        )
    } else if state.paused() {
        said("paused", "status", "Capture is paused".into(), p.warning).child(
            link_button("resume", "Resume", "Resume capture (Ctrl+Shift+P)", p)
                .on_press_tx(super::press_resume),
        )
    } else {
        said(
            "capture-error",
            "status",
            format!("Last capture failed: {}", capture_error?),
            p.warning,
        )
    };
    Some(col().w_full().child(rule(p)).child(strip))
}

fn link_button(key: &'static str, label: &'static str, help: &'static str, p: &Palette) -> El {
    row()
        .key(key)
        .role("button")
        .label(help)
        .help(help)
        .h(22.0)
        .px(8.0)
        .radius(theme::RADIUS_TAG)
        .center()
        .cursor(Cursor::Pointer)
        .hover(p.hover)
        .preserve_focus_on_press(true)
        .child(text(label).font(theme::CONTROL).color(p.brand_ink))
}

fn search_bar(state: &State, p: &Palette) -> El {
    let placeholder = match state.filter {
        Filter::Text => "Search text",
        Filter::Images => "Search images",
        Filter::Favorites => "Search pinned",
        Filter::All | Filter::Today => "Search",
    };
    let field = text_input(&state.query, theme::BODY)
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
        .placeholder(placeholder, p.ink_muted)
        .caret(p.brand_ink, p.brand_ink.alpha(0.3))
        .no_focus_ring()
        .on_edit(|state: &mut State| &mut state.query)
        .on_input_change(|state: &mut State, _| super::on_query_change(state))
        .on_input_key(super::on_key);
    let mut bar = row()
        .key("bar")
        .w_full()
        .h(BAR_HEIGHT)
        .pl(14.0)
        .pr(8.0)
        .gap(10.0)
        .center_y()
        .child(icon(&theme::ICON_SEARCH, 15.0).color(p.ink_muted))
        .child(field);
    if state.filter != Filter::All {
        bar = bar.child(
            row()
                .key("filter")
                .role("button")
                .label("Show all clips")
                .help("Show all clips")
                .h(22.0)
                .pl(8.0)
                .pr(6.0)
                .gap(4.0)
                .radius(theme::RADIUS_TAG)
                .center_y()
                .fill(p.well)
                .cursor(Cursor::Pointer)
                .preserve_focus_on_press(true)
                .on_press_tx(super::press_show_all)
                .child(
                    text(model::filter_name(state.filter))
                        .font(theme::META)
                        .color(p.ink),
                )
                .child(icon(&theme::ICON_CLOSE, 11.0).color(p.ink_muted)),
        );
    }
    bar.child(
        row()
            .key("menu-button")
            .role("button")
            .label("Menu")
            .help("Menu (Ctrl+M)")
            .size(28.0, 28.0)
            .radius(theme::RADIUS_TAG)
            .center()
            .fill(if state.menu.is_some() {
                p.hover
            } else {
                Color::TRANSPARENT
            })
            .cursor(Cursor::Pointer)
            .hover(p.hover)
            .preserve_focus_on_press(true)
            .on_press_tx(super::press_menu_button)
            .child(icon(&theme::ICON_MORE, 16.0).color(p.ink_muted)),
    )
}

/// Everything the window can do beyond choosing a clip, with its key. It
/// opens over its button, and scrolls in a window too short for it.
fn menu(state: &State, open: Menu, p: &Palette) -> El {
    let mut items = col().key("menu-items").w_full().py(5.0);
    for (index, item) in state.menu_items().into_iter().enumerate() {
        if item.leads {
            items = items.child(el().w_full().h(1.0).my(5.0).fill(p.separator));
        }
        let on = index == open.at;
        let mark = match item.chosen {
            Some(true) => icon(&theme::ICON_CHECK, 13.0).color(p.brand_ink),
            _ => el().size(13.0, 13.0),
        };
        items = items.child(
            row()
                .key(format!("menu-{index}"))
                .role("menuitem")
                .label(item.label)
                .checked(item.chosen.unwrap_or(false))
                .h(MENU_ROW)
                .mx(5.0)
                .px(8.0)
                .gap(8.0)
                .radius(theme::RADIUS_TAG)
                .center_y()
                .fill(if on { p.selected } else { Color::TRANSPARENT })
                // The pointer tints the entry it is over and no more, as on
                // the list's rows: the entry Enter runs moves with the keys.
                .hover(if on { Color::TRANSPARENT } else { p.hover })
                .cursor(Cursor::Pointer)
                .preserve_focus_on_press(true)
                .on_press_tx(move |state: &mut State, tx| super::press_menu_item(state, index, tx))
                .child(mark)
                .child(text(item.label).font(theme::BODY).color(p.ink))
                .child(spacer())
                .child(text(item.keys).font(theme::LABEL).color(p.ink_muted)),
        );
    }
    let mut sheet = col()
        .key("menu")
        .role("menu")
        .label("Actions")
        .w(MENU_WIDTH)
        .radius(theme::RADIUS_MENU)
        .clip_rounded(theme::RADIUS_MENU)
        .fill(p.card)
        .border(1.0, p.border)
        .preserve_focus_on_press(true)
        .on_press(|_: &mut State| {})
        .child(
            col()
                .key("menu-scroll")
                .w_full()
                .shrink()
                .min_h(0.0)
                .scroll(SCROLL)
                .child(items),
        );
    for shadow in p.floating() {
        sheet = sheet.box_shadow(shadow);
    }
    sheet.anchored_fit("menu-button", Side::Top, Align::End, 6.0)
}
