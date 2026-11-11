use std::path::Path;

use serde_json::{Map, Value};

use crate::error::{Error, Result};

const DEFAULT_EXCLUDED_APPS: [&str; 7] = [
    "1Password",
    "Bitwarden",
    "KeePassXC",
    "LastPass",
    "Dashlane",
    "Password Safe",
    "Keychain Access",
];

/// Window classes that paste with the terminal shortcut. A name matches the
/// whole class or the part after its last dot, whatever the case.
const DEFAULT_TERMINAL_APPS: [&str; 19] = [
    "kitty",
    "ghostty",
    "alacritty",
    "foot",
    "footclient",
    "wezterm",
    "konsole",
    "gnome-terminal-server",
    "org.gnome.console",
    "kgx",
    "ptyxis",
    "xfce4-terminal",
    "tilix",
    "terminator",
    "rio",
    "warp",
    "contour",
    "blackbox",
    "st",
];

const DEFAULT_SECRET_PATTERNS: [&str; 12] = [
    "-----BEGIN PRIVATE KEY-----",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "-----BEGIN RSA PRIVATE KEY-----",
    "-----BEGIN EC PRIVATE KEY-----",
    "ghp_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxr-",
    "xoxs-",
    "AKIA[A-Z0-9]{16}",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub config_path: String,
    pub data_dir: String,
    pub db_path: String,
    pub image_dir: String,
    pub max_entries: u32,
    pub max_image_bytes: u32,
    pub max_text_bytes: u32,
    pub delete_after_seconds: u32,
    pub allow_duplicates: bool,
    pub paste_enabled: bool,
    pub paste_buffer_ms: u32,
    pub paste_keybind: String,
    pub terminal_paste_keybind: String,
    pub terminal_apps: Vec<String>,
    pub max_preview_chars: u32,
    pub excluded_apps: Vec<String>,
    pub excluded_windows: Vec<String>,
    pub secret_patterns: Vec<String>,
}

struct Homes {
    home: String,
    config: String,
    data: String,
}

impl Config {
    pub fn load() -> Result<Config> {
        Self::load_with(&|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
    }

    pub fn load_with(env: &dyn Fn(&str) -> Option<String>) -> Result<Config> {
        let home = env("HOME").unwrap_or_else(|| ".".into());
        let homes = Homes {
            config: env("XDG_CONFIG_HOME").unwrap_or_else(|| format!("{home}/.config")),
            data: env("XDG_DATA_HOME").unwrap_or_else(|| format!("{home}/.local/share")),
            home,
        };
        let data_dir = env("DITOX_DATA_DIR").unwrap_or_else(|| format!("{}/ditox", homes.data));
        let mut config = Config {
            config_path: env("DITOX_CONFIG")
                .unwrap_or_else(|| format!("{}/ditox/config.toml", homes.config)),
            db_path: env("DITOX_DB").unwrap_or_else(|| format!("{data_dir}/ditox-v2.db")),
            image_dir: format!("{data_dir}/images-v2"),
            paste_keybind: env("DITOX_AUTO_PASTE_KEYBIND").unwrap_or_else(|| "ctrl+v".into()),
            ..Config::defaults(&data_dir)
        };

        if let Ok(bytes) = std::fs::read(&config.config_path) {
            config.apply_file(&String::from_utf8_lossy(&bytes), &homes);
        }

        for dir in [
            Some(Path::new(&config.data_dir)),
            Path::new(&config.db_path).parent(),
            Some(Path::new(&config.image_dir)),
        ] {
            if let Some(dir) = dir.filter(|dir| !dir.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)
                    .map_err(|err| Error::Config(format!("{}: {err}", dir.display())))?;
            }
        }
        Ok(config)
    }

    pub fn defaults(data_dir: &str) -> Config {
        Config {
            config_path: "config.toml".into(),
            data_dir: data_dir.into(),
            db_path: format!("{data_dir}/ditox-v2.db"),
            image_dir: format!("{data_dir}/images-v2"),
            max_entries: 1000,
            max_image_bytes: 128 * 1024 * 1024,
            max_text_bytes: 256 * 1024,
            delete_after_seconds: 0,
            allow_duplicates: false,
            paste_enabled: true,
            paste_buffer_ms: 120,
            paste_keybind: "ctrl+v".into(),
            terminal_paste_keybind: "ctrl+shift+v".into(),
            terminal_apps: DEFAULT_TERMINAL_APPS.map(String::from).to_vec(),
            max_preview_chars: 160,
            excluded_apps: DEFAULT_EXCLUDED_APPS.map(String::from).to_vec(),
            excluded_windows: Vec::new(),
            secret_patterns: DEFAULT_SECRET_PATTERNS.map(String::from).to_vec(),
        }
    }

    fn apply_file(&mut self, text: &str, homes: &Homes) {
        if text.trim_start().starts_with('{') {
            if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text) {
                self.apply_json(&object, homes);
            }
            return;
        }
        self.apply_toml(text, homes);
    }

    fn apply_json(&mut self, object: &Map<String, Value>, homes: &Homes) {
        if let Some(value) = number(object, &["max_entries", "maxHistory"]) {
            self.max_entries = value;
        }
        if let Some(value) = number(object, &["max_image_bytes", "maxImageBytes"]) {
            self.max_image_bytes = value;
        }
        if let Some(value) = number(object, &["max_text_bytes", "maxTextBytes"]) {
            self.max_text_bytes = value;
        }
        if let Some(value) = number(
            object,
            &["delete_after_seconds", "delete_after", "deleteAfter"],
        ) {
            self.delete_after_seconds = value;
        }
        if let Some(value) = flag(object, &["allow_duplicates", "allowDuplicates"]) {
            self.allow_duplicates = value;
        }
        if let Some(value) = number(object, &["max_preview_chars", "maxEntryLength"]) {
            self.max_preview_chars = value;
        }
        if let Some(value) = string(object, &["historyFile", "db_path"]) {
            self.set_path(PathSlot::Database, value, homes);
        }
        if let Some(value) = string(object, &["tempDir", "image_dir"]) {
            self.set_path(PathSlot::Images, value, homes);
        }
        if let Some(value) = strings(object, &["excluded_apps", "excludedApps"]) {
            self.excluded_apps = value;
        }
        if let Some(value) = strings(object, &["excluded_windows", "excludedWindows"]) {
            self.excluded_windows = value;
        }
        if let Some(value) = strings(object, &["secret_patterns", "secretPatterns"]) {
            self.secret_patterns = value;
        }

        if let Some(Value::Object(paste)) = object.get("paste") {
            if let Some(value) = flag(paste, &["enabled"]) {
                self.paste_enabled = value;
            }
            if let Some(value) = number(paste, &["buffer_ms", "buffer"]) {
                self.paste_buffer_ms = value;
            }
        }
        if let Some(Value::Object(auto_paste)) =
            object.get("auto_paste").or_else(|| object.get("autoPaste"))
        {
            if let Some(value) = string(auto_paste, &["keybind"]) {
                self.paste_keybind = value.to_string();
            }
            if let Some(value) = string(auto_paste, &["terminal_keybind", "terminalKeybind"]) {
                self.terminal_paste_keybind = value.to_string();
            }
            if let Some(value) = strings(auto_paste, &["terminal_apps", "terminalApps"]) {
                self.terminal_apps = value;
            }
        }
        for section in ["history", "capture"] {
            if let Some(Value::Object(nested)) = object.get(section) {
                self.apply_json(nested, homes);
            }
        }
        if let Some(Value::Object(ui)) = object.get("ui")
            && let Some(value) = number(ui, &["max_preview_chars", "maxEntryLength"])
        {
            self.max_preview_chars = value;
        }
    }

    fn apply_toml(&mut self, text: &str, homes: &Homes) {
        let mut section = String::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), strip_comment(value.trim()));
            match section.as_str() {
                "" => {
                    self.apply_history(key, value, homes);
                    self.apply_capture(key, value);
                    match key {
                        "maxEntryLength" => {
                            self.max_preview_chars = parse_u32(value, self.max_preview_chars)
                        }
                        "tempDir" => self.set_path(PathSlot::Images, value, homes),
                        _ => {}
                    }
                }
                "history" => self.apply_history(key, value, homes),
                "paste" => match key {
                    "enabled" => self.paste_enabled = parse_bool(value, self.paste_enabled),
                    "buffer_ms" => self.paste_buffer_ms = parse_u32(value, self.paste_buffer_ms),
                    _ => {}
                },
                "auto_paste" | "autoPaste" => match key {
                    "keybind" => self.paste_keybind = strip_quotes(value).to_string(),
                    "terminal_keybind" | "terminalKeybind" => {
                        self.terminal_paste_keybind = strip_quotes(value).to_string()
                    }
                    "terminal_apps" | "terminalApps" => {
                        self.terminal_apps = parse_string_array(value)
                    }
                    _ => {}
                },
                "ui" => {
                    if key == "max_preview_chars" || key == "maxEntryLength" {
                        self.max_preview_chars = parse_u32(value, self.max_preview_chars);
                    }
                }
                "capture" => self.apply_capture(key, value),
                _ => {}
            }
        }
    }

    fn apply_history(&mut self, key: &str, value: &str, homes: &Homes) {
        match key {
            "max_entries" | "maxHistory" => self.max_entries = parse_u32(value, self.max_entries),
            "max_image_bytes" | "maxImageBytes" => {
                self.max_image_bytes = parse_u32(value, self.max_image_bytes)
            }
            "max_text_bytes" | "maxTextBytes" => {
                self.max_text_bytes = parse_u32(value, self.max_text_bytes)
            }
            "delete_after_seconds" | "delete_after" | "deleteAfter" => {
                self.delete_after_seconds = parse_u32(value, self.delete_after_seconds);
            }
            "allow_duplicates" | "allowDuplicates" => {
                self.allow_duplicates = parse_bool(value, self.allow_duplicates)
            }
            "historyFile" => self.set_path(PathSlot::Database, value, homes),
            _ => {}
        }
    }

    fn apply_capture(&mut self, key: &str, value: &str) {
        match key {
            "excluded_apps" | "excludedApps" => self.excluded_apps = parse_string_array(value),
            "excluded_windows" | "excludedWindows" => {
                self.excluded_windows = parse_string_array(value)
            }
            "secret_patterns" | "secretPatterns" => {
                self.secret_patterns = parse_string_array(value)
            }
            _ => {}
        }
    }

    fn set_path(&mut self, slot: PathSlot, value: &str, homes: &Homes) {
        let raw = strip_quotes(value).trim();
        if raw.is_empty() {
            return;
        }
        let path = resolve_path(&self.config_path, raw, homes);
        match slot {
            PathSlot::Database => self.db_path = path,
            PathSlot::Images => self.image_dir = path,
        }
    }
}

enum PathSlot {
    Database,
    Images,
}

fn number(object: &Map<String, Value>, keys: &[&str]) -> Option<u32> {
    keys.iter()
        .find_map(|key| object.get(*key)?.as_u64())
        .and_then(|value| u32::try_from(value).ok())
}

fn flag(object: &Map<String, Value>, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| object.get(*key)?.as_bool())
}

fn string<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| object.get(*key)?.as_str())
}

fn strings(object: &Map<String, Value>, keys: &[&str]) -> Option<Vec<String>> {
    let items = keys.iter().find_map(|key| object.get(*key)?.as_array())?;
    Some(
        items
            .iter()
            .filter_map(|item| item.as_str().map(String::from))
            .collect(),
    )
}

fn resolve_path(config_path: &str, raw: &str, homes: &Homes) -> String {
    if raw.starts_with('/') {
        return raw.to_string();
    }
    if raw == "~" {
        return homes.home.clone();
    }
    let prefixes = [
        ("~/", &homes.home),
        ("$HOME/", &homes.home),
        ("${HOME}/", &homes.home),
        ("$XDG_CONFIG_HOME/", &homes.config),
        ("${XDG_CONFIG_HOME}/", &homes.config),
        ("$XDG_DATA_HOME/", &homes.data),
        ("${XDG_DATA_HOME}/", &homes.data),
    ];
    for (prefix, base) in prefixes {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return format!("{base}/{rest}");
        }
    }
    let dir = Path::new(config_path)
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    dir.join(raw).to_string_lossy().into_owned()
}

fn strip_comment(value: &str) -> &str {
    match value.split_once('#') {
        Some((before, _)) => before.trim(),
        None => value,
    }
}

fn strip_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value)
}

fn parse_string_array(value: &str) -> Vec<String> {
    let Some(inner) = value
        .trim()
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return Vec::new();
    };
    inner
        .split(',')
        .map(|part| strip_quotes(part.trim()))
        .filter(|item| !item.is_empty())
        .map(String::from)
        .collect()
}

fn parse_u32(value: &str, fallback: u32) -> u32 {
    value.parse().unwrap_or(fallback)
}

fn parse_bool(value: &str, fallback: bool) -> bool {
    match value {
        "true" => true,
        "false" => false,
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn homes() -> Homes {
        Homes {
            home: "/home/tester".into(),
            config: "/home/tester/.config".into(),
            data: "/home/tester/.local/share".into(),
        }
    }

    fn applied(text: &str) -> Config {
        let mut config = Config {
            config_path: "/etc/ditox/config.toml".into(),
            ..Config::defaults("/data")
        };
        config.apply_file(text, &homes());
        config
    }

    #[test]
    fn toml_sections_and_basic_keys() {
        let config = applied(
            r##"# comment line
[history]
max_entries = 42
delete_after_seconds = 60 # trailing comment
allow_duplicates = true

[paste]
enabled = false
buffer_ms = 7

[auto_paste]
keybind = "ctrl+shift+v"
terminal_keybind = "shift+insert"
terminal_apps = ["kitty", "My Terminal"]

[ui]
max_preview_chars = 12
a_key_nobody_reads = "ignored"

[capture]
excluded_apps = ["KeePassXC", "Bitwarden"]
excluded_windows = ["secret window"]
secret_patterns = ["ghp_", "AKIA[A-Z0-9]{16}"]
"##,
        );
        assert_eq!(config.max_entries, 42);
        assert_eq!(config.delete_after_seconds, 60);
        assert!(config.allow_duplicates);
        assert!(!config.paste_enabled);
        assert_eq!(config.paste_buffer_ms, 7);
        assert_eq!(config.paste_keybind, "ctrl+shift+v");
        assert_eq!(config.terminal_paste_keybind, "shift+insert");
        assert_eq!(config.terminal_apps, ["kitty", "My Terminal"]);
        assert_eq!(config.max_preview_chars, 12);
        assert_eq!(config.excluded_apps, ["KeePassXC", "Bitwarden"]);
        assert_eq!(config.excluded_windows, ["secret window"]);
        assert_eq!(config.secret_patterns, ["ghp_", "AKIA[A-Z0-9]{16}"]);
    }

    #[test]
    fn toml_top_level_aliases() {
        let config = applied(
            "maxHistory = 7\ndeleteAfter = 30\nallowDuplicates = true\nmaxEntryLength = 55\nhistoryFile = \"history.db\"\ntempDir = \"~/clips\"\nexcludedApps = [\"Signal\"]\n",
        );
        assert_eq!(config.max_entries, 7);
        assert_eq!(config.delete_after_seconds, 30);
        assert!(config.allow_duplicates);
        assert_eq!(config.max_preview_chars, 55);
        assert_eq!(config.db_path, "/etc/ditox/history.db");
        assert_eq!(config.image_dir, "/home/tester/clips");
        assert_eq!(config.excluded_apps, ["Signal"]);
    }

    #[test]
    fn json_with_nested_sections_and_aliases() {
        let config = applied(
            r#"{
  "maxHistory": 9,
  "history": { "delete_after_seconds": 120, "max_text_bytes": 1024 },
  "paste": { "enabled": false, "buffer": 15 },
  "autoPaste": { "keybind": "shift+insert", "terminalKeybind": "ctrl+alt+v", "terminalApps": ["foot"] },
  "ui": { "maxEntryLength": 80 },
  "capture": { "excludedWindows": ["Vault"], "secret_patterns": [] },
  "historyFile": "$XDG_DATA_HOME/ditox/old.db"
}"#,
        );
        assert_eq!(config.max_entries, 9);
        assert_eq!(config.delete_after_seconds, 120);
        assert_eq!(config.max_text_bytes, 1024);
        assert!(!config.paste_enabled);
        assert_eq!(config.paste_buffer_ms, 15);
        assert_eq!(config.paste_keybind, "shift+insert");
        assert_eq!(config.terminal_paste_keybind, "ctrl+alt+v");
        assert_eq!(config.terminal_apps, ["foot"]);
        assert_eq!(config.max_preview_chars, 80);
        assert_eq!(config.excluded_windows, ["Vault"]);
        assert!(config.secret_patterns.is_empty());
        assert_eq!(config.db_path, "/home/tester/.local/share/ditox/old.db");
    }

    #[test]
    fn home_and_xdg_prefixes_resolve() {
        let homes = homes();
        let resolve = |raw: &str| resolve_path("/etc/ditox/config.toml", raw, &homes);
        assert_eq!(resolve("/abs/x.db"), "/abs/x.db");
        assert_eq!(resolve("~"), "/home/tester");
        assert_eq!(resolve("$HOME/x"), "/home/tester/x");
        assert_eq!(resolve("${HOME}/x"), "/home/tester/x");
        assert_eq!(resolve("${XDG_CONFIG_HOME}/x"), "/home/tester/.config/x");
        assert_eq!(resolve("$XDG_DATA_HOME/x"), "/home/tester/.local/share/x");
        assert_eq!(resolve("relative/x"), "/etc/ditox/relative/x");
    }

    #[test]
    fn unknown_keys_and_malformed_values_leave_the_defaults() {
        let config = applied(
            "[history]\nmax_entries = lots\nallow_duplicates = maybe\nmystery = 1\n[nowhere]\nmax_entries = 3\n",
        );
        assert_eq!(
            config,
            Config {
                config_path: "/etc/ditox/config.toml".into(),
                ..Config::defaults("/data")
            }
        );
        let defaults = Config::defaults("/data");
        assert_eq!(defaults.excluded_apps.len(), 7);
        assert!(
            defaults
                .secret_patterns
                .iter()
                .any(|pattern| pattern == "AKIA[A-Z0-9]{16}")
        );
    }

    #[test]
    fn loading_reads_the_environment_and_creates_the_directories() {
        let root = std::env::temp_dir().join(format!("ditox-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("config/ditox")).unwrap();
        std::fs::write(
            root.join("config/ditox/config.toml"),
            "[history]\nmax_entries = 5\n",
        )
        .unwrap();
        let (config_home, data_home) = (root.join("config"), root.join("data"));
        let config = Config::load_with(&|name| match name {
            "HOME" => Some(root.to_string_lossy().into_owned()),
            "XDG_CONFIG_HOME" => Some(config_home.to_string_lossy().into_owned()),
            "XDG_DATA_HOME" => Some(data_home.to_string_lossy().into_owned()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.max_entries, 5);
        assert_eq!(
            config.db_path,
            format!("{}/ditox/ditox-v2.db", data_home.display())
        );
        assert!(Path::new(&config.image_dir).is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }
}
