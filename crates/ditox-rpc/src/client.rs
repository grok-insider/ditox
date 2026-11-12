use std::fmt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::frame::{FrameReader, write_frame};
use crate::settings::{ConfigView, Settings};
use crate::types::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcError {
    /// The socket is missing, refused, or broke mid-call.
    Transport(String),
    /// The daemon answered with an error.
    Remote { code: i64, message: String },
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::Transport(message) | RpcError::Remote { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for RpcError {}

/// `DITOX_SOCKET`, else `$XDG_RUNTIME_DIR/ditox/rpc.sock`. There is no
/// fallback to a shared directory.
pub fn socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DITOX_SOCKET").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|path| !path.is_empty())?;
    Some(PathBuf::from(runtime).join("ditox/rpc.sock"))
}

pub struct Client {
    stream: UnixStream,
    reader: FrameReader,
    next_id: i64,
}

impl Client {
    pub fn connect() -> Result<Self, RpcError> {
        let path = socket_path()
            .ok_or_else(|| RpcError::Transport("XDG_RUNTIME_DIR is not set".into()))?;
        Self::connect_to(&path)
    }

    pub fn connect_to(path: &Path) -> Result<Self, RpcError> {
        let stream = UnixStream::connect(path)
            .map_err(|err| RpcError::Transport(format!("{}: {err}", path.display())))?;
        // A paste waits for the compositor; nothing takes this long.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));
        Ok(Client {
            stream,
            reader: FrameReader::new(),
            next_id: 1,
        })
    }

    pub fn call<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<T, RpcError> {
        let response = self.exchange(method, params)?;
        match parse::<T>(method, &response)? {
            Some(result) => Ok(result),
            None => serde_json::from_value(Value::Null)
                .map_err(|err| RpcError::Transport(format!("{method}: {err}"))),
        }
    }

    /// Sends one request and returns the body of its answer.
    fn exchange(&mut self, method: &str, params: Value) -> Result<Vec<u8>, RpcError> {
        let id = self.next_id;
        self.next_id += 1;
        let body = serde_json::to_vec(
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .map_err(|err| RpcError::Transport(err.to_string()))?;
        write_frame(&mut self.stream, &body).map_err(|err| RpcError::Transport(err.to_string()))?;
        self.reader
            .read(&mut self.stream)
            .map_err(|err| RpcError::Transport(err.to_string()))?
            .ok_or_else(|| RpcError::Transport("the daemon closed the connection".into()))
    }

    pub fn pulse(&mut self) -> Result<Pulse, RpcError> {
        self.call("history.pulse", json!({}))
    }

    /// `compact` cuts each text to its first lines; fetch the whole clip with `get`.
    pub fn list(
        &mut self,
        query: &str,
        filter: Filter,
        limit: u32,
        compact: bool,
    ) -> Result<Vec<Entry>, RpcError> {
        let listed: Listed = self.call(
            "entries.list",
            json!({ "query": query, "filter": filter.as_str(), "limit": limit, "compact": compact }),
        )?;
        Ok(listed.entries)
    }

    pub fn get(&mut self, id: i64) -> Result<Entry, RpcError> {
        let got: Got = self.call("entries.get", json!({ "id": id }))?;
        Ok(got.entry)
    }

    /// The bytes of an image clip. The answer is tens of megabytes of
    /// base64 for a screenshot, so it is decoded where it lies.
    pub fn image(&mut self, id: i64) -> Result<Vec<u8>, RpcError> {
        #[derive(serde::Deserialize)]
        struct Data<'a> {
            #[serde(borrow)]
            data: std::borrow::Cow<'a, str>,
        }
        let method = "entries.get_image";
        let response = self.exchange(method, json!({ "id": id }))?;
        let payload = parse::<Data<'_>>(method, &response)?
            .ok_or_else(|| RpcError::Transport(format!("{method}: no result")))?;
        base64::engine::general_purpose::STANDARD
            .decode(payload.data.as_bytes())
            .map_err(|err| RpcError::Transport(err.to_string()))
    }

    pub fn paste(
        &mut self,
        id: i64,
        mode: PasteMode,
        target_window: Option<&str>,
    ) -> Result<PasteResult, RpcError> {
        let mut params = json!({ "id": id, "mode": mode.as_str() });
        if let Some(target) = target_window {
            params["target_window"] = json!(target);
        }
        self.call("entries.paste", params)
    }

    pub fn favorite(&mut self, id: i64, favorite: bool) -> Result<(), RpcError> {
        self.call::<Value>(
            "entries.favorite",
            json!({ "id": id, "favorite": favorite }),
        )
        .map(drop)
    }

    pub fn sticky(&mut self, id: i64, sticky: bool) -> Result<(), RpcError> {
        self.call::<Value>("entries.sticky", json!({ "id": id, "sticky": sticky }))
            .map(drop)
    }

    pub fn delete(&mut self, id: i64) -> Result<(), RpcError> {
        self.call::<Value>("entries.delete", json!({ "id": id }))
            .map(drop)
    }

    /// `duration_ms` 0 pauses until resumed.
    pub fn pause(&mut self, duration_ms: u32) -> Result<(), RpcError> {
        self.call::<Value>("history.pause", json!({ "duration_ms": duration_ms }))
            .map(drop)
    }

    pub fn resume(&mut self) -> Result<(), RpcError> {
        self.call::<Value>("history.resume", json!({})).map(drop)
    }

    pub fn config(&mut self) -> Result<ConfigView, RpcError> {
        self.call("config.get", json!({}))
    }

    /// Asks the daemon to take these settings and write them to its file.
    /// It answers with the settings as it holds them afterwards.
    pub fn set_config(&mut self, settings: &Settings) -> Result<ConfigView, RpcError> {
        self.call("config.set", json!({ "settings": settings }))
    }

    pub fn wake(&mut self, sensitive: bool) -> Result<(), RpcError> {
        self.call::<Value>("clipboard.wake", json!({ "sensitive": sensitive }))
            .map(drop)
    }

    pub fn ingest(&mut self, kind: Kind, bytes: &[u8]) -> Result<IngestResult, RpcError> {
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        self.call(
            "clipboard.ingest",
            json!({ "kind": kind.as_str(), "data": data }),
        )
    }
}

#[derive(serde::Deserialize)]
struct Listed {
    entries: Vec<Entry>,
}

#[derive(serde::Deserialize)]
struct Got {
    entry: Entry,
}

/// The result in a response, `None` for `null`, or the daemon's error.
fn parse<'a, T: serde::Deserialize<'a>>(
    method: &str,
    response: &'a [u8],
) -> Result<Option<T>, RpcError> {
    #[derive(serde::Deserialize)]
    struct Failure {
        code: Option<i64>,
        message: Option<String>,
    }
    #[derive(serde::Deserialize)]
    #[serde(bound(deserialize = "T: serde::Deserialize<'de>"))]
    struct Envelope<T> {
        #[serde(default = "Option::default")]
        result: Option<T>,
        #[serde(default)]
        error: Option<Failure>,
    }
    #[derive(serde::Deserialize)]
    struct Refusal {
        error: Failure,
    }
    let remote = |failure: Failure| RpcError::Remote {
        code: failure.code.unwrap_or(SERVER_ERROR),
        message: failure.message.unwrap_or_else(|| "daemon error".into()),
    };
    match serde_json::from_slice::<Envelope<T>>(response) {
        Ok(Envelope {
            error: Some(failure),
            ..
        }) => Err(remote(failure)),
        Ok(envelope) => Ok(envelope.result),
        // An error answer may carry a result of another shape.
        Err(err) => match serde_json::from_slice::<Refusal>(response) {
            Ok(refusal) => Err(remote(refusal.error)),
            Err(_) => Err(RpcError::Transport(format!("{method}: {err}"))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_response_is_its_result_nothing_or_the_daemons_error() {
        let pulse: Option<Value> =
            parse("m", br#"{"jsonrpc":"2.0","id":1,"result":{"revision":7}}"#).unwrap();
        assert_eq!(pulse, Some(json!({ "revision": 7 })));
        let nothing: Option<Value> =
            parse("m", br#"{"jsonrpc":"2.0","id":1,"result":null}"#).unwrap();
        assert_eq!(nothing, None);

        let refused =
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32004,"message":"entry not found"}}"#;
        for err in [
            parse::<Value>("m", refused).unwrap_err(),
            parse::<Vec<Entry>>("m", refused).unwrap_err(),
            parse::<Vec<Entry>>(
                "m",
                br#"{"jsonrpc":"2.0","id":1,"result":"odd","error":{"code":-32004,"message":"entry not found"}}"#,
            )
            .unwrap_err(),
        ] {
            assert!(
                matches!(&err, RpcError::Remote { code: -32004, message } if message == "entry not found"),
                "{err}"
            );
        }
        assert!(matches!(
            parse::<Vec<Entry>>("m", br#"{"jsonrpc":"2.0","id":1,"result":"odd"}"#),
            Err(RpcError::Transport(_))
        ));
        assert!(matches!(
            parse::<Value>("m", b"not json"),
            Err(RpcError::Transport(_))
        ));
        assert!(
            matches!(
                parse::<Value>(
                    "m",
                    br#"{"jsonrpc":"2.0","id":1,"result":"odd","error":{"code":-32000,"message":"no"}}"#
                ),
                Err(RpcError::Remote { code: -32000, .. })
            ),
            "an error wins over a result sent beside it"
        );
    }
}
