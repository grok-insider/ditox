//! The window a clip is pasted into, and the keys that paste it.

use std::time::{Duration, Instant};

use ditox_rpc::win;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};

use super::format::{self, Chord};
use crate::app::Desktop;
use crate::error::{Error, Result};
use crate::hyprland::ActiveWindow;

/// How long the window a clip goes to may take to have the keyboard again.
const FOCUS_WAIT: Duration = Duration::from_millis(600);
/// How long the keys that chose the clip may still be held down.
const RELEASE_WAIT: Duration = Duration::from_millis(300);
const STEP: Duration = Duration::from_millis(10);
const MODIFIERS: [u16; 5] = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN];

pub struct Windows;

impl Desktop for Windows {
    fn active_window(&self) -> Option<ActiveWindow> {
        win::foreground().map(|window| ActiveWindow {
            address: window.address,
            class: window.class,
            title: window.title,
        })
    }

    fn window_class(&self, address: &str) -> Option<String> {
        win::class_of(address)
    }

    fn send_paste_shortcut(
        &self,
        target_window: Option<&str>,
        buffer_ms: u32,
        keybind: &str,
    ) -> Result<()> {
        let chord = paste_chord(keybind)?;
        std::thread::sleep(Duration::from_millis(u64::from(buffer_ms)));
        if let Some(address) = target_window
            && !give_keyboard_to(address)
        {
            return Err(Error::PasteBack(
                "the window you were in is gone; the clip is on the clipboard".into(),
            ));
        }
        wait_for_release(&chord);
        if press(&chord) {
            Ok(())
        } else {
            Err(Error::PasteBack(
                "Windows did not send the paste shortcut; the clip is on the clipboard".into(),
            ))
        }
    }
}

/// Whether `keybind` names a shortcut Windows can be asked to send.
pub fn check_paste_shortcut(keybind: &str) -> Result<()> {
    paste_chord(keybind).map(drop)
}

fn paste_chord(keybind: &str) -> Result<Chord> {
    format::chord(keybind)
        .ok_or_else(|| Error::Config(format!("\"{keybind}\" is not a paste keybind")))
}

/// The list has the keyboard while it asks for the paste. Windows lets a
/// program hand the keyboard on only when the one that has it agrees, and the
/// list does before it asks. The window may still take a moment to have it.
fn give_keyboard_to(address: &str) -> bool {
    if win::handle(address).is_none() {
        return false;
    }
    let deadline = Instant::now() + FOCUS_WAIT;
    let mut next_ask = Instant::now();
    loop {
        if win::is_foreground(address) {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        if now >= next_ask {
            win::focus(address);
            next_ask = now + FOCUS_WAIT / 6;
        }
        std::thread::sleep(STEP);
    }
}

fn held(key: u16) -> bool {
    // SAFETY: asks whether one key is down; any key number is a valid question.
    unsafe { GetAsyncKeyState(i32::from(key)) < 0 }
}

/// A modifier still held from choosing the clip would join the shortcut:
/// Ctrl+V with Shift down is another command.
fn wait_for_release(chord: &Chord) {
    let wanted = |key: u16| match key {
        VK_CONTROL => chord.ctrl,
        VK_SHIFT => chord.shift,
        VK_MENU => chord.alt,
        _ => chord.win,
    };
    let stray = || MODIFIERS.iter().any(|key| held(*key) && !wanted(*key));
    let deadline = Instant::now() + RELEASE_WAIT;
    while stray() && Instant::now() < deadline {
        std::thread::sleep(STEP);
    }
    let released: Vec<INPUT> = MODIFIERS
        .iter()
        .filter(|key| held(**key) && !wanted(**key))
        .map(|key| key_event(*key, true))
        .collect();
    send(&released);
}

fn press(chord: &Chord) -> bool {
    let modifiers: Vec<u16> = [
        (chord.ctrl, VK_CONTROL),
        (chord.shift, VK_SHIFT),
        (chord.alt, VK_MENU),
        (chord.win, VK_LWIN),
    ]
    .into_iter()
    .filter_map(|(on, key)| on.then_some(key))
    .collect();
    let mut events: Vec<INPUT> = modifiers.iter().map(|key| key_event(*key, false)).collect();
    events.push(key_event(chord.key, false));
    events.push(key_event(chord.key, true));
    events.extend(modifiers.iter().rev().map(|key| key_event(*key, true)));
    send(&events)
}

fn key_event(key: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(events: &[INPUT]) -> bool {
    if events.is_empty() {
        return true;
    }
    // SAFETY: `events` is `len` inputs of the size Windows is told.
    let sent = unsafe {
        SendInput(
            events.len() as u32,
            events.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    sent as usize == events.len()
}
