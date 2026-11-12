//! The clipboard as the daemon sees it: a list of MIME types and the bytes
//! behind each. `wl-clipboard` answers on Linux, Windows itself on Windows,
//! and a file when `DITOX_CLIPBOARD_MOCK` names one.

use crate::error::Result;
use crate::policy;

#[cfg(not(windows))]
mod wl;
#[cfg(not(windows))]
use wl as system;

#[cfg(windows)]
use crate::windows::clipboard as system;

/// What `watcher.status` calls the way the clipboard is read.
pub const BACKEND: &str = if cfg!(windows) {
    "windows"
} else {
    "wl-clipboard"
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardRead {
    Bytes(Vec<u8>),
    Empty,
    TypeUnavailable,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MimeList {
    Types(Vec<String>),
    Empty,
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    mock: Option<String>,
}

impl Clipboard {
    pub fn from_env() -> Self {
        Clipboard {
            mock: std::env::var("DITOX_CLIPBOARD_MOCK")
                .ok()
                .filter(|path| !path.is_empty()),
        }
    }

    pub fn mock(path: &str) -> Self {
        Clipboard {
            mock: Some(path.to_string()),
        }
    }

    pub fn is_mock(&self) -> bool {
        self.mock.is_some()
    }

    pub fn list_mime_types(&self) -> MimeList {
        if let Some(path) = &self.mock {
            return match std::fs::read(path) {
                Ok(bytes) if !bytes.is_empty() => MimeList::Types(vec![mock_mime(path)]),
                _ => MimeList::Empty,
            };
        }
        system::list()
    }

    pub fn read_offer(&self, mime: &str) -> ClipboardRead {
        if let Some(path) = &self.mock {
            if policy::mime_base(mime) != mock_mime(path) {
                return ClipboardRead::TypeUnavailable;
            }
            return match std::fs::read(path) {
                Ok(bytes) if !bytes.is_empty() => ClipboardRead::Bytes(bytes),
                _ => ClipboardRead::Empty,
            };
        }
        system::read(mime)
    }

    pub fn write_text(&self, text: &str) -> Result<()> {
        self.write(text.as_bytes(), None)
    }

    pub fn write_bytes(&self, bytes: &[u8], mime: &str) -> Result<()> {
        self.write(bytes, Some(mime))
    }

    fn write(&self, bytes: &[u8], mime: Option<&str>) -> Result<()> {
        if let Some(path) = &self.mock {
            if let Some(parent) = std::path::Path::new(path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, bytes)?;
            let sidecar = format!("{path}.mime");
            match mime {
                Some(mime) => std::fs::write(sidecar, mime)?,
                None => {
                    let _ = std::fs::remove_file(sidecar);
                }
            }
            return Ok(());
        }
        system::write(bytes, mime)
    }
}

fn mock_mime(path: &str) -> String {
    std::fs::read_to_string(format!("{path}.mime"))
        .ok()
        .map(|mime| policy::mime_base(&mime).to_string())
        .filter(|mime| !mime.is_empty())
        .unwrap_or_else(|| "text/plain".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn the_mock_clipboard_keeps_bytes_and_their_type() {
        let dir = TempDir::new("clipboard");
        let clipboard = Clipboard::mock(&dir.path("nested/clip"));
        assert_eq!(clipboard.list_mime_types(), MimeList::Empty);
        assert_eq!(clipboard.read_offer("text/plain"), ClipboardRead::Empty);

        clipboard.write_text("hola").unwrap();
        assert_eq!(
            clipboard.list_mime_types(),
            MimeList::Types(vec!["text/plain".into()])
        );
        assert_eq!(
            clipboard.read_offer("text/plain;charset=utf-8"),
            ClipboardRead::Bytes(b"hola".to_vec())
        );
        assert_eq!(
            clipboard.read_offer("image/png"),
            ClipboardRead::TypeUnavailable
        );

        clipboard.write_bytes(b"png", "image/png").unwrap();
        assert_eq!(
            clipboard.list_mime_types(),
            MimeList::Types(vec!["image/png".into()])
        );
        assert_eq!(
            clipboard.read_offer("image/png"),
            ClipboardRead::Bytes(b"png".to_vec())
        );
        clipboard.write_text("text again").unwrap();
        assert_eq!(
            clipboard.list_mime_types(),
            MimeList::Types(vec!["text/plain".into()])
        );
    }
}
