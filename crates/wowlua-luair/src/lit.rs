//! Literal text kept inline when short (#19): a data addon's tables hold
//! millions of numerals and short strings, each a heap allocation as a
//! `String` or `Vec<u8>`, in every flavor's IR.

use std::fmt;
use std::ops::Deref;

/// Bytes stored inline up to [`Bytes::INLINE`], else on the heap, and a
/// flag of their owner's (an identifier's `generated`) in the room left.
/// The same size as a `Vec<u8>`.
#[derive(Clone)]
enum Bytes {
    Inline {
        len: u8,
        flag: bool,
        bytes: [u8; Bytes::INLINE],
    },
    Heap {
        flag: bool,
        bytes: Box<[u8]>,
    },
}

impl Bytes {
    const INLINE: usize = 21;

    fn new(bytes: &[u8]) -> Self {
        if bytes.len() <= Self::INLINE {
            let mut inline = [0; Self::INLINE];
            inline[..bytes.len()].copy_from_slice(bytes);
            Bytes::Inline {
                len: bytes.len() as u8,
                flag: false,
                bytes: inline,
            }
        } else {
            Bytes::Heap {
                flag: false,
                bytes: bytes.into(),
            }
        }
    }

    fn get(&self) -> &[u8] {
        match self {
            Bytes::Inline { len, bytes, .. } => &bytes[..*len as usize],
            Bytes::Heap { bytes, .. } => bytes,
        }
    }

    fn flag(&self) -> bool {
        match self {
            Bytes::Inline { flag, .. } | Bytes::Heap { flag, .. } => *flag,
        }
    }

    fn set_flag(&mut self, to: bool) {
        match self {
            Bytes::Inline { flag, .. } | Bytes::Heap { flag, .. } => *flag = to,
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

/// A name (an identifier) in the IR: text, inline when short. Reads,
/// compares, hashes and prints like the `String` it replaced.
#[derive(Clone)]
pub struct Name(Bytes);

impl Name {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.0.get()).expect("built from a str")
    }

    /// The identifier's `generated` flag, kept here to keep `Ident` at 32
    /// bytes. Not part of the name: comparing and hashing leave it out.
    pub(crate) fn flag(&self) -> bool {
        self.0.flag()
    }

    pub(crate) fn set_flag(&mut self, to: bool) {
        self.0.set_flag(to);
    }
}

impl Deref for Name {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl std::borrow::Borrow<str> for Name {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for Name {
    fn from(text: &str) -> Self {
        Name(Bytes::new(text.as_bytes()))
    }
}

impl From<&String> for Name {
    fn from(text: &String) -> Self {
        Name::from(text.as_str())
    }
}

impl From<String> for Name {
    fn from(text: String) -> Self {
        Name::from(text.as_str())
    }
}

impl From<Name> for String {
    fn from(name: Name) -> Self {
        name.as_str().to_string()
    }
}

impl Default for Name {
    fn default() -> Self {
        Name::from("")
    }
}

// Compared, ordered and hashed by their bytes, without reading them as a
// `str` first, as a `str` is: the minifier compares and hashes names all
// the time.
impl PartialEq<Name> for String {
    fn eq(&self, other: &Name) -> bool {
        self.as_bytes() == other.0.get()
    }
}

impl PartialEq<Name> for str {
    fn eq(&self, other: &Name) -> bool {
        self.as_bytes() == other.0.get()
    }
}

impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool {
        self.0.get() == other.0.get()
    }
}

impl Eq for Name {}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.0.get() == other.as_bytes()
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.0.get() == other.as_bytes()
    }
}

impl PartialEq<String> for Name {
    fn eq(&self, other: &String) -> bool {
        self.0.get() == other.as_bytes()
    }
}

impl PartialOrd for Name {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Name {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.get().cmp(other.0.get())
    }
}

impl std::hash::Hash for Name {
    // What `str` hashes (`Hasher::write_str`'s default), so a map of names
    // finds a `str` key (`Borrow<str>`).
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(self.0.get());
        state.write_u8(0xff);
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Name {
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
            let mut name = Name::from(text);
            assert!(!name.flag());
            name.set_flag(true);
            assert!(name.flag());
            assert_eq!(name, Name::from(text), "the flag is not part of the name");
            assert_eq!(name, *text);
            assert_eq!(format!("{name:?}"), format!("{:?}", String::from(text)));
            let state = std::collections::hash_map::RandomState::new();
            use std::hash::BuildHasher;
            let (a, b) = (state.hash_one(&name), state.hash_one(text));
            assert_eq!(a, b, "hashes like the text, for maps of `str`");
            let s = StrLit::from(text.as_bytes().to_vec());
            assert_eq!(&*s, text.as_bytes());
            assert_eq!(format!("{s:?}"), format!("{:?}", text.as_bytes().to_vec()));
        }
    }
}
