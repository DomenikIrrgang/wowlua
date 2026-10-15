//! Lexer, parser and syntax tree for Lua 5.1 and wowlua (spec chapter 1,
//! appendix A, compiler §13.5).
//!
//! [`parse`] accepts every valid Lua 5.1 chunk with the same meaning, and
//! rejects invalid Lua 5.1 the same way Lua does, apart from the limits Lua
//! checks while generating code (locals, upvalues, registers), which the
//! build checks on its output.

pub mod ast;
pub mod features;
pub mod lexer;
pub mod literal;
mod parser;
pub mod search;
pub mod token;
pub mod visit;

use wowlua_diagnostics::Diagnostic;
use wowlua_span::FileId;

pub use lexer::Lexed;

/// The result of parsing one file: the tree (always produced, with `Error`
/// nodes where parsing failed), the tokens with their trivia, and diagnostics.
#[derive(Debug)]
pub struct Parse {
    pub chunk: ast::Chunk,
    pub lexed: Lexed,
    pub diagnostics: Vec<Diagnostic>,
}

impl Parse {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Parses `src`, the raw bytes of `file`.
pub fn parse(src: &[u8], file: FileId) -> Parse {
    let mut lexed = lexer::lex(src, file);
    let mut diagnostics = std::mem::take(&mut lexed.diagnostics);
    let mut parser = parser::Parser::new(src, file, lexed);
    let chunk = parser.chunk();
    diagnostics.append(&mut parser.diagnostics);
    diagnostics.sort_by_key(|d| d.labels.first().map(|l| l.span.start).unwrap_or(0));
    Parse {
        chunk,
        lexed: parser.lexed,
        diagnostics,
    }
}
