use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

const OWN_WINDOW_CLASS: &str = "dev.ditox.gui";

struct AppState {
    stream: Mutex<Option<UnixStream>>,
    target: Mutex<Option<String>>,
}

pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    if runtime.is_empty() {
        return None;
    }
    Some(PathBuf::from(runtime).join("ditox/rpc.sock"))
}

pub fn connection_plan(socket_exists: bool) -> &'static str {
    if socket_exists { "connect" } else { "daemon-down" }
}

pub fn paste_target(class: &str, address: &str) -> Option<String> {
    if class == OWN_WINDOW_CLASS {
        return None;
    }
    let address = address.trim();
    if address.is_empty() { None } else { Some(address.to_string()) }
}

pub fn window_paste_target(payload: &str) -> Option<String> {
    let value: Value = serde_json::from_str(payload).ok()?;
    let class = value.get("class").and_then(Value::as_str).unwrap_or("");
    let address = value.get("address").and_then(Value::as_str).unwrap_or("");
    paste_target(class, address)
}

pub fn without_blob_paths(mut value: Value) -> Value {
    strip_blob_paths(&mut value);
    value
}

pub fn image_for_webview(value: Value) -> Result<Value, String> {
    let result = value.get("result").cloned().unwrap_or(value);
    let mime = result.get("mime").and_then(Value::as_str).unwrap_or("");
    let data = result.get("data").and_then(Value::as_str).unwrap_or("");
    if mime != "image/png" && mime != "image/jpeg" {
        return Err("not an image".to_string());
    }
    if data.is_empty() || data.contains('/') || data.contains('\\') || data.contains(',') {
        return Err("not an image".to_string());
    }
    Ok(json!({ "mime": mime, "data": data }))
}

fn strip_blob_paths(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("blob_path");
            for child in map.values_mut() {
                strip_blob_paths(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_blob_paths(item);
            }
        }
        _ => {}
    }
}

fn open_socket() -> Result<UnixStream, String> {
    let path = socket_path().ok_or_else(|| "daemon-down".to_string())?;
    if !path.exists() {
        return Err("daemon-down".to_string());
    }
    let stream = UnixStream::connect(&path).map_err(|_| "daemon-down".to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    Ok(stream)
}

fn exchange(stream: &mut UnixStream, method: &str, params: &Value) -> Result<Value, String> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    })
    .to_string();
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    stream.write_all(frame.as_bytes()).map_err(|_| "daemon-down".to_string())?;
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let read = stream.read(&mut byte).map_err(|_| "daemon-down".to_string())?;
        if read == 0 {
            return Err("daemon-down".to_string());
        }
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
        if header.len() > 1024 {
            return Err("bad response".to_string());
        }
    }
    let header_text = String::from_utf8_lossy(&header);
    let length = header_text
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .ok_or_else(|| "bad response".to_string())?;
    let mut body_bytes = vec![0u8; length];
    stream.read_exact(&mut body_bytes).map_err(|_| "daemon-down".to_string())?;
    serde_json::from_slice(&body_bytes).map_err(|error| error.to_string())
}

fn rpc(slot: &Mutex<Option<UnixStream>>, method: &str, params: Value) -> Result<Value, String> {
    let mut guard = slot.lock().map_err(|_| "daemon-down".to_string())?;
    if guard.is_none() {
        *guard = Some(open_socket()?);
    }
    let first = exchange(guard.as_mut().expect("socket"), method, &params);
    let value = match first {
        Ok(value) => value,
        Err(_) => {
            *guard = None;
            *guard = Some(open_socket()?);
            exchange(guard.as_mut().expect("socket"), method, &params)?
        }
    };
    Ok(without_blob_paths(value))
}

fn capture_active_window() -> Option<String> {
    let output = Command::new("hyprctl").args(["-j", "activewindow"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    window_paste_target(&text)
}

fn remember_target(state: &AppState) {
    if let Some(address) = capture_active_window() {
        if let Ok(mut target) = state.target.lock() {
            *target = Some(address);
        }
    }
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn daemon_status() -> String {
    connection_plan(socket_path().is_some_and(|path| path.exists())).to_string()
}

#[tauri::command(rename_all = "snake_case")]
fn list_entries(query: String, state: tauri::State<AppState>) -> Result<Value, String> {
    rpc(&state.stream, "entries.list", json!({ "query": query, "limit": 1000 }))
}

#[tauri::command(rename_all = "snake_case")]
fn entry_image(id: i64, state: tauri::State<AppState>) -> Result<Value, String> {
    let response = rpc(&state.stream, "entries.get_image", json!({ "id": id }))?;
    image_for_webview(response)
}

#[tauri::command(rename_all = "snake_case")]
fn paste_entry(id: i64, mode: String, target_window: Option<String>, state: tauri::State<AppState>) -> Result<Value, String> {
    let mode = if mode == "plain" { "plain" } else { "original" };
    let mut params = json!({ "id": id, "mode": mode });
    if let Some(window) = target_window.filter(|value| !value.trim().is_empty()) {
        params["target_window"] = json!(window);
    }
    rpc(&state.stream, "entries.paste", params)
}

#[tauri::command]
fn captured_window(state: tauri::State<AppState>) -> Option<String> {
    state.target.lock().ok().and_then(|target| target.clone())
}

#[tauri::command]
fn hide_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn main() {
    tauri::Builder::default()
        .manage(AppState {
            stream: Mutex::new(None),
            target: Mutex::new(None),
        })
        .setup(|app| {
            remember_target(app.state::<AppState>().inner());
            let _tray = tauri::tray::TrayIconBuilder::new()
                .tooltip("ditox")
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click { .. } = event {
                        let app = tray.app_handle();
                        remember_target(app.state::<AppState>().inner());
                        show_main(app);
                    }
                })
                .build(app)?;
            show_main(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            list_entries,
            entry_image,
            paste_entry,
            captured_window,
            hide_window
        ])
        .run(tauri::generate_context!())
        .expect("ditox gui failed to start");
}

#[cfg(test)]
mod tests {
    use super::{connection_plan, image_for_webview, paste_target, window_paste_target, without_blob_paths};
    use serde_json::json;

    #[test]
    fn a_missing_socket_is_daemon_down_and_a_present_socket_connects() {
        assert_eq!(connection_plan(false), "daemon-down");
        assert_eq!(connection_plan(true), "connect");
        let production = include_str!("main.rs").split("mod tests").next().unwrap();
        assert!(!production.contains("serve --stdio"));
        assert!(!production.contains("ditoxd"));
    }

    #[test]
    fn list_payload_given_to_the_webview_has_no_blob_path() {
        let raw = json!({"result":{"entries":[{"id":1,"blob_path":"/tmp/secret.png","content":"<p>kept</p>"}]}});
        let clean = without_blob_paths(raw);
        let text = clean.to_string();
        assert!(!text.contains("blob_path"));
        assert!(!text.contains("/tmp/secret.png"));
        assert!(text.contains("<p>kept</p>"));
    }

    #[test]
    fn the_selected_image_is_bytes_and_our_own_window_is_not_a_paste_target() {
        let image = image_for_webview(json!({"result":{"mime":"image/png","data":"iVBORw0KGgo=","blob_path":"/tmp/secret.png"}})).unwrap();
        assert_eq!(image, json!({"mime":"image/png","data":"iVBORw0KGgo="}));
        assert!(image_for_webview(json!({"result":{"mime":"image/png","data":"/tmp/secret.png"}})).is_err());
        assert_eq!(
            window_paste_target(r#"{"class":"firefox","address":"0xabc"}"#).as_deref(),
            Some("0xabc")
        );
        assert_eq!(paste_target("dev.ditox.gui", "0xself"), None);
    }
}
