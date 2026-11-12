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

/// A link at `path` to the file `target`. Windows lets only some users make
/// links; `false` says this one could not, and what depends on it is skipped.
pub fn link(target: impl AsRef<std::path::Path>, path: impl AsRef<std::path::Path>) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, path);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, path);
    made.is_ok() || cfg!(unix) && panic!("a link could not be made")
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
