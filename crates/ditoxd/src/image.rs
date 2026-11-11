#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageSize {
    pub width: Option<i64>,
    pub height: Option<i64>,
}

pub fn detect_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }
    None
}

pub fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        _ => "bin",
    }
}

pub fn size(bytes: &[u8], mime: &str) -> ImageSize {
    match mime {
        "image/png" => png_size(bytes),
        "image/jpeg" => jpeg_size(bytes),
        _ => ImageSize::default(),
    }
}

pub fn preview(mime: &str, byte_len: usize, size: ImageSize) -> String {
    match (size.width, size.height) {
        (Some(width), Some(height)) => format!("Image {mime} {byte_len} bytes {width}x{height}"),
        _ => format!("Image {mime} {byte_len} bytes"),
    }
}

fn png_size(bytes: &[u8]) -> ImageSize {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || &bytes[12..16] != b"IHDR" {
        return ImageSize::default();
    }
    ImageSize {
        width: Some(i64::from(u32::from_be_bytes([
            bytes[16], bytes[17], bytes[18], bytes[19],
        ]))),
        height: Some(i64::from(u32::from_be_bytes([
            bytes[20], bytes[21], bytes[22], bytes[23],
        ]))),
    }
}

fn jpeg_size(bytes: &[u8]) -> ImageSize {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return ImageSize::default();
    }
    let mut index = 2;
    while index + 9 < bytes.len() {
        if bytes[index] != 0xff {
            index += 1;
            continue;
        }
        while index < bytes.len() && bytes[index] == 0xff {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let marker = bytes[index];
        index += 1;
        if marker == 0xd9 || marker == 0xda || index + 2 > bytes.len() {
            break;
        }
        let segment = usize::from(u16::from_be_bytes([bytes[index], bytes[index + 1]]));
        if segment < 2 || index + segment > bytes.len() {
            break;
        }
        let start_of_frame =
            matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf);
        if start_of_frame && segment >= 7 {
            return ImageSize {
                height: Some(i64::from(u16::from_be_bytes([
                    bytes[index + 3],
                    bytes[index + 4],
                ]))),
                width: Some(i64::from(u16::from_be_bytes([
                    bytes[index + 5],
                    bytes[index + 6],
                ]))),
            };
        }
        index += segment;
    }
    ImageSize::default()
}

#[cfg(test)]
pub(crate) fn tiny_png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_formats_are_sniffed_and_other_bytes_are_not() {
        assert_eq!(detect_mime(&tiny_png(1, 1)), Some("image/png"));
        assert_eq!(detect_mime(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(detect_mime(b"GIF89a..."), Some("image/gif"));
        assert_eq!(detect_mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(detect_mime(b"BM\0\0"), Some("image/bmp"));
        assert_eq!(detect_mime(b"https://example.com/a.png"), None);
        assert_eq!(detect_mime(b""), None);
    }

    #[test]
    fn png_and_jpeg_report_their_dimensions() {
        assert_eq!(
            size(&tiny_png(640, 480), "image/png"),
            ImageSize {
                width: Some(640),
                height: Some(480)
            }
        );
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00,
            0x20, 0x00, 0x40, 0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01,
        ];
        assert_eq!(
            size(&jpeg, "image/jpeg"),
            ImageSize {
                width: Some(64),
                height: Some(32)
            }
        );
        assert_eq!(size(b"not an image", "image/png"), ImageSize::default());
    }

    #[test]
    fn the_preview_names_the_type_and_size() {
        assert_eq!(
            preview(
                "image/png",
                82,
                ImageSize {
                    width: Some(2),
                    height: Some(3)
                }
            ),
            "Image image/png 82 bytes 2x3"
        );
        assert_eq!(
            preview("image/gif", 7, ImageSize::default()),
            "Image image/gif 7 bytes"
        );
    }
}
