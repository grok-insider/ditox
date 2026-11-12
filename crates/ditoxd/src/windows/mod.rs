//! Ditox on Windows: the clipboard, the window that has the keyboard, the
//! paste shortcut and the key that opens the list, all asked of Windows
//! itself where Linux asks `wl-clipboard` and Hyprland.

pub mod format;

#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod clipboard;
#[cfg(windows)]
pub mod desktop;
#[cfg(windows)]
pub mod listener;
