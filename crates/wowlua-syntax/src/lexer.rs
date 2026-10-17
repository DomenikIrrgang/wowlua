//! A byte-oriented lexer implementing Lua 5.1's lexical grammar exactly
//! (llex.c), plus the wowlua tokens of spec §1.2 (compiler §13.5.1).
//!
//! Lua 5.1 details that matter for accept/reject parity:
//! - line breaks are `\n`, `\r`, `\r\n` and `\n\r` (each counts as one line),
//! - numerals are scanned like `read_numeral` (digits and dots, an optional
//!   exponent sign, then any letters, digits and `_`) and only then converted,
//! - `[[` inside a level-0 long string or comment is an error
//!   (`LUA_COMPAT_LSTR`),
//! - names are ASCII letters, digits and `_` (the C locale).

use crate::token::{Token, TokenKind, Tokens, Trivia, TriviaKind};
use std::borrow::Cow;
use wowlua_diagnostics::Diagnostic;
use wowlua_span::{FileId, Span};

/// The result of lexing one file.
#[derive(Debug, Default)]
pub struct Lexed {
    pub tokens: Tokens,
    pub trivia: Vec<Trivia>,
    /// Byte offset where each Lua line starts (line `n` starts at
    /// `line_starts[n - 1]`).
    pub line_starts: Vec<u32>,
    pub diagnostics: Vec<Diagnostic>,
    /// A leading UTF-8 byte order mark was skipped.
    pub bom: bool,
    /// Byte range of a leading `#!` line (without its line break).
    pub shebang: Option<(u32, u32)>,
}

impl Lexed {
    /// The Lua line (1-based) containing byte `offset`.
    pub fn line_of(&self, offset: u32) -> u32 {
        match self.line_starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }

    /// The column (0-based, in bytes) of byte `offset`.
    pub fn column_of(&self, offset: u32) -> u32 {
        let line = self.line_of(offset);
        offset - self.line_starts[line as usize - 1]
    }

    /// The leading trivia of token `index`: the trivia between the previous
    /// token's end (the file's start for the first) and its start.
    pub fn trivia_of(&self, index: usize) -> &[Trivia] {
        let from = match index {
            0 => 0,
            i => self.tokens.end(i - 1),
        };
        let to = self.tokens.start(index);
        let lo = self.trivia.partition_point(|t| t.start < from);
        let hi = lo + self.trivia[lo..].partition_point(|t| t.start < to);
        &self.trivia[lo..hi]
    }

    /// Frees the tokens; the trivia and line starts stay. For parses kept
    /// past the parse whose readers need no tokens (#19): a data addon
    /// has tens of millions of them.
    pub fn drop_tokens(&mut self) {
        self.tokens = Tokens::default();
    }

    /// Where the code of `src` (these tokens' source) starts: after a byte
    /// order mark and a `#!` line.
    pub fn code_start(&self) -> u32 {
        match self.shebang {
            Some((_, end)) => end,
            None if self.bom => 3,
            None => 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Inside a backtick string.
    Text,
    /// Inside `{…}` of a backtick string; `depth` counts open table braces.
    Expr { depth: u32 },
}

struct Lexer<'a> {
    src: &'a [u8],
    file: FileId,
    pos: usize,
    out: Lexed,
    modes: Vec<Mode>,
    /// Trivia is recorded (`LexOptions::trivia`).
    keep_trivia: bool,
}

/// What [`lex_with`] records.
#[derive(Clone, Copy, Debug)]
pub struct LexOptions {
    /// Records line breaks and comments in `Lexed::trivia`. Without it they
    /// are still scanned and checked, with the same diagnostics, but
    /// `trivia` stays empty.
    pub trivia: bool,
}

/// Lexes `src`, the raw bytes of `file`.
pub fn lex(src: &[u8], file: FileId) -> Lexed {
    lex_with(src, file, LexOptions { trivia: true })
}

/// [`lex`] with `options`.
pub fn lex_with(src: &[u8], file: FileId, options: LexOptions) -> Lexed {
    let mut lexer = Lexer {
        src,
        file,
        pos: 0,
        out: Lexed {
            line_starts: vec![0],
            ..Lexed::default()
        },
        modes: Vec::new(),
        keep_trivia: options.trivia,
    };
    lexer.run();
    // Growing by doubling leaves up to half of each list unused, and the
    // lists live as long as the tree (#19).
    lexer.out.tokens.shrink_to_fit();
    lexer.out.trivia.shrink_to_fit();
    lexer.out.line_starts.shrink_to_fit();
    lexer.out
}

/// The start of the first token at or after `offset` in `src` (the bytes
/// of `file`), without the tokens: past the spaces, line breaks and
/// comments there, or `src.len()` (the end-of-file token's start) when only
/// those follow. `offset` must be where code may start
/// ([`Lexed::code_start`], a token's end), not inside a backtick string's
/// text.
pub fn skip_trivia(src: &[u8], file: FileId, offset: u32) -> u32 {
    let at = (offset as usize).min(src.len());
    // Mostly a token follows at once (`,` after a field).
    match src.get(at..).unwrap_or_default() {
        [b'-', b'-', ..] | [b' ' | b'\t' | 0x0B | 0x0C | b'\n' | b'\r', ..] => {}
        _ => return at as u32,
    }
    let mut lexer = Lexer {
        src,
        file,
        pos: at,
        out: Lexed::default(),
        modes: Vec::new(),
        keep_trivia: false,
    };
    lexer.trivia();
    lexer.pos as u32
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | 0x0B | 0x0C)
}

impl<'a> Lexer<'a> {
    fn peek(&self, ahead: usize) -> u8 {
        self.src.get(self.pos + ahead).copied().unwrap_or(0)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.file, start..end)
    }

    fn error(&mut self, code: &'static str, start: usize, end: usize, message: impl Into<String>) {
        let span = self.span(start, end.max(start));
        self.out
            .diagnostics
            .push(Diagnostic::error(code, message).with_label(span, "here"));
    }

    /// Length of the line break at the current position (0 if none).
    fn newline_len(&self) -> usize {
        match self.peek(0) {
            c @ (b'\n' | b'\r') => {
                let next = self.peek(1);
                if (next == b'\n' || next == b'\r') && next != c {
                    2
                } else {
                    1
                }
            }
            _ => 0,
        }
    }

    /// Consumes a line break (the caller checked there is one).
    fn consume_newline(&mut self) {
        let len = self.newline_len();
        self.pos += len;
        self.out.line_starts.push(self.pos as u32);
    }

    fn run(&mut self) {
        if self.src.starts_with(b"\xEF\xBB\xBF") {
            self.pos = 3;
            self.out.bom = true;
        }
        if self.src[self.pos..].starts_with(b"#!") {
            let start = self.pos;
            while !self.at_end() && self.newline_len() == 0 {
                self.pos += 1;
            }
            self.out.shebang = Some((start as u32, self.pos as u32));
        }
        loop {
            if let Some(Mode::Text) = self.modes.last() {
                self.interp_text();
                continue;
            }
            self.trivia();
            let start = self.pos;
            let kind = if self.at_end() {
                TokenKind::Eof
            } else {
                self.token()
            };
            self.out.tokens.push(Token {
                kind,
                start: start as u32,
                end: self.pos as u32,
            });
            if kind == TokenKind::Eof {
                if let Some(Mode::Expr { .. }) = self.modes.last() {
                    // Reported by the parser ("'}' expected").
                }
                break;
            }
        }
    }

    fn push_trivia(&mut self, kind: TriviaKind, start: usize) {
        if !self.keep_trivia {
            return;
        }
        self.out.trivia.push(Trivia {
            kind,
            start: start as u32,
            end: self.pos as u32,
        });
    }

    // Inlined into the token loop: called from `skip_trivia` too, it would
    // not be, and lexing would cost 2% more.
    #[inline(always)]
    fn trivia(&mut self) {
        loop {
            let start = self.pos;
            let c = self.peek(0);
            if self.at_end() {
                return;
            }
            if is_space(c) {
                while !self.at_end() && is_space(self.peek(0)) {
                    self.pos += 1;
                }
            } else if self.newline_len() > 0 {
                self.consume_newline();
                self.push_trivia(TriviaKind::Newline, start);
            } else if c == b'-' && self.peek(1) == b'-' {
                self.pos += 2;
                if self.peek(0) == b'[' {
                    let save = self.pos;
                    if let Some(level) = self.long_bracket_level() {
                        self.long_bracket_body(level, start, true);
                        self.push_trivia(TriviaKind::BlockComment, start);
                        continue;
                    }
                    self.pos = save;
                }
                while !self.at_end() && self.newline_len() == 0 {
                    self.pos += 1;
                }
                self.push_trivia(TriviaKind::LineComment, start);
            } else {
                return;
            }
        }
    }

    /// At `[`: if a long bracket opens here (`[`, `=`*, `[`), consumes it and
    /// returns its level. Otherwise consumes nothing and returns `None`.
    fn long_bracket_level(&mut self) -> Option<usize> {
        let mut i = 1;
        while self.peek(i) == b'=' {
            i += 1;
        }
        if self.peek(i) == b'[' {
            self.pos += i + 1;
            Some(i - 1)
        } else {
            None
        }
    }

    /// Reads a long string or comment body after its opening bracket, up to
    /// and including the closing bracket.
    fn long_bracket_body(&mut self, level: usize, start: usize, comment: bool) {
        loop {
            if self.at_end() {
                let what = if comment { "comment" } else { "string" };
                self.error("E0102", start, self.pos, format!("unfinished long {what}"));
                return;
            }
            match self.peek(0) {
                b']' => {
                    let mut i = 1;
                    while self.peek(i) == b'=' {
                        i += 1;
                    }
                    if self.peek(i) == b']' && i - 1 == level {
                        self.pos += i + 1;
                        return;
                    }
                    self.pos += 1;
                }
                b'[' => {
                    let mut i = 1;
                    while self.peek(i) == b'=' {
                        i += 1;
                    }
                    if self.peek(i) == b'[' && i - 1 == level && level == 0 {
                        // Lua 5.1 with LUA_COMPAT_LSTR rejects this.
                        self.error(
                            "E0101",
                            self.pos,
                            self.pos + 2,
                            "nesting of [[...]] is deprecated",
                        );
                    }
                    self.pos += 1;
                }
                b'\n' | b'\r' => self.consume_newline(),
                _ => self.pos += 1,
            }
        }
    }

    fn token(&mut self) -> TokenKind {
        use TokenKind::*;
        let c = self.peek(0);
        let start = self.pos;
        if is_name_start(c) {
            while is_name_char(self.peek(0)) {
                self.pos += 1;
            }
            return TokenKind::keyword(&self.src[start..self.pos]).unwrap_or(Name);
        }
        if c.is_ascii_digit() || (c == b'.' && self.peek(1).is_ascii_digit()) {
            self.number();
            return Number;
        }
        let two = (c, self.peek(1));
        let three = self.peek(2);
        let (kind, len) = match c {
            b'"' | b'\'' => {
                self.short_string(c);
                return String;
            }
            b'[' => {
                if let Some(level) = self.long_bracket_level() {
                    self.long_bracket_body(level, start, false);
                    return LongString;
                }
                if self.peek(1) == b'=' {
                    // `[=` that does not open a long string.
                    let mut i = 1;
                    while self.peek(i) == b'=' {
                        i += 1;
                    }
                    self.pos += i;
                    self.error("E0101", start, self.pos, "invalid long string delimiter");
                    return Error;
                }
                (LBracket, 1)
            }
            b'`' => {
                self.pos += 1;
                self.modes.push(Mode::Text);
                return InterpStart;
            }
            b'{' => {
                if let Some(Mode::Expr { depth }) = self.modes.last_mut() {
                    *depth += 1;
                }
                (LBrace, 1)
            }
            b'}' => {
                if let Some(Mode::Expr { depth }) = self.modes.last_mut() {
                    if *depth == 0 {
                        self.modes.pop();
                        self.pos += 1;
                        return InterpClose;
                    }
                    *depth -= 1;
                }
                (RBrace, 1)
            }
            b'+' => {
                if two.1 == b'=' {
                    (PlusEq, 2)
                } else {
                    (Plus, 1)
                }
            }
            b'-' => match two.1 {
                b'=' => (MinusEq, 2),
                b'>' => (Arrow, 2),
                _ => (Minus, 1),
            },
            b'*' => {
                if two.1 == b'=' {
                    (StarEq, 2)
                } else {
                    (Star, 1)
                }
            }
            b'/' => match (two.1, three) {
                (b'/', b'=') => (SlashSlashEq, 3),
                (b'/', _) => (SlashSlash, 2),
                (b'=', _) => (SlashEq, 2),
                _ => (Slash, 1),
            },
            b'%' => {
                if two.1 == b'=' {
                    (PercentEq, 2)
                } else {
                    (Percent, 1)
                }
            }
            b'^' => {
                if two.1 == b'=' {
                    (CaretEq, 2)
                } else {
                    (Caret, 1)
                }
            }
            b'#' => (Hash, 1),
            b'=' => {
                if two.1 == b'=' {
                    (EqEq, 2)
                } else {
                    (Eq, 1)
                }
            }
            b'~' => {
                if two.1 == b'=' {
                    (NotEq, 2)
                } else {
                    (Tilde, 1)
                }
            }
            b'<' => match (two.1, three) {
                (b'<', b'=') => (ShlEq, 3),
                (b'<', _) => (Shl, 2),
                (b'=', _) => (LtEq, 2),
                _ => (Lt, 1),
            },
            b'>' => match (two.1, three) {
                (b'>', b'=') => (ShrEq, 3),
                (b'>', _) => (Shr, 2),
                (b'=', _) => (GtEq, 2),
                _ => (Gt, 1),
            },
            b'(' => (LParen, 1),
            b')' => (RParen, 1),
            b']' => (RBracket, 1),
            b';' => (Semi, 1),
            b':' => {
                if two.1 == b':' {
                    (ColonColon, 2)
                } else {
                    (Colon, 1)
                }
            }
            b',' => (Comma, 1),
            b'.' => match (two.1, three) {
                (b'.', b'.') => (Ellipsis, 3),
                (b'.', b'=') => (DotDotEq, 3),
                (b'.', _) => (DotDot, 2),
                _ => (Dot, 1),
            },
            b'@' => (At, 1),
            b'?' => match (two.1, three) {
                (b'?', b'=') => (QuestionQuestionEq, 3),
                (b'?', _) => (QuestionQuestion, 2),
                (b'.', _) => (QuestionDot, 2),
                (b':', _) => (QuestionColon, 2),
                _ => (Question, 1),
            },
            b'&' => {
                if two.1 == b'=' {
                    (AmpEq, 2)
                } else {
                    (Amp, 1)
                }
            }
            b'|' => {
                if two.1 == b'=' {
                    (PipeEq, 2)
                } else {
                    (Pipe, 1)
                }
            }
            _ => {
                // Not a token. One UTF-8 character, so diagnostics stay readable.
                let len = match c {
                    0xC0..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    0xF0..=0xF7 => 4,
                    _ => 1,
                };
                (Error, len.min(self.src.len() - self.pos))
            }
        };
        self.pos += len;
        kind
    }

    /// Scans a numeral like Lua 5.1's `read_numeral` (allowing `_` among the
    /// leading digits for wowlua digit separators) and validates it.
    fn number(&mut self) {
        let start = self.pos;
        while matches!(self.peek(0), b'0'..=b'9' | b'.' | b'_') {
            self.pos += 1;
        }
        if matches!(self.peek(0), b'e' | b'E') {
            self.pos += 1;
            if matches!(self.peek(0), b'+' | b'-') {
                self.pos += 1;
            }
        }
        while is_name_char(self.peek(0)) {
            self.pos += 1;
        }
        let raw = &self.src[start..self.pos];
        if !is_plain_decimal(raw)
            && let Err(message) = number_value(raw)
        {
            let text = String::from_utf8_lossy(raw).into_owned();
            self.error("E0103", start, self.pos, format!("{message} near '{text}'"));
        }
    }

    /// Reads a quoted string like Lua 5.1's `read_string`.
    fn short_string(&mut self, quote: u8) {
        let start = self.pos;
        self.pos += 1;
        loop {
            if self.at_end() {
                self.error("E0102", start, self.pos, "unfinished string");
                return;
            }
            let c = self.peek(0);
            if c == quote {
                self.pos += 1;
                return;
            }
            match c {
                b'\n' | b'\r' => {
                    self.error("E0102", start, self.pos, "unfinished string");
                    return;
                }
                b'\\' => {
                    self.pos += 1;
                    match self.peek(0) {
                        b'\n' | b'\r' => self.consume_newline(),
                        b'0'..=b'9' => {
                            let digits_start = self.pos;
                            let mut value = 0u32;
                            let mut n = 0;
                            while n < 3 && self.peek(0).is_ascii_digit() {
                                value = value * 10 + u32::from(self.peek(0) - b'0');
                                self.pos += 1;
                                n += 1;
                            }
                            if value > 255 {
                                self.error(
                                    "E0101",
                                    digits_start - 1,
                                    self.pos,
                                    "escape sequence too large",
                                );
                            }
                        }
                        _ if self.at_end() => {}
                        _ => self.pos += 1,
                    }
                }
                _ => self.pos += 1,
            }
        }
    }

    /// Inside a backtick string: reads text up to `{`, `` ` `` or the end.
    fn interp_text(&mut self) {
        let start = self.pos;
        let mut kind = None;
        while kind.is_none() {
            if self.at_end() {
                break;
            }
            match self.peek(0) {
                b'`' | b'{' => kind = Some(self.peek(0)),
                b'}' => {
                    self.error(
                        "E0116",
                        self.pos,
                        self.pos + 1,
                        "unbalanced `}` in an interpolated string; write `\\}` for a literal brace",
                    );
                    self.pos += 1;
                }
                b'\\' => self.interp_escape(),
                b'\n' | b'\r' => self.consume_newline(),
                _ => self.pos += 1,
            }
        }
        if self.pos > start {
            self.push_token(TokenKind::InterpText, start);
        }
        let start = self.pos;
        match kind {
            Some(b'`') => {
                self.pos += 1;
                self.modes.pop();
                self.push_token(TokenKind::InterpEnd, start);
            }
            Some(_) => {
                self.pos += 1;
                self.modes.push(Mode::Expr { depth: 0 });
                self.push_token(TokenKind::InterpOpen, start);
            }
            None => {
                // End of file inside the string: the parser reports the
                // missing backtick; leave text mode.
                self.error("E0102", start, start, "unfinished interpolated string");
                self.modes.clear();
            }
        }
    }

    fn push_token(&mut self, kind: TokenKind, start: usize) {
        // No trivia of its own: nothing lies between the previous token's
        // end and its start.
        self.out.tokens.push(Token {
            kind,
            start: start as u32,
            end: self.pos as u32,
        });
    }

    /// Validates one escape in a backtick string (spec §1.2.4).
    fn interp_escape(&mut self) {
        let start = self.pos;
        self.pos += 1;
        let c = self.peek(0);
        match c {
            b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'\\' | b'"' | b'\'' | b'{' | b'}'
            | b'`' | b'z' => self.pos += 1,
            b'\n' | b'\r' => self.consume_newline(),
            b'0'..=b'9' => {
                let mut value = 0u32;
                let mut n = 0;
                while n < 3 && self.peek(0).is_ascii_digit() {
                    value = value * 10 + u32::from(self.peek(0) - b'0');
                    self.pos += 1;
                    n += 1;
                }
                if value > 255 {
                    self.error("E0105", start, self.pos, "escape sequence too large");
                }
            }
            b'x' => {
                self.pos += 1;
                if self.peek(0).is_ascii_hexdigit() && self.peek(1).is_ascii_hexdigit() {
                    self.pos += 2;
                } else {
                    self.error(
                        "E0105",
                        start,
                        self.pos,
                        "`\\x` needs two hexadecimal digits",
                    );
                }
            }
            b'u' => {
                self.pos += 1;
                let mut ok = self.peek(0) == b'{';
                if ok {
                    self.pos += 1;
                    let digits_start = self.pos;
                    while self.peek(0).is_ascii_hexdigit() {
                        self.pos += 1;
                    }
                    let digits = &self.src[digits_start..self.pos];
                    let value = std::str::from_utf8(digits)
                        .ok()
                        .and_then(|d| u32::from_str_radix(d, 16).ok());
                    ok = !digits.is_empty()
                        && self.peek(0) == b'}'
                        && value.is_some_and(|v| v <= 0x7FFF_FFFF);
                    if self.peek(0) == b'}' {
                        self.pos += 1;
                    }
                }
                if !ok {
                    self.error("E0105", start, self.pos, "`\\u` needs the form `\\u{XXXX}`");
                }
            }
            _ => {
                if !self.at_end() {
                    self.pos += 1;
                }
                let text = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                self.error(
                    "E0105",
                    start,
                    self.pos,
                    format!("invalid escape `{text}` in a backtick string"),
                );
            }
        }
    }
}

/// A numeral in wowlua's own form: digit separators or a binary literal
/// (spec §1.2.2), in either case of `0b`.
pub fn is_wowlua_numeral(raw: &str) -> bool {
    raw.contains('_')
        || raw
            .as_bytes()
            .get(..2)
            .is_some_and(|p| p.eq_ignore_ascii_case(b"0b"))
}

/// Digits with at most one `.` (`12`, `0.5`, `3.`, `.5`): always a valid
/// numeral, known without converting it. Most numerals are like this.
fn is_plain_decimal(raw: &[u8]) -> bool {
    let mut dots = 0;
    let mut digits = 0;
    for &c in raw {
        match c {
            b'0'..=b'9' => digits += 1,
            b'.' => dots += 1,
            _ => return false,
        }
    }
    digits > 0 && dots <= 1
}

/// The value of a numeral, or why it is malformed. Lua 5.1 numerals follow
/// `luaO_str2d` (C `strtod`, hexadecimal through `strtoul`); wowlua adds digit
/// separators and binary literals (spec §1.2.2).
pub fn number_value(raw: &[u8]) -> Result<f64, &'static str> {
    if is_plain_decimal(raw) {
        // What `lua_number` ends with for these.
        let text = std::str::from_utf8(raw).expect("ASCII");
        return Ok(text.parse().expect("a plain decimal"));
    }
    let text = std::str::from_utf8(raw).map_err(|_| "malformed number")?;
    // Most numerals have no upper-case letter: those are read in place.
    let lower = if text.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(text.to_ascii_lowercase())
    } else {
        Cow::Borrowed(text)
    };
    if !is_wowlua_numeral(text) {
        return lua_number(&lower).ok_or("malformed number");
    }
    // Digit separators: each `_` sits between two digits of the same run.
    let (prefix, digits, radix) = if let Some(rest) = lower.strip_prefix("0x") {
        ("0x", rest, 16)
    } else if let Some(rest) = lower.strip_prefix("0b") {
        ("0b", rest, 2)
    } else {
        ("", &*lower, 10)
    };
    let is_digit = |c: u8| match radix {
        16 => c.is_ascii_hexdigit(),
        2 => c == b'0' || c == b'1',
        _ => c.is_ascii_digit(),
    };
    let bytes = digits.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        if c == b'_' {
            let before = i.checked_sub(1).map(|j| bytes[j]);
            let after = bytes.get(i + 1).copied();
            let ok = |b: Option<u8>| {
                b.is_some_and(|b| {
                    // In a decimal exponent, `e` is not a digit.
                    is_digit(b) && !(radix == 10 && b == b'e')
                })
            };
            if !ok(before) || !ok(after) {
                return Err("malformed number (misplaced `_` digit separator)");
            }
        }
    }
    let stripped: String = digits.chars().filter(|&c| c != '_').collect();
    match prefix {
        "0b" => {
            if stripped.is_empty() || !stripped.bytes().all(|c| c == b'0' || c == b'1') {
                return Err("malformed number");
            }
            Ok(stripped
                .bytes()
                .fold(0.0, |acc, c| acc * 2.0 + f64::from(c - b'0')))
        }
        "0x" => lua_number(&format!("0x{stripped}")).ok_or("malformed number"),
        _ => lua_number(&stripped).ok_or("malformed number"),
    }
}

/// Lua 5.1 `luaO_str2d` on a lowercase numeral without separators.
fn lua_number(text: &str) -> Option<f64> {
    if let Some(hex) = text.strip_prefix("0x") {
        // strtod (C99) reads hexadecimal with an optional `p` exponent; the
        // scanner never includes a `.` or exponent sign after `0x`.
        let (mantissa, exponent) = match hex.split_once('p') {
            Some((m, e)) => (m, Some(e)),
            None => (hex, None),
        };
        if mantissa.is_empty() || !mantissa.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let mut value = mantissa.bytes().fold(0.0f64, |acc, c| {
            acc * 16.0 + f64::from((c as char).to_digit(16).unwrap_or(0))
        });
        if let Some(e) = exponent {
            if e.is_empty() || !e.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let e: i32 = e.parse().unwrap_or(i32::MAX);
            value *= 2f64.powi(e);
        }
        return Some(value);
    }
    // Decimal: digits [. digits] [e [+-] digits], at least one mantissa digit.
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut mantissa_digits = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
        mantissa_digits += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
            mantissa_digits += 1;
        }
    }
    if mantissa_digits == 0 {
        return None;
    }
    if i < bytes.len() && bytes[i] == b'e' {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return None;
        }
    }
    if i != bytes.len() {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{is_plain_decimal, is_wowlua_numeral, lex, number_value, skip_trivia};
    use wowlua_span::SourceDb;

    // From a token's end (or where the code starts), the scan finds the next
    // token's start, as the lexer does.
    #[test]
    fn skipping_trivia_finds_the_next_token() {
        let mut db = SourceDb::new();
        for src in [
            "",
            "   ",
            "-- only a comment",
            "--[[ open",
            "\u{feff}#!/bin/lua\nx = 1",
            "#!/bin/lua",
            "\u{feff}x",
            "local a = 1 -- c\n--[==[ long\n]==]\n\r\n  b = {1, 2,\n 3}",
            "f(a --[[x]], b)\n-- last\n",
            "if x then\treturn end --[[ unfinished",
            "a = -- \n --[[]] --[=[]]]=] -1",
        ] {
            let file = db.add("t.lua", src.to_string());
            let lexed = lex(src.as_bytes(), file);
            let tokens = &lexed.tokens;
            let mut from = lexed.code_start();
            for i in 0..tokens.len() {
                let start = tokens.start(i);
                assert_eq!(
                    skip_trivia(src.as_bytes(), file, from),
                    start,
                    "{src:?} at {from}"
                );
                from = tokens.end(i);
            }
        }
    }

    // The fast path gives what the full conversion gives.
    #[test]
    fn plain_decimals_are_valid_numerals() {
        let parts = ["", "0", "7", "00", "123", ".", "..", "e", "_", "x"];
        let mut plain = 0;
        for a in parts {
            for b in parts {
                for c in parts {
                    let raw = format!("{a}{b}{c}");
                    if is_plain_decimal(raw.as_bytes()) {
                        plain += 1;
                        // As before the fast path: lowercased, then `lua_number`.
                        let slow = super::lua_number(&raw.to_ascii_lowercase());
                        assert_eq!(number_value(raw.as_bytes()).ok(), slow, "{raw:?}");
                        assert!(slow.is_some(), "{raw:?}");
                    }
                }
            }
        }
        assert!(plain > 50);
        let long = "9".repeat(400) + "." + &"9".repeat(400);
        assert!(is_plain_decimal(long.as_bytes()));
        assert_eq!(number_value(long.as_bytes()).ok(), super::lua_number(&long));
        assert!(!is_plain_decimal(b".") && !is_plain_decimal(b"1.2.3"));
    }

    // The same answer as lowercasing the numeral and testing for `_` and a
    // leading `0b`, which is what callers did before.
    #[test]
    fn wowlua_numerals_without_lowercasing() {
        let parts = [
            "", "0", "1", "b", "B", "x", "X", "_", "0b", "0B", "0x", "e", "E", ".",
        ];
        for a in parts {
            for b in parts {
                for c in parts {
                    let raw = format!("{a}{b}{c}");
                    let old = raw.contains('_') || raw.to_ascii_lowercase().starts_with("0b");
                    assert_eq!(is_wowlua_numeral(&raw), old, "{raw:?}");
                }
            }
        }
    }
}
