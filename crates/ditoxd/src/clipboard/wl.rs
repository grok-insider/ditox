//! The Wayland clipboard, through `wl-paste` and `wl-copy`.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{ClipboardRead, MimeList};
use crate::error::{Error, Result};
use crate::policy::{self, MAX_IMAGE_READ_BYTES, MAX_TEXT_READ_BYTES};
use crate::process::{self, Output};

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn list() -> MimeList {
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

pub fn read(mime: &str) -> ClipboardRead {
    let text = policy::mime_base(mime).starts_with("text/");
    let args = read_args(mime);
    let limit = if text {
        MAX_TEXT_READ_BYTES
    } else {
        MAX_IMAGE_READ_BYTES
    };
    classify(process::run("wl-paste", &args, limit, READ_TIMEOUT))
}

pub fn write(bytes: &[u8], mime: Option<&str>) -> Result<()> {
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

/// How `wl-paste` is asked for one type. Without `--no-newline` it adds a
/// line break to every text, which would be stored and pasted back.
fn read_args(mime: &str) -> Vec<&str> {
    if policy::mime_base(mime).starts_with("text/") {
        vec!["--no-newline", "--type", mime]
    } else {
        vec!["--type", mime]
    }
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
}
