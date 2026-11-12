//! Where a user's files go on this system.
//!
//! The `XDG_*` variables win wherever they are set. Without them Linux uses
//! the directories under `$HOME` the XDG specification names, and Windows the
//! roaming and local application data folders.

use std::path::PathBuf;

/// Reads one variable of the environment; empty counts as unset.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The process's own environment.
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The user's home directory.
pub fn home(env: Env) -> Option<PathBuf> {
    env("HOME")
        .or_else(|| env("USERPROFILE").filter(|_| cfg!(windows)))
        .map(PathBuf::from)
}

/// Settings: `XDG_CONFIG_HOME`, `~/.config`, or `%APPDATA%`.
pub fn config_home(env: Env) -> Option<PathBuf> {
    env("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| platform(env, "APPDATA", ".config"))
}

/// What the program keeps: `XDG_DATA_HOME`, `~/.local/share`, or `%LOCALAPPDATA%`.
pub fn data_home(env: Env) -> Option<PathBuf> {
    env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| platform(env, "LOCALAPPDATA", ".local/share"))
}

/// What the program remembers between runs: `XDG_STATE_HOME`,
/// `~/.local/state`, or `%LOCALAPPDATA%`.
pub fn state_home(env: Env) -> Option<PathBuf> {
    env("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| platform(env, "LOCALAPPDATA", ".local/state"))
}

/// Where the socket lives: `XDG_RUNTIME_DIR`, or on Windows `%LOCALAPPDATA%`,
/// which only its user can write. Linux has no fallback to a shared directory.
pub fn runtime_dir(env: Env) -> Option<PathBuf> {
    env("XDG_RUNTIME_DIR")
        .or_else(|| env("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(PathBuf::from)
}

/// The variable whose absence leaves the daemon without a socket path.
pub const RUNTIME_VARIABLE: &str = if cfg!(windows) {
    "LOCALAPPDATA"
} else {
    "XDG_RUNTIME_DIR"
};

fn platform(env: Env, windows: &str, under_home: &str) -> Option<PathBuf> {
    if cfg!(windows) {
        env(windows).map(PathBuf::from)
    } else {
        home(env).map(|home| home.join(under_home))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn the_xdg_variables_win_on_every_system() {
        let set = env(&[
            ("HOME", "/home/u"),
            ("APPDATA", "C:\\Users\\u\\AppData\\Roaming"),
            ("LOCALAPPDATA", "C:\\Users\\u\\AppData\\Local"),
            ("XDG_CONFIG_HOME", "/c"),
            ("XDG_DATA_HOME", "/d"),
            ("XDG_STATE_HOME", "/s"),
            ("XDG_RUNTIME_DIR", "/r"),
        ]);
        assert_eq!(config_home(&set), Some(PathBuf::from("/c")));
        assert_eq!(data_home(&set), Some(PathBuf::from("/d")));
        assert_eq!(state_home(&set), Some(PathBuf::from("/s")));
        assert_eq!(runtime_dir(&set), Some(PathBuf::from("/r")));
    }

    #[cfg(not(windows))]
    #[test]
    fn without_them_the_directories_are_the_ones_under_home() {
        let set = env(&[("HOME", "/home/u"), ("LOCALAPPDATA", "C:\\nowhere")]);
        assert_eq!(config_home(&set), Some(PathBuf::from("/home/u/.config")));
        assert_eq!(data_home(&set), Some(PathBuf::from("/home/u/.local/share")));
        assert_eq!(
            state_home(&set),
            Some(PathBuf::from("/home/u/.local/state"))
        );
        assert_eq!(runtime_dir(&set), None, "no shared directory stands in");
        assert_eq!(config_home(&env(&[])), None);
    }

    #[cfg(windows)]
    #[test]
    fn without_them_the_directories_are_the_application_data_folders() {
        const ROAMING: &str = "C:\\Users\\u\\AppData\\Roaming";
        const LOCAL: &str = "C:\\Users\\u\\AppData\\Local";
        let set = env(&[
            ("USERPROFILE", "C:\\Users\\u"),
            ("APPDATA", ROAMING),
            ("LOCALAPPDATA", LOCAL),
        ]);
        let (roaming, local) = (ROAMING, LOCAL);
        assert_eq!(config_home(&set), Some(PathBuf::from(roaming)));
        assert_eq!(data_home(&set), Some(PathBuf::from(local)));
        assert_eq!(state_home(&set), Some(PathBuf::from(local)));
        assert_eq!(runtime_dir(&set), Some(PathBuf::from(local)));
        assert_eq!(home(&set), Some(PathBuf::from("C:\\Users\\u")));
    }
}
