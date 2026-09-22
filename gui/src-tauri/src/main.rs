use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::window::Color;
use tauri::{Emitter, Manager};

const OWN_WINDOW_CLASS: &str = "dev.ditox.gui";

fn set_linux_window_class() {
    #[cfg(target_os = "linux")]
    unsafe {
        extern "C" {
            fn g_set_prgname(name: *const std::os::raw::c_char);
            fn gdk_set_program_class(class_name: *const std::os::raw::c_char);
        }
        let class = c"dev.ditox.gui".as_ptr();
        g_set_prgname(class);
        gdk_set_program_class(class);
    }
}

struct TransparencyChecks {
    opaque: CheckMenuItem<tauri::Wry>,
    ten: CheckMenuItem<tauri::Wry>,
    twenty: CheckMenuItem<tauri::Wry>,
    thirty: CheckMenuItem<tauri::Wry>,
}

struct AppState {
    stream: Mutex<Option<UnixStream>>,
    target: Mutex<Option<String>>,
    transparency: Mutex<u8>,
    checks: Mutex<Option<TransparencyChecks>>,
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

pub fn accepted_transparency(value: u64) -> u8 {
    match value {
        10 => 10,
        20 => 20,
        30 => 30,
        _ => 0,
    }
}

pub fn transparency_from_file(text: &str) -> u8 {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return 0;
    };
    let number = value.get("transparency").and_then(Value::as_u64).unwrap_or(0);
    accepted_transparency(number)
}

pub fn read_transparency(path: &std::path::Path) -> u8 {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    transparency_from_file(&text)
}

pub fn write_transparency(path: &std::path::Path, percent: u8) -> std::io::Result<()> {
    let percent = accepted_transparency(u64::from(percent));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json!({ "transparency": percent }).to_string())
}

fn window_config_path() -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    config_home.join("ditox").join("window.json")
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

fn is_standard_base64(data: &str) -> bool {
    let bytes = data.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let pad = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if pad > 2 || bytes.len() == pad {
        return false;
    }
    bytes[..bytes.len() - pad]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'+' || *byte == b'/')
}

pub fn image_for_webview(value: Value) -> Result<Value, String> {
    let result = value.get("result").cloned().unwrap_or(value);
    let mime = result.get("mime").and_then(Value::as_str).unwrap_or("");
    let data = result.get("data").and_then(Value::as_str).unwrap_or("");
    if mime != "image/png" && mime != "image/jpeg" {
        return Err("not an image".to_string());
    }
    if !is_standard_base64(data) {
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

fn clear_window_background(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_background_color(Some(Color(0, 0, 0, 0)));
    }
}

fn save_transparency(app: &tauri::AppHandle, percent: u8) {
    let percent = accepted_transparency(u64::from(percent));
    let _ = write_transparency(&window_config_path(), percent);
    if let Ok(mut slot) = app.state::<AppState>().transparency.lock() {
        *slot = percent;
    }
    if let Ok(checks) = app.state::<AppState>().checks.lock() {
        if let Some(checks) = checks.as_ref() {
            let _ = checks.opaque.set_checked(percent == 0);
            let _ = checks.ten.set_checked(percent == 10);
            let _ = checks.twenty.set_checked(percent == 20);
            let _ = checks.thirty.set_checked(percent == 30);
        }
    }
    let _ = app.emit("ditox-transparency", percent);
}

fn emit_paste(app: &tauri::AppHandle, mode: &str) {
    remember_target(app.state::<AppState>().inner());
    let _ = app.emit("ditox-paste", mode.to_string());
}

fn install_tray(app: &tauri::AppHandle, percent: u8) -> tauri::Result<()> {
    let icon = Image::from_bytes(include_bytes!("../icons/icon.png"))?;
    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let paste = MenuItem::with_id(app, "paste", "Paste", true, None::<&str>)?;
    let paste_plain = MenuItem::with_id(app, "paste-plain", "Paste as text", true, None::<&str>)?;
    let opaque = CheckMenuItem::with_id(app, "transparency-0", "Opaque", true, percent == 0, None::<&str>)?;
    let ten = CheckMenuItem::with_id(app, "transparency-10", "10%", true, percent == 10, None::<&str>)?;
    let twenty = CheckMenuItem::with_id(app, "transparency-20", "20%", true, percent == 20, None::<&str>)?;
    let thirty = CheckMenuItem::with_id(app, "transparency-30", "30%", true, percent == 30, None::<&str>)?;
    let transparency = Submenu::with_items(app, "Transparency", true, &[&opaque, &ten, &twenty, &thirty])?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &paste, &paste_plain, &transparency, &quit])?;
    let tray = tauri::tray::TrayIconBuilder::with_id("ditox")
        .icon(icon)
        .tooltip("ditox")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                remember_target(app.state::<AppState>().inner());
                show_main(app);
            }
            "paste" => emit_paste(app, "original"),
            "paste-plain" => emit_paste(app, "plain"),
            "transparency-0" => save_transparency(app, 0),
            "transparency-10" => save_transparency(app, 10),
            "transparency-20" => save_transparency(app, 20),
            "transparency-30" => save_transparency(app, 30),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            else {
                return;
            };
            let app = tray.app_handle();
            remember_target(app.state::<AppState>().inner());
            show_main(app);
        })
        .build(app)?;
    // Dropping the tray marks the indicator passive and Waybar hides it.
    app.manage(tray);
    if let Ok(mut slot) = app.state::<AppState>().checks.lock() {
        *slot = Some(TransparencyChecks { opaque, ten, twenty, thirty });
    }
    Ok(())
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

#[tauri::command]
fn window_transparency(state: tauri::State<AppState>) -> u8 {
    state.transparency.lock().map(|value| *value).unwrap_or(0)
}

fn main() {
    set_linux_window_class();
    tauri::Builder::default()
        .manage(AppState {
            stream: Mutex::new(None),
            target: Mutex::new(None),
            transparency: Mutex::new(0),
            checks: Mutex::new(None),
        })
        .setup(|app| {
            let percent = read_transparency(&window_config_path());
            if let Ok(mut slot) = app.state::<AppState>().transparency.lock() {
                *slot = percent;
            }
            remember_target(app.state::<AppState>().inner());
            clear_window_background(app.handle());
            install_tray(app.handle(), percent)?;
            show_main(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            list_entries,
            entry_image,
            paste_entry,
            captured_window,
            hide_window,
            window_transparency
        ])
        .run(tauri::generate_context!())
        .expect("ditox gui failed to start");
}

#[cfg(test)]
mod tests {
    use super::{
        accepted_transparency, connection_plan, image_for_webview, paste_target, read_transparency, transparency_from_file,
        window_paste_target, without_blob_paths, write_transparency,
    };
    use serde_json::json;

    #[test]
    fn a_missing_socket_is_daemon_down_and_a_present_socket_connects() {
        assert_eq!(connection_plan(false), "daemon-down");
        assert_eq!(connection_plan(true), "connect");
        let production = include_str!("main.rs").split("mod tests").next().unwrap();
        assert!(!production.contains("serve --stdio"));
        assert!(!production.contains("ditoxd"));
        assert!(production.contains("Paste as text"));
        assert!(production.contains("transparency-30"));
    }

    #[test]
    fn transparency_accepts_only_the_menu_steps() {
        assert_eq!(accepted_transparency(0), 0);
        assert_eq!(accepted_transparency(10), 10);
        assert_eq!(accepted_transparency(20), 20);
        assert_eq!(accepted_transparency(30), 30);
        assert_eq!(accepted_transparency(15), 0);
        assert_eq!(accepted_transparency(300), 0);
        assert_eq!(transparency_from_file(r#"{"transparency":20}"#), 20);
        assert_eq!(transparency_from_file(r#"{"transparency":15}"#), 0);
        assert_eq!(transparency_from_file("not json"), 0);
        let dir = std::env::temp_dir().join(format!("ditox-window-test-{}", std::process::id()));
        let path = dir.join("window.json");
        write_transparency(&path, 30).unwrap();
        assert_eq!(read_transparency(&path), 30);
        write_transparency(&path, 7).unwrap();
        assert_eq!(read_transparency(&path), 0);
        let _ = std::fs::remove_dir_all(dir);
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
        let image = image_for_webview(json!({"result":{"mime":"image/png","data":"/w==","blob_path":"/tmp/secret.png"}})).unwrap();
        assert_eq!(image, json!({"mime":"image/png","data":"/w=="}));
        assert!(image_for_webview(json!({"result":{"mime":"image/png","data":"/tmp/secret.png"}})).is_err());
        assert_eq!(
            window_paste_target(r#"{"class":"firefox","address":"0xabc"}"#).as_deref(),
            Some("0xabc")
        );
        assert_eq!(paste_target("dev.ditox.gui", "0xself"), None);
    }
}
