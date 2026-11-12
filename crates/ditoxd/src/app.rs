use std::borrow::Cow;

use ditox_rpc::{ConfigView, IngestResult, Kind, PasteMode, PasteResult, Settings, WatcherStatus};

use crate::clipboard::{Clipboard, ClipboardRead, MimeList};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::hyprland::{self, ActiveWindow};
use crate::image;
use crate::policy::{self, CaptureFacts, OfferOutcome, OfferPart};
use crate::storage::{Offer, OfferFormat, Storage};
use crate::text;

pub trait Desktop {
    fn active_window(&self) -> Option<ActiveWindow>;
    fn window_class(&self, address: &str) -> Option<String>;
    fn send_paste_shortcut(
        &self,
        target_window: Option<&str>,
        buffer_ms: u32,
        keybind: &str,
    ) -> Result<()>;
}

pub struct Hyprland;

impl Desktop for Hyprland {
    fn active_window(&self) -> Option<ActiveWindow> {
        hyprland::active_window()
    }

    fn window_class(&self, address: &str) -> Option<String> {
        hyprland::window_class(address)
    }

    fn send_paste_shortcut(
        &self,
        target_window: Option<&str>,
        buffer_ms: u32,
        keybind: &str,
    ) -> Result<()> {
        hyprland::send_paste_shortcut(target_window, buffer_ms, keybind)
    }
}

pub struct NoDesktop;

impl Desktop for NoDesktop {
    fn active_window(&self) -> Option<ActiveWindow> {
        None
    }

    fn window_class(&self, _: &str) -> Option<String> {
        None
    }

    fn send_paste_shortcut(&self, _: Option<&str>, _: u32, _: &str) -> Result<()> {
        Ok(())
    }
}

pub struct App {
    pub cfg: Config,
    pub store: Storage,
    pub clipboard: Clipboard,
    pub desktop: Box<dyn Desktop>,
    pub watch_running: bool,
    revision: u64,
    capture_pending: bool,
}

impl App {
    pub fn open(cfg: Config, clipboard: Clipboard, desktop: Box<dyn Desktop>) -> Result<App> {
        let store = Storage::open(&cfg)?;
        store.apply_retention(&cfg)?;
        Ok(App {
            cfg,
            store,
            clipboard,
            desktop,
            watch_running: false,
            revision: 1,
            capture_pending: false,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn touch(&mut self) {
        self.revision += 1;
    }

    pub fn request_capture(&mut self) {
        self.capture_pending = true;
    }

    pub fn take_capture_request(&mut self) -> bool {
        std::mem::take(&mut self.capture_pending)
    }

    pub fn config_view(&self) -> ConfigView {
        ConfigView {
            settings: self.cfg.settings(),
            path: self.cfg.config_path.clone(),
            writable: self.cfg.writable(),
            database: self.cfg.db_path.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    /// Takes new settings: checks them, writes the file, then lives by them.
    /// A setting the daemon cannot take, or a file it cannot write, changes nothing.
    pub fn set_settings(&mut self, settings: &Settings) -> Result<ConfigView> {
        let mut next = self.cfg.clone();
        next.apply_settings(settings)?;
        next.save()?;
        self.cfg = next;
        self.store.apply_retention(&self.cfg)?;
        Ok(self.config_view())
    }

    pub fn watcher_status(&self) -> Result<WatcherStatus> {
        Ok(WatcherStatus {
            running: self.watch_running,
            paused: self.store.is_watcher_paused()?,
            backend: crate::clipboard::BACKEND.into(),
            last_error: self.store.watcher_last_error()?,
        })
    }

    pub fn capture(&mut self) {
        match self.capture_current_offer() {
            Ok(result) => {
                let _ = self.store.clear_watcher_error();
                if result.is_some_and(|result| result.stored) {
                    self.touch();
                }
            }
            Err(err) => {
                eprintln!("ditoxd: capture failed: {err}");
                let _ = self.store.set_watcher_error(&err.to_string());
            }
        }
    }

    pub fn capture_current_offer(&mut self) -> Result<Option<IngestResult>> {
        if self.store.is_watcher_paused()? {
            return Ok(None);
        }
        let types = match self.clipboard.list_mime_types() {
            MimeList::Types(types) => types,
            MimeList::Empty => return Ok(None),
            MimeList::Failed(detail) => return Err(Error::Clipboard(detail)),
        };

        let (mut image_listed, mut image_ok) = (false, false);
        let mut failure = None;
        let mut parts: Vec<OfferPart> = Vec::new();
        for mime in &types {
            if !policy::mime_allowed(mime) {
                continue;
            }
            let base = policy::mime_base(mime);
            let is_image = base.starts_with("image/");
            image_listed |= is_image;
            if parts.iter().any(|part| part.mime == base) {
                continue;
            }
            match self.clipboard.read_offer(mime) {
                ClipboardRead::Bytes(bytes) => {
                    if is_image && image::detect_mime(&bytes).is_none() {
                        continue;
                    }
                    image_ok |= is_image;
                    parts.push(OfferPart {
                        mime: base.to_string(),
                        bytes,
                    });
                }
                ClipboardRead::Failed(detail) => failure = Some(detail),
                ClipboardRead::Empty | ClipboardRead::TypeUnavailable => {}
            }
        }
        if policy::drop_unreadable_image(image_listed, image_ok) {
            return Ok(None);
        }
        if parts.is_empty() {
            return failure.map_or(Ok(None), |detail| Err(Error::Clipboard(detail)));
        }
        self.store_offer(&parts).map(Some)
    }

    pub fn store_offer(&mut self, parts: &[OfferPart]) -> Result<IngestResult> {
        let plan = match policy::plan_offer(parts, &self.cfg.secret_patterns) {
            OfferOutcome::Empty => return Ok(refused("empty")),
            OfferOutcome::Secret => return Ok(refused("secret")),
            OfferOutcome::Store(plan) => plan,
        };
        let primary = &parts[plan.primary];
        let is_image = plan.kind == Kind::Image;
        let image_mime = if is_image {
            image::detect_mime(&primary.bytes)
        } else {
            None
        };
        if is_image && image_mime.is_none() {
            return Ok(refused("not-image"));
        }
        let primary_bytes: Cow<'_, [u8]> = if is_image {
            Cow::Borrowed(&primary.bytes)
        } else {
            Cow::Owned(self.clean_text(&primary.bytes).into_bytes())
        };

        let source_app = match self.admit(plan.kind, &primary_bytes, image_mime)? {
            Ok(source_app) => source_app,
            Err(reason) => return Ok(refused(reason)),
        };

        let formats: Vec<OfferFormat> = plan
            .kept
            .iter()
            .map(|index| {
                let source = &parts[*index];
                let mime = policy::mime_base(&source.mime).to_string();
                let image = mime.starts_with("image/");
                let bytes = if image {
                    Cow::Borrowed(source.bytes.as_slice())
                } else {
                    Cow::Owned(self.clean_text(&source.bytes).into_bytes())
                };
                OfferFormat { mime, bytes, image }
            })
            .collect();
        let preview = plan
            .preview
            .map(|index| self.clean_text(&parts[index].bytes))
            .unwrap_or_default();
        let mime = policy::mime_base(&primary.mime);
        let id = self.store.add_offer(
            &self.cfg,
            &Offer {
                kind: plan.kind,
                mime,
                bytes: &primary_bytes,
                preview: &preview,
                source_app: source_app.as_deref(),
                formats: &formats,
                size: if is_image {
                    image::size(&primary.bytes, mime)
                } else {
                    Default::default()
                },
            },
        )?;
        Ok(IngestResult {
            stored: true,
            id: Some(id),
            reason: String::new(),
        })
    }

    pub fn ingest(&mut self, kind: Kind, bytes: &[u8]) -> Result<IngestResult> {
        match kind {
            Kind::Image => {
                let mime = image::detect_mime(bytes);
                let source_app = match self.admit(Kind::Image, bytes, mime)? {
                    Ok(source_app) => source_app,
                    Err(reason) => return Ok(refused(reason)),
                };
                let Some(mime) = mime else {
                    return Ok(refused("not-image"));
                };
                let id = self.store.add_image_from(
                    &self.cfg,
                    mime,
                    bytes,
                    image::size(bytes, mime),
                    source_app.as_deref(),
                )?;
                Ok(IngestResult {
                    stored: true,
                    id: Some(id),
                    reason: String::new(),
                })
            }
            Kind::Text => {
                let content = self.clean_text(bytes);
                let source_app = match self.admit(Kind::Text, content.as_bytes(), None)? {
                    Ok(source_app) => source_app,
                    Err(reason) => return Ok(refused(reason)),
                };
                let id = self
                    .store
                    .add_text_from(&self.cfg, &content, source_app.as_deref())?;
                Ok(IngestResult {
                    stored: true,
                    id: Some(id),
                    reason: String::new(),
                })
            }
        }
    }

    fn clean_text(&self, bytes: &[u8]) -> String {
        let sanitized = text::sanitize_text(bytes);
        text::cap_utf8(&sanitized, self.cfg.max_text_bytes).to_string()
    }

    fn admit(
        &self,
        kind: Kind,
        bytes: &[u8],
        image_mime: Option<&str>,
    ) -> Result<std::result::Result<Option<String>, &'static str>> {
        let window = self.desktop.active_window().map(|window| window.snap());
        let hash = text::sha256_hex(bytes);
        let saved = self.store.self_write_hash()?;
        let decision = policy::decide_capture(&CaptureFacts {
            kind,
            bytes,
            image_mime,
            window: window.as_ref(),
            content_hash: &hash,
            self_hash: saved.as_deref(),
            excluded_apps: &self.cfg.excluded_apps,
            excluded_windows: &self.cfg.excluded_windows,
            secret_patterns: &self.cfg.secret_patterns,
        });
        if saved.is_some() && policy::clears_self_write(decision.reason) {
            self.store.clear_self_write_hash()?;
        }
        Ok(if decision.store {
            Ok(decision.source_app)
        } else {
            Err(decision.reason)
        })
    }

    pub fn paste(
        &mut self,
        id: i64,
        target_window: Option<&str>,
        mode: PasteMode,
    ) -> Result<PasteResult> {
        let not_pasted = |reason: &str| PasteResult {
            pasted: false,
            reason: reason.into(),
        };
        let Some(entry) = self.store.get(id)? else {
            return Ok(not_pasted("missing"));
        };
        let hash = match mode {
            PasteMode::Plain => {
                let plain = match self.store.format_text(id, "text/plain")? {
                    Some(text) => text,
                    None if entry.kind == Kind::Text
                        && policy::mime_base(&entry.mime) == "text/plain" =>
                    {
                        entry.content.clone()
                    }
                    None => return Ok(not_pasted("no-plain-text")),
                };
                self.clipboard.write_text(&plain)?;
                text::sha256_hex(plain.as_bytes())
            }
            PasteMode::Original => {
                match entry.kind {
                    Kind::Image => self.clipboard.write_bytes(
                        &self.read_image_blob(entry.blob_path.as_deref())?,
                        &entry.mime,
                    )?,
                    Kind::Text => self
                        .clipboard
                        .write_bytes(entry.content.as_bytes(), &entry.mime)?,
                }
                entry.hash.clone()
            }
        };
        self.store.mark_self_write(&hash)?;
        self.store.mark_used(id)?;
        if self.cfg.paste_enabled && policy::paste_sends_shortcut(target_window.is_some()) {
            let class = target_window.and_then(|address| self.desktop.window_class(address));
            let keybind = policy::paste_keybind(
                class.as_deref(),
                &self.cfg.paste_keybind,
                &self.cfg.terminal_paste_keybind,
                &self.cfg.terminal_apps,
            );
            self.desktop
                .send_paste_shortcut(target_window, self.cfg.paste_buffer_ms, keybind)?;
        }
        Ok(PasteResult {
            pasted: true,
            reason: String::new(),
        })
    }

    pub fn read_image_blob(&self, path: Option<&str>) -> Result<Vec<u8>> {
        let path = path
            .filter(|path| !path.is_empty())
            .ok_or_else(|| Error::Io("the image has no stored file".into()))?;
        std::fs::read(path).map_err(|err| Error::Io(format!("{path}: {err}")))
    }
}

fn refused(reason: &str) -> IngestResult {
    IngestResult {
        stored: false,
        id: None,
        reason: reason.into(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use ditox_rpc::Filter;

    use super::*;
    use crate::image::tiny_png;
    use crate::storage::ListQuery;
    use crate::testing::TempDir;

    pub type Sent = Rc<RefCell<Vec<(Option<String>, String)>>>;

    #[derive(Default)]
    pub struct FakeDesktop {
        pub window: Option<ActiveWindow>,
        pub target_class: Option<String>,
        pub shortcuts: Sent,
        pub fail_paste: bool,
    }

    impl Desktop for FakeDesktop {
        fn active_window(&self) -> Option<ActiveWindow> {
            self.window.clone()
        }

        fn window_class(&self, _: &str) -> Option<String> {
            self.target_class.clone()
        }

        fn send_paste_shortcut(
            &self,
            target_window: Option<&str>,
            _: u32,
            keybind: &str,
        ) -> Result<()> {
            if self.fail_paste {
                return Err(Error::PasteBack("the window you were in is gone".into()));
            }
            self.shortcuts
                .borrow_mut()
                .push((target_window.map(String::from), keybind.to_string()));
            Ok(())
        }
    }

    pub fn app(label: &str, desktop: FakeDesktop) -> (TempDir, App) {
        let dir = TempDir::new(label);
        let cfg = Config::defaults(&dir.path("data"));
        std::fs::create_dir_all(&cfg.image_dir).unwrap();
        let clipboard = Clipboard::mock(&dir.path("clipboard"));
        (dir, App::open(cfg, clipboard, Box::new(desktop)).unwrap())
    }

    fn window(class: &str) -> Option<ActiveWindow> {
        Some(ActiveWindow {
            address: "0xabc".into(),
            class: class.into(),
            title: "Title".into(),
        })
    }

    fn part(mime: &str, bytes: &[u8]) -> OfferPart {
        OfferPart {
            mime: mime.into(),
            bytes: bytes.to_vec(),
        }
    }

    fn count(app: &App) -> usize {
        app.store
            .list(&ListQuery::new("", Filter::All, 50))
            .unwrap()
            .len()
    }

    #[test]
    fn a_copied_text_is_captured_once_with_its_source_window() {
        let (_dir, mut app) = app(
            "capture",
            FakeDesktop {
                window: window("firefox"),
                ..Default::default()
            },
        );
        app.clipboard.write_text("copied \x1b[1mtext").unwrap();
        let stored = app.capture_current_offer().unwrap().unwrap();
        assert!(stored.stored);
        let entry = app.store.get(stored.id.unwrap()).unwrap().unwrap();
        assert_eq!(entry.content, "copied text");
        assert_eq!(entry.source_app.as_deref(), Some("firefox"));
        assert_eq!(app.capture_current_offer().unwrap().unwrap().id, stored.id);
        assert_eq!(count(&app), 1);

        let before = app.revision();
        app.clipboard.write_text("second").unwrap();
        app.capture();
        assert_eq!(app.revision(), before + 1);
        assert_eq!(count(&app), 2);
    }

    #[test]
    fn nothing_is_captured_while_paused_from_an_excluded_app_or_for_a_secret() {
        let (_dir, mut app) = app(
            "refuse",
            FakeDesktop {
                window: window("org.keepassxc.KeePassXC"),
                ..Default::default()
            },
        );
        app.clipboard.write_text("hunter2").unwrap();
        assert_eq!(
            app.capture_current_offer().unwrap().unwrap().reason,
            "excluded"
        );

        app.desktop = Box::new(FakeDesktop::default());
        app.clipboard
            .write_text("token ghp_0123456789abcdef")
            .unwrap();
        assert_eq!(
            app.capture_current_offer().unwrap().unwrap().reason,
            "secret"
        );

        app.clipboard.write_text("fine").unwrap();
        app.store.pause_watcher(0).unwrap();
        assert_eq!(app.capture_current_offer().unwrap(), None);
        app.store.resume_watcher().unwrap();
        assert!(app.capture_current_offer().unwrap().unwrap().stored);
        assert_eq!(count(&app), 1);
    }

    #[test]
    fn an_image_that_is_not_an_image_is_dropped_and_not_replaced_by_text() {
        let (_dir, mut app) = app("fake-image", FakeDesktop::default());
        app.clipboard
            .write_bytes(b"https://example.com/a.png", "image/png")
            .unwrap();
        assert_eq!(app.capture_current_offer().unwrap(), None);
        assert_eq!(count(&app), 0);

        app.clipboard
            .write_bytes(&tiny_png(2, 3), "image/png")
            .unwrap();
        let stored = app.capture_current_offer().unwrap().unwrap();
        let entry = app.store.get(stored.id.unwrap()).unwrap().unwrap();
        assert_eq!(
            (entry.kind, entry.image_width, entry.image_height),
            (Kind::Image, Some(2), Some(3))
        );
    }

    #[test]
    fn an_offer_with_an_image_and_its_link_is_one_row() {
        let (_dir, mut app) = app("offer", FakeDesktop::default());
        let image = tiny_png(8, 8);
        let stored = app
            .store_offer(&[
                part("text/plain;charset=utf-8", b"https://example.com/a"),
                part("image/png", &image),
            ])
            .unwrap();
        let entry = app.store.get(stored.id.unwrap()).unwrap().unwrap();
        assert_eq!(entry.kind, Kind::Image);
        assert!(entry.preview.contains("example.com/a"));
        assert_eq!(
            app.store.format_mimes(entry.id).unwrap(),
            ["image/png", "text/plain"]
        );
        assert_eq!(app.store_offer(&[]).unwrap().reason, "empty");
        assert_eq!(
            app.store_offer(&[part("image/png", b"not a png")])
                .unwrap()
                .reason,
            "not-image"
        );
    }

    #[test]
    fn a_paste_writes_the_clip_marks_it_used_and_is_not_captured_again() {
        let shortcuts = Rc::new(RefCell::new(Vec::new()));
        let (_dir, mut app) = app(
            "paste",
            FakeDesktop {
                shortcuts: shortcuts.clone(),
                ..Default::default()
            },
        );
        let first = app.store.add_text(&app.cfg, "first").unwrap();
        app.store.add_text(&app.cfg, "second").unwrap();

        assert!(
            app.paste(first, Some("0xabc"), PasteMode::Original)
                .unwrap()
                .pasted
        );
        assert_eq!(
            app.clipboard.read_offer("text/plain"),
            ClipboardRead::Bytes(b"first".to_vec())
        );
        assert!(
            app.store
                .get(first)
                .unwrap()
                .unwrap()
                .last_used_at_ms
                .is_some()
        );
        assert_eq!(
            *shortcuts.borrow(),
            [(Some("0xabc".to_string()), "ctrl+v".to_string())]
        );

        assert_eq!(app.capture_current_offer().unwrap().unwrap().reason, "self");
        assert_eq!(app.store.self_write_hash().unwrap(), None);
        assert_eq!(
            app.paste(999, None, PasteMode::Original).unwrap().reason,
            "missing"
        );
        assert_eq!(count(&app), 2);
    }

    #[test]
    fn a_paste_into_a_terminal_sends_the_terminal_shortcut() {
        let shortcuts = Rc::new(RefCell::new(Vec::new()));
        let (_dir, mut app) = app(
            "paste-terminal",
            FakeDesktop {
                shortcuts: shortcuts.clone(),
                target_class: Some("com.mitchellh.ghostty".into()),
                ..Default::default()
            },
        );
        app.cfg.terminal_apps = vec!["ghostty".into()];
        let id = app.store.add_text(&app.cfg, "ls -la").unwrap();
        assert!(
            app.paste(id, Some("0xabc"), PasteMode::Original)
                .unwrap()
                .pasted
        );
        app.cfg.terminal_paste_keybind = "shift+insert".into();
        assert!(
            app.paste(id, Some("0xabc"), PasteMode::Original)
                .unwrap()
                .pasted
        );
        app.cfg.terminal_apps.clear();
        assert!(
            app.paste(id, Some("0xabc"), PasteMode::Original)
                .unwrap()
                .pasted
        );
        let sent: Vec<String> = shortcuts
            .borrow()
            .iter()
            .map(|(_, keybind)| keybind.clone())
            .collect();
        assert_eq!(sent, ["ctrl+shift+v", "shift+insert", "ctrl+v"]);
    }

    #[test]
    fn without_a_window_or_with_paste_off_the_clip_is_only_copied() {
        let shortcuts = Rc::new(RefCell::new(Vec::new()));
        let (_dir, mut app) = app(
            "copy-only",
            FakeDesktop {
                shortcuts: shortcuts.clone(),
                ..Default::default()
            },
        );
        let id = app.store.add_text(&app.cfg, "clip").unwrap();
        assert!(app.paste(id, None, PasteMode::Original).unwrap().pasted);
        app.cfg.paste_enabled = false;
        assert!(
            app.paste(id, Some("0xabc"), PasteMode::Original)
                .unwrap()
                .pasted
        );
        assert!(shortcuts.borrow().is_empty());
        assert_eq!(
            app.clipboard.read_offer("text/plain"),
            ClipboardRead::Bytes(b"clip".to_vec())
        );
    }

    #[test]
    fn a_paste_that_cannot_reach_its_window_fails_with_the_clip_on_the_clipboard() {
        let (_dir, mut app) = app(
            "paste-fail",
            FakeDesktop {
                fail_paste: true,
                ..Default::default()
            },
        );
        let id = app.store.add_text(&app.cfg, "clip").unwrap();
        let err = app
            .paste(id, Some("0xdead"), PasteMode::Original)
            .unwrap_err();
        assert!(err.to_string().contains("gone"));
        assert_eq!(
            app.clipboard.read_offer("text/plain"),
            ClipboardRead::Bytes(b"clip".to_vec())
        );
        assert_eq!(app.capture_current_offer().unwrap().unwrap().reason, "self");
    }

    #[test]
    fn plain_paste_uses_the_text_sibling_and_refuses_a_bare_image() {
        let (_dir, mut app) = app("plain", FakeDesktop::default());
        let image = tiny_png(1, 1);
        let with_link = app
            .store_offer(&[
                part("image/png", &image),
                part("text/plain", b"https://example.com/a"),
            ])
            .unwrap()
            .id
            .unwrap();
        assert!(app.paste(with_link, None, PasteMode::Plain).unwrap().pasted);
        assert_eq!(
            app.clipboard.read_offer("text/plain"),
            ClipboardRead::Bytes(b"https://example.com/a".to_vec())
        );

        let mut other = tiny_png(1, 1);
        other.push(1);
        let bare = app
            .store
            .add_image(&app.cfg, "image/png", &other, Default::default())
            .unwrap();
        assert_eq!(
            app.paste(bare, None, PasteMode::Plain).unwrap().reason,
            "no-plain-text"
        );

        assert!(app.paste(bare, None, PasteMode::Original).unwrap().pasted);
        assert_eq!(
            app.clipboard.read_offer("image/png"),
            ClipboardRead::Bytes(other)
        );
    }

    #[test]
    fn ingest_stores_text_and_images_and_refuses_what_is_not_an_image() {
        let (_dir, mut app) = app("ingest", FakeDesktop::default());
        assert!(app.ingest(Kind::Text, b"from stdin").unwrap().stored);
        assert!(app.ingest(Kind::Image, &tiny_png(1, 1)).unwrap().stored);
        assert_eq!(
            app.ingest(Kind::Image, b"nope").unwrap().reason,
            "not-image"
        );
        assert_eq!(app.ingest(Kind::Text, b"").unwrap().reason, "empty");
        assert_eq!(count(&app), 2);
    }

    #[test]
    fn a_failed_capture_is_reported_and_cleared_by_the_next_good_one() {
        let (_dir, mut app) = app("status", FakeDesktop::default());
        app.store.set_watcher_error("clipboard: exit 1").unwrap();
        assert_eq!(
            app.watcher_status().unwrap().last_error.as_deref(),
            Some("clipboard: exit 1")
        );
        app.clipboard.write_text("ok").unwrap();
        app.capture();
        let status = app.watcher_status().unwrap();
        assert_eq!(
            (status.last_error, status.paused, status.running),
            (None, false, false)
        );
    }
}
