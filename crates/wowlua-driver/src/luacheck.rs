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

thread_local! {
    /// Compile checks share one state per thread: compiling runs no code,
    /// and Lua 5.1's parser keeps no state between loads.
    static LUA: Lua = Lua::new();
}

/// The payload of the panic when Lua runs out of memory while compiling:
/// that is a failure of the compiler, never a result of the code (#8).
#[derive(Debug)]
pub struct LuaOutOfMemory;

impl std::fmt::Display for LuaOutOfMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Lua ran out of memory while compiling")
    }
}

/// `result`, unless Lua ran out of memory: that panics with
/// [`LuaOutOfMemory`] instead of becoming a compile result.
pub(crate) fn no_memory_error<T>(result: mlua::Result<T>) -> mlua::Result<T> {
    if let Err(mlua::Error::MemoryError(_)) = &result {
        std::panic::panic_any(LuaOutOfMemory);
    }
    result
}

/// Compiles `code` in the thread's shared state, as `name` when given.
/// Called from plain Rust only, never from inside Lua: Lua's nesting limits
/// count from the C call depth at load time.
fn compile(code: &[u8], name: Option<String>) -> mlua::Result<()> {
    LUA.with(|lua| {
        let mut chunk = lua.load(code);
        if let Some(name) = name {
            chunk = chunk.set_name(name);
        }
        let result = no_memory_error(chunk.into_function().map(drop));
        // Back to the baseline heap, as when the state was closed.
        let _ = lua.gc_collect();
        result
    })
}

/// Whether Lua 5.1 compiles `code` as it is (no byte order mark skipped).
pub(crate) fn compiles(code: &[u8]) -> bool {
    compile(code, None).is_ok()
}

/// [`check`] for code Lua 5.1 already compiled as it is (`compiled`): then
/// it compiles again, so it is skipped. Only the chunk name and a byte order
/// mark differ between the two compiles. The name never decides whether
/// code compiles, unless it cannot be a C string; a leading byte order mark
/// is checked as before.
pub fn check_unless_compiled(
    db: &SourceDb,
    file: FileId,
    code: &[u8],
    compiled: bool,
) -> Option<Diagnostic> {
    if compiled && !code.starts_with(b"\xEF\xBB\xBF") && !chunk_name(db, file).contains('\0') {
        debug_assert!(check(db, file, code).is_none(), "compiled once, not twice");
        return None;
    }
    check(db, file, code)
}

fn chunk_name(db: &SourceDb, file: FileId) -> String {
    format!("={}", db.get(file).path().to_string_lossy())
}

/// Compiles `code` (the raw bytes of `file`) as Lua 5.1. A leading UTF-8 byte
/// order mark is skipped, as WoW does. Returns `None` if it compiles,
/// otherwise a diagnostic pointing at the reported line.
pub fn check(db: &SourceDb, file: FileId, code: &[u8]) -> Option<Diagnostic> {
    let code = code.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(code);
    let error = compile(code, Some(chunk_name(db, file))).err()?;
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
    /// A Lua state out of memory panics with `LuaOutOfMemory` instead of
    /// reporting the code as not compiling; other errors are results.
    #[test]
    fn out_of_memory_is_not_a_compile_result() {
        let lua = Lua::new();
        lua.set_memory_limit(lua.used_memory() + 64 * 1024).unwrap();
        let items: String = (0..50_000).map(|i| format!("\"s{i}\", ")).collect();
        let big = format!("return {{ {items} }}\n");
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            no_memory_error(lua.load(big.as_bytes()).into_function().map(drop))
        }));
        let payload = caught.expect_err("out of memory panics");
        assert!(payload.downcast_ref::<LuaOutOfMemory>().is_some());
        let syntax = no_memory_error(Lua::new().load(&b"x = = 1"[..]).into_function().map(drop));
        assert!(
            matches!(syntax, Err(mlua::Error::SyntaxError { .. })),
            "{syntax:?}"
        );
    }

    use super::*;

    fn run(text: &str) -> Option<Diagnostic> {
        let mut db = SourceDb::new();
        let file = db.add("src/a.lua", text);
        check(&db, file, text.as_bytes())
    }

    #[test]
    fn code_that_did_not_compile_is_checked() {
        let mut db = SourceDb::new();
        let file = db.add("src/a.lua", "");
        let locals: String = (0..201).map(|i| format!("local v{i} = {i}\n")).collect();
        let error = check_unless_compiled(&db, file, locals.as_bytes(), false).expect("E5001");
        assert_eq!(error.code, Some("E5001"));
        assert!(check_unless_compiled(&db, file, b"local a = 1\n", true).is_none());
        // A byte order mark is compiled again, as `check` skips it.
        assert!(check_unless_compiled(&db, file, b"\xEF\xBB\xBFlocal a = 1\n", true).is_none());
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

    /// One state for many compiles answers as a fresh state for each: at the
    /// nesting and assignment limits, after errors, with byte order marks.
    #[test]
    fn the_shared_state_compiles_as_a_fresh_one() {
        let nested = |n: usize| format!("x = {}1{}", "{".repeat(n), "}".repeat(n));
        let mut inputs: Vec<Vec<u8>> = vec![
            b"local a = 1\nprint(a)\n".to_vec(),
            (0..210)
                .map(|i| format!("local v{i} = {i}\n"))
                .collect::<String>()
                .into_bytes(),
            b"x = = 1\n".to_vec(),
        ];
        inputs.extend((197..=201).map(|n| nested(n).into_bytes()));
        // "variables in assignment" counts down from the C call depth.
        for n in [190, 199, 200, 201, 260] {
            let targets: Vec<String> = (0..n).map(|i| format!("t.v{i}")).collect();
            inputs.push(format!("local t = {{}}\n{} = 1\n", targets.join(", ")).into_bytes());
        }
        inputs.push(b"\xEF\xBB\xBFlocal a = 1\n".to_vec());
        inputs.push(b"\xEF\xBB\xBFx = = 1\n".to_vec());
        inputs.push(b"local s = \"caf\xE9\"\n".to_vec());
        inputs.push(b"local a = 1\n".to_vec());
        let mut db = SourceDb::new();
        let file = db.add("src/a.lua", "");
        let fresh = |code: &[u8]| {
            let code_without_bom = code.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(code);
            let named = Lua::new()
                .load(code_without_bom)
                .set_name(chunk_name(&db, file))
                .into_function()
                .map(drop);
            let unnamed = Lua::new().load(code).into_function().is_ok();
            (format!("{named:?}"), unnamed)
        };
        for round in 0..2 {
            for code in &inputs {
                let shared = (
                    format!(
                        "{:?}",
                        compile(
                            code.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(code),
                            Some(chunk_name(&db, file))
                        )
                    ),
                    compiles(code),
                );
                assert_eq!(
                    shared,
                    fresh(code),
                    "round {round}: {:?}",
                    String::from_utf8_lossy(code)
                );
            }
        }
        // The inputs reach both sides of each limit.
        assert!(compiles(nested(197).as_bytes()) && !compiles(nested(201).as_bytes()));
        assert!(compiles(&inputs[8]) && !compiles(&inputs[12]));
        assert_eq!(
            format!("{:?}", check(&db, file, &nested(199).into_bytes())),
            format!("{:?}", check(&db, file, &nested(199).into_bytes()))
        );
    }

    #[test]
    fn limit_errors_are_e5001() {
        let locals: String = (0..210).map(|i| format!("local v{i} = {i}\n")).collect();
        let d = run(&locals).unwrap();
        assert_eq!(d.code, Some("E5001"), "{}", d.message);
    }
}
