//! The pipeline before #19 phase 4b: the recursive-descent parser building
//! the boxed tree, flattened afterwards. Kept for the differential test of
//! the parser that builds the flat tree directly; removed once that has
//! passed over the corpus.

pub(crate) mod ast;
mod ast_visit;
mod flatten;
pub(crate) mod parser;

use crate::{Parse, deep, lexer};
use wowlua_span::FileId;

/// `crate::parse`, the legacy way.
pub(crate) fn parse(src: &[u8], file: FileId) -> Parse {
    let mut lexed = lexer::lex(src, file);
    let mut diagnostics = std::mem::take(&mut lexed.diagnostics);
    let mut parser = parser::Parser::new(src, file, lexed);
    let chunk = parser.chunk();
    let tree = flatten::from_ast(&chunk, src, file);
    if deep::links() >= deep::DEEP {
        deep::run(move || drop(chunk));
    } else {
        drop(chunk);
    }
    diagnostics.append(&mut parser.diagnostics);
    diagnostics.sort_by_key(|d| d.labels.first().map(|l| l.span.start).unwrap_or(0));
    Parse {
        tree,
        lexed: parser.lexed,
        diagnostics,
    }
}

#[cfg(test)]
mod tests;
