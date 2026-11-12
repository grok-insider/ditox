//! Starting the daemon with the session: the user's `Run` key, where Linux
//! has a systemd user unit.

use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
    RegSetValueExW,
};

use crate::error::{Error, Result};

const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const NAME: &str = "Ditox";

/// The command the session runs: this program, as the daemon.
pub fn command(exe: &str) -> String {
    format!("\"{exe}\" daemon")
}

/// Start the daemon each time the user signs in.
pub fn install(exe: &str) -> Result<()> {
    let value = wide(&command(exe));
    with_run_key(|key| {
        // SAFETY: `value` is a NUL-terminated string of the byte length given.
        unsafe {
            RegSetValueExW(
                key,
                wide(NAME).as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        }
    })
}

/// Stop starting the daemon with the session. Not being there is not an error.
pub fn uninstall() -> Result<()> {
    with_run_key(|key| {
        // SAFETY: the name is a NUL-terminated string that outlives the call.
        match unsafe { RegDeleteValueW(key, wide(NAME).as_ptr()) } {
            ERROR_FILE_NOT_FOUND => 0,
            status => status,
        }
    })
}

fn with_run_key(change: impl FnOnce(HKEY) -> u32) -> Result<()> {
    let failed = |status: u32| {
        Error::Config(format!(
            "the session's start-up list: {}",
            std::io::Error::from_raw_os_error(status as i32)
        ))
    };
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: `key` receives a handle that is closed below on every path.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide(RUN).as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if opened != 0 {
        return Err(failed(opened));
    }
    let status = change(key);
    // SAFETY: `key` was opened above.
    unsafe { RegCloseKey(key) };
    if status == 0 {
        Ok(())
    } else {
        Err(failed(status))
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
