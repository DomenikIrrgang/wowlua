//! Literal text kept inline when short (#19): a data addon's tables hold
//! millions of numerals and short strings, each a heap allocation as a
//! `String` or `Vec<u8>`, in every flavor's IR.

use std::fmt;
use std::ops::Deref;

/// Bytes stored inline up to [`Bytes::INLINE`], else on the heap. The same
/// size as a `Vec<u8>`.
#[derive(Clone)]
enum Bytes {
    Inline { len: u8, bytes: [u8; Bytes::INLINE] },
    Heap(Box<[u8]>),
}

impl Bytes {
    const INLINE: usize = 22;

    fn new(bytes: &[u8]) -> Self {
        if bytes.len() <= Self::INLINE {
            let mut inline = [0; Self::INLINE];
            inline[..bytes.len()].copy_from_slice(bytes);
            Bytes::Inline {
                len: bytes.len() as u8,
                bytes: inline,
            }
        } else {
            Bytes::Heap(bytes.into())
        }
    }

    fn get(&self) -> &[u8] {
        match self {
            Bytes::Inline { len, bytes } => &bytes[..*len as usize],
            Bytes::Heap(bytes) => bytes,
        }
    }
}

/// A numeral as it is emitted: ASCII text. Reads and prints like the
/// `String` it replaced.
#[derive(Clone)]
pub struct Numeral(Bytes);

impl Numeral {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.0.get()).expect("numerals are ASCII")
    }
}

impl Deref for Numeral {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for Numeral {
    fn from(text: &str) -> Self {
        Numeral(Bytes::new(text.as_bytes()))
    }
}

impl From<String> for Numeral {
    fn from(text: String) -> Self {
        Numeral::from(text.as_str())
    }
}

impl PartialEq<str> for Numeral {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl fmt::Debug for Numeral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Numeral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A string literal as it is emitted, quotes or brackets included. Reads
/// and prints like the `Vec<u8>` it replaced.
#[derive(Clone)]
pub struct StrLit(Bytes);

impl Deref for StrLit {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.0.get()
    }
}

impl From<&[u8]> for StrLit {
    fn from(raw: &[u8]) -> Self {
        StrLit(Bytes::new(raw))
    }
}

impl From<Vec<u8>> for StrLit {
    fn from(raw: Vec<u8>) -> Self {
        StrLit::from(raw.as_slice())
    }
}

impl fmt::Debug for StrLit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Inline or not, the text reads and prints as the `String` or `Vec<u8>`
    // it replaced, in the same space.
    #[test]
    fn literals_read_like_what_they_replaced() {
        assert_eq!(
            std::mem::size_of::<Numeral>(),
            std::mem::size_of::<String>()
        );
        assert_eq!(
            std::mem::size_of::<StrLit>(),
            std::mem::size_of::<Vec<u8>>()
        );
        for text in [
            "",
            "1",
            "0x1F",
            "1234567890123456789012",
            "12345678901234567890123",
        ] {
            let n = Numeral::from(text);
            assert_eq!(&*n, text);
            assert_eq!(n.to_string(), text);
            assert_eq!(format!("{n:?}"), format!("{:?}", String::from(text)));
            let s = StrLit::from(text.as_bytes().to_vec());
            assert_eq!(&*s, text.as_bytes());
            assert_eq!(format!("{s:?}"), format!("{:?}", text.as_bytes().to_vec()));
        }
    }
}
