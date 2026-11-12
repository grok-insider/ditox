#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
#[cfg(unix)]
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use ditox_rpc::frame::{FrameReader, write_frame};
use ditox_rpc::sock::{UnixListener, UnixStream};

use crate::app::App;
use crate::config::Config;
use crate::error::{Error, Result};
#[cfg(unix)]
use crate::policy;
use crate::rpc;

const IDLE_TICK: Duration = Duration::from_secs(1);
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// How long after the last change the clipboard is read. A Windows program
/// fills the clipboard in several steps and has to open it for each; read in
/// between, it fails to finish the copy, so it is given longer there.
const CAPTURE_SETTLE: Duration = Duration::from_millis(if cfg!(windows) { 150 } else { 40 });
const CAPTURE_DEADLINE: Duration = Duration::from_millis(if cfg!(windows) { 600 } else { 250 });

pub struct Request {
    pub body: Vec<u8>,
    pub reply: Sender<Vec<u8>>,
}

/// `XDG_RUNTIME_DIR` must be the user's own and closed to everyone else, or
/// somebody else could swap the socket directory under the daemon.
#[cfg(unix)]
pub fn check_runtime_dir(dir: &Path) -> Result<()> {
    let meta =
        std::fs::metadata(dir).map_err(|err| Error::Io(format!("{}: {err}", dir.display())))?;
    let user = unsafe { libc::getuid() };
    if !policy::runtime_dir_acceptable(meta.uid(), meta.mode(), user) {
        return Err(Error::Io(format!(
            "{} belongs to someone else or is writable by others",
            dir.display()
        )));
    }
    Ok(())
}

/// The directory of the socket, made for its user alone when it is missing.
#[cfg(unix)]
fn private_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|err| Error::Io(format!("{}: {err}", dir.display())))?;
    }
    check_runtime_dir(dir)
}

/// The directory of the socket. On Windows it lies under the user's local
/// application data, which other users cannot write: a directory made there
/// inherits that.
#[cfg(windows)]
fn private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|err| Error::Io(format!("{}: {err}", dir.display())))
}

#[cfg(unix)]
fn private_socket(path: &Path) -> Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(policy::SOCKET_MODE))?;
    Ok(())
}

#[cfg(windows)]
fn private_socket(_: &Path) -> Result<()> {
    Ok(())
}

pub fn listen(path: &Path) -> Result<UnixListener> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| Error::Io("the socket path has no directory".into()))?;
    private_dir(dir)?;
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(Error::Io(format!(
                "another ditoxd already listens on {}",
                path.display()
            )));
        }
        std::fs::remove_file(path)
            .map_err(|err| Error::Io(format!("{}: {err}", path.display())))?;
    }
    let listener =
        UnixListener::bind(path).map_err(|err| Error::Io(format!("{}: {err}", path.display())))?;
    private_socket(path)?;
    Ok(listener)
}

pub fn accept(listener: UnixListener, requests: Sender<Request>) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let requests = requests.clone();
                    std::thread::spawn(move || serve_connection(stream, requests));
                }
                Err(err) => {
                    eprintln!("ditoxd: accept: {err}");
                    std::thread::sleep(ACCEPT_BACKOFF);
                }
            }
        }
    });
}

fn serve_connection(mut stream: UnixStream, requests: Sender<Request>) {
    let mut reader = FrameReader::new();
    while let Ok(Some(body)) = reader.read(&mut stream) {
        let (reply, answer) = mpsc::channel();
        if requests.send(Request { body, reply }).is_err() {
            return;
        }
        let Ok(response) = answer.recv() else { return };
        if write_frame(&mut stream, &response).is_err() {
            return;
        }
    }
}

/// What tells the daemon that the clipboard changed.
pub trait Watcher {
    /// Starts it when it is not running. A failure is told once, the first
    /// time, and `Err(None)` for as long as it lasts.
    fn ensure_running(&mut self) -> std::result::Result<(), Option<String>>;

    /// Shown the settings every tick, for a watcher that lives by some of
    /// them. Returns a problem worth telling the user.
    fn follow(&mut self, _: &Config) -> Option<String> {
        None
    }
}

/// `wl-paste --watch`, which runs `ditoxd ingest wake` on every change.
#[cfg(unix)]
pub struct Watch {
    exe: String,
    child: Option<Child>,
    failing: bool,
}

#[cfg(unix)]
impl Watch {
    pub fn new(exe: String) -> Self {
        Watch {
            exe,
            child: None,
            failing: false,
        }
    }

    pub fn args(exe: &str) -> [&str; 4] {
        ["--watch", exe, "ingest", "wake"]
    }

    fn spawn(&self) -> std::io::Result<Child> {
        let mut command = Command::new("wl-paste");
        command
            .args(Self::args(&self.exe))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        command.spawn()
    }
}

#[cfg(unix)]
impl Watcher for Watch {
    fn ensure_running(&mut self) -> std::result::Result<(), Option<String>> {
        let exited = match &mut self.child {
            Some(child) => match child.try_wait() {
                Ok(None) => {
                    self.failing = false;
                    return Ok(());
                }
                Ok(Some(status)) => format!("wl-paste --watch stopped ({status})"),
                Err(err) => format!("wl-paste --watch: {err}"),
            },
            None => String::new(),
        };
        let problem = match self.spawn() {
            Ok(child) => {
                self.child = Some(child);
                if exited.is_empty() {
                    return Ok(());
                }
                exited
            }
            Err(err) => {
                self.child = None;
                format!("wl-paste: {err}")
            }
        };
        let first = !std::mem::replace(&mut self.failing, true);
        Err(first.then_some(problem))
    }
}

#[cfg(unix)]
impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A copy often sets the clipboard more than once. The capture waits until
/// the wakes stop, and never longer than the deadline after the first one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Settle {
    first: Instant,
    due: Instant,
}

impl Settle {
    fn after_wake(pending: Option<Settle>, now: Instant) -> Settle {
        let first = pending.map_or(now, |pending| pending.first);
        Settle {
            first,
            due: (now + CAPTURE_SETTLE).min(first + CAPTURE_DEADLINE),
        }
    }
}

pub fn run(mut app: App, requests: Receiver<Request>, mut watch: Option<Box<dyn Watcher>>) {
    let mut settle: Option<Settle> = None;
    let mut next_tick = Instant::now();
    loop {
        if Instant::now() >= next_tick {
            next_tick = Instant::now() + IDLE_TICK;
            if let Some(watch) = &mut watch {
                match watch.ensure_running() {
                    Ok(()) => app.watch_running = true,
                    Err(problem) => {
                        app.watch_running = false;
                        if let Some(problem) = problem {
                            eprintln!("ditoxd: {problem}");
                            let _ = app.store.set_watcher_error(&problem);
                        }
                    }
                }
                if let Some(problem) = watch.follow(&app.cfg) {
                    eprintln!("ditoxd: {problem}");
                    let _ = app.store.set_watcher_error(&problem);
                }
            }
        }

        let wake_at = settle.map_or(next_tick, |settle| settle.due.min(next_tick));
        match requests.recv_timeout(wake_at.saturating_duration_since(Instant::now())) {
            Ok(request) => {
                let response = rpc::handle(&mut app, &request.body);
                let _ = request.reply.send(response);
                if app.take_capture_request() {
                    settle = Some(Settle::after_wake(settle, Instant::now()));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if settle.is_some_and(|settle| Instant::now() >= settle.due) {
            settle = None;
            app.capture();
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use ditox_rpc::{Client, Filter};

    use super::*;
    use crate::app::NoDesktop;
    use crate::clipboard::Clipboard;
    use crate::config::Config;
    use crate::testing::TempDir;

    #[cfg(unix)]
    #[test]
    fn the_watch_asks_ingest_to_wake() {
        assert_eq!(
            Watch::args("/bin/ditoxd"),
            ["--watch", "/bin/ditoxd", "ingest", "wake"]
        );
    }

    #[test]
    fn a_capture_waits_for_the_wakes_to_stop_but_not_past_the_deadline() {
        let start = Instant::now();
        let first = Settle::after_wake(None, start);
        assert_eq!(
            first,
            Settle {
                first: start,
                due: start + CAPTURE_SETTLE
            }
        );

        let second = Settle::after_wake(Some(first), start + Duration::from_millis(30));
        assert_eq!(
            second.due,
            start + Duration::from_millis(30) + CAPTURE_SETTLE
        );

        let mut pending = second;
        for step in 2..20 {
            pending = Settle::after_wake(Some(pending), start + Duration::from_millis(30 * step));
        }
        assert_eq!(
            pending,
            Settle {
                first: start,
                due: start + CAPTURE_DEADLINE
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_socket_is_private_and_a_writable_directory_is_refused() {
        let dir = TempDir::new("socket");
        let path = dir.path("run/rpc.sock");
        let listener = listen(Path::new(&path)).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(dir.path("run"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        drop(listener);
        assert!(listen(Path::new(&path)).is_ok());

        let open = dir.path("open");
        std::fs::create_dir_all(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(listen(Path::new(&format!("{open}/rpc.sock"))).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_runtime_directory_others_can_write_is_refused() {
        let dir = TempDir::new("runtime");
        let private = dir.path("private");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(check_runtime_dir(Path::new(&private)).is_ok());
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(check_runtime_dir(Path::new(&private)).is_err());
        assert!(check_runtime_dir(Path::new(&dir.path("missing"))).is_err());
    }

    #[test]
    fn a_second_daemon_does_not_take_a_live_socket() {
        let dir = TempDir::new("live");
        let path = dir.path("run/rpc.sock");
        let _listener = listen(Path::new(&path)).unwrap();
        assert!(
            listen(Path::new(&path))
                .unwrap_err()
                .to_string()
                .contains("already listens")
        );
    }

    #[test]
    fn clients_are_served_in_turn_and_a_wake_captures_the_clipboard() {
        let dir = TempDir::new("serve");
        let (data, clip, path) = (
            dir.path("data"),
            dir.path("clipboard"),
            dir.path("run/rpc.sock"),
        );
        let listener = listen(Path::new(&path)).unwrap();
        let (requests, queue) = mpsc::channel();
        accept(listener, requests);
        std::thread::spawn(move || {
            let cfg = Config::defaults(&data);
            std::fs::create_dir_all(&cfg.image_dir).unwrap();
            let clipboard = Clipboard::mock(&clip);
            clipboard.write_text("copied while running").unwrap();
            run(
                App::open(cfg, clipboard, Box::new(NoDesktop)).unwrap(),
                queue,
                None,
            );
        });

        let mut first = Client::connect_to(Path::new(&path)).unwrap();
        let mut second = Client::connect_to(Path::new(&path)).unwrap();
        assert!(first.pulse().is_ok());
        let before = second.pulse().unwrap().revision;
        drop(first);

        second.wake(false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while second.pulse().unwrap().revision == before {
            assert!(Instant::now() < deadline, "the wake was never captured");
            std::thread::sleep(Duration::from_millis(20));
        }
        let listed = second.list("", Filter::All, 10, true).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].content, "copied while running");
        assert_eq!(
            second.get(listed[0].id).unwrap().content,
            "copied while running"
        );
    }
}
