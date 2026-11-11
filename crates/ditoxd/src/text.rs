use sha2::{Digest, Sha256};

pub fn sha256_hex(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0xf), 16).unwrap_or('0'));
    }
    out
}

pub fn sanitize_bytes(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        let byte = input[index];
        if byte == 0x1b && input.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < input.len() && !(0x40..=0x7e).contains(&input[index]) {
                index += 1;
            }
            if index < input.len() {
                index += 1;
            }
            continue;
        }
        if byte == b'\n' || byte == b'\r' || byte == b'\t' || byte >= 0x20 {
            out.push(byte);
        }
        index += 1;
    }
    out
}

pub fn sanitize_text(input: &[u8]) -> String {
    match String::from_utf8(sanitize_bytes(input)) {
        Ok(text) => text,
        Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
    }
}

pub fn cap_utf8(text: &str, max_bytes: u32) -> &str {
    let max = max_bytes as usize;
    if max == 0 || text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub fn preview(input: &str, limit: u32) -> String {
    let sanitized = sanitize_text(input.as_bytes());
    let limit = limit as usize;
    if sanitized.len() <= limit {
        return sanitized;
    }
    let mut end = limit;
    while end > 0 && !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &sanitized[..end])
}

pub fn contains_ignore_ascii_case(value: &str, pattern: &str) -> bool {
    let (value, pattern) = (value.as_bytes(), pattern.as_bytes());
    if pattern.is_empty() || pattern.len() > value.len() {
        return false;
    }
    value
        .windows(pattern.len())
        .any(|window| window.eq_ignore_ascii_case(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_ansi_and_controls() {
        assert_eq!(
            sanitize_text(b"a\x1b[31mred\x1b[0m\x00b\tc\n"),
            "aredb\tc\n"
        );
        assert_eq!(sanitize_text(b"plain"), "plain");
        assert_eq!(
            sanitize_text("ma\u{f1}ana \x1b[1mya".as_bytes()),
            "ma\u{f1}ana ya"
        );
    }

    #[test]
    fn sha256_matches_the_known_digest() {
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn a_cap_keeps_whole_codepoints_and_is_stable() {
        let text = "a\u{f1}\u{f1}\u{f1}";
        let capped = cap_utf8(text, 4);
        assert_eq!(capped, "a\u{f1}");
        assert_eq!(cap_utf8(capped, 4), capped);
        assert_eq!(cap_utf8(text, 0), text);
    }

    #[test]
    fn a_preview_is_cut_on_a_character_and_marked() {
        assert_eq!(preview("short", 160), "short");
        assert_eq!(preview("abcdef", 3), "abc...");
        assert_eq!(preview("a\u{f1}b", 2), "a...");
    }

    #[test]
    fn matching_ignores_ascii_case() {
        assert!(contains_ignore_ascii_case(
            "org.keepassxc.KeePassXC",
            "KeePassXC"
        ));
        assert!(contains_ignore_ascii_case(
            "private SECRET window title",
            "Secret Window"
        ));
        assert!(!contains_ignore_ascii_case("Alacritty", "Bitwarden"));
        assert!(!contains_ignore_ascii_case("anything", ""));
    }
}
