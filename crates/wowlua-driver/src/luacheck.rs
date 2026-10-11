//! Lua 5.1 limit checks with the embedded Lua 5.1 VM: files that parse are
//! compiled (never run), which reports the limits Lua checks while
//! generating code (too many locals, upvalues, constants, registers) as
//! E5001. Syntax is checked by the compiler's own parser before this.

use mlua::Lua;
use std::sync::LazyLock;
use wowlua_diagnostics::Diagnostic;
use wowlua_span::{FileId, SourceDb};

/// Messages of Lua 5.1 limit errors (spec §13.8, item 3).
const LIMIT_MESSAGES: &[&str] = &[
    "has more than",
    "too many local variables",
    "function or expression too complex",
    "constant table overflow",
    "control structure too long",
    "too many syntax levels",
    "chunk has too many syntax levels",
    "upvalues",
    "too many registers",
];

static LINE_PREFIX: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^.*?:(\d+): (.*)$").expect("valid pattern"));

/// Compiles `code` (the raw bytes of `file`) as Lua 5.1. A leading UTF-8 byte
/// order mark is skipped, as WoW does. Returns `None` if it compiles,
/// otherwise a diagnostic pointing at the reported line.
pub fn check(db: &SourceDb, file: FileId, code: &[u8]) -> Option<Diagnostic> {
    let source = db.get(file);
    let code = code.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(code);
    let lua = Lua::new();
    let chunk_name = format!("={}", source.path().to_string_lossy());
    let result = lua.load(code).set_name(chunk_name).into_function();
    let error = result.err()?;
    let raw = match &error {
        mlua::Error::SyntaxError { message, .. } => message.clone(),
        other => other.to_string(),
    };
    let first_line = raw.lines().next().unwrap_or(&raw).to_string();
    let (line, message) = match LINE_PREFIX.captures(&first_line) {
        Some(c) => (c[1].parse::<usize>().unwrap_or(1), c[2].to_string()),
        None => (1, first_line.clone()),
    };
    let span = db.line_span(file, line);
    let is_limit = LIMIT_MESSAGES.iter().any(|m| message.contains(m));
    let diagnostic = if is_limit {
        Diagnostic::error("E5001", format!("Lua 5.1 limit exceeded: {message}"))
            .with_label(span, "in the function containing this line")
            .with_help("split the function or move locals into a table")
    } else {
        // The compiler's parser accepted the file, Lua 5.1 did not.
        Diagnostic::error("E0101", format!("Lua syntax error: {message}"))
            .with_label(span, "here")
            .with_note("wowlua's parser accepted this file; please report this as a bug")
    };
    Some(diagnostic)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> Option<Diagnostic> {
        let mut db = SourceDb::new();
        let file = db.add("src/a.lua", text);
        check(&db, file, text.as_bytes())
    }

    #[test]
    fn byte_order_marks_and_non_utf8_strings_are_accepted() {
        let mut db = SourceDb::new();
        let file = db.add("src/a.lua", "");
        assert!(check(&db, file, b"\xEF\xBB\xBFlocal a = 1\n").is_none());
        assert!(check(&db, file, b"local s = \"caf\xE9\"\n").is_none());
    }

    #[test]
    fn valid_lua_51_passes() {
        assert!(run("local a = 1\nprint(a .. 'x')\n").is_none());
    }

    #[test]
    fn limit_errors_are_e5001() {
        let locals: String = (0..210).map(|i| format!("local v{i} = {i}\n")).collect();
        let d = run(&locals).unwrap();
        assert_eq!(d.code, Some("E5001"), "{}", d.message);
    }
}
