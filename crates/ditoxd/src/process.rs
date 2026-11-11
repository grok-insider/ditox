use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

pub fn run(
    program: &str,
    args: &[&str],
    stdout_limit: usize,
    timeout: Duration,
) -> Result<Output, String> {
    let deadline = Instant::now() + timeout;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("{program}: {err}"))?;

    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.take(stdout_limit as u64 + 1).read_to_end(&mut bytes);
            let _ = stdout_tx.send(bytes);
        });
    }
    if let Some(mut stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut head = Vec::new();
            let _ = (&mut stderr).take(4096).read_to_end(&mut head);
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            let _ = stderr_tx.send(head);
        });
    }

    let late = || format!("{program}: no answer in {} s", timeout.as_secs());
    let stdout = match stdout_rx.recv_timeout(remaining(deadline)) {
        Ok(bytes) if bytes.len() <= stdout_limit => bytes,
        Ok(_) => {
            stop(&mut child);
            return Err(format!("{program}: more than {stdout_limit} bytes"));
        }
        Err(_) => {
            stop(&mut child);
            return Err(late());
        }
    };
    let Ok(stderr) = stderr_rx.recv_timeout(remaining(deadline)) else {
        stop(&mut child);
        return Err(late());
    };
    let Some(status) = wait_until(&mut child, deadline) else {
        stop(&mut child);
        return Err(late());
    };
    Ok(Output {
        code: status.code(),
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The child's exit status, or `None` when it is still running at the deadline.
pub fn wait_until(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_captured_capped_and_timed() {
        let ok = run(
            "sh",
            &["-c", "printf hello; printf oops >&2; exit 3"],
            64,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(
            (ok.code, ok.stdout.as_slice(), ok.stderr.as_str()),
            (Some(3), &b"hello"[..], "oops")
        );
        assert!(!ok.success());

        let large = run(
            "sh",
            &["-c", "head -c 4096 /dev/zero"],
            16,
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(large.contains("more than 16 bytes"));

        let slow = run("sh", &["-c", "sleep 5"], 16, Duration::from_millis(100)).unwrap_err();
        assert!(slow.contains("no answer"));

        assert!(run("ditox-no-such-program", &[], 16, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn a_child_that_closes_its_output_and_lingers_is_stopped_at_the_deadline() {
        let started = Instant::now();
        let lingering = run(
            "sh",
            &["-c", "printf hi; exec 1>&-; sleep 5"],
            16,
            Duration::from_millis(200),
        )
        .unwrap_err();
        assert!(lingering.contains("no answer"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
