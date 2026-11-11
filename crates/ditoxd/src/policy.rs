use ditox_rpc::Kind;

use crate::text::contains_ignore_ascii_case;

pub const MAX_IMAGE_READ_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_TEXT_READ_BYTES: usize = 16 * 1024 * 1024;
pub const SOCKET_MODE: u32 = 0o600;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowSnap {
    pub class: String,
    pub title: String,
}

pub struct CaptureFacts<'a> {
    pub kind: Kind,
    pub bytes: &'a [u8],
    pub image_mime: Option<&'a str>,
    pub window: Option<&'a WindowSnap>,
    pub content_hash: &'a str,
    pub self_hash: Option<&'a str>,
    pub excluded_apps: &'a [String],
    pub excluded_windows: &'a [String],
    pub secret_patterns: &'a [String],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureDecision {
    pub store: bool,
    pub reason: &'static str,
    pub source_app: Option<String>,
}

impl CaptureDecision {
    fn refuse(reason: &'static str) -> Self {
        CaptureDecision {
            store: false,
            reason,
            source_app: None,
        }
    }
}

pub fn offer_is_sensitive(state: Option<&str>) -> bool {
    state == Some("sensitive")
}

pub fn wake_stores(sensitive: bool) -> bool {
    !sensitive
}

pub fn paste_sends_shortcut(has_target: bool) -> bool {
    has_target
}

/// Whether `class` is one of the terminals in `apps`: the whole class or the
/// part after its last dot, whatever the case. `foot` is not `football`.
pub fn is_terminal(class: &str, apps: &[String]) -> bool {
    let class = class.trim();
    let name = class.rsplit('.').next().unwrap_or(class);
    !class.is_empty()
        && apps
            .iter()
            .any(|app| app.eq_ignore_ascii_case(class) || app.eq_ignore_ascii_case(name))
}

/// The shortcut that pastes into a window of class `class`. A terminal
/// takes Ctrl+V as a control character and pastes with another chord.
pub fn paste_keybind<'a>(
    class: Option<&str>,
    keybind: &'a str,
    terminal_keybind: &'a str,
    terminal_apps: &[String],
) -> &'a str {
    match class {
        Some(class) if !terminal_keybind.trim().is_empty() && is_terminal(class, terminal_apps) => {
            terminal_keybind
        }
        _ => keybind,
    }
}

pub fn drop_unreadable_image(image_listed: bool, image_ok: bool) -> bool {
    image_listed && !image_ok
}

pub fn runtime_dir_acceptable(owner: u32, mode: u32, user: u32) -> bool {
    owner == user && mode & 0o022 == 0
}

pub fn mime_base(mime: &str) -> &str {
    let trimmed = mime.trim();
    match trimmed.split_once(';') {
        Some((base, _)) => base.trim(),
        None => trimmed,
    }
}

pub fn mime_allowed(mime: &str) -> bool {
    matches!(
        mime_base(mime),
        "text/plain" | "text/html" | "text/uri-list" | "image/png" | "image/jpeg"
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferPart {
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferPlan {
    pub kind: Kind,
    pub primary: usize,
    pub preview: Option<usize>,
    pub kept: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfferOutcome {
    Empty,
    Secret,
    Store(OfferPlan),
}

pub fn plan_offer(parts: &[OfferPart], secret_patterns: &[String]) -> OfferOutcome {
    let (mut plain, mut html, mut uri, mut png, mut jpeg) = (None, None, None, None, None);
    for (index, part) in parts.iter().enumerate() {
        if !mime_allowed(&part.mime) || part.bytes.is_empty() {
            continue;
        }
        let base = mime_base(&part.mime);
        let slot = match base {
            "text/plain" => &mut plain,
            "text/html" => &mut html,
            "text/uri-list" => &mut uri,
            "image/png" => &mut png,
            _ => &mut jpeg,
        };
        slot.get_or_insert(index);
        if base.starts_with("text/") && matches_secret(&part.bytes, secret_patterns) {
            return OfferOutcome::Secret;
        }
    }

    let image = png.or(jpeg);
    let text_primary = plain.or(html).or(uri);
    let Some(primary) = image.or(text_primary) else {
        return OfferOutcome::Empty;
    };

    let mut kept = vec![primary];
    kept.extend(
        [plain, html, uri]
            .into_iter()
            .flatten()
            .filter(|index| *index != primary),
    );

    OfferOutcome::Store(OfferPlan {
        kind: if image.is_some() {
            Kind::Image
        } else {
            Kind::Text
        },
        primary,
        preview: if image.is_some() { plain.or(uri) } else { None },
        kept,
    })
}

pub fn decide_capture(facts: &CaptureFacts<'_>) -> CaptureDecision {
    if facts.bytes.is_empty() {
        return CaptureDecision::refuse("empty");
    }
    if let Some(window) = facts.window {
        let excluded = facts
            .excluded_apps
            .iter()
            .any(|app| contains_ignore_ascii_case(&window.class, app))
            || facts
                .excluded_windows
                .iter()
                .any(|title| contains_ignore_ascii_case(&window.title, title));
        if excluded {
            return CaptureDecision::refuse("excluded");
        }
    }
    match facts.kind {
        Kind::Image if facts.image_mime.is_none() => return CaptureDecision::refuse("not-image"),
        Kind::Text if matches_secret(facts.bytes, facts.secret_patterns) => {
            return CaptureDecision::refuse("secret");
        }
        _ => {}
    }
    if facts.self_hash == Some(facts.content_hash) {
        return CaptureDecision::refuse("self");
    }
    CaptureDecision {
        store: true,
        reason: "",
        source_app: facts
            .window
            .map(|window| window.class.clone())
            .filter(|class| !class.is_empty()),
    }
}

pub fn clears_self_write(reason: &str) -> bool {
    matches!(reason, "" | "self" | "secret")
}

pub fn matches_secret(text: &[u8], patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| pattern_matches(text, pattern.as_bytes()))
}

fn pattern_matches(text: &[u8], pattern: &[u8]) -> bool {
    let literal =
        || !pattern.is_empty() && text.windows(pattern.len()).any(|window| window == pattern);
    let Some(open) = pattern.iter().position(|byte| *byte == b'[') else {
        return literal();
    };
    let Some(close) = pattern[open..]
        .iter()
        .position(|byte| *byte == b']')
        .map(|offset| open + offset)
    else {
        return literal();
    };
    if pattern.get(close + 1) != Some(&b'{') {
        return literal();
    }
    let Some(end) = pattern[close + 2..]
        .iter()
        .position(|byte| *byte == b'}')
        .map(|offset| close + 2 + offset)
    else {
        return false;
    };
    let Some(count) = std::str::from_utf8(&pattern[close + 2..end])
        .ok()
        .and_then(|digits| digits.parse::<usize>().ok())
    else {
        return false;
    };
    let prefix = &pattern[..open];
    let class = &pattern[open + 1..close];
    let Some(needed) = prefix.len().checked_add(count) else {
        return false;
    };
    if count == 0 || needed > text.len() {
        return false;
    }
    (0..=text.len() - needed).any(|start| {
        text[start..].starts_with(prefix)
            && text[start + prefix.len()..start + needed]
                .iter()
                .all(|byte| in_class(*byte, class))
    })
}

fn in_class(byte: u8, class: &[u8]) -> bool {
    let mut index = 0;
    while index < class.len() {
        if index + 2 < class.len() && class[index + 1] == b'-' {
            if (class[index]..=class[index + 2]).contains(&byte) {
                return true;
            }
            index += 3;
            continue;
        }
        if byte == class[index] {
            return true;
        }
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const GITHUB: &str = "ghp_0123456789abcdef";
    const AWS: &str = "AKIAIOSFODNN7EXAMPLE";

    fn part(mime: &str, bytes: &str) -> OfferPart {
        OfferPart {
            mime: mime.into(),
            bytes: bytes.as_bytes().to_vec(),
        }
    }

    fn patterns() -> Vec<String> {
        vec![
            "ghp_".into(),
            "AKIA[A-Z0-9]{16}".into(),
            "-----BEGIN PRIVATE KEY-----".into(),
        ]
    }

    fn facts<'a>(kind: Kind, bytes: &'a [u8], secrets: &'a [String]) -> CaptureFacts<'a> {
        CaptureFacts {
            kind,
            bytes,
            image_mime: None,
            window: None,
            content_hash: "hash",
            self_hash: None,
            excluded_apps: &[],
            excluded_windows: &[],
            secret_patterns: secrets,
        }
    }

    #[test]
    fn an_offer_keeps_one_image_and_its_text_siblings() {
        let parts = [
            part("text/plain;charset=utf-8", "https://example.com/a"),
            part("text/html", "<p>https://example.com/a</p>"),
            part("image/jpeg", "jpeg"),
            part("image/png", "png"),
            part("application/x-private", "nope"),
        ];
        let OfferOutcome::Store(plan) = plan_offer(&parts, &patterns()) else {
            panic!("stored")
        };
        assert_eq!(plan.kind, Kind::Image);
        assert_eq!(plan.primary, 3);
        assert_eq!(plan.preview, Some(0));
        assert_eq!(plan.kept, vec![3, 0, 1]);
    }

    #[test]
    fn a_secret_in_a_text_sibling_drops_the_whole_offer() {
        let parts = [
            part("image/png", "png"),
            part("text/html", "token ghp_0123456789abcdef"),
        ];
        assert_eq!(plan_offer(&parts, &patterns()), OfferOutcome::Secret);
    }

    #[test]
    fn html_without_plain_text_stays_a_text_row() {
        let OfferOutcome::Store(plan) = plan_offer(&[part("text/html", "<p>hola</p>")], &[]) else {
            panic!("stored")
        };
        assert_eq!(
            (plan.kind, plan.primary, plan.preview),
            (Kind::Text, 0, None)
        );
    }

    #[test]
    fn a_charset_parameter_is_still_plain_text_and_private_types_are_rejected() {
        let OfferOutcome::Store(plan) =
            plan_offer(&[part("text/plain;charset=utf-8", "hola")], &[])
        else {
            panic!("stored")
        };
        assert_eq!(plan.kept, vec![0]);
        assert_eq!(
            plan_offer(
                &[part("application/x-kde-passwordManagerHint", "secret")],
                &[]
            ),
            OfferOutcome::Empty
        );
        assert_eq!(
            plan_offer(&[part("text/plain", "")], &[]),
            OfferOutcome::Empty
        );
    }

    #[test]
    fn the_small_rules_say_what_they_mean() {
        assert!(offer_is_sensitive(Some("sensitive")));
        assert!(!offer_is_sensitive(Some("data")));
        assert!(!offer_is_sensitive(None));
        assert!(!wake_stores(true) && wake_stores(false));
        assert!(!paste_sends_shortcut(false) && paste_sends_shortcut(true));
        assert!(drop_unreadable_image(true, false));
        assert!(!drop_unreadable_image(true, true) && !drop_unreadable_image(false, false));
        assert!(!runtime_dir_acceptable(1, 0o700, 2));
        assert!(!runtime_dir_acceptable(1, 0o702, 1));
        assert!(runtime_dir_acceptable(1, 0o700, 1));
    }

    #[test]
    fn plain_text_from_a_normal_window_is_stored_with_its_class() {
        let window = WindowSnap {
            class: "firefox".into(),
            title: "Docs".into(),
        };
        let secrets = patterns();
        let mut facts = facts(Kind::Text, b"hello", &secrets);
        facts.window = Some(&window);
        let decision = decide_capture(&facts);
        assert!(decision.store);
        assert_eq!(decision.source_app.as_deref(), Some("firefox"));
    }

    #[test]
    fn an_excluded_window_is_not_stored() {
        let window = WindowSnap {
            class: "org.keepassxc.KeePassXC".into(),
            title: "Vault".into(),
        };
        let apps = vec!["KeePassXC".to_string()];
        let mut facts = facts(Kind::Text, b"hunter2", &[]);
        facts.window = Some(&window);
        facts.excluded_apps = &apps;
        assert_eq!(decide_capture(&facts).reason, "excluded");
    }

    #[test]
    fn tokens_and_keys_are_not_stored_and_an_empty_pattern_list_keeps_them() {
        let secrets = patterns();
        assert_eq!(
            decide_capture(&facts(Kind::Text, GITHUB.as_bytes(), &secrets)).reason,
            "secret"
        );
        assert!(decide_capture(&facts(Kind::Text, GITHUB.as_bytes(), &[])).store);
        assert_eq!(
            decide_capture(&facts(
                Kind::Text,
                format!("key {AWS} end").as_bytes(),
                &secrets
            ))
            .reason,
            "secret"
        );
        assert!(decide_capture(&facts(Kind::Text, b"AKIA-short", &secrets)).store);
        let key = b"-----BEGIN PRIVATE KEY-----\nabc";
        assert_eq!(
            decide_capture(&facts(Kind::Text, key, &secrets)).reason,
            "secret"
        );
    }

    #[test]
    fn a_pattern_with_an_absurd_repeat_count_matches_nothing_and_does_not_panic() {
        for pattern in [
            "k[A]{18446744073709551615}",
            "k[A]{18446744073709551616}",
            "k[A]{0}",
            "k[A]{x}",
            "k[A]{3",
            "k[A",
        ] {
            assert!(
                !matches_secret(b"kAAAA and more text", &[pattern.to_string()]),
                "{pattern}"
            );
        }
        assert!(matches_secret(b"kAAA", &["k[A]{3}".to_string()]));
    }

    #[test]
    fn a_terminal_gets_its_own_paste_shortcut_and_nothing_else_does() {
        let apps: Vec<String> = ["kitty", "ghostty", "foot", "org.gnome.console"]
            .map(String::from)
            .to_vec();
        for class in [
            "kitty",
            "com.mitchellh.ghostty",
            "Foot",
            "org.gnome.Console",
        ] {
            assert!(is_terminal(class, &apps), "{class}");
            assert_eq!(
                paste_keybind(Some(class), "ctrl+v", "ctrl+shift+v", &apps),
                "ctrl+shift+v"
            );
        }
        for class in ["firefox", "football", "kitty-picker", "console", ""] {
            assert!(!is_terminal(class, &apps), "{class}");
            assert_eq!(
                paste_keybind(Some(class), "ctrl+v", "ctrl+shift+v", &apps),
                "ctrl+v"
            );
        }
        assert_eq!(
            paste_keybind(None, "ctrl+v", "ctrl+shift+v", &apps),
            "ctrl+v"
        );
        assert_eq!(paste_keybind(Some("kitty"), "ctrl+v", " ", &apps), "ctrl+v");
        assert_eq!(
            paste_keybind(Some("kitty"), "ctrl+v", "ctrl+shift+v", &[]),
            "ctrl+v"
        );
    }

    #[test]
    fn our_own_paste_is_skipped() {
        let mut facts = facts(Kind::Text, b"hello", &[]);
        facts.self_hash = Some("hash");
        assert_eq!(decide_capture(&facts).reason, "self");
        facts.self_hash = Some("other");
        assert!(decide_capture(&facts).store);
    }

    #[test]
    fn image_bytes_without_a_sniffed_type_are_not_stored() {
        assert_eq!(
            decide_capture(&facts(Kind::Image, b"not an image", &[])).reason,
            "not-image"
        );
        let mut facts = facts(Kind::Image, b"png", &[]);
        facts.image_mime = Some("image/png");
        assert!(decide_capture(&facts).store);
        assert_eq!(
            decide_capture(&super::tests::facts(Kind::Text, b"", &[])).reason,
            "empty"
        );
    }
}
