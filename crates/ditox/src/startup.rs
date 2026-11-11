//! What the process arranges before the window exists, so that it stays small.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const SELECT: &str = "VK_LOADER_DRIVERS_SELECT";
/// Set for the second try, after a window that could not open with the drivers picked.
const EVERY_DRIVER: &str = "DITOX_ALL_GPU_DRIVERS";
/// Any of these means the user already chose which drivers load.
const USER_CHOICES: [&str; 5] = [
    SELECT,
    "VK_LOADER_DRIVERS_DISABLE",
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "VK_ADD_DRIVER_FILES",
];

static PICKED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub driver: String,
    pub connected: bool,
}

/// Call first in `main`, before any thread exists: it changes the environment.
pub fn prepare() {
    return_freed_memory();
    let chosen = USER_CHOICES
        .iter()
        .chain([&EVERY_DRIVER])
        .any(|name| std::env::var_os(name).is_some());
    if chosen {
        return;
    }
    if let Some(drivers) = vulkan_drivers(&cards(Path::new("/sys/class/drm"))) {
        // SAFETY: nothing else runs yet; `main` calls this before it starts a thread.
        unsafe { std::env::set_var(SELECT, drivers) };
        PICKED.store(true, Ordering::Relaxed);
    }
}

/// Starts the program again with every Vulkan driver allowed, when this run
/// had picked some and the window could not open with them. Returns when
/// there is nothing more to try.
pub fn retry_with_every_driver() {
    use std::os::unix::process::CommandExt;
    if !PICKED.load(Ordering::Relaxed) {
        return;
    }
    let Ok(program) = std::env::current_exe() else {
        return;
    };
    let failed = std::process::Command::new(program)
        .args(std::env::args_os().skip(1))
        .env_remove(SELECT)
        .env(EVERY_DRIVER, "1")
        .exec();
    eprintln!("ditox: could not start again: {failed}");
}

/// The Vulkan drivers worth loading, as the loader's `VK_LOADER_DRIVERS_SELECT`
/// reads them. Left alone, the loader opens every driver installed: on a
/// machine with one GPU that is a dozen libraries and LLVM for the software
/// renderer, tens of megabytes this window never draws with.
///
/// The cards that drive a screen are the ones the compositor draws on; a
/// laptop's second GPU stays asleep. A card this does not know means no
/// choice at all.
pub fn vulkan_drivers(cards: &[Card]) -> Option<String> {
    let lit: Vec<&Card> = cards.iter().filter(|card| card.connected).collect();
    let used = if lit.is_empty() {
        cards.iter().collect()
    } else {
        lit
    };
    let mut drivers: Vec<&str> = Vec::new();
    for card in used {
        // The names the drivers' manifests go by: Mesa's and the vendors' own.
        let named: &[&str] = match card.driver.as_str() {
            "nvidia" => &["nvidia*"],
            "amdgpu" | "radeon" => &["radeon*", "amd_icd*"],
            "i915" | "xe" => &["intel*"],
            "nouveau" => &["nouveau*"],
            _ => return None,
        };
        for driver in named {
            if !drivers.contains(driver) {
                drivers.push(driver);
            }
        }
    }
    (!drivers.is_empty()).then(|| drivers.join(","))
}

/// The graphics cards under `/sys/class/drm`: each `cardN`, the kernel driver
/// bound to it, and whether one of its outputs has a screen.
pub fn cards(drm: &Path) -> Vec<Card> {
    let names: Vec<String> = std::fs::read_dir(drm)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    let mut cards = Vec::new();
    for name in &names {
        let Some(number) = name.strip_prefix("card") else {
            continue;
        };
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let driver = std::fs::read_link(drm.join(name).join("device/driver"))
            .ok()
            .and_then(|link| {
                link.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        let output = format!("{name}-");
        let connected = names
            .iter()
            .filter(|other| other.starts_with(&output))
            .any(|other| {
                std::fs::read_to_string(drm.join(other).join("status"))
                    .is_ok_and(|status| status.trim() == "connected")
            });
        cards.push(Card { driver, connected });
    }
    cards
}

/// glibc keeps freed memory by default: one arena per thread, and a threshold
/// for handing large blocks back that only ever grows. A picker that showed
/// one screenshot would stay that much heavier. Two arenas and fixed
/// thresholds give the memory back when it is freed.
fn return_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: `mallopt` only sets allocator parameters; no allocation is in flight on another thread.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
        libc::mallopt(libc::M_MMAP_THRESHOLD, 128 * 1024);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 256 * 1024);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(driver: &str, connected: bool) -> Card {
        Card {
            driver: driver.into(),
            connected,
        }
    }

    #[test]
    fn only_the_cards_that_drive_a_screen_have_their_driver_loaded() {
        assert_eq!(
            vulkan_drivers(&[card("amdgpu", false), card("nvidia", true)]).as_deref(),
            Some("nvidia*")
        );
        assert_eq!(
            vulkan_drivers(&[card("i915", true), card("nvidia", false)]).as_deref(),
            Some("intel*")
        );
        assert_eq!(
            vulkan_drivers(&[
                card("amdgpu", true),
                card("nvidia", true),
                card("radeon", true)
            ])
            .as_deref(),
            Some("radeon*,amd_icd*,nvidia*")
        );
    }

    #[test]
    fn with_no_screen_known_every_card_counts_and_an_unknown_card_means_no_choice() {
        assert_eq!(
            vulkan_drivers(&[card("xe", false), card("nouveau", false)]).as_deref(),
            Some("intel*,nouveau*")
        );
        assert_eq!(vulkan_drivers(&[]), None);
        assert_eq!(vulkan_drivers(&[card("virtio_gpu", true)]), None);
        assert_eq!(
            vulkan_drivers(&[card("nvidia", true), card("", true)]),
            None
        );
        assert_eq!(
            vulkan_drivers(&[card("nvidia", true), card("asahi", false)]).as_deref(),
            Some("nvidia*"),
            "a card without a screen is not asked about"
        );
    }

    #[test]
    fn cards_are_read_from_the_drm_directory() {
        let root = std::env::temp_dir().join(format!("ditox-drm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let drm = root.join("class/drm");
        for (name, driver) in [("card0", "amdgpu"), ("card1", "nvidia")] {
            let device = root.join("devices").join(name);
            std::fs::create_dir_all(&device).unwrap();
            std::fs::create_dir_all(root.join("drivers").join(driver)).unwrap();
            std::os::unix::fs::symlink(root.join("drivers").join(driver), device.join("driver"))
                .unwrap();
            std::fs::create_dir_all(drm.join(name)).unwrap();
            std::os::unix::fs::symlink(&device, drm.join(name).join("device")).unwrap();
        }
        for (output, status) in [
            ("card0-DP-1", "disconnected\n"),
            ("card1-DP-3", "disconnected\n"),
            ("card1-HDMI-A-1", "connected\n"),
        ] {
            std::fs::create_dir_all(drm.join(output)).unwrap();
            std::fs::write(drm.join(output).join("status"), status).unwrap();
        }
        std::fs::create_dir_all(drm.join("renderD128")).unwrap();
        std::fs::write(drm.join("version"), "drm 1.1.0").unwrap();

        let mut found = cards(&drm);
        found.sort_by(|a, b| a.driver.cmp(&b.driver));
        assert_eq!(found, [card("amdgpu", false), card("nvidia", true)]);
        assert_eq!(cards(&root.join("nowhere")), []);
        let _ = std::fs::remove_dir_all(&root);
    }
}
