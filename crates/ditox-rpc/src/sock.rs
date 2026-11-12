//! The socket the daemon and its windows talk over: a Unix domain socket on
//! every system. Windows has had them since Windows 10 1803; the standard
//! library does not offer them there, `uds_windows` does with the same names.

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};

#[cfg(windows)]
pub use uds_windows::{UnixListener, UnixStream};
