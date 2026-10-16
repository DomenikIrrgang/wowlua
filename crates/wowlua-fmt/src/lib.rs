//! The wowlua formatter (spec tooling §16.3): StyLua-like, idempotent,
//! comment-preserving, for Lua 5.1 and all of wowlua's syntax (M9 S6).

mod canon;
mod doc;
mod format;
mod wowlua;

pub use format::QuoteStyle;
use wowlua_diagnostics::Diagnostic;
use wowlua_span::FileId;

#[derive(Clone, Debug)]
pub struct Config {
    /// One indentation unit (`"\t"` or spaces).
    pub indent: String,
    /// Columns one indentation unit counts for line width.
    pub indent_width: usize,
    pub line_width: usize,
    pub quote: QuoteStyle,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            indent: "\t".into(),
            indent_width: 4,
            line_width: 100,
            quote: QuoteStyle::Double,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    /// The file does not parse.
    Syntax(Vec<Diagnostic>),
    /// The file uses wowlua syntax, which milestone M1 does not format.
    Unsupported(Diagnostic),
    /// The formatted output would mean something else (a formatter bug);
    /// the file is left unchanged.
    Unstable(String),
}

/// Formats `src` (the bytes of `file`).
pub fn format(src: &[u8], file: FileId, config: &Config) -> Result<Vec<u8>, Error> {
    let mut parsed = wowlua_syntax::parse(src, file);
    if parsed.has_errors() {
        return Err(Error::Syntax(std::mem::take(&mut parsed.diagnostics)));
    }
    let mut formatter = format::Formatter::new(&parsed, src, config.quote);
    let doc = formatter.chunk(&parsed.chunk);
    let printed = doc::print(&doc, config.line_width, &config.indent, config.indent_width);
    let mut out = Vec::with_capacity(printed.len() + 8);
    if parsed.lexed.bom {
        out.extend_from_slice(b"\xEF\xBB\xBF");
    }
    if let Some((start, end)) = parsed.lexed.shebang {
        out.extend_from_slice(&src[start as usize..end as usize]);
        out.push(b'\n');
    }
    let skip = printed.iter().take_while(|&&c| c == b'\n').count();
    out.extend_from_slice(&printed[skip..]);
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    // Formatting must not change the program.
    let reparsed = wowlua_syntax::parse(&out, file);
    if reparsed.has_errors() {
        return Err(Error::Unstable(format!(
            "the formatted file does not parse: {}",
            reparsed.diagnostics[0].message
        )));
    }
    if canon::fingerprint(&reparsed.chunk) != canon::fingerprint(&parsed.chunk) {
        return Err(Error::Unstable(
            "the formatted file means something else".into(),
        ));
    }
    Ok(out)
}

/// The fingerprints of `src` and of its unchecked formatting, for
/// diagnosing formatter bugs.
#[doc(hidden)]
pub fn debug_fingerprints(src: &[u8], file: FileId, config: &Config) -> (String, String, String) {
    let parsed = wowlua_syntax::parse(src, file);
    let mut formatter = format::Formatter::new(&parsed, src, config.quote);
    let doc = formatter.chunk(&parsed.chunk);
    let printed = doc::print(&doc, config.line_width, &config.indent, config.indent_width);
    let reparsed = wowlua_syntax::parse(&printed, file);
    (
        canon::fingerprint(&parsed.chunk),
        canon::fingerprint(&reparsed.chunk),
        String::from_utf8_lossy(&printed).into_owned(),
    )
}
