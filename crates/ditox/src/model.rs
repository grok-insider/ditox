use ditox_rpc::{Entry, Filter, Kind, WatcherStatus};

pub const FILTERS: [(Filter, &str); 4] = [
    (Filter::All, "All"),
    (Filter::Text, "Text"),
    (Filter::Images, "Images"),
    (Filter::Favorites, "Pinned"),
];

pub const PREVIEW_CHARS: usize = 6000;
pub const PREVIEW_LINES: usize = 400;
const TAB_COLUMNS: usize = 4;
const MAX_INDENT: usize = 24;
const APP_CHARS: usize = 24;
const MAX_PREVIEW_FILE: i64 = 48 * 1024 * 1024;
const MAX_PREVIEW_DECODED: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Offline(String),
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Bad,
}

pub fn title(clip: &Entry) -> String {
    let raw = match clip.kind {
        Kind::Image
            if clip.preview.starts_with("Image image/") || clip.preview.trim().is_empty() =>
        {
            return match (clip.image_width, clip.image_height) {
                (Some(width), Some(height)) => format!("Image, {width} \u{d7} {height}"),
                _ => "Image".into(),
            };
        }
        Kind::Image => clip.preview.as_str(),
        Kind::Text if clip.preview.trim().is_empty() => clip.content.as_str(),
        Kind::Text => clip.preview.as_str(),
    };
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        "Blank text".into()
    } else {
        line
    }
}

pub fn age(now_ms: i64, at_ms: i64) -> String {
    let seconds = (now_ms.saturating_sub(at_ms) / 1000).max(0);
    match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{}m", seconds / 60),
        3600..172_800 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

pub fn age_long(now_ms: i64, at_ms: i64) -> String {
    let seconds = (now_ms.saturating_sub(at_ms) / 1000).max(0);
    let plural =
        |count: i64, unit: &str| format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" });
    match seconds {
        0..60 => "just now".into(),
        60..3600 => plural(seconds / 60, "minute"),
        3600..172_800 => plural(seconds / 3600, "hour"),
        _ => plural(seconds / 86_400, "day"),
    }
}

pub fn size(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes < 1000.0 {
        format!("{bytes:.0} B")
    } else if bytes < 1_000_000.0 {
        format!("{:.1} kB", bytes / 1000.0)
    } else {
        format!("{:.1} MB", bytes / 1_000_000.0)
    }
}

/// What kind of thing a clip is, in a word, when that says more than
/// "plain text" would.
fn kind_word(clip: &Entry) -> Option<String> {
    let mime = clip.mime.split(';').next().unwrap_or(&clip.mime).trim();
    match (shape(clip), mime) {
        (Shape::Link, _) => Some("Link".into()),
        (Shape::Color(_), _) => Some("Color".into()),
        (_, "text/plain" | "") => None,
        (_, "text/html") => Some("HTML".into()),
        (_, "text/uri-list") => Some("File list".into()),
        (_, "image/jpeg") => Some("JPEG".into()),
        (_, "image/webp") => Some("WebP".into()),
        (Shape::Image, mime) => Some(
            mime.strip_prefix("image/")
                .unwrap_or(mime)
                .to_ascii_uppercase(),
        ),
        (_, mime) => Some(mime.into()),
    }
}

/// The caption under the preview: what the clip is, how large, how long.
pub fn facts(clip: &Entry, text: Option<&str>) -> String {
    let mut facts: Vec<String> = kind_word(clip).into_iter().collect();
    match clip.kind {
        Kind::Image => {
            if let (Some(width), Some(height)) = (clip.image_width, clip.image_height) {
                facts.push(format!("{width} \u{d7} {height}"));
            }
            facts.push(size(clip.byte_len));
        }
        Kind::Text => {
            facts.push(size(clip.byte_len));
            match text.map(|text| text.lines().count().max(1)) {
                Some(1) => facts.push("1 line".into()),
                Some(lines) => facts.push(format!("{lines} lines")),
                None => {}
            }
        }
    }
    facts.join(" \u{b7} ")
}

pub fn origin(clip: &Entry, now_ms: i64) -> String {
    let when = age_long(now_ms, clip.touched_ms());
    match clip.source_app.as_deref().filter(|app| !app.is_empty()) {
        Some(app) if app.chars().count() > APP_CHARS => {
            let (start, _) = head(app, APP_CHARS - 1);
            format!("{start}\u{2026} \u{b7} {when}")
        }
        Some(app) => format!("{app} \u{b7} {when}"),
        None => when,
    }
}

/// What a picture takes once decoded: four bytes a pixel, or a guess from
/// its file when the daemon could not read its size.
pub fn decoded_bytes(clip: &Entry) -> u64 {
    match (clip.image_width, clip.image_height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => (width as u64)
            .saturating_mul(height as u64)
            .saturating_mul(4),
        _ => (clip.byte_len.max(0) as u64).saturating_mul(4),
    }
}

/// Whether the window draws the picture, rather than only describing it.
pub fn previewable(clip: &Entry) -> bool {
    clip.byte_len <= MAX_PREVIEW_FILE && decoded_bytes(clip) <= MAX_PREVIEW_DECODED
}

/// What a clip is, as far as its row can tell at a glance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Text,
    Image,
    Link,
    Color([u8; 3]),
}

pub fn shape(clip: &Entry) -> Shape {
    if clip.kind == Kind::Image {
        return Shape::Image;
    }
    if let Some(color) = color(&clip.content) {
        return Shape::Color(color);
    }
    if is_complete(clip) && link(&clip.content).is_some() {
        return Shape::Link;
    }
    Shape::Text
}

/// The text as a web address, when it is exactly one.
pub fn link(text: &str) -> Option<&str> {
    let text = text.trim();
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))?;
    let plain = !rest.is_empty()
        && !rest.starts_with(['/', '.'])
        && text.len() <= 2048
        && !text.chars().any(|ch| ch.is_whitespace() || ch.is_control());
    plain.then_some(text)
}

/// The text as a colour: `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn color(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().strip_prefix('#')?;
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let value = |at: usize, len: usize| u8::from_str_radix(&digits[at..at + len], 16).ok();
    match digits.len() {
        3 => Some([value(0, 1)? * 17, value(1, 1)? * 17, value(2, 1)? * 17]),
        6 | 8 => Some([value(0, 2)?, value(2, 2)?, value(4, 2)?]),
        _ => None,
    }
}

/// Where the words of `query` appear in `title`, whatever their case, as
/// byte ranges in order and without overlap. Only ASCII letters fold, so a
/// find always starts and ends between characters.
pub fn matches(title: &str, query: &str) -> Vec<(usize, usize)> {
    let hay = title.as_bytes();
    let mut found: Vec<(usize, usize)> = Vec::new();
    for word in query.split_whitespace() {
        let needle = word.as_bytes();
        let mut at = 0;
        while at + needle.len() <= hay.len() {
            if hay[at..at + needle.len()].eq_ignore_ascii_case(needle) {
                found.push((at, at + needle.len()));
                at += needle.len();
            } else {
                at += 1;
            }
        }
    }
    found.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(found.len());
    for (start, end) in found {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

pub fn is_complete(clip: &Entry) -> bool {
    clip.kind == Kind::Text && clip.content.len() as i64 >= clip.byte_len
}

pub fn head(text: &str, max_chars: usize) -> (&str, bool) {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => (&text[..index], true),
        None => (text, false),
    }
}

/// Each line with the columns it is indented by, so a wrapped line keeps its indent.
pub fn indented_lines(text: &str, max_lines: usize) -> (Vec<(usize, &str)>, bool) {
    let mut lines = Vec::new();
    let mut all = text.lines();
    for line in all.by_ref().take(max_lines) {
        let rest = line.trim_start_matches([' ', '\t']);
        let columns: usize = line[..line.len() - rest.len()]
            .bytes()
            .map(|byte| if byte == b'\t' { TAB_COLUMNS } else { 1 })
            .sum();
        lines.push((columns.min(MAX_INDENT), rest.trim_end()));
    }
    (lines, all.next().is_some())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListRow {
    Section(&'static str),
    Clip(usize),
}

pub fn is_pinned(clip: &Entry) -> bool {
    clip.favorite || clip.sticky
}

const DAY_MS: i64 = 86_400_000;

/// The heading a clip touched at `at_ms` goes under, given when today began.
fn day_label(at_ms: i64, day_start_ms: Option<i64>) -> &'static str {
    match day_start_ms {
        None => "Recent",
        Some(start) if at_ms >= start => "Today",
        Some(start) if at_ms >= start - DAY_MS => "Yesterday",
        Some(start) if at_ms >= start - 6 * DAY_MS => "Last 7 days",
        Some(_) => "Older",
    }
}

/// The list as it is drawn: the pinned clips, then the rest by day, each
/// group under its heading. One group alone, or search results, have none.
pub fn rows(clips: &[Entry], searching: bool, day_start_ms: Option<i64>) -> Vec<ListRow> {
    let plain = || (0..clips.len()).map(ListRow::Clip).collect();
    if searching {
        return plain();
    }
    let mut rows = Vec::with_capacity(clips.len() + 5);
    let mut headings = 0;
    let mut current = None;
    let mut pinned_lead = true;
    for (index, clip) in clips.iter().enumerate() {
        pinned_lead &= is_pinned(clip);
        let label = if pinned_lead {
            "Pinned"
        } else {
            day_label(clip.touched_ms(), day_start_ms)
        };
        if current != Some(label) {
            current = Some(label);
            headings += 1;
            rows.push(ListRow::Section(label));
        }
        rows.push(ListRow::Clip(index));
    }
    if headings < 2 { plain() } else { rows }
}

/// How many pinned clips lead the list, when it shows them apart from the rest.
fn pinned_head(clips: &[Entry], searching: bool) -> Option<usize> {
    if searching {
        return None;
    }
    let head = clips.iter().take_while(|clip| is_pinned(clip)).count();
    (head > 0 && head < clips.len()).then_some(head)
}

/// Where the selection starts: the newest clip, below the pinned ones.
pub fn home(clips: &[Entry], searching: bool) -> usize {
    pinned_head(clips, searching).unwrap_or(0)
}

pub fn keep_selection(clips: &[Entry], previous_id: Option<i64>, searching: bool) -> usize {
    previous_id
        .and_then(|id| clips.iter().position(|clip| clip.id == id))
        .unwrap_or_else(|| home(clips, searching))
}

pub fn step(len: usize, selected: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (selected as isize + delta).clamp(0, len as isize - 1) as usize
}

pub fn cycle_filter(current: Filter, forward: bool) -> Filter {
    let index = FILTERS
        .iter()
        .position(|(filter, _)| *filter == current)
        .unwrap_or(0);
    let next = if forward {
        (index + 1) % FILTERS.len()
    } else {
        (index + FILTERS.len() - 1) % FILTERS.len()
    };
    FILTERS[next].0
}

pub fn count_label(count: usize, limit: usize) -> String {
    match count {
        1 => "1 clip".into(),
        count if count >= limit => format!("{limit}+ clips"),
        count => format!("{count} clips"),
    }
}

pub fn watcher_reading(link: &Link, watcher: Option<&WatcherStatus>) -> (Tone, &'static str) {
    match (link, watcher) {
        (Link::Offline(_), _) => (Tone::Bad, "Daemon offline"),
        (Link::Connecting, _) | (_, None) => (Tone::Warn, "Connecting"),
        (_, Some(status)) if status.paused => (Tone::Warn, "Paused"),
        (_, Some(status)) if !status.running => (Tone::Bad, "Not watching"),
        (_, Some(status)) if status.last_error.is_some() => (Tone::Warn, "Last capture failed"),
        _ => (Tone::Good, "Watching"),
    }
}

pub fn refusal(reason: &str) -> String {
    match reason {
        "missing" => "That clip is gone.".into(),
        "no-plain-text" => "This clip has no plain text to paste.".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn clip(id: i64, text: &str) -> Entry {
        Entry {
            id,
            kind: Kind::Text,
            mime: "text/plain;charset=utf-8".into(),
            content: text.into(),
            preview: text.into(),
            hash: String::new(),
            favorite: false,
            created_at_ms: 0,
            last_used_at_ms: None,
            byte_len: text.len() as i64,
            source_app: None,
            blob_path: None,
            image_width: None,
            image_height: None,
            sticky: false,
        }
    }

    fn image(preview: &str, size: Option<(i64, i64)>) -> Entry {
        Entry {
            kind: Kind::Image,
            mime: "image/png".into(),
            preview: preview.into(),
            byte_len: 82_000,
            image_width: size.map(|size| size.0),
            image_height: size.map(|size| size.1),
            ..clip(9, "")
        }
    }

    #[test]
    fn a_title_is_one_line_and_an_image_says_its_size_or_its_link() {
        assert_eq!(title(&clip(1, "  two\n  lines\there ")), "two lines here");
        assert_eq!(title(&clip(1, " \n ")), "Blank text");
        assert_eq!(
            title(&image(
                "Image image/png 82000 bytes 640x480",
                Some((640, 480))
            )),
            "Image, 640 \u{d7} 480"
        );
        assert_eq!(title(&image("", None)), "Image");
        assert_eq!(
            title(&image("https://example.com/a.png", Some((1, 1)))),
            "https://example.com/a.png"
        );
    }

    #[test]
    fn ages_use_the_largest_unit_that_fits() {
        assert_eq!(age(59_000, 0), "now");
        assert_eq!(age(90_000, 0), "1m");
        assert_eq!(age(3_600_000, 0), "1h");
        assert_eq!(age(3 * 86_400_000, 0), "3d");
        assert_eq!(age(0, 5_000), "now");
        assert_eq!(age_long(60_000, 0), "1 minute ago");
        assert_eq!(age_long(7_200_000, 0), "2 hours ago");
        assert_eq!(age_long(1_000, 0), "just now");
    }

    #[test]
    fn facts_name_the_kind_when_it_is_not_plain_text_the_size_and_the_shape() {
        assert_eq!(
            facts(&clip(1, "one\ntwo"), Some("one\ntwo")),
            "7 B \u{b7} 2 lines"
        );
        assert_eq!(facts(&clip(1, "one"), Some("one")), "3 B \u{b7} 1 line");
        assert_eq!(facts(&clip(1, "one"), None), "3 B");
        assert_eq!(
            facts(&clip(1, "https://a.io"), Some("https://a.io")),
            "Link \u{b7} 12 B \u{b7} 1 line"
        );
        assert_eq!(
            facts(&clip(1, "#fff"), Some("#fff")),
            "Color \u{b7} 4 B \u{b7} 1 line"
        );
        let mut html = clip(1, "<b>x</b>");
        html.mime = "text/html".into();
        assert_eq!(facts(&html, None), "HTML \u{b7} 8 B");
        html.mime = "text/x-python".into();
        assert_eq!(facts(&html, None), "text/x-python \u{b7} 8 B");
        assert_eq!(
            facts(&image("", Some((640, 480))), None),
            "PNG \u{b7} 640 \u{d7} 480 \u{b7} 82.0 kB"
        );
        let mut photo = image("", None);
        photo.mime = "image/jpeg".into();
        assert_eq!(facts(&photo, None), "JPEG \u{b7} 82.0 kB");
        assert_eq!(size(2_500_000), "2.5 MB");
    }

    #[test]
    fn the_origin_names_the_app_when_it_is_known() {
        let mut entry = clip(1, "x");
        entry.created_at_ms = 1_000;
        entry.last_used_at_ms = Some(61_000);
        assert_eq!(origin(&entry, 181_000), "2 minutes ago");
        entry.source_app = Some("firefox".into());
        assert_eq!(origin(&entry, 61_000), "firefox \u{b7} just now");
        entry.source_app = Some("org.example.AVeryLongApplicationClassName".into());
        assert_eq!(
            origin(&entry, 61_000),
            "org.example.AVeryLongAp\u{2026} \u{b7} just now"
        );
    }

    #[test]
    fn a_huge_picture_is_described_and_not_drawn() {
        assert_eq!(decoded_bytes(&image("", Some((640, 480)))), 640 * 480 * 4);
        assert_eq!(decoded_bytes(&image("", None)), 82_000 * 4);
        assert!(previewable(&image("", Some((3840, 2160)))));
        assert!(!previewable(&image("", Some((16_384, 16_384)))));
        let mut heavy = image("", Some((100, 100)));
        heavy.byte_len = 64 * 1024 * 1024;
        assert!(!previewable(&heavy));
    }

    #[test]
    fn a_cut_clip_is_not_complete() {
        let mut entry = clip(1, "short");
        assert!(is_complete(&entry));
        entry.byte_len = 9000;
        assert!(!is_complete(&entry));
        assert!(!is_complete(&image("", None)));
        assert_eq!(head("a\u{f1}b", 2), ("a\u{f1}", true));
        assert_eq!(head("ab", 2), ("ab", false));
    }

    #[test]
    fn lines_keep_their_indent_in_columns_and_stop_at_the_cap() {
        let (lines, more) = indented_lines("fn a() {\n    b();\n\tc();\n\n}", 10);
        assert_eq!(
            lines,
            [(0, "fn a() {"), (4, "b();"), (4, "c();"), (0, ""), (0, "}")]
        );
        assert!(!more);
        let (lines, more) = indented_lines("one\ntwo\nthree", 2);
        assert_eq!(lines, [(0, "one"), (0, "two")]);
        assert!(more);
        let deep = format!("{}x", " ".repeat(80));
        assert_eq!(indented_lines(&deep, 1).0, [(24, "x")]);
    }

    #[test]
    fn selection_follows_the_clip_and_stops_at_the_ends() {
        let clips = [clip(4, "a"), clip(9, "b"), clip(2, "c")];
        assert_eq!(keep_selection(&clips, Some(9), false), 1);
        assert_eq!(keep_selection(&clips, Some(7), false), 0);
        assert_eq!(keep_selection(&[], Some(9), false), 0);
        assert_eq!(step(3, 2, 1), 2);
        assert_eq!(step(3, 0, -1), 0);
        assert_eq!(step(3, 0, 10), 2);
        assert_eq!(step(0, 0, 1), 0);
    }

    #[test]
    fn the_selection_starts_on_the_newest_clip_below_the_pinned_ones() {
        let mut clips = [
            clip(1, "pinned"),
            clip(2, "sticky"),
            clip(3, "new"),
            clip(4, "old"),
        ];
        clips[0].favorite = true;
        clips[1].sticky = true;
        assert_eq!(home(&clips, false), 2);
        assert_eq!(keep_selection(&clips, None, false), 2);
        assert_eq!(keep_selection(&clips, Some(1), false), 0);
        assert_eq!(home(&clips, true), 0);
        assert_eq!(home(&clips[..2], false), 0);
        assert_eq!(home(&clips[2..], false), 0);
        assert_eq!(home(&[], false), 0);
    }

    #[test]
    fn the_list_rows_are_headed_groups_or_the_clips_alone() {
        use ListRow::{Clip, Section};
        let now = 10 * DAY_MS + 15 * 3_600_000;
        let start = Some(10 * DAY_MS);
        let at = |id: i64, hours_ago: i64| {
            let mut entry = clip(id, "x");
            entry.created_at_ms = now - hours_ago * 3_600_000;
            entry
        };
        let mut clips = vec![
            at(1, 300),
            at(2, 1),
            at(3, 14),
            at(4, 16),
            at(5, 30),
            at(6, 80),
            at(7, 400),
        ];
        clips[0].favorite = true;
        assert_eq!(
            rows(&clips, false, start),
            [
                Section("Pinned"),
                Clip(0),
                Section("Today"),
                Clip(1),
                Clip(2),
                Section("Yesterday"),
                Clip(3),
                Clip(4),
                Section("Last 7 days"),
                Clip(5),
                Section("Older"),
                Clip(6),
            ]
        );
        assert_eq!(
            rows(&clips, false, None)[..4],
            [Section("Pinned"), Clip(0), Section("Recent"), Clip(1)]
        );
        assert_eq!(rows(&clips, false, None).len(), 9);
        let plain: Vec<ListRow> = (0..7).map(Clip).collect();
        assert_eq!(
            rows(&clips, true, start),
            plain,
            "search results have no headings"
        );
        assert_eq!(
            rows(&clips[1..3], false, start),
            [Clip(0), Clip(1)],
            "one group needs no heading"
        );
        assert_eq!(rows(&clips[..1], false, start), [Clip(0)]);
        assert_eq!(rows(&[], false, start), []);

        clips[3].sticky = true;
        assert_eq!(
            rows(&clips[2..5], false, start),
            [
                Section("Today"),
                Clip(0),
                Section("Yesterday"),
                Clip(1),
                Clip(2)
            ],
            "a pinned clip below the first unpinned one stays with its day"
        );
    }

    #[test]
    fn a_row_knows_a_link_and_a_colour_from_plain_text() {
        assert_eq!(
            shape(&clip(1, "https://ui.fabrials.com/docs?q=1#top")),
            Shape::Link
        );
        assert_eq!(shape(&clip(1, " http://localhost:3000 \n")), Shape::Link);
        for text in [
            "https://",
            "https://a b",
            "see https://a.io",
            "ftp://a.io",
            "https:///x",
            "https://a.io\nhttps://b.io",
        ] {
            assert_eq!(shape(&clip(1, text)), Shape::Text, "{text}");
        }
        let mut cut = clip(1, "https://example.com/long");
        cut.byte_len = 9000;
        assert_eq!(
            shape(&cut),
            Shape::Text,
            "a clip seen only in part is not judged"
        );

        assert_eq!(shape(&clip(1, "#8fb8f5")), Shape::Color([0x8f, 0xb8, 0xf5]));
        assert_eq!(color("#FFF"), Some([255, 255, 255]));
        assert_eq!(color(" #0a0b0cff "), Some([10, 11, 12]));
        for text in [
            "#12",
            "#12345",
            "8fb8f5",
            "#ggg",
            "#",
            "#1234567",
            "#caf\u{e9}",
        ] {
            assert_eq!(color(text), None, "{text}");
        }
        assert_eq!(shape(&image("", None)), Shape::Image);
    }

    #[test]
    fn the_words_searched_for_are_found_in_a_title_whatever_their_case() {
        assert_eq!(
            matches("cargo test --workspace", "CARGO work"),
            [(0, 5), (13, 17)]
        );
        assert_eq!(
            matches("aaaa", "aa a"),
            [(0, 4)],
            "finds that touch are one stretch"
        );
        assert_eq!(
            matches("ma\u{f1}ana ma\u{f1}", "a\u{f1}"),
            [(1, 4), (9, 12)]
        );
        assert_eq!(matches("\u{4f60}\u{597d}", "\u{597d}"), [(3, 6)]);
        assert!(matches("short", "much longer than the title").is_empty());
        assert!(matches("anything", "  ").is_empty());
        assert!(matches("", "x").is_empty());
    }

    #[test]
    fn filters_cycle_both_ways() {
        assert_eq!(cycle_filter(Filter::All, true), Filter::Text);
        assert_eq!(cycle_filter(Filter::Favorites, true), Filter::All);
        assert_eq!(cycle_filter(Filter::All, false), Filter::Favorites);
    }

    #[test]
    fn the_reading_says_what_the_watcher_is_doing() {
        let status = |running, paused, error: Option<&str>| WatcherStatus {
            running,
            paused,
            backend: "wl-clipboard".into(),
            last_error: error.map(String::from),
        };
        assert_eq!(
            watcher_reading(&Link::Ready, Some(&status(true, false, None))),
            (Tone::Good, "Watching")
        );
        assert_eq!(
            watcher_reading(&Link::Ready, Some(&status(true, true, None))),
            (Tone::Warn, "Paused")
        );
        assert_eq!(
            watcher_reading(&Link::Ready, Some(&status(false, false, None))),
            (Tone::Bad, "Not watching")
        );
        assert_eq!(
            watcher_reading(&Link::Ready, Some(&status(true, false, Some("x")))),
            (Tone::Warn, "Last capture failed")
        );
        assert_eq!(
            watcher_reading(&Link::Offline("x".into()), None),
            (Tone::Bad, "Daemon offline")
        );
        assert_eq!(
            watcher_reading(&Link::Connecting, None),
            (Tone::Warn, "Connecting")
        );
        assert_eq!(count_label(1, 300), "1 clip");
        assert_eq!(count_label(12, 300), "12 clips");
        assert_eq!(count_label(300, 300), "300+ clips");
        assert_eq!(
            refusal("no-plain-text"),
            "This clip has no plain text to paste."
        );
    }
}
