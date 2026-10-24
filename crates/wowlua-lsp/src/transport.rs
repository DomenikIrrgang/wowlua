//! The Language Server Protocol's framing: `Content-Length: N` headers, a
//! blank line, then N bytes of JSON.

use serde_json::Value as Json;
use std::io::{BufRead, Write};

/// The largest message read: more is a broken stream, not a document.
const MAX: usize = 1 << 30;

/// The next message, or `None` at the end of the input. A body that is no
/// JSON is an `InvalidData` error after which the next message can be
/// read; any other error ends the stream.
pub fn read(input: &mut impl BufRead) -> std::io::Result<Option<Json>> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some(n) = line
            .split_once(':')
            .filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.trim().parse().ok())
        {
            length = Some(n);
        }
    }
    let length = length.unwrap_or(0);
    if length > MAX {
        return Err(std::io::Error::other(format!(
            "a message of {length} bytes"
        )));
    }
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Writes one message.
pub fn write(output: &mut impl Write, message: &Json) -> std::io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut out = Vec::new();
        write(&mut out, &serde_json::json!({ "id": 1, "method": "x" })).unwrap();
        let mut input = std::io::Cursor::new(out);
        let got = read(&mut input).unwrap().unwrap();
        assert_eq!(got["method"], "x");
        assert!(read(&mut input).unwrap().is_none());
    }

    #[test]
    fn a_broken_message_is_skipped_a_broken_stream_ends() {
        let mut out = b"Content-Length: 5\r\n\r\n{nope".to_vec();
        write(&mut out, &serde_json::json!({ "id": 2 })).unwrap();
        let mut input = std::io::Cursor::new(out);
        let e = read(&mut input).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(read(&mut input).unwrap().unwrap()["id"], 2);
        let mut huge = std::io::Cursor::new(b"Content-Length: 99999999999\r\n\r\n".to_vec());
        assert_ne!(
            read(&mut huge).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}
