use base64::Engine;
use ditox_rpc::{
    BLOB_MISSING, ENTRY_NOT_FOUND, Filter, Health, INVALID_PARAMS, INVALID_REQUEST, ImagePayload,
    Kind, METHOD_NOT_FOUND, NOT_AN_IMAGE, PARSE_ERROR, PasteMode, Pulse, SERVER_ERROR, Settings,
};
use serde_json::{Map, Value, json};

use crate::app::App;
use crate::policy;
use crate::storage::ListQuery;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Failure {
    code: i64,
    message: String,
}

impl Failure {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Failure {
            code,
            message: message.into(),
        }
    }

    fn params(message: impl Into<String>) -> Self {
        Failure::new(INVALID_PARAMS, message)
    }
}

impl From<crate::error::Error> for Failure {
    fn from(err: crate::error::Error) -> Self {
        Failure::new(SERVER_ERROR, err.to_string())
    }
}

type Reply = Result<Value, Failure>;

pub fn handle(app: &mut App, request: &[u8]) -> Vec<u8> {
    let (id, reply) = match serde_json::from_slice::<Value>(request) {
        Ok(value) => {
            let id = value.get("id").cloned().unwrap_or(Value::Null);
            (id, route(app, &value))
        }
        Err(err) => (Value::Null, Err(Failure::new(PARSE_ERROR, err.to_string()))),
    };
    let body = match reply {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(failure) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": failure.code, "message": failure.message } })
        }
    };
    serde_json::to_vec(&body).unwrap_or_else(|_| {
        br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32000,"message":"unserializable"}}"#
            .to_vec()
    })
}

fn route(app: &mut App, request: &Value) -> Reply {
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(Failure::new(INVALID_REQUEST, "invalid jsonrpc version"));
    }
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::new(INVALID_REQUEST, "missing method"))?;
    let params = request.get("params").filter(|params| !params.is_null());
    let reply = dispatch(app, method, params)?;
    if changes_history(method) {
        app.touch();
    }
    Ok(reply)
}

fn changes_history(method: &str) -> bool {
    !matches!(
        method,
        "health.check"
            | "config.get"
            | "history.pulse"
            | "entries.list"
            | "entries.get"
            | "entries.get_image"
            | "history.pause"
            | "history.resume"
            | "clipboard.wake"
    )
}

fn dispatch(app: &mut App, method: &str, params: Option<&Value>) -> Reply {
    match method {
        "health.check" => {
            Params::none(params)?;
            Ok(json!(Health {
                ok: true,
                name: "ditoxd".into(),
                version: VERSION.into(),
                storage: "sqlite".into()
            }))
        }
        "config.get" => {
            Params::none(params)?;
            Ok(json!(app.config_view()))
        }
        "config.set" => {
            let params = Params::new(params, &["settings"], &[])?;
            let settings: Settings = params
                .value("settings")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .ok_or_else(|| Failure::params("invalid settings"))?;
            app.set_settings(&settings)
                .map(|view| json!(view))
                .map_err(|err| match err {
                    crate::error::Error::Config(detail) => Failure::params(detail),
                    other => other.into(),
                })
        }
        "history.pulse" => {
            Params::none(params)?;
            Ok(json!(Pulse {
                revision: app.revision(),
                watcher: app.watcher_status()?
            }))
        }
        "entries.list" => {
            let params = Params::new(params, &[], &["query", "filter", "limit", "compact"])?;
            let filter = match params.string("filter")? {
                Some(name) => {
                    Filter::parse(name).ok_or_else(|| Failure::params("invalid filter"))?
                }
                None => Filter::All,
            };
            let limit = match params.integer("limit")? {
                Some(limit) if (1..=1000).contains(&limit) => limit as u32,
                Some(_) => return Err(Failure::params("invalid limit")),
                None => 50,
            };
            let entries = app.store.list(&ListQuery {
                query: params.string("query")?.unwrap_or(""),
                filter,
                limit,
                compact: params.boolean("compact")?.unwrap_or(false),
            })?;
            Ok(json!({ "entries": entries }))
        }
        "entries.get" => {
            let id = Params::new(params, &["id"], &[])?.id()?;
            let entry = app
                .store
                .get(id)?
                .ok_or_else(|| Failure::new(ENTRY_NOT_FOUND, "entry not found"))?;
            Ok(json!({ "entry": entry }))
        }
        "entries.get_image" => {
            let id = Params::new(params, &["id"], &[])?.id()?;
            let entry = app
                .store
                .get(id)?
                .ok_or_else(|| Failure::new(ENTRY_NOT_FOUND, "entry not found"))?;
            if entry.kind != Kind::Image {
                return Err(Failure::new(NOT_AN_IMAGE, "entry is not an image"));
            }
            let bytes = app
                .read_image_blob(entry.blob_path.as_deref())
                .map_err(|_| Failure::new(BLOB_MISSING, "image blob is not stored"))?;
            Ok(json!(ImagePayload {
                mime: entry.mime,
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
                width: entry.image_width,
                height: entry.image_height,
            }))
        }
        "entries.paste" => {
            let params = Params::new(params, &["id"], &["target_window", "mode"])?;
            let mode = match params.string("mode")? {
                None | Some("original") => PasteMode::Original,
                Some("plain") => PasteMode::Plain,
                Some(_) => return Err(Failure::params("invalid mode")),
            };
            Ok(json!(app.paste(
                params.id()?,
                params.string("target_window")?,
                mode
            )?))
        }
        "entries.delete" => {
            let id = Params::new(params, &["id"], &[])?.id()?;
            Ok(json!({ "deleted": app.store.delete(id)? }))
        }
        "entries.favorite" => {
            let params = Params::new(params, &["id", "favorite"], &[])?;
            let favorite = params
                .boolean("favorite")?
                .ok_or_else(|| Failure::params("invalid favorite"))?;
            Ok(json!({ "updated": app.store.favorite(params.id()?, favorite)? }))
        }
        "entries.sticky" => {
            let params = Params::new(params, &["id", "sticky"], &[])?;
            let sticky = params
                .boolean("sticky")?
                .ok_or_else(|| Failure::params("invalid sticky"))?;
            Ok(json!({ "updated": app.store.set_sticky(params.id()?, sticky)? }))
        }
        "history.pause" => {
            let params = Params::new(params, &[], &["duration_ms"])?;
            let duration_ms = match params.integer("duration_ms")? {
                Some(duration) => {
                    u32::try_from(duration).map_err(|_| Failure::params("invalid duration_ms"))?
                }
                None => 0,
            };
            app.store.pause_watcher(duration_ms)?;
            Ok(json!({ "paused_for_ms": duration_ms }))
        }
        "history.resume" => {
            Params::none(params)?;
            app.store.resume_watcher()?;
            Ok(json!({ "resumed": true }))
        }
        "clipboard.wake" => {
            let params = Params::new(params, &[], &["sensitive"])?;
            if policy::wake_stores(params.boolean("sensitive")?.unwrap_or(false)) {
                app.request_capture();
            }
            Ok(json!({ "ok": true }))
        }
        "clipboard.ingest" => {
            let params = Params::new(params, &["data"], &["kind"])?;
            let kind = match params.string("kind")? {
                None | Some("text") => Kind::Text,
                Some("image") => Kind::Image,
                Some(_) => return Err(Failure::params("invalid kind")),
            };
            let data = params
                .string("data")?
                .ok_or_else(|| Failure::params("invalid data"))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| Failure::params("invalid data"))?;
            Ok(json!(app.ingest(kind, &bytes)?))
        }
        _ => Err(Failure::new(METHOD_NOT_FOUND, "method not found")),
    }
}

struct Params<'a> {
    object: Option<&'a Map<String, Value>>,
}

impl<'a> Params<'a> {
    fn new(
        value: Option<&'a Value>,
        required: &[&str],
        optional: &[&str],
    ) -> Result<Self, Failure> {
        let object = match value {
            Some(Value::Object(object)) => Some(object),
            Some(_) => return Err(Failure::params("params must be an object")),
            None => None,
        };
        if required
            .iter()
            .any(|key| object.is_none_or(|object| !object.contains_key(*key)))
        {
            return Err(Failure::params("missing params"));
        }
        if object.is_some_and(|object| {
            object
                .keys()
                .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
        }) {
            return Err(Failure::params("unexpected params"));
        }
        Ok(Params { object })
    }

    fn none(value: Option<&'a Value>) -> Result<(), Failure> {
        Params::new(value, &[], &[]).map(drop)
    }

    fn value(&self, key: &str) -> Option<&'a Value> {
        self.object?.get(key).filter(|value| !value.is_null())
    }

    fn integer(&self, key: &str) -> Result<Option<i64>, Failure> {
        self.value(key)
            .map(|value| {
                value
                    .as_i64()
                    .ok_or_else(|| Failure::params(format!("invalid {key}")))
            })
            .transpose()
    }

    fn string(&self, key: &str) -> Result<Option<&'a str>, Failure> {
        self.value(key)
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| Failure::params(format!("invalid {key}")))
            })
            .transpose()
    }

    fn boolean(&self, key: &str) -> Result<Option<bool>, Failure> {
        self.value(key)
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| Failure::params(format!("invalid {key}")))
            })
            .transpose()
    }

    fn id(&self) -> Result<i64, Failure> {
        self.integer("id")?
            .ok_or_else(|| Failure::params("invalid id"))
    }
}

#[cfg(test)]
mod tests {
    use ditox_rpc::{ConfigView, Entry, PasteResult};

    use super::*;
    use crate::app::tests::{FakeDesktop, app};
    use crate::image::tiny_png;

    fn call(app: &mut App, method: &str, params: Value) -> Value {
        let request = json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params });
        let response: Value =
            serde_json::from_slice(&handle(app, &serde_json::to_vec(&request).unwrap())).unwrap();
        assert_eq!(response["id"], 7);
        response
    }

    fn ok(app: &mut App, method: &str, params: Value) -> Value {
        let response = call(app, method, params);
        assert!(response.get("error").is_none(), "{method}: {response}");
        response["result"].clone()
    }

    fn error_code(app: &mut App, method: &str, params: Value) -> i64 {
        let response = call(app, method, params);
        response["error"]["code"]
            .as_i64()
            .unwrap_or_else(|| panic!("{method} answered {response}"))
    }

    #[test]
    fn the_history_is_listed_pinned_and_deleted_over_rpc() {
        let (_dir, mut app) = app("rpc-basic", FakeDesktop::default());
        assert_eq!(ok(&mut app, "health.check", json!({}))["name"], "ditoxd");
        let start = ok(&mut app, "history.pulse", json!({}))["revision"]
            .as_u64()
            .unwrap();

        let data = base64::engine::general_purpose::STANDARD.encode("hello rpc");
        let id = ok(&mut app, "clipboard.ingest", json!({ "data": data }))["id"]
            .as_i64()
            .unwrap();
        assert!(
            ok(&mut app, "history.pulse", json!({}))["revision"]
                .as_u64()
                .unwrap()
                > start
        );

        let listed: Vec<Entry> = serde_json::from_value(
            ok(
                &mut app,
                "entries.list",
                json!({ "query": "rpc", "limit": 10 }),
            )["entries"]
                .clone(),
        )
        .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].content, "hello rpc");

        for (method, flag) in [
            ("entries.favorite", "favorite"),
            ("entries.sticky", "sticky"),
        ] {
            assert_eq!(
                ok(&mut app, method, json!({ "id": id, flag: true }))["updated"],
                true
            );
        }
        let pinned: Entry = serde_json::from_value(
            ok(&mut app, "entries.get", json!({ "id": id }))["entry"].clone(),
        )
        .unwrap();
        assert!(pinned.favorite && pinned.sticky);
        assert_eq!(
            ok(&mut app, "entries.delete", json!({ "id": id }))["deleted"],
            true
        );
        assert_eq!(
            error_code(&mut app, "entries.get", json!({ "id": id })),
            ENTRY_NOT_FOUND
        );
    }

    #[test]
    fn a_read_does_not_move_the_revision() {
        let (_dir, mut app) = app("rpc-revision", FakeDesktop::default());
        let id = app.store.add_text(&app.cfg, "read me").unwrap();
        let before = app.revision();
        for method in ["health.check", "history.pulse", "entries.list"] {
            ok(&mut app, method, json!({}));
        }
        ok(&mut app, "entries.get", json!({ "id": id }));
        ok(&mut app, "history.pause", json!({ "duration_ms": 0 }));
        ok(&mut app, "history.resume", json!({}));
        ok(&mut app, "clipboard.wake", json!({}));
        assert_eq!(app.revision(), before);
        ok(&mut app, "entries.delete", json!({ "id": id }));
        assert_eq!(app.revision(), before + 1);
    }

    #[test]
    fn params_are_checked_per_method() {
        let (_dir, mut app) = app("rpc-params", FakeDesktop::default());
        for (method, params) in [
            ("entries.get_image", json!({})),
            ("entries.list", json!({ "limit": 1001 })),
            ("entries.list", json!({ "query": "ok", "extra": 1 })),
            ("entries.list", json!({ "filter": "yesterday" })),
            ("entries.favorite", json!({ "id": 1 })),
            ("entries.paste", json!({ "id": 1, "mode": "fancy" })),
            ("history.pause", json!({ "duration_ms": -1 })),
            ("health.check", json!({ "x": 1 })),
            ("clipboard.ingest", json!({ "data": "***" })),
            ("entries.delete", json!([1])),
        ] {
            assert_eq!(
                error_code(&mut app, method, params.clone()),
                INVALID_PARAMS,
                "{method} {params}"
            );
        }
        for gone in [
            "nothing.here",
            "entries.clear",
            "groups.create",
            "stats.get",
        ] {
            assert_eq!(
                error_code(&mut app, gone, json!({})),
                METHOD_NOT_FOUND,
                "{gone}"
            );
        }

        let bad_version: Value = serde_json::from_slice(&handle(
            &mut app,
            br#"{"jsonrpc":"1.0","id":1,"method":"health.check"}"#,
        ))
        .unwrap();
        assert_eq!(bad_version["error"]["code"], INVALID_REQUEST);
        let garbage: Value = serde_json::from_slice(&handle(&mut app, b"not json")).unwrap();
        assert_eq!(garbage["error"]["code"], PARSE_ERROR);
        let no_params: Value = serde_json::from_slice(&handle(
            &mut app,
            br#"{"jsonrpc":"2.0","id":1,"method":"health.check"}"#,
        ))
        .unwrap();
        assert_eq!(no_params["result"]["ok"], true);
    }

    #[test]
    fn settings_are_read_saved_to_the_file_and_lived_by_at_once() {
        let (dir, mut app) = app("rpc-config", FakeDesktop::default());
        app.cfg.config_path = dir.path("config/ditox/config.toml");
        for text in ["one", "two", "three"] {
            app.store.add_text(&app.cfg, text).unwrap();
        }

        let before = app.revision();
        let view: ConfigView =
            serde_json::from_value(ok(&mut app, "config.get", json!({}))).unwrap();
        assert!(view.writable);
        assert_eq!(view.path, app.cfg.config_path);
        assert_eq!(view.settings, app.cfg.settings());
        assert_eq!(app.revision(), before);

        let mut wanted = view.settings.clone();
        wanted.max_entries = 2;
        wanted.ui.lines = 3;
        wanted.excluded_apps = vec!["Signal".into()];
        let saved: ConfigView =
            serde_json::from_value(ok(&mut app, "config.set", json!({ "settings": wanted })))
                .unwrap();
        assert_eq!(saved.settings, wanted);
        assert_eq!(app.cfg.max_entries, 2);
        assert_eq!(app.cfg.excluded_apps, ["Signal"]);
        assert!(app.revision() > before);
        let listed = ok(&mut app, "entries.list", json!({}))["entries"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(listed, 2);
        let written = std::fs::read_to_string(&app.cfg.config_path).unwrap();
        assert!(written.contains("max_entries = 2") && written.contains("lines = 3"));
        assert_eq!(ditox_rpc::Ui::parse(&written).lines, 3);

        let mut bad = wanted.clone();
        bad.max_entries = 0;
        bad.ui.lines = 5;
        for params in [
            json!({ "settings": bad }),
            json!({ "settings": { "max_entries": 5 } }),
            json!({ "settings": "all of them" }),
            json!({}),
        ] {
            assert_eq!(
                error_code(&mut app, "config.set", params.clone()),
                INVALID_PARAMS,
                "{params}"
            );
        }
        assert_eq!(app.cfg.settings(), wanted);
        assert_eq!(
            std::fs::read_to_string(&app.cfg.config_path).unwrap(),
            written
        );

        std::fs::remove_file(&app.cfg.config_path).unwrap();
        std::fs::write(dir.path("store.toml"), "").unwrap();
        if crate::testing::link(dir.path("store.toml"), &app.cfg.config_path) {
            let mut more = wanted.clone();
            more.max_entries = 9;
            assert_eq!(
                error_code(&mut app, "config.set", json!({ "settings": more })),
                INVALID_PARAMS
            );
            assert_eq!(app.cfg.max_entries, 2);
            let view: ConfigView =
                serde_json::from_value(ok(&mut app, "config.get", json!({}))).unwrap();
            assert!(!view.writable);
        }
    }

    #[test]
    fn images_travel_as_base64_and_text_entries_refuse() {
        let (_dir, mut app) = app("rpc-image", FakeDesktop::default());
        let bytes = tiny_png(3, 2);
        let image = app
            .store
            .add_image(
                &app.cfg,
                "image/png",
                &bytes,
                crate::image::size(&bytes, "image/png"),
            )
            .unwrap();
        let text = app.store.add_text(&app.cfg, "text").unwrap();

        let payload: ImagePayload =
            serde_json::from_value(ok(&mut app, "entries.get_image", json!({ "id": image })))
                .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(payload.data)
                .unwrap(),
            bytes
        );
        assert_eq!(
            (payload.mime.as_str(), payload.width, payload.height),
            ("image/png", Some(3), Some(2))
        );
        assert_eq!(
            error_code(&mut app, "entries.get_image", json!({ "id": text })),
            NOT_AN_IMAGE
        );

        std::fs::remove_file(app.store.get(image).unwrap().unwrap().blob_path.unwrap()).unwrap();
        assert_eq!(
            error_code(&mut app, "entries.get_image", json!({ "id": image })),
            BLOB_MISSING
        );
    }

    #[test]
    fn a_wake_asks_for_a_capture_unless_the_offer_is_sensitive() {
        let (_dir, mut app) = app("rpc-wake", FakeDesktop::default());
        ok(&mut app, "clipboard.wake", json!({ "sensitive": true }));
        assert!(!app.take_capture_request());
        ok(&mut app, "clipboard.wake", json!({}));
        assert!(app.take_capture_request());
        assert!(!app.take_capture_request());
    }

    #[test]
    fn paste_pause_and_ingest_answer_with_their_results() {
        let (_dir, mut app) = app("rpc-paste", FakeDesktop::default());
        let id = app.store.add_text(&app.cfg, "paste me").unwrap();
        let pasted: PasteResult = serde_json::from_value(ok(
            &mut app,
            "entries.paste",
            json!({ "id": id, "mode": "plain" }),
        ))
        .unwrap();
        assert!(pasted.pasted);
        let missing: PasteResult =
            serde_json::from_value(ok(&mut app, "entries.paste", json!({ "id": 999 }))).unwrap();
        assert_eq!(missing.reason, "missing");

        ok(&mut app, "history.pause", json!({}));
        let paused: Pulse =
            serde_json::from_value(ok(&mut app, "history.pulse", json!({}))).unwrap();
        assert!(paused.watcher.paused);
        ok(&mut app, "history.resume", json!({}));
        let resumed: Pulse =
            serde_json::from_value(ok(&mut app, "history.pulse", json!({}))).unwrap();
        assert!(!resumed.watcher.paused);

        let data = base64::engine::general_purpose::STANDARD.encode("from a pipe");
        assert_eq!(
            ok(
                &mut app,
                "clipboard.ingest",
                json!({ "kind": "text", "data": data })
            )["stored"],
            true
        );
    }
}
