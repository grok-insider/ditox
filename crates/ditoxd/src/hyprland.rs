use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::policy::WindowSnap;
use crate::process;

const TIMEOUT: Duration = Duration::from_secs(3);
const REPLY_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveWindow {
    pub address: String,
    pub class: String,
    pub title: String,
}

impl ActiveWindow {
    pub fn snap(&self) -> WindowSnap {
        WindowSnap {
            class: self.class.clone(),
            title: self.title.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchSyntax {
    Legacy,
    Lua,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DispatchOutcome {
    Ok,
    Failed,
    NeedsLua,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PasteShortcut {
    mods: String,
    key: String,
}

/// What the request socket said, when this session has one. `hyprctl` asks
/// the same socket and costs a process each time.
enum Socket {
    Reply(Vec<u8>),
    /// The compositor was asked and did not answer in time. Asking again by
    /// another way could do the thing twice.
    Silent,
    Absent,
}

fn ask(command: &str) -> Socket {
    let Some(path) = ditox_rpc::hypr::socket_path() else {
        return Socket::Absent;
    };
    match ditox_rpc::hypr::request_at(&path, command, TIMEOUT) {
        Ok(reply) => Socket::Reply(reply),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            Socket::Absent
        }
        Err(_) => Socket::Silent,
    }
}

/// A question that changes nothing: the socket's answer, or `hyprctl`'s.
fn query(command: &str, limit: usize) -> Option<Vec<u8>> {
    match ask(&format!("j/{command}")) {
        Socket::Reply(reply) => Some(reply),
        Socket::Silent => None,
        Socket::Absent => {
            let output = process::run("hyprctl", &["-j", command], limit, TIMEOUT).ok()?;
            output.success().then_some(output.stdout)
        }
    }
}

pub fn active_window() -> Option<ActiveWindow> {
    parse_active_window(&query("activewindow", 1024 * 1024)?)
}

/// The class of the window at `address`, to choose how to paste into it.
pub fn window_class(address: &str) -> Option<String> {
    class_of(&query("clients", 1024 * 1024)?, address)
}

fn class_of(clients: &[u8], address: &str) -> Option<String> {
    let clients: Value = serde_json::from_slice(clients).ok()?;
    clients.as_array()?.iter().find_map(|client| {
        (client.get("address")?.as_str()? == address)
            .then(|| client.get("class")?.as_str().map(String::from))?
    })
}

fn parse_active_window(reply: &[u8]) -> Option<ActiveWindow> {
    let value: Value = serde_json::from_slice(reply).ok()?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    Some(ActiveWindow {
        address: value.get("address")?.as_str()?.to_string(),
        class: field("class"),
        title: field("title"),
    })
}

pub fn send_paste_shortcut(
    target_window: Option<&str>,
    buffer_ms: u32,
    keybind: &str,
) -> Result<()> {
    let shortcut = parse_paste_shortcut(keybind)?;
    let mut syntax = detect_dispatch_syntax();

    std::thread::sleep(Duration::from_millis(u64::from(buffer_ms)));

    if let Some(address) = target_window {
        let mut focused = focus_window(syntax, address);
        if focused == DispatchOutcome::NeedsLua {
            syntax = DispatchSyntax::Lua;
            focused = focus_window(syntax, address);
        }
        if !shortcut_may_follow_focus(syntax, focused) {
            return Err(Error::PasteBack(
                "the window you were in is gone; the clip is on the clipboard".into(),
            ));
        }
    }

    let mut outcome = send_shortcut(syntax, &shortcut);
    if outcome == DispatchOutcome::NeedsLua {
        outcome = send_shortcut(DispatchSyntax::Lua, &shortcut);
    }
    if outcome != DispatchOutcome::Ok {
        return Err(Error::PasteBack(
            "Hyprland did not send the paste shortcut; the clip is on the clipboard".into(),
        ));
    }
    Ok(())
}

fn detect_dispatch_syntax() -> DispatchSyntax {
    match query("version", REPLY_LIMIT) {
        Some(reply) => syntax_from_version_reply(&reply),
        None => DispatchSyntax::Legacy,
    }
}

fn syntax_from_version_reply(reply: &[u8]) -> DispatchSyntax {
    serde_json::from_slice::<Value>(reply)
        .ok()
        .and_then(|value| {
            value
                .as_object()?
                .get("version")?
                .as_str()
                .map(syntax_for_version)
        })
        .unwrap_or(DispatchSyntax::Legacy)
}

fn syntax_for_version(version: &str) -> DispatchSyntax {
    let rest = version.trim();
    let rest = rest.strip_prefix(['v', 'V']).unwrap_or(rest);
    let mut parts = rest.split('.');
    let (Some(major), Some(minor)) = (
        parts.next().and_then(leading_number),
        parts.next().and_then(leading_number),
    ) else {
        return DispatchSyntax::Legacy;
    };
    if major > 0 || minor >= 57 {
        DispatchSyntax::Lua
    } else {
        DispatchSyntax::Legacy
    }
}

fn leading_number(text: &str) -> Option<u32> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    text[..digits].parse().ok()
}

fn shortcut_may_follow_focus(syntax: DispatchSyntax, focused: DispatchOutcome) -> bool {
    match syntax {
        DispatchSyntax::Legacy => true,
        DispatchSyntax::Lua => focused == DispatchOutcome::Ok,
    }
}

fn focus_window(syntax: DispatchSyntax, address: &str) -> DispatchOutcome {
    match syntax {
        DispatchSyntax::Legacy => run_dispatch(
            syntax,
            &["dispatch", "focuswindow", &format!("address:{address}")],
        ),
        DispatchSyntax::Lua => run_dispatch(syntax, &["dispatch", &lua_focus_expr(address)]),
    }
}

fn send_shortcut(syntax: DispatchSyntax, shortcut: &PasteShortcut) -> DispatchOutcome {
    match syntax {
        DispatchSyntax::Legacy => run_dispatch(
            syntax,
            &["dispatch", "sendshortcut", &legacy_shortcut_arg(shortcut)],
        ),
        DispatchSyntax::Lua => run_dispatch(syntax, &["dispatch", &lua_shortcut_expr(shortcut)]),
    }
}

fn run_dispatch(syntax: DispatchSyntax, args: &[&str]) -> DispatchOutcome {
    if syntax == DispatchSyntax::Lua {
        match ask(&format!("/{}", args.join(" "))) {
            Socket::Reply(reply) => {
                return classify_dispatch(syntax, Some(0), &String::from_utf8_lossy(&reply), "");
            }
            Socket::Silent => return DispatchOutcome::Failed,
            Socket::Absent => {}
        }
    }
    match process::run("hyprctl", args, REPLY_LIMIT, TIMEOUT) {
        Ok(output) => classify_dispatch(
            syntax,
            output.code,
            &String::from_utf8_lossy(&output.stdout),
            &output.stderr,
        ),
        Err(_) => DispatchOutcome::Failed,
    }
}

fn classify_dispatch(
    syntax: DispatchSyntax,
    code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> DispatchOutcome {
    let Some(code) = code else {
        return DispatchOutcome::Failed;
    };
    match syntax {
        DispatchSyntax::Legacy if is_lua_parse_error(stdout) || is_lua_parse_error(stderr) => {
            DispatchOutcome::NeedsLua
        }
        DispatchSyntax::Legacy if code == 0 => DispatchOutcome::Ok,
        DispatchSyntax::Lua if code == 0 && stdout.trim() == "ok" => DispatchOutcome::Ok,
        _ => DispatchOutcome::Failed,
    }
}

fn is_lua_parse_error(reply: &str) -> bool {
    reply.contains("[string \"return hl.dispatch(")
}

/// Whether `keybind` names a shortcut Hyprland can be asked to send.
pub fn check_paste_shortcut(keybind: &str) -> Result<()> {
    parse_paste_shortcut(keybind).map(drop)
}

fn parse_paste_shortcut(keybind: &str) -> Result<PasteShortcut> {
    let invalid = || Error::Config(format!("\"{keybind}\" is not a paste keybind"));
    let lower = keybind.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Err(invalid());
    }
    let mut mods = Vec::new();
    let mut key = None;
    for part in lower.split('+').map(str::trim) {
        if part.is_empty() {
            return Err(invalid());
        }
        match paste_modifier(part) {
            Some(modifier) => mods.push(modifier),
            None if key.is_none() => key = Some(hyprland_key(part)),
            None => return Err(invalid()),
        }
    }
    Ok(PasteShortcut {
        mods: mods.join(" "),
        key: key.ok_or_else(invalid)?,
    })
}

fn paste_modifier(part: &str) -> Option<&'static str> {
    match part {
        "ctrl" | "control" => Some("CTRL"),
        "shift" => Some("SHIFT"),
        "alt" | "option" => Some("ALT"),
        "super" | "meta" | "cmd" | "command" | "win" => Some("SUPER"),
        _ => None,
    }
}

fn hyprland_key(part: &str) -> String {
    match part {
        "enter" | "return" => "RETURN".into(),
        "escape" | "esc" => "ESCAPE".into(),
        _ => part.to_ascii_uppercase(),
    }
}

fn legacy_shortcut_arg(shortcut: &PasteShortcut) -> String {
    format!("{},{},", shortcut.mods, shortcut.key)
}

fn lua_focus_expr(address: &str) -> String {
    format!(
        "hl.dsp.focus({{ window = \"address:{}\" }})",
        lua_string_body(address)
    )
}

fn lua_shortcut_expr(shortcut: &PasteShortcut) -> String {
    format!(
        "hl.dsp.send_shortcut({{ mods = \"{}\", key = \"{}\" }})",
        lua_string_body(&shortcut.mods),
        lua_string_body(&shortcut.key)
    )
}

fn lua_string_body(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\0'..='\x1f' | '\x7f' => out.push_str(&format!("\\{:03}", ch as u32)),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::DispatchOutcome::{Failed, NeedsLua, Ok as Sent};
    use super::DispatchSyntax::{Legacy, Lua};
    use super::*;

    #[test]
    fn the_dispatch_syntax_follows_the_hyprland_version() {
        for (version, syntax) in [
            ("0.56.2", Legacy),
            ("0.57.0", Lua),
            ("v0.57.1", Lua),
            ("0.58.0-dev", Lua),
            ("1.0.0", Lua),
            ("0.41", Legacy),
            ("0", Legacy),
            ("", Legacy),
            ("unknown", Legacy),
        ] {
            assert_eq!(syntax_for_version(version), syntax, "{version}");
        }
    }

    #[test]
    fn an_unreadable_version_reply_keeps_the_legacy_syntax() {
        assert_eq!(
            syntax_from_version_reply(br#"{"branch":"main","version":"0.57.0","tag":"v0.57.0"}"#),
            Lua
        );
        for reply in [
            &br#"{"version":"0.56.2"}"#[..],
            b"",
            b"Hyprland 0.57.0 built from branch main",
            br#"["0.57.0"]"#,
            br#"{"version":57}"#,
            b"{}",
        ] {
            assert_eq!(syntax_from_version_reply(reply), Legacy);
        }
    }

    #[test]
    fn paste_shortcuts_render_in_both_syntaxes() {
        for (keybind, legacy, lua) in [
            (
                "ctrl+v",
                "CTRL,V,",
                r#"hl.dsp.send_shortcut({ mods = "CTRL", key = "V" })"#,
            ),
            (
                " Ctrl + Shift + v ",
                "CTRL SHIFT,V,",
                r#"hl.dsp.send_shortcut({ mods = "CTRL SHIFT", key = "V" })"#,
            ),
            (
                "super+alt+space",
                "SUPER ALT,SPACE,",
                r#"hl.dsp.send_shortcut({ mods = "SUPER ALT", key = "SPACE" })"#,
            ),
            (
                "insert",
                ",INSERT,",
                r#"hl.dsp.send_shortcut({ mods = "", key = "INSERT" })"#,
            ),
            (
                "ctrl+\"",
                "CTRL,\",",
                r#"hl.dsp.send_shortcut({ mods = "CTRL", key = "\"" })"#,
            ),
            (
                "ctrl+\\",
                "CTRL,\\,",
                r#"hl.dsp.send_shortcut({ mods = "CTRL", key = "\\" })"#,
            ),
            (
                "ctrl+esc",
                "CTRL,ESCAPE,",
                r#"hl.dsp.send_shortcut({ mods = "CTRL", key = "ESCAPE" })"#,
            ),
        ] {
            let shortcut = parse_paste_shortcut(keybind).unwrap();
            assert_eq!(legacy_shortcut_arg(&shortcut), legacy);
            assert_eq!(lua_shortcut_expr(&shortcut), lua);
        }
    }

    #[test]
    fn invalid_keybinds_are_refused() {
        for keybind in ["", "ctrl+shift", "ctrl+v+x", "v+"] {
            assert!(parse_paste_shortcut(keybind).is_err(), "{keybind}");
        }
    }

    #[test]
    fn the_focus_target_cannot_end_the_lua_string() {
        assert_eq!(
            lua_focus_expr("0x568e7a8e23f0"),
            r#"hl.dsp.focus({ window = "address:0x568e7a8e23f0" })"#
        );
        assert_eq!(
            lua_focus_expr("0xabc\" }) hl.dsp.exit() --\\\n\x7f"),
            "hl.dsp.focus({ window = \"address:0xabc\\\" }) hl.dsp.exit() --\\\\\\010\\127\" })"
        );
    }

    #[test]
    fn a_lua_dispatch_only_succeeds_on_the_exact_reply_ok() {
        assert_eq!(classify_dispatch(Lua, Some(0), "ok\n", ""), Sent);
        assert_eq!(
            classify_dispatch(Lua, Some(0), "warning: window not found\n", ""),
            Failed
        );
        assert_eq!(classify_dispatch(Lua, Some(0), "", ""), Failed);
        assert_eq!(classify_dispatch(Lua, Some(7), "ok", ""), Failed);
        assert_eq!(classify_dispatch(Lua, None, "ok", ""), Failed);
    }

    #[test]
    fn a_legacy_dispatch_keeps_the_exit_status_rule_and_spots_a_lua_compositor() {
        assert_eq!(classify_dispatch(Legacy, Some(0), "ok\n", ""), Sent);
        assert_eq!(classify_dispatch(Legacy, Some(0), "", ""), Sent);
        assert_eq!(classify_dispatch(Legacy, Some(1), "", ""), Failed);
        let parse_error = "error: 3 [string \"return hl.dispatch(sendshortcut CTRL,V,)\"]:1: ')' expected near 'CTRL'\n";
        assert_eq!(
            classify_dispatch(Legacy, Some(7), parse_error, ""),
            NeedsLua
        );
        assert_eq!(
            classify_dispatch(Legacy, Some(7), "", parse_error),
            NeedsLua
        );
    }

    #[test]
    fn on_hyprland_57_the_shortcut_only_follows_a_focus_that_replied_ok() {
        assert!(shortcut_may_follow_focus(Lua, Sent));
        assert!(!shortcut_may_follow_focus(Lua, Failed));
        assert!(!shortcut_may_follow_focus(
            Lua,
            classify_dispatch(
                Lua,
                Some(0),
                "warning: =[C]:-1: hl.focus: window not found\n",
                ""
            )
        ));
        assert!(shortcut_may_follow_focus(Legacy, Sent));
        assert!(shortcut_may_follow_focus(Legacy, Failed));
    }

    #[test]
    fn the_active_window_reply_gives_address_class_and_title() {
        let window =
            parse_active_window(br#"{"address":"0xabc","class":"firefox","title":"Docs"}"#)
                .unwrap();
        assert_eq!(
            window,
            ActiveWindow {
                address: "0xabc".into(),
                class: "firefox".into(),
                title: "Docs".into()
            }
        );
        assert_eq!(parse_active_window(b"{}"), None);
        assert_eq!(parse_active_window(b"not json"), None);
    }

    #[test]
    fn the_class_of_a_window_is_found_by_its_address() {
        let clients = br#"[{"address":"0xaaa","class":"firefox"},{"address":"0xbbb","class":"kitty"},{"address":"0xccc"}]"#;
        assert_eq!(class_of(clients, "0xbbb").as_deref(), Some("kitty"));
        assert_eq!(class_of(clients, "0xccc"), None);
        assert_eq!(class_of(clients, "0xddd"), None);
        assert_eq!(class_of(b"{}", "0xbbb"), None);
    }
}
