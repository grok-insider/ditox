//! What the daemon and the window say to each other: JSON-RPC 2.0 in
//! Content-Length frames over `$XDG_RUNTIME_DIR/ditox/rpc.sock`, or
//! `%LOCALAPPDATA%\ditox\rpc.sock` on Windows.

pub mod client;
pub mod dirs;
pub mod frame;
pub mod hypr;
pub mod settings;
pub mod sock;
pub mod types;
#[cfg(windows)]
pub mod win;

pub use client::{Client, RpcError, no_socket_path, socket_path};
pub use settings::{ConfigView, MAX_LINES, MIN_LINES, Settings, Theme, Ui, config_path};
pub use types::*;
