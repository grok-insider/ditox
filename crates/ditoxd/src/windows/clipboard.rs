//! The Windows clipboard, read and written as the Wayland one is: by MIME
//! type. Every `unsafe` block of the daemon's clipboard is in this file.

use std::io::Cursor;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Shell::{DROPFILES, DragQueryFileW};

use super::format::{self, Formats};
use crate::clipboard::{ClipboardRead, MimeList};
use crate::error::{Error, Result};
use crate::policy::{self, MAX_IMAGE_READ_BYTES, MAX_TEXT_READ_BYTES};

const OPEN_TRIES: u32 = 40;
const OPEN_STEP: Duration = Duration::from_millis(25);
const TEXT: u32 = CF_UNICODETEXT as u32;
const BITMAP: u32 = CF_DIB as u32;
const FILES: u32 = CF_HDROP as u32;

/// The clipboard, open for this thread until dropped. Only one program has
/// it open at a time, so it is asked for again for a second.
struct Open;

impl Open {
    fn new() -> std::result::Result<Open, String> {
        for _ in 0..OPEN_TRIES {
            // SAFETY: a null window makes the calling task the opener.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(Open);
            }
            std::thread::sleep(OPEN_STEP);
        }
        Err("another program is holding the clipboard".into())
    }

    fn has(&self, format: u32) -> bool {
        // SAFETY: asks about a format number; any number is a valid question.
        unsafe { IsClipboardFormatAvailable(format) != 0 }
    }

    fn formats(&self) -> Formats {
        Formats {
            text: self.has(TEXT),
            html: self.has(named("HTML Format")),
            files: self.has(FILES),
            png: self.has(named("PNG")) || self.has(named("image/png")),
            jpeg: self.has(named("JFIF")) || self.has(named("image/jpeg")),
            bitmap: self.has(BITMAP),
        }
    }

    /// Whether the program that set the clipboard asked for its content to
    /// stay out of clipboard histories, as password managers do.
    fn is_private(&self) -> bool {
        if self.has(named("ExcludeClipboardContentFromMonitorProcessing")) {
            return true;
        }
        let history = named("CanIncludeInClipboardHistory");
        self.has(history)
            && self
                .bytes(history, 64)
                .is_some_and(|value| value.iter().take(4).all(|byte| *byte == 0))
    }

    fn is_empty(&self) -> bool {
        // SAFETY: the clipboard is open; 0 asks for the first format.
        unsafe { EnumClipboardFormats(0) == 0 }
    }

    /// The bytes held under `format`, or `None` when there are none or
    /// more than `limit`.
    fn bytes(&self, format: u32, limit: usize) -> Option<Vec<u8>> {
        // SAFETY: the clipboard is open, so the handle stays the clipboard's
        // and valid until it is closed. It is locked while its `GlobalSize`
        // bytes are copied out, and unlocked before returning.
        unsafe {
            let handle = GetClipboardData(format);
            if handle.is_null() {
                return None;
            }
            let size = GlobalSize(handle);
            if size == 0 || size > limit {
                return None;
            }
            let data = GlobalLock(handle) as *const u8;
            if data.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(data, size).to_vec();
            GlobalUnlock(handle);
            Some(bytes)
        }
    }

    fn first(&self, names: &[&str], limit: usize) -> Option<Vec<u8>> {
        names
            .iter()
            .map(|name| named(name))
            .filter(|format| self.has(*format))
            .find_map(|format| self.bytes(format, limit))
    }

    fn text(&self) -> Option<Vec<u8>> {
        // Twice the limit: UTF-16 is at most twice as long as its UTF-8.
        let bytes = self.bytes(TEXT, MAX_TEXT_READ_BYTES * 2)?;
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .take_while(|unit| *unit != 0)
            .collect();
        Some(String::from_utf16_lossy(&units).into_bytes())
    }

    fn files(&self) -> Option<Vec<String>> {
        // SAFETY: the clipboard is open and `CF_HDROP` data is a drop
        // handle. Each name is written into a buffer of the length asked.
        unsafe {
            let drop = GetClipboardData(FILES);
            if drop.is_null() {
                return None;
            }
            let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
            let mut paths = Vec::with_capacity(count as usize);
            for index in 0..count {
                let length = DragQueryFileW(drop, index, std::ptr::null_mut(), 0) as usize;
                let mut name = vec![0u16; length + 1];
                let written = DragQueryFileW(drop, index, name.as_mut_ptr(), name.len() as u32);
                paths.push(String::from_utf16_lossy(&name[..written as usize]));
            }
            Some(paths)
        }
    }

    /// Hands `bytes` to the clipboard under `format`.
    fn set(&self, format: u32, bytes: &[u8]) -> std::result::Result<(), String> {
        // SAFETY: the block is allocated with room for `bytes`, filled while
        // locked, and from `SetClipboardData` on belongs to the clipboard;
        // it is freed here only when the clipboard did not take it.
        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
            if handle.is_null() {
                return Err("no memory for the clip".into());
            }
            let data = GlobalLock(handle) as *mut u8;
            if data.is_null() {
                GlobalFree(handle);
                return Err("no memory for the clip".into());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
            GlobalUnlock(handle);
            if SetClipboardData(format, handle as HANDLE).is_null() {
                GlobalFree(handle);
                return Err(format!(
                    "Windows did not take the clip ({})",
                    std::io::Error::last_os_error()
                ));
            }
        }
        Ok(())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: this thread opened the clipboard in `Open::new`.
        unsafe { CloseClipboard() };
    }
}

fn named(name: &str) -> u32 {
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    // SAFETY: `wide` is a NUL-terminated string that outlives the call.
    unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
}

pub fn list() -> MimeList {
    let open = match Open::new() {
        Ok(open) => open,
        Err(detail) => return MimeList::Failed(detail),
    };
    // What a program marked as private is not offered at all: Wayland says
    // the same of a whole offer with `CLIPBOARD_STATE=sensitive`.
    if open.is_empty() || open.is_private() {
        return MimeList::Empty;
    }
    let types = format::offers(open.formats());
    if types.is_empty() {
        MimeList::Empty
    } else {
        MimeList::Types(types)
    }
}

pub fn read(mime: &str) -> ClipboardRead {
    let open = match Open::new() {
        Ok(open) => open,
        Err(detail) => return ClipboardRead::Failed(detail),
    };
    let found = match policy::mime_base(mime) {
        "text/plain" => open.text(),
        "text/html" => open
            .first(&["HTML Format"], MAX_TEXT_READ_BYTES)
            .and_then(|html| format::html_fragment(&html)),
        "text/uri-list" => open
            .files()
            .map(|paths| format::uri_list(&paths).into_bytes()),
        "image/png" => open
            .first(&["PNG", "image/png"], MAX_IMAGE_READ_BYTES)
            .or_else(|| {
                open.bytes(BITMAP, MAX_IMAGE_READ_BYTES)
                    .and_then(|bitmap| png_of(&bitmap))
            }),
        "image/jpeg" => open.first(&["JFIF", "image/jpeg"], MAX_IMAGE_READ_BYTES),
        _ => return ClipboardRead::TypeUnavailable,
    };
    match found {
        Some(bytes) if !bytes.is_empty() => ClipboardRead::Bytes(bytes),
        Some(_) => ClipboardRead::Empty,
        None => ClipboardRead::TypeUnavailable,
    }
}

pub fn write(bytes: &[u8], mime: Option<&str>) -> Result<()> {
    let failed = |detail: String| Error::Clipboard(detail);
    let base = mime.map_or("text/plain", policy::mime_base);
    let open = Open::new().map_err(failed)?;
    // SAFETY: the clipboard is open for this thread.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(failed(format!(
            "Windows did not clear the clipboard ({})",
            std::io::Error::last_os_error()
        )));
    }
    let text = || String::from_utf8_lossy(bytes);
    match base {
        "text/html" => open
            .set(named("HTML Format"), &format::html_format(&text()))
            .map_err(failed),
        "text/uri-list" => {
            let paths = format::file_paths(&text());
            if paths.is_empty() {
                open.set(TEXT, &wide(&text())).map_err(failed)
            } else {
                open.set(FILES, &dropped(&paths)).map_err(failed)
            }
        }
        image if image.starts_with("image/") => {
            open.set(named(format::native_name(image)), bytes)
                .map_err(failed)?;
            // A program that does not know the file's own format pastes the
            // bitmap. A picture that cannot be decoded goes without one.
            if let Some(bitmap) = bitmap_of(bytes) {
                let _ = open.set(BITMAP, &bitmap);
            }
            Ok(())
        }
        _ => open.set(TEXT, &wide(&text())).map_err(failed),
    }
}

fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// The files as Explorer puts them on the clipboard: a header, then the
/// paths one after another, each ended by a NUL and the list by one more.
fn dropped(paths: &[String]) -> Vec<u8> {
    let header = DROPFILES {
        pFiles: std::mem::size_of::<DROPFILES>() as u32,
        pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
        fNC: 0,
        fWide: 1,
    };
    // SAFETY: `DROPFILES` is plain data; its bytes are read for its own size.
    let mut out = unsafe {
        std::slice::from_raw_parts(
            &header as *const DROPFILES as *const u8,
            std::mem::size_of::<DROPFILES>(),
        )
    }
    .to_vec();
    for path in paths {
        out.extend(path.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    }
    out.extend([0, 0]);
    out
}

/// A screenshot or a picture copied as a bitmap, as the PNG the daemon keeps.
fn png_of(bitmap: &[u8]) -> Option<Vec<u8>> {
    use image::codecs::bmp::BmpDecoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let decoder = BmpDecoder::new_without_file_header(Cursor::new(bitmap)).ok()?;
    let mut picture = image::DynamicImage::from_decoder(decoder)
        .ok()?
        .into_rgba8();
    // Many programs leave the fourth byte of each pixel at zero without
    // meaning transparency: a picture with nothing visible is opaque.
    if picture.pixels().all(|pixel| pixel[3] == 0) {
        picture.pixels_mut().for_each(|pixel| pixel[3] = 255);
    }
    let mut png = Vec::new();
    let encoder =
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive);
    let opaque = picture.pixels().all(|pixel| pixel[3] == 255);
    if opaque {
        image::DynamicImage::ImageRgba8(picture)
            .into_rgb8()
            .write_with_encoder(encoder)
            .ok()?;
    } else {
        picture.write_with_encoder(encoder).ok()?;
    }
    Some(png)
}

fn bitmap_of(picture: &[u8]) -> Option<Vec<u8>> {
    let picture = image::load_from_memory(picture).ok()?.into_rgba8();
    format::bitmap(picture.width(), picture.height(), picture.as_raw())
}
