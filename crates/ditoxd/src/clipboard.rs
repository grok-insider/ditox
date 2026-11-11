use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::policy::{self, MAX_IMAGE_READ_BYTES, MAX_TEXT_READ_BYTES};
use crate::process::{self, Output};

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

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
        match classify(process::run(
            "wl-paste",
            &["--list-types"],
            1024 * 1024,
            READ_TIMEOUT,
        )) {
            ClipboardRead::Bytes(stdout) => {
                let types: Vec<String> = String::from_utf8_lossy(&stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(String::from)
                    .collect();
                if types.is_empty() {
                    MimeList::Empty
                } else {
                    MimeList::Types(types)
                }
            }
            ClipboardRead::Empty | ClipboardRead::TypeUnavailable => MimeList::Empty,
            ClipboardRead::Failed(detail) => MimeList::Failed(detail),
        }
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
        let text = policy::mime_base(mime).starts_with("text/");
        let args = read_args(mime);
        let limit = if text {
            MAX_TEXT_READ_BYTES
        } else {
            MAX_IMAGE_READ_BYTES
        };
        classify(process::run("wl-paste", &args, limit, READ_TIMEOUT))
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

        let mut command = Command::new("wl-copy");
        if let Some(mime) = mime {
            command.args(["--type", mime]);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| Error::Clipboard(format!("wl-copy: {err}")))?;
        let stdin = child.stdin.take();
        let deadline = Instant::now() + WRITE_TIMEOUT;
        let (written, status) = std::thread::scope(|scope| {
            let writer = scope.spawn(move || stdin.map(|mut stdin| stdin.write_all(bytes)));
            let status = process::wait_until(&mut child, deadline);
            if status.is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
            (writer.join().ok().flatten(), status)
        });
        match (written, status) {
            (Some(Ok(())), Some(status)) if status.success() => Ok(()),
            (_, None) => Err(Error::Clipboard(format!(
                "wl-copy did not finish in {} s",
                WRITE_TIMEOUT.as_secs()
            ))),
            (Some(Err(err)), _) => Err(Error::Clipboard(format!("wl-copy: {err}"))),
            (_, Some(status)) => Err(Error::Clipboard(format!(
                "wl-copy exited with {}",
                status
                    .code()
                    .map_or("a signal".into(), |code| code.to_string())
            ))),
        }
    }
}

/// How `wl-paste` is asked for one type. Without `--no-newline` it adds a
/// line break to every text, which would be stored and pasted back.
fn read_args(mime: &str) -> Vec<&str> {
    if policy::mime_base(mime).starts_with("text/") {
        vec!["--no-newline", "--type", mime]
    } else {
        vec!["--type", mime]
    }
}

fn mock_mime(path: &str) -> String {
    std::fs::read_to_string(format!("{path}.mime"))
        .ok()
        .map(|mime| policy::mime_base(&mime).to_string())
        .filter(|mime| !mime.is_empty())
        .unwrap_or_else(|| "text/plain".into())
}

fn classify(run: std::result::Result<Output, String>) -> ClipboardRead {
    let output = match run {
        Ok(output) => output,
        Err(detail) => return ClipboardRead::Failed(detail),
    };
    if output.success() {
        return if output.stdout.is_empty() {
            ClipboardRead::Empty
        } else {
            ClipboardRead::Bytes(output.stdout)
        };
    }
    if output.stderr.contains("Nothing is copied") {
        return ClipboardRead::Empty;
    }
    if output.stderr.contains("not available as requested type") {
        return ClipboardRead::TypeUnavailable;
    }
    ClipboardRead::Failed(failure_detail(output.code, &output.stderr))
}

fn failure_detail(code: Option<i32>, stderr: &str) -> String {
    match stderr.trim().lines().next().filter(|line| !line.is_empty()) {
        Some(line) => line.to_string(),
        None => code.map_or_else(
            || "wl-paste did not exit".to_string(),
            |code| format!("exit {code}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn failed(code: i32, stderr: &str) -> ClipboardRead {
        classify(Ok(Output {
            code: Some(code),
            stdout: Vec::new(),
            stderr: stderr.into(),
        }))
    }

    #[test]
    fn an_empty_clipboard_and_a_refused_type_are_not_failures() {
        assert_eq!(failed(1, "Nothing is copied\n"), ClipboardRead::Empty);
        assert_eq!(
            failed(
                1,
                "Clipboard content is not available as requested type \"image/png\"\n"
            ),
            ClipboardRead::TypeUnavailable
        );
        let compositor = "The compositor does not seem to implement zwlr_data_control_manager_v1\n";
        assert_eq!(
            failed(1, compositor),
            ClipboardRead::Failed(compositor.trim().into())
        );
        assert_eq!(failed(1, ""), ClipboardRead::Failed("exit 1".into()));
        assert_eq!(
            classify(Err("wl-paste: not found".into())),
            ClipboardRead::Failed("wl-paste: not found".into())
        );
        assert_eq!(
            classify(Ok(Output {
                code: Some(0),
                stdout: Vec::new(),
                stderr: String::new()
            })),
            ClipboardRead::Empty
        );
    }

    #[test]
    fn text_is_read_as_it_was_copied_and_an_image_by_its_type() {
        assert_eq!(
            read_args("text/plain;charset=utf-8"),
            ["--no-newline", "--type", "text/plain;charset=utf-8"]
        );
        assert_eq!(
            read_args("text/html"),
            ["--no-newline", "--type", "text/html"]
        );
        assert_eq!(read_args("image/png"), ["--type", "image/png"]);
    }

    #[test]
    fn the_failure_detail_is_the_first_stderr_line_or_the_exit_code() {
        assert_eq!(
            failure_detail(Some(1), "Clipboard exploded\nsecond line\n"),
            "Clipboard exploded"
        );
        assert_eq!(failure_detail(Some(1), " \n"), "exit 1");
        assert_eq!(failure_detail(None, ""), "wl-paste did not exit");
    }

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
