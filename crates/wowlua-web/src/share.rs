//! Share links: a package's files in a URL's fragment. The files as JSON
//! (`{"v":1,"files":{"<path>":"<text>",…},"run":{…}}`; `run`, how it runs:
//! its flavor, world profile, Blizzard's UI, may be left out), deflated,
//! in URL-safe base64 without padding.

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use serde_json::{Map, Value as Json, json};
use std::io::{Read, Write};

/// The format's version.
const VERSION: u64 = 1;

/// The most a link's files may unpack to: more is no playground's.
const MAX: u64 = 8 << 20;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// A link's contents: the files, and how it runs.
pub type Shared = (Vec<(String, String)>, Option<Json>);

pub fn encode(files: &[(String, String)], run: Option<&Json>) -> String {
    let map: Map<String, Json> = files
        .iter()
        .map(|(p, t)| (p.clone(), Json::String(t.clone())))
        .collect();
    let mut body = json!({ "v": VERSION, "files": map });
    if let Some(run) = run.filter(|r| r.is_object()) {
        body["run"] = run.clone();
    }
    let body = body.to_string();
    let mut deflate = DeflateEncoder::new(Vec::new(), Compression::best());
    // (writing into memory cannot fail)
    let _ = deflate.write_all(body.as_bytes());
    let packed = deflate.finish().unwrap_or_default();
    base64(&packed)
}

/// The files of a link (its fragment, with or without the `#`); `None`
/// for anything else.
pub fn decode(link: &str) -> Option<Shared> {
    let fragment = link.rsplit_once('#').map_or(link, |(_, f)| f).trim();
    let packed = unbase64(fragment)?;
    let mut body = String::new();
    DeflateDecoder::new(&packed[..])
        .take(MAX)
        .read_to_string(&mut body)
        .ok()?;
    let value: Json = serde_json::from_str(&body).ok()?;
    if value["v"].as_u64() != Some(VERSION) {
        return None;
    }
    let files = value["files"].as_object()?;
    let files = files
        .iter()
        .map(|(p, t)| {
            // (no way out of the package's folder)
            let safe = !p.split('/').any(|c| c == ".." || c.is_empty()) && !p.starts_with('/');
            safe.then(|| Some((p.clone(), t.as_str()?.to_string())))?
        })
        .collect::<Option<Vec<_>>>()?;
    let run = value.get("run").filter(|r| r.is_object()).cloned();
    Some((files, run))
}

pub(crate) fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 3);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let digits: Vec<u32> = text
        .bytes()
        .map(|c| ALPHABET.iter().position(|a| *a == c).map(|d| d as u32))
        .collect::<Option<_>>()?;
    for chunk in digits.chunks(4) {
        if chunk.len() == 1 {
            return None;
        }
        let n = chunk.iter().fold(0u32, |n, d| (n << 6) | d) << (6 * (4 - chunk.len()));
        for i in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_gives_back_its_files() {
        let files = vec![
            ("src/main.wowlua".to_string(), "print(\"é ✓\")\n".repeat(50)),
            ("wowlua.json".to_string(), "{}".to_string()),
        ];
        let run = json!({ "flavor": "camelot", "profile": "stormwind" });
        let link = encode(&files, Some(&run));
        assert!(link.bytes().all(|b| ALPHABET.contains(&b)), "{link}");
        assert!(link.len() < 220, "compressed: {}", link.len());
        let (mut back, back_run) =
            decode(&format!("https://wowlua.dev/playground#{link}")).unwrap();
        back.sort();
        assert_eq!(back, files);
        assert_eq!(back_run, Some(run));
        assert_eq!(decode(&encode(&files, None)).unwrap().1, None);
        for n in 0..8 {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 37) as u8).collect();
            assert_eq!(unbase64(&base64(&bytes)).unwrap(), bytes);
        }
        assert_eq!(decode("not a link!"), None);
    }

    #[test]
    fn a_link_cannot_leave_the_package() {
        let link = encode(&[("../etc/passwd".to_string(), "x".to_string())], None);
        assert_eq!(decode(&link), None);
    }
}
