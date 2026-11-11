use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Image,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Image => "image",
        }
    }

    pub fn parse(value: &str) -> Option<Kind> {
        match value {
            "text" => Some(Kind::Text),
            "image" => Some(Kind::Image),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: i64,
    pub kind: Kind,
    pub mime: String,
    pub content: String,
    pub preview: String,
    pub hash: String,
    pub favorite: bool,
    pub created_at_ms: i64,
    #[serde(default)]
    pub last_used_at_ms: Option<i64>,
    pub byte_len: i64,
    #[serde(default)]
    pub source_app: Option<String>,
    #[serde(default)]
    pub blob_path: Option<String>,
    #[serde(default)]
    pub image_width: Option<i64>,
    #[serde(default)]
    pub image_height: Option<i64>,
    #[serde(default)]
    pub sticky: bool,
}

impl Entry {
    /// When the clip was last copied or pasted.
    pub fn touched_ms(&self) -> i64 {
        self.created_at_ms.max(self.last_used_at_ms.unwrap_or(0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filter {
    All,
    Text,
    Images,
    Favorites,
    Today,
}

impl Filter {
    pub fn as_str(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Text => "text",
            Filter::Images => "images",
            Filter::Favorites => "favorites",
            Filter::Today => "today",
        }
    }

    pub fn parse(value: &str) -> Option<Filter> {
        match value {
            "all" => Some(Filter::All),
            "text" => Some(Filter::Text),
            "images" => Some(Filter::Images),
            "favorites" => Some(Filter::Favorites),
            "today" => Some(Filter::Today),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PasteMode {
    Original,
    Plain,
}

impl PasteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PasteMode::Original => "original",
            PasteMode::Plain => "plain",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub ok: bool,
    pub name: String,
    pub version: String,
    pub storage: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatcherStatus {
    /// The clipboard watch is alive.
    pub running: bool,
    pub paused: bool,
    pub backend: String,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasteResult {
    pub pasted: bool,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestResult {
    pub stored: bool,
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImagePayload {
    pub mime: String,
    /// Standard base64.
    pub data: String,
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
}

/// One answer for a client that polls: has the history changed, and is the
/// watcher well.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pulse {
    /// Grows whenever a clip is added, changed or removed.
    pub revision: u64,
    pub watcher: WatcherStatus,
}

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const SERVER_ERROR: i64 = -32000;
pub const ENTRY_NOT_FOUND: i64 = -32004;
pub const NOT_AN_IMAGE: i64 = -32005;
pub const BLOB_MISSING: i64 = -32006;
