//! The Language Server Protocol's framing: `Content-Length: N` headers, a
//! blank line, then N bytes of JSON.

use serde_json::Value as Json;
use std::io::{BufRead, Write};

/// The next message, or `None` at the end of the input.
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
    let mut body = vec![0; length.unwrap_or(0)];
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
}
