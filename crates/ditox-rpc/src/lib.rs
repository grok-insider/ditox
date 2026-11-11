//! What the daemon and the window say to each other: JSON-RPC 2.0 in
//! Content-Length frames over `$XDG_RUNTIME_DIR/ditox/rpc.sock`.

pub mod client;
pub mod frame;
pub mod hypr;
pub mod types;

pub use client::{Client, RpcError, socket_path};
pub use types::*;
