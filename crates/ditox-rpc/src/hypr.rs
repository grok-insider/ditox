//! Hyprland's request socket, spoken directly. `hyprctl` does the same and
//! costs a process each time: tens of milliseconds before a window opens or
//! a paste lands.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const REPLY_LIMIT: usize = 1024 * 1024;

/// Where the running Hyprland answers requests, if this session has one.
pub fn socket_path() -> Option<PathBuf> {
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").filter(|s| !s.is_empty())?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|s| !s.is_empty())?;
    let path = Path::new(&runtime)
        .join("hypr")
        .join(signature)
        .join(".socket.sock");
    path.exists().then_some(path)
}

/// One request to the running Hyprland, as `hyprctl` words it on the wire:
/// `j/activewindow`, `/dispatch <expression>`.
pub fn request(command: &str, timeout: Duration) -> io::Result<Vec<u8>> {
    let path = socket_path().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    request_at(&path, command, timeout)
}

pub fn request_at(path: &Path, command: &str, timeout: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut stream = UnixStream::connect(path)?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(command.as_bytes())?;
    let mut reply = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        stream.set_read_timeout(Some(left))?;
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(reply),
            Ok(read) => {
                reply.extend_from_slice(&chunk[..read]);
                if reply.len() > REPLY_LIMIT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "reply is too large",
                    ));
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                return Err(io::ErrorKind::TimedOut.into());
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn socket(label: &str) -> (PathBuf, UnixListener) {
        let dir = std::env::temp_dir().join(format!("ditox-hypr-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".socket.sock");
        let listener = UnixListener::bind(&path).unwrap();
        (path, listener)
    }

    #[test]
    fn a_request_is_written_once_and_the_reply_read_to_its_end() {
        let (path, listener) = socket("reply");
        let server = std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let mut asked = [0u8; 64];
            let read = peer.read(&mut asked).unwrap();
            peer.write_all(b"{\"address\":").unwrap();
            peer.write_all(b"\"0xabc\"}").unwrap();
            String::from_utf8_lossy(&asked[..read]).into_owned()
        });
        let reply = request_at(&path, "j/activewindow", Duration::from_secs(2)).unwrap();
        assert_eq!(reply, b"{\"address\":\"0xabc\"}");
        assert_eq!(server.join().unwrap(), "j/activewindow");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_compositor_that_never_answers_is_given_up_on_at_the_deadline() {
        let (path, listener) = socket("silent");
        let server = std::thread::spawn(move || {
            let (peer, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(600));
            drop(peer);
        });
        let started = Instant::now();
        let err = request_at(&path, "j/version", Duration::from_millis(150)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(500));
        server.join().unwrap();
        assert!(
            request_at(
                &path.with_file_name("gone.sock"),
                "j/version",
                Duration::from_secs(1)
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
