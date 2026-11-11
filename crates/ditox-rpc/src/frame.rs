//! `Content-Length: N\r\n\r\n<body>` framing.

use std::io::{self, Read, Write};

/// Larger than the biggest image the daemon keeps, base64 encoded.
pub const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

/// A header is one short line. Anything this long without a blank line is not a frame.
pub const MAX_HEADER_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSpan {
    pub body_start: usize,
    pub body_len: usize,
}

/// Where the header ends and how long its terminator is: the first blank
/// line, written with either line ending. A body may hold either sequence.
fn header_end(bytes: &[u8]) -> Option<(usize, usize)> {
    // Only the start is searched: a reader calls this for every chunk of a
    // large body, and a body has no business deciding where its header ends.
    let bytes = &bytes[..bytes.len().min(MAX_HEADER_BYTES)];
    match (find(bytes, b"\r\n\r\n"), find(bytes, b"\n\n")) {
        (Some(crlf), Some(lf)) if lf < crlf => Some((lf, 2)),
        (Some(crlf), _) => Some((crlf, 4)),
        (None, Some(lf)) => Some((lf, 2)),
        (None, None) => None,
    }
}

/// The first complete frame in `bytes`, if all of it has arrived.
pub fn frame_span(bytes: &[u8]) -> Option<FrameSpan> {
    let (header_end, separator) = header_end(bytes)?;
    let body_len = content_length(&bytes[..header_end])?;
    let body_start = header_end + separator;
    if bytes.len() < body_start.checked_add(body_len)? {
        return None;
    }
    Some(FrameSpan {
        body_start,
        body_len,
    })
}

/// The length a header announces, also before its body has arrived.
pub fn announced_length(bytes: &[u8]) -> Option<usize> {
    let (header_end, _) = header_end(bytes)?;
    content_length(&bytes[..header_end])
}

fn content_length(header: &[u8]) -> Option<usize> {
    for line in header.split(|byte| *byte == b'\n') {
        let line = std::str::from_utf8(line).ok()?.trim();
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

pub fn encode(body: &[u8]) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out
}

pub fn write_frame(writer: &mut impl Write, body: &[u8]) -> io::Result<()> {
    writer.write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())?;
    writer.write_all(body)?;
    writer.flush()
}

/// Reads frames from a stream, keeping what arrived after the last one.
#[derive(Default)]
pub struct FrameReader {
    pending: Vec<u8>,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next frame's body, or `None` when the peer closed between frames.
    pub fn read(&mut self, reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            if let Some(span) = frame_span(&self.pending) {
                let end = span.body_start + span.body_len;
                if self.pending.len() == end {
                    // The usual case, and the one that matters for a picture
                    // of many megabytes: the buffer becomes the body.
                    let mut body = std::mem::take(&mut self.pending);
                    body.drain(..span.body_start);
                    return Ok(Some(body));
                }
                let body = self.pending[span.body_start..end].to_vec();
                self.pending.drain(..end);
                return Ok(Some(body));
            }
            if self.pending.len() >= MAX_HEADER_BYTES && header_end(&self.pending).is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "no frame header",
                ));
            }
            if announced_length(&self.pending).is_some_and(|length| length > MAX_FRAME_BYTES)
                || self.pending.len() > MAX_FRAME_BYTES + MAX_HEADER_BYTES
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "frame is too large",
                ));
            }
            let read = reader.read(&mut chunk)?;
            if read == 0 {
                if self.pending.iter().all(u8::is_ascii_whitespace) {
                    return Ok(None);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed inside a frame",
                ));
            }
            self.pending.extend_from_slice(&chunk[..read]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn only_complete_content_length_frames_parse() {
        assert_eq!(frame_span(b"Content-Length: 5\r\n"), None);
        assert_eq!(frame_span(b"Content-Length: 5\r\n\r\nab"), None);
        assert_eq!(frame_span(b"X-Other: 1\r\n\r\n{}"), None);

        let exact = frame_span(b"Content-Length: 2\r\n\r\n{}").unwrap();
        assert_eq!(
            exact,
            FrameSpan {
                body_start: 21,
                body_len: 2
            }
        );

        let pipelined =
            frame_span(b"content-length: 4\r\n\r\nabcdContent-Length: 2\r\n\r\n{}").unwrap();
        assert_eq!(
            pipelined,
            FrameSpan {
                body_start: 21,
                body_len: 4
            }
        );

        let bare = frame_span(b"Content-Length: 2\n\n{}").unwrap();
        assert_eq!(
            bare,
            FrameSpan {
                body_start: 19,
                body_len: 2
            }
        );
    }

    #[test]
    fn a_body_with_windows_line_endings_does_not_move_a_bare_header_end() {
        let frame = b"Content-Length: 9\n\nab\r\n\r\ncde";
        let span = frame_span(frame).unwrap();
        assert_eq!(
            &frame[span.body_start..span.body_start + span.body_len],
            b"ab\r\n\r\ncde"
        );
        assert_eq!(
            announced_length(b"Content-Length: 9\n\nab\r\n\r\n"),
            Some(9)
        );
    }

    #[test]
    fn a_large_frame_is_read_in_time_that_grows_with_its_size_not_its_square() {
        let body = vec![b'a'; 48 * 1024 * 1024];
        let mut cursor = Cursor::new(encode(&body));
        let started = std::time::Instant::now();
        let read = FrameReader::new().read(&mut cursor).unwrap().unwrap();
        assert_eq!(read.len(), body.len());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "48 MiB took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn bytes_that_never_become_a_header_are_refused_early() {
        let mut cursor = Cursor::new(vec![b'x'; 64 * 1024]);
        let err = FrameReader::new().read(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(
            cursor.position() <= 2 * 16 * 1024,
            "it stops after the first chunk"
        );
    }

    #[test]
    fn a_reader_returns_pipelined_frames_in_order_and_then_the_close() {
        let mut bytes = encode(br#"{"a":1}"#);
        bytes.extend(encode(br#"{"b":2}"#));
        let mut cursor = Cursor::new(bytes);
        let mut reader = FrameReader::new();
        assert_eq!(reader.read(&mut cursor).unwrap().unwrap(), br#"{"a":1}"#);
        assert_eq!(reader.read(&mut cursor).unwrap().unwrap(), br#"{"b":2}"#);
        assert_eq!(reader.read(&mut cursor).unwrap(), None);
    }

    #[test]
    fn a_close_inside_a_frame_is_an_error() {
        let mut cursor = Cursor::new(b"Content-Length: 9\r\n\r\n{}".to_vec());
        assert!(FrameReader::new().read(&mut cursor).is_err());
    }

    #[test]
    fn an_oversized_announcement_is_refused_before_its_body_is_read() {
        let header = format!("Content-Length: {}\r\n\r\n", MAX_FRAME_BYTES + 1);
        let mut cursor = Cursor::new(header.into_bytes());
        assert!(FrameReader::new().read(&mut cursor).is_err());
    }
}
