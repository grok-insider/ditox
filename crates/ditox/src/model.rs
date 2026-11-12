use ditox_rpc::{Entry, Filter, Kind};

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

/// What a clip's row says: for text its first lines, each on one line of
/// the row; for an image its size, or the name it was copied under.
pub fn row_text(clip: &Entry, lines: usize) -> String {
    let squeeze = |line: &str| line.split_whitespace().collect::<Vec<_>>().join(" ");
    if clip.kind == Kind::Image {
        if clip.preview.starts_with("Image image/") || clip.preview.trim().is_empty() {
            return match (clip.image_width, clip.image_height) {
                (Some(width), Some(height)) => format!("Image, {width} \u{d7} {height}"),
                _ => "Image".into(),
            };
        }
        return squeeze(&clip.preview);
    }
    let source = if clip.content.trim().is_empty() {
        &clip.preview
    } else {
        &clip.content
    };
    let shown: Vec<String> = source
        .lines()
        .map(squeeze)
        .filter(|line| !line.is_empty())
        .take(lines.max(1))
        .collect();
    if shown.is_empty() {
        "Blank text".into()
    } else {
        shown.join("\n")
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
    match (clip.kind, mime) {
        (Kind::Text, _) if is_link(clip) => Some("Link".into()),
        (_, "text/plain" | "") => None,
        (_, "text/html") => Some("HTML".into()),
        (_, "text/uri-list") => Some("File list".into()),
        (_, "image/jpeg") => Some("JPEG".into()),
        (_, "image/webp") => Some("WebP".into()),
        (Kind::Image, mime) => Some(
            mime.strip_prefix("image/")
                .unwrap_or(mime)
                .to_ascii_uppercase(),
        ),
        (_, mime) => Some(mime.into()),
    }
}

/// The line under a clip seen whole: what it is, how large, where from, when.
pub fn caption(clip: &Entry, text: Option<&str>, now_ms: i64) -> String {
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
    match clip.source_app.as_deref().filter(|app| !app.is_empty()) {
        Some(app) if app.chars().count() > APP_CHARS => {
            facts.push(format!("{}\u{2026}", head(app, APP_CHARS - 1).0));
        }
        Some(app) => facts.push(app.into()),
        None => {}
    }
    facts.push(age_long(now_ms, clip.touched_ms()));
    facts.join(" \u{b7} ")
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
    clip.kind == Kind::Image
        && clip.byte_len <= MAX_PREVIEW_FILE
        && decoded_bytes(clip) <= MAX_PREVIEW_DECODED
}

/// Whether the clip is one web address and nothing else.
pub fn is_link(clip: &Entry) -> bool {
    is_complete(clip) && link(&clip.content).is_some()
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

/// Where the words of `query` appear in `text`, whatever their case, as
/// byte ranges in order and without overlap. Only ASCII letters fold, so a
/// find always starts and ends between characters.
pub fn matches(text: &str, query: &str) -> Vec<(usize, usize)> {
    let hay = text.as_bytes();
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

pub fn is_pinned(clip: &Entry) -> bool {
    clip.favorite || clip.sticky
}

/// Where the selection starts: the newest clip, below the pinned ones that
/// lead the list. A search starts on its best match.
pub fn home(clips: &[Entry], searching: bool) -> usize {
    if searching {
        return 0;
    }
    let pinned = clips.iter().take_while(|clip| is_pinned(clip)).count();
    if pinned < clips.len() { pinned } else { 0 }
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

/// What the list is narrowed to, in the words the window uses for it.
pub fn filter_name(filter: Filter) -> &'static str {
    match filter {
        Filter::Text => "Text",
        Filter::Images => "Images",
        Filter::Favorites => "Pinned",
        Filter::All | Filter::Today => "All clips",
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
pub mod tests {
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
    fn a_row_shows_the_first_lines_that_say_something_and_an_image_its_size() {
        let code = clip(1, "\n  fn main() {\n\n\t\tlet a   =  1;\n  }\n");
        assert_eq!(row_text(&code, 2), "fn main() {\nlet a = 1;");
        assert_eq!(row_text(&code, 1), "fn main() {");
        assert_eq!(row_text(&code, 0), "fn main() {");
        assert_eq!(row_text(&code, 5), "fn main() {\nlet a = 1;\n}");
        assert_eq!(row_text(&clip(1, " \n\t "), 2), "Blank text");

        let mut cut = clip(1, "");
        cut.preview = "only  the preview".into();
        assert_eq!(row_text(&cut, 2), "only the preview");

        let shot = image("Image image/png 82000 bytes 640x480", Some((640, 480)));
        assert_eq!(row_text(&shot, 2), "Image, 640 \u{d7} 480");
        assert_eq!(row_text(&image("", None), 2), "Image");
        assert_eq!(row_text(&image("photo.png", None), 2), "photo.png");
    }

    #[test]
    fn the_caption_says_what_the_clip_is_how_large_where_from_and_when() {
        let minute = 60_000;
        let mut text = clip(1, "one\ntwo");
        text.created_at_ms = 0;
        assert_eq!(
            caption(&text, Some("one\ntwo"), 5 * minute),
            "7 B \u{b7} 2 lines \u{b7} 5 minutes ago"
        );
        assert_eq!(caption(&text, None, 30_000), "7 B \u{b7} just now");
        text.source_app = Some("kitty".into());
        assert_eq!(
            caption(&text, Some("one"), 3_600_000),
            "7 B \u{b7} 1 line \u{b7} kitty \u{b7} 1 hour ago"
        );
        text.source_app = Some("a".repeat(40));
        assert!(caption(&text, None, 0).contains(&format!("{}\u{2026}", "a".repeat(23))));

        let address = clip(2, "https://fabrials.com/docs");
        assert!(caption(&address, None, 0).starts_with("Link \u{b7} 25 B"));
        let mut page = clip(3, "<b>x</b>");
        page.mime = "text/html".into();
        assert!(caption(&page, None, 0).starts_with("HTML \u{b7}"));

        let shot = image("", Some((640, 480)));
        assert_eq!(
            caption(&shot, None, 3 * 86_400_000),
            "PNG \u{b7} 640 \u{d7} 480 \u{b7} 82.0 kB \u{b7} 3 days ago"
        );
        let mut photo = image("", None);
        photo.mime = "image/jpeg".into();
        photo.byte_len = 2_500_000;
        assert!(caption(&photo, None, 0).starts_with("JPEG \u{b7} 2.5 MB"));
        assert_eq!(size(999), "999 B");
    }

    #[test]
    fn a_huge_picture_is_described_and_not_drawn() {
        assert!(previewable(&image("", Some((3840, 2160)))));
        assert!(!previewable(&image("", Some((20_000, 20_000)))));
        let mut heavy = image("", Some((100, 100)));
        heavy.byte_len = MAX_PREVIEW_FILE + 1;
        assert!(!previewable(&heavy));
        let mut unknown = image("", None);
        unknown.byte_len = 70 * 1024 * 1024;
        assert!(!previewable(&unknown));
        assert!(!previewable(&clip(1, "text")));
    }

    #[test]
    fn a_cut_clip_is_not_complete_and_not_a_link() {
        let whole = clip(1, "https://example.com/a");
        assert!(is_complete(&whole) && is_link(&whole));
        let mut cut = whole.clone();
        cut.byte_len += 10;
        assert!(!is_complete(&cut) && !is_link(&cut));
        for not in [
            "https://",
            "https:///x",
            "https://.x",
            "see https://example.com",
            "https://a b",
            "ftp://example.com",
        ] {
            assert_eq!(link(not), None, "{not}");
        }
        assert_eq!(link(" http://example.com \n"), Some("http://example.com"));
        assert_eq!(link(&format!("https://e.com/{}", "a".repeat(2050))), None);
    }

    #[test]
    fn lines_keep_their_indent_in_columns_and_stop_at_the_cap() {
        let (lines, more) = indented_lines("fn a() {\n    b();\n\tc();\n\n}", 10);
        assert_eq!(
            lines,
            [(0, "fn a() {"), (4, "b();"), (4, "c();"), (0, ""), (0, "}")]
        );
        assert!(!more);
        let (lines, more) = indented_lines("a\nb\nc", 2);
        assert_eq!(lines.len(), 2);
        assert!(more);
        let deep = format!("{}x", " ".repeat(60));
        assert_eq!(indented_lines(&deep, 1).0, [(MAX_INDENT, "x")]);
        assert_eq!(head("h\u{e9}llo", 2), ("h\u{e9}", true));
        assert_eq!(head("hi", 2), ("hi", false));
    }

    #[test]
    fn selection_follows_the_clip_and_stops_at_the_ends() {
        let clips = [clip(1, "a"), clip(2, "b"), clip(3, "c")];
        assert_eq!(keep_selection(&clips, Some(3), false), 2);
        assert_eq!(keep_selection(&clips, Some(9), false), 0);
        assert_eq!(keep_selection(&clips, None, false), 0);
        assert_eq!(step(3, 0, -1), 0);
        assert_eq!(step(3, 1, 1), 2);
        assert_eq!(step(3, 2, 8), 2);
        assert_eq!(step(0, 0, 1), 0);
    }

    #[test]
    fn the_selection_starts_on_the_newest_clip_below_the_pinned_ones() {
        let mut pinned = clip(1, "pinned");
        pinned.favorite = true;
        let mut sticky = clip(2, "sticky");
        sticky.sticky = true;
        let clips = [
            pinned.clone(),
            sticky.clone(),
            clip(3, "new"),
            clip(4, "old"),
        ];
        assert_eq!(home(&clips, false), 2);
        assert_eq!(home(&clips, true), 0);
        assert_eq!(home(&[pinned, sticky], false), 0);
        assert_eq!(home(&[], false), 0);
        assert_eq!(keep_selection(&clips, Some(9), false), 2);
    }

    #[test]
    fn the_words_searched_for_are_found_whatever_their_case() {
        assert_eq!(matches("Cargo test cargo", "cargo"), [(0, 5), (11, 16)]);
        assert_eq!(matches("abcabc", "bc ab"), [(0, 6)]);
        assert_eq!(
            matches("caf\u{e9} CAF\u{e9}", "caf\u{e9}"),
            [(0, 5), (6, 11)]
        );
        assert!(matches("anything", "  ").is_empty());
        assert!(matches("short", "much longer than it").is_empty());
    }

    #[test]
    fn a_filter_and_a_refusal_are_said_in_words() {
        assert_eq!(filter_name(Filter::All), "All clips");
        assert_eq!(filter_name(Filter::Favorites), "Pinned");
        assert_eq!(refusal("missing"), "That clip is gone.");
        assert_eq!(
            refusal("no-plain-text"),
            "This clip has no plain text to paste."
        );
        assert_eq!(refusal("something else"), "something else");
    }
}
