//! The settings the options window edits, and the part of them the clip
//! window reads from the config file before it draws.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dirs;

pub const MIN_LINES: u32 = 1;
pub const MAX_LINES: u32 = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    pub fn parse(value: &str) -> Option<Theme> {
        match value {
            "system" => Some(Theme::System),
            "dark" => Some(Theme::Dark),
            "light" => Some(Theme::Light),
            _ => None,
        }
    }

    /// Dark or light, where the theme says; `None` follows the desktop.
    pub fn dark(self) -> Option<bool> {
        match self {
            Theme::System => None,
            Theme::Dark => Some(true),
            Theme::Light => Some(false),
        }
    }
}

/// How the clip window looks and behaves: the `[ui]` section of the config.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ui {
    pub theme: Theme,
    /// Lines of text each clip gets in the list.
    pub lines: u32,
    /// The clip window closes when another window takes the keyboard.
    pub close_on_blur: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            theme: Theme::System,
            lines: 2,
            close_on_blur: true,
        }
    }
}

impl Ui {
    /// What the config file says, or the defaults where it says nothing.
    pub fn load() -> Ui {
        config_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Ui::parse(&text))
            .unwrap_or_default()
    }

    /// The `[ui]` section of a TOML config, or the `ui` object of a JSON one.
    pub fn parse(text: &str) -> Ui {
        let mut ui = Ui::default();
        if text.trim_start().starts_with('{') {
            if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text)
                && let Some(Value::Object(section)) = object.get("ui")
            {
                for (key, value) in section {
                    match value {
                        Value::String(value) => ui.set(key, value),
                        other => ui.set(key, &other.to_string()),
                    }
                }
            }
            return ui;
        }
        let mut inside = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') && line.ends_with(']') {
                inside = line[1..line.len() - 1].trim() == "ui";
                continue;
            }
            if !inside || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                ui.set(key.trim(), scalar(value));
            }
        }
        ui
    }

    /// Takes one setting by its name in the file. Anything else is left alone.
    pub fn set(&mut self, key: &str, value: &str) {
        match key {
            "theme" => {
                if let Some(theme) = Theme::parse(value) {
                    self.theme = theme;
                }
            }
            "lines" => {
                if let Ok(lines) = value.parse::<u32>() {
                    self.lines = lines.clamp(MIN_LINES, MAX_LINES);
                }
            }
            "close_on_blur" | "closeOnBlur" => match value {
                "true" => self.close_on_blur = true,
                "false" => self.close_on_blur = false,
                _ => {}
            },
            _ => {}
        }
    }
}

/// A TOML value as written: without its comment and its quotes.
fn scalar(value: &str) -> &str {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('"') {
        return rest.split('"').next().unwrap_or(rest);
    }
    value.split('#').next().unwrap_or(value).trim()
}

/// `DITOX_CONFIG`, else `ditox/config.toml` under `XDG_CONFIG_HOME`,
/// `~/.config`, or `%APPDATA%` on Windows.
pub fn config_path() -> Option<PathBuf> {
    if let Some(path) = dirs::process_env("DITOX_CONFIG") {
        return Some(PathBuf::from(path));
    }
    Some(
        dirs::config_home(&dirs::process_env)?
            .join("ditox")
            .join("config.toml"),
    )
}

/// Everything the options window can change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub max_entries: u32,
    /// 0 keeps clips until the history is full.
    pub delete_after_seconds: u32,
    pub allow_duplicates: bool,
    pub paste_enabled: bool,
    pub paste_buffer_ms: u32,
    pub paste_keybind: String,
    pub terminal_paste_keybind: String,
    pub terminal_apps: Vec<String>,
    /// The key the daemon listens for to open the list, where it is the
    /// daemon that listens: on Windows. Empty for none.
    #[serde(default)]
    pub open_keybind: String,
    pub excluded_apps: Vec<String>,
    pub excluded_windows: Vec<String>,
    pub secret_patterns: Vec<String>,
    pub ui: Ui,
}

/// The settings as the daemon holds them, and where they are kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigView {
    pub settings: Settings,
    pub path: String,
    /// The daemon can write the file. A file put there by a system
    /// configuration (a link into a read-only store) cannot be changed.
    pub writable: bool,
    pub database: String,
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ui_section_is_read_from_toml_and_json() {
        let toml = "[history]\nlines = 9\n\n[ui]\ntheme = \"light\" # bright\nlines = 4\nclose_on_blur = false\n\n[paste]\ntheme = \"dark\"\n";
        assert_eq!(
            Ui::parse(toml),
            Ui {
                theme: Theme::Light,
                lines: 4,
                close_on_blur: false
            }
        );
        let json = r#"{"ui": {"theme": "dark", "lines": 3, "closeOnBlur": false}}"#;
        assert_eq!(
            Ui::parse(json),
            Ui {
                theme: Theme::Dark,
                lines: 3,
                close_on_blur: false
            }
        );
    }

    #[test]
    fn what_the_file_does_not_say_or_says_wrong_stays_the_default() {
        assert_eq!(Ui::parse(""), Ui::default());
        assert_eq!(Ui::parse("{not json"), Ui::default());
        let odd = "[ui]\ntheme = \"purple\"\nlines = many\nclose_on_blur = maybe\n";
        assert_eq!(Ui::parse(odd), Ui::default());
        assert_eq!(Ui::parse("[ui]\nlines = 40\n").lines, MAX_LINES);
        assert_eq!(Ui::parse("[ui]\nlines = 0\n").lines, MIN_LINES);
    }
}
