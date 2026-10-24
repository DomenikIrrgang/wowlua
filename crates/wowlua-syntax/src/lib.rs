//! Lexer, parser and syntax tree for Lua 5.1 and wowlua (spec chapter 1,
//! appendix A, compiler §13.5).
//!
//! [`parse`] accepts every valid Lua 5.1 chunk with the same meaning, and
//! rejects invalid Lua 5.1 the same way Lua does, apart from the limits Lua
//! checks while generating code (locals, upvalues, registers), which the
//! build checks on its output.

pub mod deep;
pub mod features;
pub mod lexer;
pub mod literal;
mod ops;
mod parser;
pub mod search;
pub mod token;
mod tree;
pub mod view;
pub mod visit;

use std::sync::Arc;
use tree::SourceText;
use wowlua_diagnostics::Diagnostic;
use wowlua_span::FileId;

pub use lexer::Lexed;

/// The result of parsing one file: the tree (always produced, with `Error`
/// nodes where parsing failed), the tokens with their trivia, and diagnostics.
#[derive(Debug)]
pub struct Parse {
    tree: tree::Tree,
    pub lexed: Lexed,
    pub diagnostics: Vec<Diagnostic>,
}

impl Parse {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// The bytes the tree was parsed from.
    pub fn source(&self) -> &[u8] {
        self.tree.src.bytes()
    }

    /// The heap the tree holds, by array: name, bytes, items (for `cargo
    /// xtask parse-stats`).
    pub fn tree_heap(&self) -> Vec<(&'static str, usize, usize)> {
        self.tree.heap()
    }
}

/// Parses `src`, the raw bytes of `file`. The tree keeps a copy of them.
pub fn parse(src: &[u8], file: FileId) -> Parse {
    parse_keeping(src, file, SourceText::new(src))
}

/// [`parse`] of a text the tree shares instead of copying it.
pub fn parse_shared(text: &Arc<str>, file: FileId) -> Parse {
    parse_keeping(text.as_bytes(), file, SourceText::Shared(text.clone()))
}

/// [`parse`] of a text compiled into the program (the WoW API), which the
/// tree refers to instead of copying it.
pub fn parse_static(text: &'static str, file: FileId) -> Parse {
    parse_keeping(text.as_bytes(), file, SourceText::Static(text))
}

/// Parses `src` into a tree that keeps `text`, the same bytes.
fn parse_keeping(src: &[u8], file: FileId, text: SourceText) -> Parse {
    let mut lexed = lexer::lex(src, file);
    let mut diagnostics = std::mem::take(&mut lexed.diagnostics);
    let mut parser = parser::Parser::new(src, file, lexed, text);
    parser.chunk();
    diagnostics.append(&mut parser.diagnostics);
    diagnostics.sort_by_key(|d| d.labels.first().map(|l| l.span.start).unwrap_or(0));
    Parse {
        tree: parser.t,
        lexed: parser.lexed,
        diagnostics,
    }
}

/// Whether `src` (the raw bytes of `file`) parses without errors: always
/// `!parse(src, file).has_errors()`, found without keeping a syntax tree or
/// trivia. For callers that need only the answer (the build's check of its
/// own output).
pub fn parses(src: &[u8], file: FileId) -> bool {
    let answer = validate(src, file);
    #[cfg(debug_assertions)]
    {
        let full = parse(src, file);
        assert_eq!(
            answer,
            !full.has_errors(),
            "`parses` disagrees with `parse` on {file:?}: {:?}",
            full.diagnostics.first().map(|d| &d.message)
        );
    }
    answer
}

fn validate(src: &[u8], file: FileId) -> bool {
    let lexed = lexer::lex_with(src, file, lexer::LexOptions { trivia: false });
    if lexed.diagnostics.iter().any(Diagnostic::is_error) {
        return false;
    }
    let mut parser = parser::Parser::validating(src, file, lexed);
    parser.chunk();
    if parser.unsure {
        return !parse(src, file).has_errors();
    }
    !parser.diagnostics.iter().any(Diagnostic::is_error)
}
