use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::config::Config;
use crate::storage::Storage;

pub struct TempDir {
    root: PathBuf,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "ditoxd-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        TempDir { root }
    }

    pub fn path(&self, name: &str) -> String {
        self.root.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub struct TestStore {
    pub dir: TempDir,
    pub cfg: Config,
    pub store: Storage,
}

pub fn store(label: &str) -> TestStore {
    let dir = TempDir::new(label);
    let mut cfg = Config::defaults(&dir.path("data"));
    cfg.excluded_apps.clear();
    cfg.secret_patterns.clear();
    std::fs::create_dir_all(&cfg.image_dir).expect("image dir");
    let store = Storage::open(&cfg).expect("open store");
    TestStore { dir, cfg, store }
}
