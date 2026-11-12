//! What the Windows clipboard holds, turned into what the daemon stores and
//! back. Nothing here asks Windows anything, so it is tested on every system.

/// The formats of one clipboard content that the daemon cares about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Formats {
    pub text: bool,
    pub html: bool,
    pub files: bool,
    pub png: bool,
    pub jpeg: bool,
    pub bitmap: bool,
}

/// The MIME types a clipboard with these formats offers, text first as
/// `wl-paste --list-types` gives them.
///
/// A program that copies text often adds a picture of it (a spreadsheet does
/// for its cells). A bare bitmap beside plain text is that picture, not an
/// image somebody copied, so it is not offered.
pub fn offers(formats: Formats) -> Vec<String> {
    let mut types = Vec::new();
    if formats.text {
        types.push("text/plain;charset=utf-8");
    }
    if formats.html {
        types.push("text/html");
    }
    if formats.files {
        types.push("text/uri-list");
    }
    if formats.png || (formats.bitmap && !formats.jpeg && !formats.text) {
        types.push("image/png");
    } else if formats.jpeg {
        types.push("image/jpeg");
    }
    types.into_iter().map(String::from).collect()
}

/// The name Windows programs use for the clipboard format of a MIME type.
pub fn native_name(mime: &str) -> &str {
    match mime {
        "image/png" => "PNG",
        "image/jpeg" => "JFIF",
        "image/gif" => "GIF",
        other => other,
    }
}

/// The text between the fragment marks of the `HTML Format`: what was
/// copied, without the header or the page around it.
pub fn html_fragment(format: &[u8]) -> Option<Vec<u8>> {
    let offset = |name: &str| {
        let at = find(format, name.as_bytes())? + name.len();
        let digits: Vec<u8> = format[at..]
            .iter()
            .copied()
            .skip_while(|byte| *byte == b' ')
            .take_while(u8::is_ascii_digit)
            .collect();
        std::str::from_utf8(&digits).ok()?.parse::<usize>().ok()
    };
    let (start, end) = match (offset("StartFragment:"), offset("EndFragment:")) {
        (Some(start), Some(end)) => (start, end),
        _ => (offset("StartHTML:")?, offset("EndHTML:")?),
    };
    (start <= end && end <= format.len()).then(|| format[start..end].to_vec())
}

/// `html` as the `HTML Format` wants it: a header of byte offsets, then the
/// fragment inside a minimal page.
pub fn html_format(html: &str) -> Vec<u8> {
    const BEFORE: &str = "<html><body><!--StartFragment-->";
    const AFTER: &str = "<!--EndFragment--></body></html>";
    let header = |start_html: usize, end_html: usize, start: usize, end: usize| {
        format!(
            "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start:010}\r\nEndFragment:{end:010}\r\n"
        )
    };
    let start_html = header(0, 0, 0, 0).len();
    let start = start_html + BEFORE.len();
    let end = start + html.len();
    let end_html = end + AFTER.len();
    let mut out = header(start_html, end_html, start, end).into_bytes();
    out.extend_from_slice(BEFORE.as_bytes());
    out.extend_from_slice(html.as_bytes());
    out.extend_from_slice(AFTER.as_bytes());
    out
}

/// Copied files as a `text/uri-list`: one `file:` address a line.
pub fn uri_list(paths: &[String]) -> String {
    paths
        .iter()
        .map(|path| file_uri(path))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `C:\Users\a b\x.txt` is `file:///C:/Users/a%20b/x.txt`; a share
/// (`\\server\share\x`) is `file://server/share/x`.
pub fn file_uri(path: &str) -> String {
    let slashed = path.replace('\\', "/");
    let (prefix, rest) = match slashed.strip_prefix("//") {
        Some(share) => ("file://", share),
        None => ("file:///", slashed.trim_start_matches('/')),
    };
    let mut out = String::from(prefix);
    for byte in rest.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(char::from(byte))
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The files a `text/uri-list` names, as Windows paths. Comments and
/// addresses that are not files are left out.
pub fn file_paths(uri_list: &str) -> Vec<String> {
    uri_list
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(file_path)
        .collect()
}

fn file_path(uri: &str) -> Option<String> {
    let rest = uri
        .get(..7)
        .filter(|scheme| scheme.eq_ignore_ascii_case("file://"))
        .map(|_| &uri[7..])?;
    let decoded = percent_decoded(rest)?;
    let path = match decoded.strip_prefix('/') {
        Some(local) => local.to_string(),
        None => format!("//{decoded}"),
    };
    (!path.is_empty()).then(|| path.replace('/', "\\"))
}

fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A picture as the device-independent bitmap every Windows program can
/// paste: 24 bits a pixel, bottom row first, each row padded to four bytes.
/// The bitmap has no transparency, so the picture is laid on white.
pub fn bitmap(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() != w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let row = (w * 3).div_ceil(4) * 4;
    let pixels = row.checked_mul(h)?;
    let mut out = Vec::with_capacity(40 + pixels);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&i32::try_from(width).ok()?.to_le_bytes());
    out.extend_from_slice(&i32::try_from(height).ok()?.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(pixels).ok()?.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for y in (0..h).rev() {
        let line = &rgba[y * w * 4..(y + 1) * w * 4];
        for pixel in line.as_chunks::<4>().0 {
            let alpha = u32::from(pixel[3]);
            let on_white =
                |channel: u8| ((u32::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8;
            out.extend_from_slice(&[on_white(pixel[2]), on_white(pixel[1]), on_white(pixel[0])]);
        }
        out.resize(out.len() + row - w * 3, 0);
    }
    Some(out)
}

/// A key chord as Windows numbers its keys: the modifiers, then the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
    pub key: u16,
}

/// The chord a keybind such as `ctrl+shift+v` names, or `None` when its key
/// is not one Windows has a number for.
pub fn chord(keybind: &str) -> Option<Chord> {
    let mut chord = Chord {
        ctrl: false,
        shift: false,
        alt: false,
        win: false,
        key: 0,
    };
    for part in keybind
        .trim()
        .to_ascii_lowercase()
        .split('+')
        .map(str::trim)
    {
        match part {
            "ctrl" | "control" => chord.ctrl = true,
            "shift" => chord.shift = true,
            "alt" | "option" => chord.alt = true,
            "super" | "meta" | "cmd" | "command" | "win" => chord.win = true,
            key if chord.key == 0 => chord.key = virtual_key(key)?,
            _ => return None,
        }
    }
    (chord.key != 0).then_some(chord)
}

fn virtual_key(name: &str) -> Option<u16> {
    let bytes = name.as_bytes();
    if let [byte] = bytes {
        return match byte {
            b'a'..=b'z' => Some(u16::from(byte.to_ascii_uppercase())),
            b'0'..=b'9' => Some(u16::from(*byte)),
            b'`' => Some(0xC0),
            b'-' => Some(0xBD),
            b'=' => Some(0xBB),
            b'[' => Some(0xDB),
            b']' => Some(0xDD),
            b'\\' => Some(0xDC),
            b';' => Some(0xBA),
            b'\'' => Some(0xDE),
            b',' => Some(0xBC),
            b'.' => Some(0xBE),
            b'/' => Some(0xBF),
            _ => None,
        };
    }
    if let Some(number) = name.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
        return (1..=24).contains(&number).then(|| 0x70 + number - 1);
    }
    Some(match name {
        "insert" | "ins" => 0x2D,
        "delete" | "del" => 0x2E,
        "enter" | "return" => 0x0D,
        "escape" | "esc" => 0x1B,
        "space" => 0x20,
        "tab" => 0x09,
        "backspace" => 0x08,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "prior" => 0x21,
        "pagedown" | "next" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "grave" => 0xC0,
        _ => return None,
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clipboard_offers_what_wl_paste_would_list() {
        let has = |set: fn(&mut Formats)| {
            let mut formats = Formats::default();
            set(&mut formats);
            offers(formats)
        };
        assert_eq!(has(|f| f.text = true), ["text/plain;charset=utf-8"]);
        assert_eq!(
            has(|f| {
                f.text = true;
                f.html = true
            }),
            ["text/plain;charset=utf-8", "text/html"]
        );
        assert_eq!(has(|f| f.files = true), ["text/uri-list"]);
        assert_eq!(has(|f| f.bitmap = true), ["image/png"], "a screenshot");
        assert_eq!(
            has(|f| {
                f.html = true;
                f.bitmap = true;
                f.png = true
            }),
            ["text/html", "image/png"],
            "an image copied in a browser"
        );
        assert_eq!(
            has(|f| {
                f.jpeg = true;
                f.bitmap = true
            }),
            ["image/jpeg"],
            "the file as it was, not a bitmap made from it"
        );
        assert_eq!(
            has(|f| {
                f.text = true;
                f.html = true;
                f.bitmap = true
            }),
            ["text/plain;charset=utf-8", "text/html"],
            "spreadsheet cells come with a picture of themselves"
        );
        assert!(has(|_| {}).is_empty());
    }

    #[test]
    fn the_html_format_is_read_back_to_what_was_written() {
        let fragment = "<b>caf\u{e9}</b> &amp; more";
        let format = html_format(fragment);
        assert_eq!(html_fragment(&format).unwrap(), fragment.as_bytes());
        let text = String::from_utf8(format.clone()).unwrap();
        assert!(text.starts_with("Version:0.9\r\nStartHTML:0000000105\r\n"));
        let start_html: usize = text[23..33].parse().unwrap();
        assert!(text[start_html..].starts_with("<html><body><!--StartFragment-->"));
        assert!(text.ends_with("<!--EndFragment--></body></html>"));
    }

    #[test]
    fn a_browser_s_html_format_gives_its_fragment() {
        let page = "<html><body>\r\n<!--StartFragment--><a href=\"https://example.com\">link</a><!--EndFragment-->\r\n</body></html>";
        let fragment_at = page.find("<a href").unwrap();
        let fragment_end = page.find("<!--EndFragment").unwrap();
        let header_len = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\nSourceURL:https://example.com/\r\n".len();
        let format = format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\nSourceURL:https://example.com/\r\n{page}",
            header_len,
            header_len + page.len(),
            header_len + fragment_at,
            header_len + fragment_end,
        );
        assert_eq!(
            html_fragment(format.as_bytes()).unwrap(),
            b"<a href=\"https://example.com\">link</a>"
        );
        assert_eq!(html_fragment(b"no header at all"), None);
        assert_eq!(
            html_fragment(b"StartFragment:0000000900\r\nEndFragment:0000000950\r\n"),
            None,
            "offsets past the end"
        );
    }

    #[test]
    fn copied_files_become_file_addresses_and_back() {
        let paths = vec![
            "C:\\Users\\a b\\notes #1.txt".to_string(),
            "D:\\m\u{fa}sica\\canci\u{f3}n.mp3".to_string(),
            "\\\\server\\share\\report.pdf".to_string(),
        ];
        let list = uri_list(&paths);
        assert_eq!(
            list,
            "file:///C:/Users/a%20b/notes%20%231.txt\nfile:///D:/m%C3%BAsica/canci%C3%B3n.mp3\nfile://server/share/report.pdf"
        );
        assert_eq!(file_paths(&list), paths);
        assert_eq!(
            file_paths("# a comment\r\nhttps://example.com/x\r\nfile:///C:/a.txt\r\n"),
            ["C:\\a.txt"]
        );
        assert!(file_paths("file:///%zz").is_empty());
    }

    #[test]
    fn a_picture_becomes_a_bottom_up_bitmap_on_white() {
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, // top row: red green blue
            0, 0, 0, 0, 0, 0, 0, 128, 10, 20, 30,
            255, // bottom row: clear, half black, a colour
        ];
        let dib = bitmap(3, 2, &rgba).unwrap();
        assert_eq!(&dib[..4], &40u32.to_le_bytes());
        assert_eq!(&dib[4..8], &3i32.to_le_bytes());
        assert_eq!(
            &dib[8..12],
            &2i32.to_le_bytes(),
            "positive: bottom row first"
        );
        assert_eq!(&dib[14..16], &24u16.to_le_bytes());
        assert_eq!(dib.len(), 40 + 2 * 12, "nine bytes a row, padded to twelve");
        assert_eq!(
            &dib[40..52],
            &[255, 255, 255, 127, 127, 127, 30, 20, 10, 0, 0, 0],
            "the bottom row, blue first, clear pixels white"
        );
        assert_eq!(&dib[52..64], &[0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0]);
        assert_eq!(bitmap(0, 2, &[]), None);
        assert_eq!(
            bitmap(2, 2, &[0; 4]),
            None,
            "pixels that do not fit the size"
        );
    }

    #[test]
    fn a_keybind_names_keys_windows_can_press() {
        assert_eq!(
            chord("ctrl+v"),
            Some(Chord {
                ctrl: true,
                shift: false,
                alt: false,
                win: false,
                key: 0x56
            })
        );
        assert_eq!(
            chord(" Ctrl + Shift + V "),
            Some(Chord {
                ctrl: true,
                shift: true,
                alt: false,
                win: false,
                key: 0x56
            })
        );
        assert_eq!(chord("shift+insert").unwrap().key, 0x2D);
        assert_eq!(chord("super+alt+f12").unwrap().key, 0x7B);
        assert_eq!(chord("ctrl+`").unwrap().key, 0xC0);
        assert_eq!(chord("ctrl+1").unwrap().key, 0x31);
        assert_eq!(chord("ctrl+shift"), None, "no key");
        assert_eq!(chord("ctrl+v+c"), None, "two keys");
        assert_eq!(chord("ctrl+XF86Paste"), None, "a key Windows does not have");
        assert_eq!(chord(""), None);
    }
}
