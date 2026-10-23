//! Hand-written recursive-descent parser with Pratt-style expression parsing
//! (spec compiler §13.5.2). Accepts exactly what Lua 5.1 accepts for pure Lua
//! input, plus the wowlua grammar of spec appendix A. Errors are recovered at
//! statement boundaries, so one run reports several independent errors.
//!
//! It builds the flat [`Tree`] directly (#19): an expression or type comes
//! back as a node value, pushed into its array when its parent is built; the
//! items of a list stay values until the list ends and are then pushed
//! together, so that every list is a contiguous range of its array.

use crate::lexer::Lexed;
use crate::literal;
use crate::ops::*;
use crate::search::partition_point_near;
use crate::token::{Token, TokenKind};
use crate::tree::*;
use std::cell::Cell;
use wowlua_diagnostics::Diagnostic;
use wowlua_span::{FileId, Span};

use TokenKind as T;

/// Lua 5.1's `LUAI_MAXCCALLS`: nested blocks and subexpressions.
const MAX_LEVELS: u32 = 200;

#[derive(Debug)]
pub(crate) struct Failed;

type PResult<T> = Result<T, Failed>;

#[derive(Default)]
struct FuncCtx {
    vararg: bool,
    loops: Vec<LoopCtx>,
}

struct LoopCtx {
    has_continue: bool,
}

/// A parameter of a parenthesized type group, its type not pushed yet (the
/// types of a pack are one list).
struct GroupParam {
    name: Option<Sp>,
    ty: TypeNode,
}

pub(crate) struct Parser<'a> {
    src: &'a [u8],
    file: FileId,
    pub(crate) lexed: Lexed,
    /// The tree being built.
    pub(crate) t: Tree,
    pos: usize,
    pub(crate) diagnostics: Vec<Diagnostic>,
    depth: u32,
    funcs: Vec<FuncCtx>,
    /// Nesting of blocks inside the chunk (0 = top level).
    block_depth: u32,
    /// Position of the last error, to avoid reporting twice at one token.
    last_error_at: Option<usize>,
    /// The fields of the tables being parsed, innermost last; a finished
    /// table pushes its own together.
    field_stack: Vec<FieldData>,
    /// The same for the expression lists being parsed.
    expr_stack: Vec<ExprNode>,
    /// The same for the statements of the blocks being parsed.
    stmt_stack: Vec<StmtNode>,
    /// Where the last line lookup ended in `lexed.line_starts`: the parser
    /// moves forward, so the next lookup starts its search there.
    line_hint: Cell<usize>,
    /// The tree is kept. Without it (`validating`) the parser only finds
    /// the diagnostics: it parses everything as usual, but keeps no block
    /// statements, expression lists or table fields, and decodes no
    /// literals. Every check that reads back one of those must set
    /// `unsure` when the tree is not kept, as `repeat_stat` does.
    keep_tree: bool,
    /// A check was skipped because the tree is not kept: the diagnostics
    /// may be incomplete.
    pub(crate) unsure: bool,
    /// The next parameter list is a constructor's: its parameters may be
    /// parameter properties (`private sink: Sink`, decision L35).
    ctor_params: bool,
}

fn is_block_follow(kind: TokenKind) -> bool {
    matches!(kind, T::Else | T::Elseif | T::End | T::Until | T::Eof)
}

fn compound_op(kind: TokenKind) -> Option<CompoundOp> {
    Some(match kind {
        T::PlusEq => CompoundOp::Add,
        T::MinusEq => CompoundOp::Sub,
        T::StarEq => CompoundOp::Mul,
        T::SlashEq => CompoundOp::Div,
        T::SlashSlashEq => CompoundOp::IDiv,
        T::PercentEq => CompoundOp::Mod,
        T::CaretEq => CompoundOp::Pow,
        T::DotDotEq => CompoundOp::Concat,
        T::AmpEq => CompoundOp::BAnd,
        T::PipeEq => CompoundOp::BOr,
        T::ShlEq => CompoundOp::Shl,
        T::ShrEq => CompoundOp::Shr,
        T::QuestionQuestionEq => CompoundOp::Coalesce,
        _ => return None,
    })
}

fn binary_op(kind: TokenKind) -> Option<BinOp> {
    Some(match kind {
        T::Plus => BinOp::Add,
        T::Minus => BinOp::Sub,
        T::Star => BinOp::Mul,
        T::Slash => BinOp::Div,
        T::SlashSlash => BinOp::IDiv,
        T::Percent => BinOp::Mod,
        T::Caret => BinOp::Pow,
        T::DotDot => BinOp::Concat,
        T::EqEq => BinOp::Eq,
        T::NotEq => BinOp::Ne,
        T::Lt => BinOp::Lt,
        T::LtEq => BinOp::Le,
        T::Gt => BinOp::Gt,
        T::GtEq => BinOp::Ge,
        T::And => BinOp::And,
        T::Or => BinOp::Or,
        T::QuestionQuestion => BinOp::Coalesce,
        T::Amp => BinOp::BAnd,
        T::Pipe => BinOp::BOr,
        T::Tilde => BinOp::BXor,
        T::Shl => BinOp::Shl,
        T::Shr => BinOp::Shr,
        _ => return None,
    })
}

fn unary_op(kind: TokenKind) -> Option<UnOp> {
    Some(match kind {
        T::Not => UnOp::Not,
        T::Minus => UnOp::Neg,
        T::Hash => UnOp::Len,
        T::Tilde => UnOp::BNot,
        _ => return None,
    })
}

const MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "static",
    "readonly",
    "override",
    "abstract",
    "overload",
];

fn sp(s: Span) -> Sp {
    Sp {
        start: s.start,
        end: s.end,
    }
}

fn opt_id<T>(x: Option<T>, f: impl FnOnce(T) -> u32) -> u32 {
    x.map_or(NONE, f)
}

/// Pushes `items` to `$self.t.$arr` together: their range.
macro_rules! push_all {
    ($self:ident, $arr:ident, $items:expr) => {{
        let start = $self.t.$arr.len() as u32;
        $self.t.$arr.extend($items);
        Range {
            start,
            len: $self.t.$arr.len() as u32 - start,
        }
    }};
}

/// Pushes one item to `$self.t.$arr`: its index.
macro_rules! push {
    ($self:ident, $arr:ident, $item:expr) => {{
        let item = $item;
        $self.t.$arr.push(item);
        $self.t.$arr.len() as u32 - 1
    }};
}

impl<'a> Parser<'a> {
    /// A parser of `src`, whose tree keeps `text` (`src` itself).
    pub(crate) fn new(src: &'a [u8], file: FileId, lexed: Lexed, text: SourceText) -> Self {
        Parser {
            src,
            file,
            lexed,
            t: Tree::empty(text, file),
            pos: 0,
            diagnostics: Vec::new(),
            depth: 0,
            funcs: vec![FuncCtx {
                vararg: true,
                loops: Vec::new(),
            }],
            block_depth: 0,
            last_error_at: None,
            field_stack: Vec::new(),
            expr_stack: Vec::new(),
            stmt_stack: Vec::new(),
            line_hint: Cell::new(0),
            keep_tree: true,
            unsure: false,
            ctor_params: false,
        }
    }

    /// A parser that keeps no tree (see `keep_tree`).
    pub(crate) fn validating(src: &'a [u8], file: FileId, lexed: Lexed) -> Self {
        Parser {
            keep_tree: false,
            ..Parser::new(src, file, lexed, SourceText::new(src))
        }
    }

    // -------------------------------------------------------------- nodes --

    fn push_expr(&mut self, e: ExprNode) -> u32 {
        if let Some(leaf) = Tree::leaf(&e) {
            let i = push!(self, leaves, leaf);
            assert!(i < LEAF - 1, "too many expressions");
            i | LEAF
        } else {
            let i = push!(self, exprs, e);
            assert!(i < LEAF, "too many expressions");
            i
        }
    }

    fn push_type(&mut self, t: TypeNode) -> u32 {
        push!(self, types, t)
    }

    fn push_block(&mut self, stmts: Range, span: Span) -> u32 {
        push!(
            self,
            blocks,
            BlockNode {
                stmts,
                span: sp(span),
            }
        )
    }

    fn expr_node(&self, span: Span, data: ExprData) -> ExprNode {
        ExprNode {
            span: sp(span),
            data,
        }
    }

    fn type_node(&self, span: Span, data: TypeData) -> TypeNode {
        TypeNode {
            span: sp(span),
            data,
        }
    }

    fn names(&mut self, names: Vec<Sp>) -> Range {
        push_all!(self, names, names)
    }

    fn types(&mut self, types: Vec<TypeNode>) -> Range {
        push_all!(self, types, types)
    }

    /// The text of the name at `s`.
    fn text_at(&self, s: Sp) -> &'a [u8] {
        &self.src[s.start as usize..s.end as usize]
    }

    // ------------------------------------------------------------ tokens --

    fn tok(&self, ahead: usize) -> Token {
        let tokens = &self.lexed.tokens;
        tokens.token((self.pos + ahead).min(tokens.len() - 1))
    }

    // The parser asks for kinds far more often than for whole tokens: read
    // only the kinds (tokens are stored by field).
    fn kind(&self) -> TokenKind {
        self.kind_at(0)
    }

    fn kind_at(&self, ahead: usize) -> TokenKind {
        let tokens = &self.lexed.tokens;
        tokens.kind((self.pos + ahead).min(tokens.len() - 1))
    }

    fn text_of(&self, token: &Token) -> &'a [u8] {
        &self.src[token.start as usize..token.end as usize]
    }

    /// The current token is the name `word`.
    fn is_word(&self, word: &str) -> bool {
        self.is_word_at(0, word)
    }

    fn is_word_at(&self, ahead: usize, word: &str) -> bool {
        let t = self.tok(ahead);
        t.kind == T::Name && self.text_of(&t) == word.as_bytes()
    }

    fn advance(&mut self) -> Token {
        let t = self.tok(0);
        if t.kind != T::Eof {
            self.pos += 1;
        }
        t
    }

    /// Steps over the current token without reading it (the caller knows
    /// its kind).
    fn bump(&mut self) {
        if self.kind() != T::Eof {
            self.pos += 1;
        }
    }

    /// Where token `ahead` starts.
    fn start_at(&self, ahead: usize) -> u32 {
        let tokens = &self.lexed.tokens;
        tokens.start((self.pos + ahead).min(tokens.len() - 1))
    }

    /// Where token `ahead` ends.
    fn end_at(&self, ahead: usize) -> u32 {
        let tokens = &self.lexed.tokens;
        tokens.end((self.pos + ahead).min(tokens.len() - 1))
    }

    /// Where the previous token ends.
    fn prev_end(&self) -> u32 {
        self.lexed.tokens.end(self.pos.saturating_sub(1))
    }

    fn span_from(&self, start: u32) -> Span {
        let end = if self.pos == 0 {
            start
        } else {
            self.prev_end()
        };
        Span {
            file: self.file,
            start,
            end: end.max(start),
        }
    }

    /// Lines are not stored with the tokens; they come from the line starts,
    /// the same way [`Lexed::line_of`] finds them: the number of line starts
    /// at or before the offset.
    fn line_count_to(&self, offset: u32) -> usize {
        let starts = &self.lexed.line_starts;
        let n = partition_point_near(starts, self.line_hint.get(), |&s| s <= offset);
        self.line_hint.set(n);
        n
    }

    /// The Lua line (1-based) containing byte `offset`.
    fn line_of(&self, offset: u32) -> u32 {
        self.line_count_to(offset) as u32
    }

    /// Bytes `from` and `to` (`from <= to`) lie on one Lua line: no line
    /// starts after `from` and at or before `to`.
    fn same_line(&self, from: u32, to: u32) -> bool {
        let n = self.line_count_to(from);
        self.lexed.line_starts.get(n).is_none_or(|&next| next > to)
    }

    fn token_span(&self, t: &Token) -> Span {
        Span {
            file: self.file,
            start: t.start,
            end: t.end,
        }
    }

    fn sp_span(&self, s: Sp) -> Span {
        Span {
            file: self.file,
            start: s.start,
            end: s.end,
        }
    }

    fn check(&self, kind: TokenKind) -> bool {
        self.kind() == kind
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    // ------------------------------------------------------- diagnostics --

    /// `'x'` for the current token, as Lua writes it after "near".
    fn near(&self) -> String {
        let t = self.tok(0);
        match t.kind {
            // Lua quotes it too: `near '<eof>'`.
            T::Eof => "'<eof>'".to_string(),
            T::InterpText | T::String | T::LongString | T::Number | T::Name | T::Error => {
                // A control character, as Lua names it (`luaX_token2str`).
                if let [b] = self.text_of(&t)
                    && (*b < 32 || *b == 127)
                {
                    return format!("'char({b})'");
                }
                let text = String::from_utf8_lossy(self.text_of(&t));
                let short: String = text.chars().take(40).collect();
                format!("'{short}'")
            }
            k => k.describe().to_string(),
        }
    }

    fn error_at(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(code, message).with_label(span, "here"));
    }

    /// Reports a syntax error at the current token (once per token).
    fn syntax_error(&mut self, message: impl Into<String>) -> Failed {
        if self.last_error_at != Some(self.pos) {
            self.last_error_at = Some(self.pos);
            let span = self.token_span(&self.tok(0).clone());
            // The lexer already reported invalid characters' neighbors.
            let message = format!("{} near {}", message.into(), self.near());
            self.error_at("E0101", span, message);
        }
        Failed
    }

    fn expect(&mut self, kind: TokenKind, what: &str) -> PResult<Token> {
        if self.check(kind) {
            Ok(self.advance())
        } else {
            Err(self.syntax_error(format!("'{what}' expected")))
        }
    }

    /// Lua's `check_match`: `close` closing `open`, the token that starts
    /// at byte `opened`.
    fn expect_match(&mut self, close: TokenKind, open: &str, opened: u32) -> PResult<Token> {
        if self.check(close) {
            return Ok(self.advance());
        }
        let what = close.describe();
        let failed = if self.same_line(opened, self.start_at(0)) {
            self.syntax_error(format!("{what} expected"))
        } else {
            let line = self.line_of(opened);
            self.syntax_error(format!(
                "{what} expected (to close '{open}' at line {line})"
            ))
        };
        // At the end of the file nothing can follow: the construct closes
        // there and keeps what it has, for the compiler's limits and the
        // editor.
        if self.check(T::Eof) {
            return Ok(self.tok(0));
        }
        Err(failed)
    }

    fn name(&mut self) -> PResult<Sp> {
        if self.check(T::Name) {
            let t = self.advance();
            Ok(self.make_name(&t))
        } else {
            Err(self.syntax_error("<name> expected"))
        }
    }

    /// A decorator name may be a reserved word (`@if`).
    fn decorator_name(&mut self) -> PResult<Sp> {
        if self.kind().is_keyword() {
            let t = self.advance();
            return Ok(self.make_name(&t));
        }
        self.name()
    }

    /// A name is its token: its text is the source there.
    fn make_name(&self, t: &Token) -> Sp {
        Sp {
            start: t.start,
            end: t.end,
        }
    }

    fn enter_level(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > MAX_LEVELS {
            let span = self.token_span(&self.tok(0).clone());
            if self.last_error_at != Some(self.pos) {
                self.last_error_at = Some(self.pos);
                self.diagnostics.push(
                    Diagnostic::error("E5001", "chunk has too many syntax levels")
                        .with_label(span, "nested too deeply")
                        .with_help("Lua 5.1 allows 200 nested blocks and subexpressions"),
                );
            }
            self.depth -= 1;
            return Err(Failed);
        }
        Ok(())
    }

    fn leave_level(&mut self) {
        self.depth -= 1;
    }

    fn func(&mut self) -> &mut FuncCtx {
        self.funcs.last_mut().expect("a function context")
    }

    // ------------------------------------------------------------- chunk --

    /// Parses the file: its tree, finished.
    pub(crate) fn chunk(&mut self) {
        self.t.directives = self.directives();
        let start = self.start_at(0);
        let mark = self.stmt_stack.len();
        loop {
            grow(|| self.block_stmts());
            if self.check(T::Eof) {
                break;
            }
            let _ = self.syntax_error("'<eof>' expected");
            self.bump();
        }
        let stmts = push_all!(self, stmts, self.stmt_stack.drain(mark..));
        debug_assert!(
            self.field_stack.is_empty() && self.expr_stack.is_empty() && self.stmt_stack.is_empty()
        );
        let end = self.src.len() as u32;
        self.t.root = self.push_block(
            stmts,
            Span {
                file: self.file,
                start,
                end,
            },
        );
        self.t.span = Sp { start: 0, end };
        self.t.shrink();
    }

    /// `--!name` comments before the first token (spec §1.1.1).
    fn directives(&self) -> Vec<Directive> {
        let mut out = Vec::new();
        // The lexer always ends with an `Eof` token, so there is a token 0.
        for trivia in self.lexed.trivia_of(0) {
            if trivia.kind != crate::token::TriviaKind::LineComment {
                continue;
            }
            let text = &self.src[trivia.start as usize..trivia.end as usize];
            if let Some(rest) = text.strip_prefix(b"--!") {
                let rest = String::from_utf8_lossy(rest);
                let name = rest.split_whitespace().next().unwrap_or("");
                if !name.is_empty() {
                    out.push(Directive {
                        name: name.into(),
                        span: Span {
                            file: self.file,
                            start: trivia.start,
                            end: trivia.end,
                        },
                    });
                }
            }
        }
        out
    }

    // ------------------------------------------------------------ blocks --

    /// A block: its index.
    fn block(&mut self) -> u32 {
        grow(|| {
            let start = self.start_at(0);
            let mark = self.stmt_stack.len();
            self.block_stmts();
            let stmts = push_all!(self, stmts, self.stmt_stack.drain(mark..));
            let span = self.span_from(start);
            self.push_block(stmts, span)
        })
    }

    /// The statements of a block, left on `stmt_stack`.
    fn block_stmts(&mut self) {
        if self.enter_level().is_err() {
            self.skip_to_block_end();
            return;
        }
        self.block_depth += 1;
        loop {
            if is_block_follow(self.kind()) {
                break;
            }
            let before = self.pos;
            let last = matches!(self.kind(), T::Return | T::Break) || self.is_continue();
            match self.statement() {
                Ok(stmt) => {
                    let is_continue = matches!(stmt.data, StmtData::Continue);
                    if self.keep_tree {
                        self.stmt_stack.push(stmt);
                    }
                    self.eat(T::Semi);
                    if last {
                        if !is_block_follow(self.kind()) {
                            if is_continue {
                                let span = self.token_span(&self.tok(0).clone());
                                self.error_at(
                                    "E0114",
                                    span,
                                    "`continue` must be the last statement of its block; write `do continue end`",
                                );
                            } else {
                                let _ = self.syntax_error("'end' expected");
                            }
                            // Keep parsing the rest of the block.
                            continue;
                        }
                        break;
                    }
                }
                Err(Failed) => {
                    self.recover(before);
                    if self.keep_tree {
                        let span = self.span_from(self.lexed.tokens.start(before));
                        self.stmt_stack.push(StmtNode {
                            span: sp(span),
                            decorators: Range::default(),
                            data: StmtData::Error,
                        });
                    }
                }
            }
        }
        self.block_depth -= 1;
        self.leave_level();
    }

    /// Skips to the next statement boundary after an error.
    fn recover(&mut self, before: usize) {
        let error_line = self.line_of(self.start_at(0));
        if self.pos == before {
            self.bump();
        }
        loop {
            // A name or `(` starting a later line likely starts a statement.
            if self.line_of(self.start_at(0)) > error_line
                && matches!(self.kind(), T::Name | T::LParen | T::At)
            {
                return;
            }
            match self.kind() {
                T::Eof | T::End | T::Else | T::Elseif | T::Until => return,
                T::Local
                | T::Function
                | T::If
                | T::For
                | T::While
                | T::Repeat
                | T::Return
                | T::Do
                | T::Break => return,
                T::Semi => {
                    self.bump();
                    return;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn skip_to_block_end(&mut self) {
        let mut depth = 0u32;
        loop {
            match self.kind() {
                T::Eof => return,
                T::Do | T::Then | T::Function | T::Repeat => depth += 1,
                T::End | T::Until if depth == 0 => return,
                T::End | T::Until => depth -= 1,
                _ => {}
            }
            self.bump();
        }
    }

    // -------------------------------------------------------- statements --

    /// `continue` at statement start (spec §1.3.6).
    fn is_continue(&self) -> bool {
        if !self.is_word("continue") {
            return false;
        }
        let next = self.tok(1);
        let follows = matches!(
            next.kind,
            T::Eq
                | T::Comma
                | T::Dot
                | T::LBracket
                | T::Colon
                | T::String
                | T::LongString
                | T::LBrace
                | T::QuestionDot
                | T::QuestionColon
                | T::ColonColon
        ) || compound_op(next.kind).is_some()
            || (next.kind == T::LParen && self.same_line(self.end_at(0), next.start));
        !follows
    }

    fn statement(&mut self) -> PResult<StmtNode> {
        let start = self.start_at(0);
        let decorators = if self.check(T::At) {
            self.decorators()?
        } else {
            Range::default()
        };
        let decorated = decorators.len > 0;
        let data = match self.kind() {
            T::If => self.if_stat()?,
            T::While => self.while_stat()?,
            T::Do => {
                let opened = self.advance().start;
                let block = self.block();
                self.expect_match(T::End, "do", opened)?;
                StmtData::Do(block)
            }
            T::For => self.for_stat()?,
            T::Repeat => self.repeat_stat()?,
            T::Function => self.function_stat()?,
            T::Local => self.local_stat()?,
            T::Return => self.return_stat()?,
            T::Break => {
                let t = self.advance();
                if self.func().loops.is_empty() {
                    let span = self.token_span(&t);
                    let message = format!("no loop to break near {}", self.near());
                    self.error_at("E0106", span, message);
                }
                StmtData::Break
            }
            T::Name => {
                if let Some(data) = self.contextual_stat(start)? {
                    data
                } else {
                    self.expr_stat()?
                }
            }
            _ => self.expr_stat()?,
        };
        let span = self.span_from(start);
        if decorated && !decoratable(&data) {
            self.error_at(
                "E0109",
                span,
                "a decorator must be followed by a declaration (function, local, const, class, …)",
            );
        }
        Ok(StmtNode {
            span: sp(span),
            decorators,
            data,
        })
    }

    /// Statements introduced by contextual keywords (spec §1.2.5).
    fn contextual_stat(&mut self, start: u32) -> PResult<Option<StmtData>> {
        let next = self.kind_at(1);
        let next_is_name = next == T::Name;
        let top_level = self.block_depth == 1;
        let place = |p: &mut Self, code: &'static str, what: &str| {
            if !top_level {
                let span = p.token_span(&p.tok(0).clone());
                p.error_at(
                    code,
                    span,
                    format!("{what} is only allowed at the top level of a file"),
                );
            }
        };
        if self.is_continue() {
            let t = self.advance();
            let span = self.token_span(&t);
            match self.func().loops.last_mut() {
                Some(l) => l.has_continue = true,
                None => self.error_at("E0106", span, "`continue` outside a loop"),
            }
            return Ok(Some(StmtData::Continue));
        }
        if (self.is_word("class") && next_is_name)
            || (self.is_word("abstract") && self.is_word_at(1, "class"))
        {
            place(self, "E0115", "`class`");
            return Ok(Some(StmtData::Class(self.class_decl()?)));
        }
        if self.is_word("interface") && next_is_name {
            return Ok(Some(StmtData::Interface(self.interface_decl()?)));
        }
        if self.is_word("enum") && next_is_name {
            place(self, "E0115", "`enum`");
            return Ok(Some(StmtData::Enum(self.enum_decl()?)));
        }
        if self.is_word("type") && next_is_name {
            return Ok(Some(StmtData::TypeAlias(self.type_alias()?)));
        }
        if self.is_word("const") && next_is_name {
            return Ok(Some(self.const_decl(start)?));
        }
        if self.is_word("declare")
            && (next == T::Function
                || [
                    "global",
                    "class",
                    "interface",
                    "namespace",
                    "event",
                    "message",
                    "module",
                ]
                .iter()
                .any(|w| self.is_word_at(1, w)))
        {
            place(self, "E0115", "`declare`");
            return Ok(Some(StmtData::Declare(self.declare()?)));
        }
        if self.is_word("import") && (next_is_name || next == T::Star) {
            place(self, "E0108", "`import`");
            return Ok(Some(StmtData::Import(self.import()?)));
        }
        if self.is_word("export")
            && (matches!(next, T::Local | T::Function)
                || ["const", "class", "abstract", "interface", "enum", "type"]
                    .iter()
                    .any(|w| self.is_word_at(1, w)))
        {
            place(self, "E0108", "`export`");
            return Ok(Some(self.export(start)?));
        }
        Ok(None)
    }

    fn decorators(&mut self) -> PResult<Range> {
        let mut out = Vec::new();
        while self.check(T::At) {
            let at = self.advance();
            let mut path = vec![self.decorator_name()?];
            while self.check(T::Dot) {
                self.bump();
                path.push(self.decorator_name()?);
            }
            // Arguments must start on the decorator's line.
            let same_line = self.same_line(self.prev_end(), self.start_at(0));
            let args = if same_line
                && matches!(
                    self.kind(),
                    T::LParen | T::LBrace | T::String | T::LongString
                ) {
                Some(self.call_args()?)
            } else {
                None
            };
            let span = self.span_from(at.start);
            let call = self.decorator_call(&path, args, span);
            let path = self.names(path);
            out.push(DecoratorNode {
                path,
                args: args.unwrap_or(NONE),
                span: sp(span),
                call,
            });
        }
        Ok(push_all!(self, decorators, out))
    }

    /// The decorator as an expression (`@a.b(x)` is `a.b(x)`, spec §4.5):
    /// the path's names, each field access from the first name to its own,
    /// called with the arguments over the decorator's span.
    fn decorator_call(&mut self, path: &[Sp], args: Option<u32>, span: Span) -> u32 {
        let first = path[0];
        let mut e = self.push_expr(ExprNode {
            span: first,
            data: ExprData::Name(first),
        });
        for &n in &path[1..] {
            e = self.push_expr(ExprNode {
                span: Sp {
                    start: first.start,
                    end: n.end,
                },
                data: ExprData::Field { obj: e, name: n },
            });
        }
        match args {
            Some(args) => self.push_expr(ExprNode {
                span: sp(span),
                data: ExprData::Call { callee: e, args },
            }),
            None => e,
        }
    }

    fn if_stat(&mut self) -> PResult<StmtData> {
        let opened = self.advance().start;
        let cond = self.expr()?;
        self.expect(T::Then, "then")?;
        let then_block = self.block();
        let mut elseifs = Vec::new();
        let mut else_block = None;
        loop {
            match self.kind() {
                T::Elseif => {
                    self.bump();
                    let c = self.expr()?;
                    self.expect(T::Then, "then")?;
                    let c = self.push_expr(c);
                    elseifs.push((c, self.block()));
                }
                T::Else => {
                    self.bump();
                    else_block = Some(self.block());
                    self.expect_match(T::End, "if", opened)?;
                    break;
                }
                _ => {
                    self.expect_match(T::End, "if", opened)?;
                    break;
                }
            }
        }
        Ok(StmtData::If {
            cond: self.push_expr(cond),
            then_block,
            elseifs: push_all!(self, elseifs, elseifs),
            else_block: else_block.unwrap_or(NONE),
        })
    }

    fn loop_body(&mut self) -> (u32, bool) {
        self.func().loops.push(LoopCtx {
            has_continue: false,
        });
        let block = self.block();
        let ctx = self.func().loops.pop().expect("loop context");
        (block, ctx.has_continue)
    }

    fn while_stat(&mut self) -> PResult<StmtData> {
        let opened = self.advance().start;
        let cond = self.expr()?;
        self.expect(T::Do, "do")?;
        let (body, _) = self.loop_body();
        self.expect_match(T::End, "while", opened)?;
        Ok(StmtData::While {
            cond: self.push_expr(cond),
            body,
        })
    }

    fn repeat_stat(&mut self) -> PResult<StmtData> {
        let opened = self.advance().start;
        let (body, has_continue) = self.loop_body();
        self.expect_match(T::Until, "repeat", opened)?;
        let cond = self.expr()?;
        let cond = self.push_expr(cond);
        if has_continue && !self.keep_tree {
            // Reads the body's statements and all of `cond`.
            self.unsure = true;
        } else if has_continue {
            let locals = self.top_level_locals(body);
            if let Some(name) = self.first_name_use(cond, &locals) {
                self.error_at(
                    "E0107",
                    self.sp_span(name),
                    format!(
                        "`continue` cannot be used in this `repeat` loop: its `until` condition uses the body local `{}`",
                        String::from_utf8_lossy(self.text_at(name))
                    ),
                );
            }
        }
        Ok(StmtData::Repeat { body, cond })
    }

    fn typed_name(&mut self) -> PResult<TypedNameNode> {
        let name = self.name()?;
        let ty = if self.eat(T::Colon) {
            let t = self.ty()?;
            self.push_type(t)
        } else {
            NONE
        };
        Ok(TypedNameNode { name, ty })
    }

    fn for_stat(&mut self) -> PResult<StmtData> {
        let opened = self.advance().start;
        let first = self.typed_name()?;
        match self.kind() {
            T::Eq => {
                self.bump();
                let start = self.expr()?;
                self.expect(T::Comma, ",")?;
                let limit = self.expr()?;
                let step = if self.eat(T::Comma) {
                    Some(self.expr()?)
                } else {
                    None
                };
                self.expect(T::Do, "do")?;
                let (body, _) = self.loop_body();
                self.expect_match(T::End, "for", opened)?;
                let var = push!(self, typed_names, first);
                let start = self.push_expr(start);
                let limit = self.push_expr(limit);
                let step = opt_id(step, |e| self.push_expr(e));
                Ok(StmtData::NumericFor {
                    var,
                    start,
                    limit,
                    step,
                    body,
                })
            }
            T::Comma | T::In => {
                let mut vars = vec![first];
                while self.eat(T::Comma) {
                    vars.push(self.typed_name()?);
                }
                self.expect(T::In, "in")?;
                let exprs = self.expr_list()?;
                self.expect(T::Do, "do")?;
                let (body, _) = self.loop_body();
                self.expect_match(T::End, "for", opened)?;
                Ok(StmtData::GenericFor {
                    vars: push_all!(self, typed_names, vars),
                    exprs,
                    body,
                })
            }
            _ => Err(self.syntax_error("'=' or 'in' expected")),
        }
    }

    /// `a.b.c:m`: its index, and whether it is a single plain name.
    fn func_name(&mut self) -> PResult<(u32, bool)> {
        let start = self.start_at(0);
        let mut path = vec![self.name()?];
        while self.check(T::Dot) {
            self.bump();
            path.push(self.name()?);
        }
        let method = if self.eat(T::Colon) {
            Some(self.name()?)
        } else {
            None
        };
        let plain = path.len() == 1 && method.is_none();
        let path = self.names(path);
        let span = self.span_from(start);
        Ok((
            push!(
                self,
                func_names,
                FuncNameNode {
                    path,
                    method,
                    span: sp(span),
                }
            ),
            plain,
        ))
    }

    fn function_stat(&mut self) -> PResult<StmtData> {
        let opened = self.advance().start;
        let (name, _) = self.func_name()?;
        let func = self.func_body(opened)?;
        Ok(StmtData::Function { name, func })
    }

    fn local_stat(&mut self) -> PResult<StmtData> {
        self.bump();
        if self.check(T::Function) {
            let opened = self.advance().start;
            let name = self.name()?;
            let func = self.func_body(opened)?;
            return Ok(StmtData::LocalFunction { name, func });
        }
        if self.check(T::LBrace) {
            return self.destructure();
        }
        let mut names = vec![self.typed_name()?];
        while self.eat(T::Comma) {
            names.push(self.typed_name()?);
        }
        let values = if self.eat(T::Eq) {
            self.expr_list()?
        } else {
            Range::default()
        };
        Ok(StmtData::Local {
            names: push_all!(self, typed_names, names),
            values,
        })
    }

    /// `local { a, b = 1 }: T = e` (spec §1.3.3)
    fn destructure(&mut self) -> PResult<StmtData> {
        let open = self.advance();
        let mut fields = Vec::new();
        loop {
            let name = self.name()?;
            let default = if self.eat(T::Eq) {
                let e = self.expr()?;
                self.push_expr(e)
            } else {
                NONE
            };
            fields.push(DestructNode { name, default });
            if !self.eat(T::Comma) || self.check(T::RBrace) {
                break;
            }
        }
        self.expect_match(T::RBrace, "{", open.start)?;
        let ty = if self.eat(T::Colon) {
            let t = self.ty()?;
            self.push_type(t)
        } else {
            NONE
        };
        let value = if self.eat(T::Eq) {
            let e = self.expr()?;
            self.push_expr(e)
        } else {
            NONE
        };
        Ok(StmtData::LocalDestructure {
            fields: push_all!(self, destructs, fields),
            ty,
            value,
        })
    }

    fn const_decl(&mut self, start: u32) -> PResult<StmtData> {
        self.advance(); // const
        let name = self.name()?;
        let ty = if self.eat(T::Colon) {
            let t = self.ty()?;
            self.push_type(t)
        } else {
            NONE
        };
        if self.check(T::Comma) {
            let span = self.token_span(&self.tok(0).clone());
            self.error_at("E0112", span, "`const` declares exactly one name");
            while self.eat(T::Comma) {
                self.typed_name()?;
            }
        }
        let value = if self.eat(T::Eq) {
            let e = self.expr()?;
            self.push_expr(e)
        } else {
            let span = self.span_from(start);
            self.error_at("E0112", span, "`const` needs an initializer");
            NONE
        };
        Ok(StmtData::Const { name, ty, value })
    }

    fn return_stat(&mut self) -> PResult<StmtData> {
        self.bump();
        if is_block_follow(self.kind()) || self.check(T::Semi) {
            return Ok(StmtData::Return(Range::default()));
        }
        Ok(StmtData::Return(self.expr_list()?))
    }

    fn expr_stat(&mut self) -> PResult<StmtData> {
        let first = self.suffixed_expr()?;
        if self.check(T::Eq) || self.check(T::Comma) {
            let mut targets = vec![first];
            while self.eat(T::Comma) {
                targets.push(self.suffixed_expr()?);
            }
            if compound_op(self.kind()).is_some() {
                let span = self.token_span(&self.tok(0).clone());
                self.error_at("E0111", span, "compound assignment has a single target");
                return Err(Failed);
            }
            for target in &targets {
                self.check_target(target)?;
            }
            self.expect(T::Eq, "=")?;
            let values = self.expr_list()?;
            return Ok(StmtData::Assign {
                targets: push_all!(self, exprs, targets),
                values,
            });
        }
        if let Some(op) = compound_op(self.kind()) {
            self.check_target(&first)?;
            self.bump();
            let value = self.expr()?;
            return Ok(StmtData::Compound {
                target: self.push_expr(first),
                op,
                value: self.push_expr(value),
            });
        }
        if is_call(&first.data) {
            return Ok(StmtData::Call(self.push_expr(first)));
        }
        Err(self.syntax_error("syntax error"))
    }

    fn check_target(&mut self, target: &ExprNode) -> PResult<()> {
        if self.contains_optional(target) {
            self.error_at(
                "E0113",
                self.sp_span(target.span),
                "an optional chain cannot be assigned to",
            );
            return Err(Failed);
        }
        match target.data {
            ExprData::Name(_) | ExprData::Index { .. } | ExprData::Field { .. } => Ok(()),
            _ => Err(self.syntax_error("syntax error")),
        }
    }

    /// The expression is or contains (along its prefix chain) an optional
    /// access.
    fn contains_optional(&self, e: &ExprNode) -> bool {
        let mut e = *e;
        loop {
            match e.data {
                ExprData::OptField { .. }
                | ExprData::OptIndex { .. }
                | ExprData::OptMethod { .. }
                | ExprData::OptCall { .. } => return true,
                ExprData::Field { obj, .. }
                | ExprData::Index { obj, .. }
                | ExprData::Call { callee: obj, .. }
                | ExprData::Method { obj, .. } => e = self.t.expr(obj),
                _ => return false,
            }
        }
    }

    /// Names declared at the top level of block `block` (visible in
    /// `until`).
    fn top_level_locals(&self, block: u32) -> Vec<&'a [u8]> {
        let mut out = Vec::new();
        let r = self.t.blocks[block as usize].stmts;
        for stmt in &self.t.stmts[r.start as usize..(r.start + r.len) as usize] {
            match stmt.data {
                StmtData::Local { names, .. } => out.extend(
                    self.t.typed_names[names.start as usize..(names.start + names.len) as usize]
                        .iter()
                        .map(|n| self.text_at(n.name)),
                ),
                StmtData::LocalFunction { name, .. } | StmtData::Const { name, .. } => {
                    out.push(self.text_at(name))
                }
                StmtData::LocalDestructure { fields, .. } => out.extend(
                    self.t.destructs[fields.start as usize..(fields.start + fields.len) as usize]
                        .iter()
                        .map(|f| self.text_at(f.name)),
                ),
                _ => {}
            }
        }
        out
    }

    /// The first use of one of `names` in expression `e` (approximate:
    /// ignores shadowing inside nested functions).
    fn first_name_use(&self, e: u32, names: &[&[u8]]) -> Option<Sp> {
        let mut found = None;
        crate::visit::walk_expr(crate::view::expr_at(&self.t, e), &mut |x| {
            if found.is_none()
                && let crate::view::ExprKind::Name(n) = x.kind()
                && names.contains(&n.text().as_bytes())
            {
                found = Some(sp(n.span()));
            }
        });
        found
    }

    // ------------------------------------------------------- expressions --

    fn expr_list(&mut self) -> PResult<Range> {
        // Taken off the stack on every path, so that a list that fails
        // leaves nothing behind for the list it is nested in.
        let mark = self.expr_stack.len();
        let result = self.expr_items();
        let list = push_all!(self, exprs, self.expr_stack.drain(mark..));
        result.map(|()| list)
    }

    fn expr_items(&mut self) -> PResult<()> {
        let first = self.expr()?;
        if self.keep_tree {
            self.expr_stack.push(first);
        }
        while self.eat(T::Comma) {
            let e = self.expr()?;
            if self.keep_tree {
                self.expr_stack.push(e);
            }
        }
        Ok(())
    }

    pub(crate) fn expr(&mut self) -> PResult<ExprNode> {
        self.subexpr(0)
    }

    fn subexpr(&mut self, limit: u8) -> PResult<ExprNode> {
        self.enter_level()?;
        let result = grow(|| self.subexpr_inner(limit));
        self.leave_level();
        result
    }

    fn subexpr_inner(&mut self, limit: u8) -> PResult<ExprNode> {
        let start = self.start_at(0);
        let mut lhs = if let Some(op) = unary_op(self.kind()) {
            self.bump();
            let operand = self.subexpr(UNARY_PRIORITY)?;
            let operand = self.push_expr(operand);
            self.expr_node(self.span_from(start), ExprData::Unary { op, operand })
        } else {
            self.simple_expr()?
        };
        let mut links = 0;
        while let Some(op) = binary_op(self.kind()) {
            let (left, right) = op.priority();
            if left <= limit {
                break;
            }
            links += 1;
            let op_token = self.advance();
            let rhs = self.subexpr(right)?;
            self.check_coalesce_mix(op, &lhs, &rhs, &op_token);
            let lhs_id = self.push_expr(lhs);
            let rhs_id = self.push_expr(rhs);
            lhs = self.expr_node(
                self.span_from(start),
                ExprData::Binary {
                    op,
                    lhs: lhs_id,
                    rhs: rhs_id,
                },
            );
        }
        crate::deep::note(links);
        Ok(lhs)
    }

    /// `??` and `and`/`or` cannot be mixed without parentheses (E0110).
    fn check_coalesce_mix(&mut self, op: BinOp, lhs: &ExprNode, rhs: &ExprNode, op_token: &Token) {
        let is = |e: &ExprNode, ops: &[BinOp]| matches!(e.data, ExprData::Binary { op, .. } if ops.contains(&op));
        let mixed = match op {
            BinOp::Coalesce => {
                is(lhs, &[BinOp::And, BinOp::Or]) || is(rhs, &[BinOp::And, BinOp::Or])
            }
            BinOp::And | BinOp::Or => is(lhs, &[BinOp::Coalesce]) || is(rhs, &[BinOp::Coalesce]),
            _ => false,
        };
        if mixed {
            let span = self.token_span(op_token);
            self.error_at(
                "E0110",
                span,
                "`??` cannot be mixed with `and`/`or` without parentheses",
            );
        }
    }

    fn simple_expr(&mut self) -> PResult<ExprNode> {
        let start = self.start_at(0);
        let data = match self.kind() {
            T::Number => {
                let t = self.advance();
                ExprData::Number(self.make_name(&t))
            }
            T::String | T::LongString => {
                let t = self.advance();
                ExprData::String(self.string_lit(&t))
            }
            T::Nil => {
                self.bump();
                ExprData::Nil
            }
            T::True => {
                self.bump();
                ExprData::True
            }
            T::False => {
                self.bump();
                ExprData::False
            }
            T::Ellipsis => {
                let t = self.advance();
                if !self.func().vararg {
                    let span = self.token_span(&t);
                    self.error_at(
                        "E0101",
                        span,
                        "cannot use '...' outside a vararg function near '...'",
                    );
                }
                ExprData::Vararg
            }
            T::LBrace => self.table()?,
            T::Function => {
                let opened = self.advance().start;
                ExprData::Function(self.func_body(opened)?)
            }
            T::InterpStart => self.interp()?,
            _ => {
                let e = self.suffixed_expr()?;
                return self.casts(e, start);
            }
        };
        let e = self.expr_node(self.span_from(start), data);
        self.casts(e, start)
    }

    /// `e :: T` (spec §1.4.3).
    fn casts(&mut self, mut e: ExprNode, start: u32) -> PResult<ExprNode> {
        while self.check(T::ColonColon) {
            self.bump();
            let ty = self.ty()?;
            let expr = self.push_expr(e);
            let ty = self.push_type(ty);
            e = self.expr_node(self.span_from(start), ExprData::Cast { expr, ty });
        }
        Ok(e)
    }

    /// Where `value` lies in the source within `raw` (from `from` bytes
    /// in), if it does.
    fn locate(&self, raw: Sp, value: &[u8], from: usize) -> Option<Sp> {
        let bytes = self.text_at(raw);
        let last = bytes.len().checked_sub(value.len())?;
        (from..=last.min(from + 16))
            .find(|&i| &bytes[i..i + value.len()] == value)
            .map(|i| Sp {
                start: raw.start + i as u32,
                end: raw.start + (i + value.len()) as u32,
            })
    }

    /// `value`: in the source within `raw`, or else copied to `strbuf`.
    fn store_value(&mut self, raw: Sp, value: &[u8], from: usize) -> (Sp, bool) {
        if let Some(at) = self.locate(raw, value, from) {
            return (at, false);
        }
        let start = self.t.strbuf.len() as u32;
        self.t.strbuf.extend_from_slice(value);
        (
            Sp {
                start,
                end: self.t.strbuf.len() as u32,
            },
            true,
        )
    }

    /// A string literal: its index.
    fn string_lit(&mut self, t: &Token) -> u32 {
        let long = t.kind == T::LongString;
        let raw = self.make_name(t);
        if !self.keep_tree {
            return push!(
                self,
                strs,
                StrNode {
                    raw,
                    value: Sp { start: 0, end: 0 },
                    in_buf: false,
                    long,
                }
            );
        }
        let bytes = self.text_of(t);
        let value = if long {
            literal::long_string_value(bytes)
        } else {
            literal::short_string_value(bytes)
        };
        let (value, in_buf) = self.store_value(raw, &value, 1);
        push!(
            self,
            strs,
            StrNode {
                raw,
                value,
                in_buf,
                long,
            }
        )
    }

    fn primary_expr(&mut self) -> PResult<ExprNode> {
        let start = self.start_at(0);
        match self.kind() {
            T::Name => {
                let t = self.advance();
                let name = self.make_name(&t);
                Ok(ExprNode {
                    span: name,
                    data: ExprData::Name(name),
                })
            }
            T::LParen => {
                let opened = self.advance().start;
                let inner = self.expr()?;
                self.expect_match(T::RParen, "(", opened)?;
                let inner = self.push_expr(inner);
                Ok(self.expr_node(self.span_from(start), ExprData::Paren(inner)))
            }
            _ => Err(self.syntax_error("unexpected symbol")),
        }
    }

    fn suffixed_expr(&mut self) -> PResult<ExprNode> {
        let start = self.start_at(0);
        let mut e = self.primary_expr()?;
        let mut links = 0;
        loop {
            links += 1;
            let data = match self.kind() {
                T::Dot => {
                    self.bump();
                    let name = self.name()?;
                    ExprData::Field {
                        obj: self.push_expr(e),
                        name,
                    }
                }
                T::LBracket => {
                    self.bump();
                    let key = self.expr()?;
                    self.expect(T::RBracket, "]")?;
                    ExprData::Index {
                        obj: self.push_expr(e),
                        key: self.push_expr(key),
                    }
                }
                T::Colon => {
                    self.bump();
                    let name = self.name()?;
                    let args = self.call_args()?;
                    ExprData::Method {
                        obj: self.push_expr(e),
                        method: push!(self, methods, MethodNode { name, args }),
                    }
                }
                T::LParen | T::LBrace | T::String | T::LongString => {
                    let args = self.call_args()?;
                    ExprData::Call {
                        callee: self.push_expr(e),
                        args,
                    }
                }
                T::QuestionDot => {
                    self.bump();
                    match self.kind() {
                        T::LBracket => {
                            self.bump();
                            let key = self.expr()?;
                            self.expect(T::RBracket, "]")?;
                            ExprData::OptIndex {
                                obj: self.push_expr(e),
                                key: self.push_expr(key),
                            }
                        }
                        T::LParen => {
                            let args = self.paren_args()?;
                            ExprData::OptCall {
                                callee: self.push_expr(e),
                                args,
                            }
                        }
                        _ => {
                            let name = self.name()?;
                            ExprData::OptField {
                                obj: self.push_expr(e),
                                name,
                            }
                        }
                    }
                }
                T::QuestionColon => {
                    self.bump();
                    let name = self.name()?;
                    let args = self.call_args()?;
                    ExprData::OptMethod {
                        obj: self.push_expr(e),
                        method: push!(self, methods, MethodNode { name, args }),
                    }
                }
                _ => {
                    crate::deep::note(links);
                    return Ok(e);
                }
            };
            e = self.expr_node(self.span_from(start), data);
        }
    }

    /// `( … )` arguments: their index.
    fn paren_args(&mut self) -> PResult<u32> {
        let open = self.advance();
        let args = if self.check(T::RParen) {
            Range::default()
        } else {
            self.expr_list()?
        };
        self.expect_match(T::RParen, "(", open.start)?;
        let span = self.span_from(open.start);
        Ok(push!(
            self,
            args,
            ArgsData::Parens {
                args,
                span: sp(span),
            }
        ))
    }

    /// A call's arguments: their index.
    fn call_args(&mut self) -> PResult<u32> {
        match self.kind() {
            T::LParen => {
                let t = self.tok(0);
                if !self.same_line(self.prev_end(), t.start) {
                    let span = self.token_span(&t);
                    self.error_at(
                        "E0104",
                        span,
                        "ambiguous syntax (function call x new statement)",
                    );
                }
                self.paren_args()
            }
            T::LBrace => {
                let start = self.start_at(0);
                let data = self.table()?;
                let e = self.expr_node(self.span_from(start), data);
                let e = self.push_expr(e);
                Ok(push!(self, args, ArgsData::Table(e)))
            }
            T::String | T::LongString => {
                let t = self.advance();
                let lit = self.string_lit(&t);
                let e = self.expr_node(self.token_span(&t), ExprData::String(lit));
                let e = self.push_expr(e);
                Ok(push!(self, args, ArgsData::String(e)))
            }
            _ => Err(self.syntax_error("function arguments expected")),
        }
    }

    fn table(&mut self) -> PResult<ExprData> {
        let open = self.advance();
        // As in `expr_list`, the fields come off the stack on every path.
        let mark = self.field_stack.len();
        let result = self.table_fields(open.start);
        let start = self.t.fields.len() as u32;
        for f in self.field_stack.drain(mark..) {
            let node = self.t.pack_field(f);
            self.t.fields.push(node);
        }
        let fields = Range {
            start,
            len: self.t.fields.len() as u32 - start,
        };
        result.map(|()| ExprData::Table(fields))
    }

    fn table_fields(&mut self, opened: u32) -> PResult<()> {
        loop {
            if self.check(T::RBrace) {
                break;
            }
            let start = self.start_at(0);
            let field = match self.kind() {
                T::LBracket => {
                    self.bump();
                    let key = self.expr()?;
                    self.expect(T::RBracket, "]")?;
                    self.expect(T::Eq, "=")?;
                    let value = self.expr()?;
                    if self.keep_tree {
                        let key = self.push_expr(key);
                        let value = self.push_expr(value);
                        let span = sp(self.span_from(start));
                        Some(FieldData::Keyed {
                            key,
                            value,
                            span: push!(self, spans, span),
                        })
                    } else {
                        None
                    }
                }
                T::Name if self.kind_at(1) == T::Eq && !self.keep_tree => {
                    // The name and `=`, with nothing to check.
                    self.bump();
                    self.bump();
                    self.expr()?;
                    None
                }
                T::Name if self.kind_at(1) == T::Eq => {
                    let name = self.name()?;
                    self.bump();
                    let value = self.expr()?;
                    // The field's span is not stored: it runs from the name
                    // to the end of the value. That relies on `expr()`
                    // returning a value whose span ends at the last token it
                    // consumed, the same end `span_from` takes.
                    debug_assert_eq!((name.start, value.span.end), {
                        let s = self.span_from(start);
                        (s.start, s.end)
                    });
                    Some(FieldData::Named {
                        name,
                        value: self.push_expr(value),
                    })
                }
                _ => {
                    let value = self.expr()?;
                    self.keep_tree
                        .then(|| FieldData::Positional(self.push_expr(value)))
                }
            };
            if let Some(field) = field {
                self.field_stack.push(field);
            }
            if !(self.eat(T::Comma) || self.eat(T::Semi)) {
                break;
            }
        }
        self.expect_match(T::RBrace, "{", opened)?;
        Ok(())
    }

    /// A backtick string (spec §1.2.4).
    fn interp(&mut self) -> PResult<ExprData> {
        let open = self.advance();
        let mut parts = Vec::new();
        loop {
            match self.kind() {
                T::InterpText => {
                    let t = self.advance();
                    let raw = self.make_name(&t);
                    let value = literal::interp_text_value(self.text_of(&t));
                    let (value, in_buf) = self.store_value(raw, &value, 0);
                    parts.push(InterpData::Text {
                        raw,
                        value,
                        in_buf,
                        span: raw,
                    });
                }
                T::InterpOpen => {
                    let opened = self.advance().start;
                    let e = self.expr()?;
                    parts.push(InterpData::Expr(self.push_expr(e)));
                    self.expect_match(T::InterpClose, "{", opened)?;
                }
                T::InterpEnd => {
                    self.bump();
                    break;
                }
                _ => {
                    return Err(self.syntax_error(format!(
                        "'`' expected (to close '`' at line {})",
                        self.line_of(open.start)
                    )));
                }
            }
        }
        Ok(ExprData::Interp(push_all!(self, interps, parts)))
    }

    // --------------------------------------------------------- functions --

    /// `[<T>] ( params ) [: ret] block end`: its index.
    fn func_body(&mut self, opened: u32) -> PResult<u32> {
        let start = self.start_at(0);
        let type_params = self.type_params_opt()?;
        let params_start = self.start_at(0);
        let open = self.expect(T::LParen, "(")?;
        let (params, vararg) = self.param_list(false)?;
        self.expect_match(T::RParen, "(", open.start)?;
        let params_span = self.span_from(params_start);
        let ret = if self.eat(T::Colon) {
            self.ret_type()?
        } else {
            NONE
        };
        self.funcs.push(FuncCtx {
            vararg: vararg.is_some(),
            loops: Vec::new(),
        });
        let body = self.block();
        self.funcs.pop();
        let end = self.expect_match(T::End, "function", opened)?;
        let end_span = self.token_span(&end);
        let span = self.span_from(start);
        Ok(push!(
            self,
            funcs,
            FuncNode {
                type_params,
                params,
                vararg,
                ret,
                body,
                params_span: sp(params_span),
                end_span: sp(end_span),
                span: sp(span),
            }
        ))
    }

    /// A signature without body: `[<T>] ( params ) [: ret]`; its index.
    fn signature(&mut self, declaration: bool) -> PResult<u32> {
        let start = self.start_at(0);
        let type_params = self.type_params_opt()?;
        let params_start = self.start_at(0);
        let open = self.expect(T::LParen, "(")?;
        let (params, vararg) = self.param_list(declaration)?;
        self.expect_match(T::RParen, "(", open.start)?;
        let params_span = self.span_from(params_start);
        let ret = if self.eat(T::Colon) {
            self.ret_type()?
        } else {
            NONE
        };
        let span = self.span_from(start);
        let empty = Sp {
            start: span.end,
            end: span.end,
        };
        let body = self.push_block(Range::default(), self.sp_span(empty));
        Ok(push!(
            self,
            funcs,
            FuncNode {
                type_params,
                params,
                vararg,
                ret,
                body,
                params_span: sp(params_span),
                end_span: empty,
                span: sp(span),
            }
        ))
    }

    #[allow(clippy::type_complexity)]
    fn param_list(&mut self, declaration: bool) -> PResult<(Range, Option<(Sp, u32)>)> {
        let mut params = Vec::new();
        let ctor = std::mem::take(&mut self.ctor_params);
        if self.check(T::RParen) {
            return Ok((Range::default(), None));
        }
        loop {
            // A constructor's parameter property: `private sink: Sink`.
            let mut property = 0u8;
            while ctor
                && self.is_modifier()
                && !matches!(self.kind_at(1), T::RParen | T::Comma | T::Eq)
            {
                let at = self.pos;
                let m = self.modifier();
                if !matches!(
                    m,
                    Modifier::Public | Modifier::Private | Modifier::Protected | Modifier::Readonly
                ) {
                    self.pos = at;
                    return Err(self.syntax_error(
                        "only public, protected, private and readonly make a parameter property",
                    ));
                }
                property |= match m {
                    Modifier::Public => crate::tree::PROPERTY_PUBLIC,
                    Modifier::Protected => crate::tree::PROPERTY_PROTECTED,
                    Modifier::Private => crate::tree::PROPERTY_PRIVATE,
                    _ => crate::tree::PROPERTY_READONLY,
                };
            }
            let decorators = if declaration && self.check(T::At) {
                self.decorators()?
            } else {
                Range::default()
            };
            if self.check(T::Ellipsis) {
                let t = self.advance();
                let ty = if self.eat(T::Colon) {
                    let v = self.vararg_type()?;
                    push!(self, vararg_types, v)
                } else {
                    NONE
                };
                let vararg = (sp(self.span_from(t.start)), ty);
                return Ok((push_all!(self, params, params), Some(vararg)));
            }
            let name = match self.kind() {
                T::Name => self.name()?,
                _ => return Err(self.syntax_error("<name> or '...' expected")),
            };
            let ty = if self.eat(T::Colon) {
                let t = self.ty()?;
                self.push_type(t)
            } else {
                NONE
            };
            let default = if !declaration && self.eat(T::Eq) {
                let e = self.expr()?;
                self.push_expr(e)
            } else {
                NONE
            };
            params.push(ParamNode {
                decorators_start: decorators.start,
                decorators_len: u16::try_from(decorators.len).unwrap_or(u16::MAX),
                property,
                name,
                ty,
                default,
            });
            if !self.eat(T::Comma) {
                return Ok((push_all!(self, params, params), None));
            }
        }
    }

    // ------------------------------------------------------------- types --

    /// Splits a `>>`, `>=`, `>>=` (or `?:`) token at its first character, in
    /// type context (spec §1.2.1).
    fn split_first(&mut self, first: TokenKind, rest: TokenKind) {
        let t = self.lexed.tokens.token(self.pos);
        let mut head = t;
        head.kind = first;
        head.end = t.start + 1;
        let mut tail = t;
        tail.kind = rest;
        tail.start = t.start + 1;
        self.lexed.tokens.set(self.pos, head);
        self.lexed.tokens.insert(self.pos + 1, tail);
    }

    fn expect_gt(&mut self) -> PResult<()> {
        match self.kind() {
            T::Gt => {}
            T::Shr => self.split_first(T::Gt, T::Gt),
            T::GtEq => self.split_first(T::Gt, T::Eq),
            T::ShrEq => self.split_first(T::Gt, T::GtEq),
            _ => return Err(self.syntax_error("'>' expected")),
        }
        self.bump();
        Ok(())
    }

    fn type_params_opt(&mut self) -> PResult<Range> {
        if !self.check(T::Lt) {
            return Ok(Range::default());
        }
        self.bump();
        let mut params = Vec::new();
        loop {
            let name = self.name()?;
            if self.eat(T::Ellipsis) {
                params.push(TypeParamNode {
                    name,
                    constraint: NONE,
                    pack: true,
                });
            } else {
                let constraint = if self.is_word("extends") {
                    self.bump();
                    let t = self.ty()?;
                    self.push_type(t)
                } else {
                    NONE
                };
                params.push(TypeParamNode {
                    name,
                    constraint,
                    pack: false,
                });
            }
            if !self.eat(T::Comma) {
                break;
            }
        }
        self.expect_gt()?;
        Ok(push_all!(self, type_params, params))
    }

    pub(crate) fn ty(&mut self) -> PResult<TypeNode> {
        self.enter_level()?;
        let start = self.start_at(0);
        let result = grow(|| {
            self.eat(T::Pipe);
            let first = self.intersection()?;
            self.union_rest(first, start)
        });
        self.leave_level();
        result
    }

    fn union_rest(&mut self, first: TypeNode, start: u32) -> PResult<TypeNode> {
        if !self.check(T::Pipe) {
            return Ok(first);
        }
        let mut members = vec![first];
        while self.eat(T::Pipe) {
            members.push(self.intersection()?);
        }
        let members = self.types(members);
        Ok(self.type_node(self.span_from(start), TypeData::Union(members)))
    }

    fn intersection(&mut self) -> PResult<TypeNode> {
        let start = self.start_at(0);
        let first = self.postfix_type()?;
        self.intersection_rest(first, start)
    }

    fn intersection_rest(&mut self, first: TypeNode, start: u32) -> PResult<TypeNode> {
        if !self.check(T::Amp) {
            return Ok(first);
        }
        let mut members = vec![first];
        while self.eat(T::Amp) {
            members.push(self.postfix_type()?);
        }
        let members = self.types(members);
        Ok(self.type_node(self.span_from(start), TypeData::Intersection(members)))
    }

    fn postfix_type(&mut self) -> PResult<TypeNode> {
        let start = self.start_at(0);
        let base = self.primary_type()?;
        self.postfix_rest(base, start)
    }

    fn postfix_rest(&mut self, mut base: TypeNode, start: u32) -> PResult<TypeNode> {
        loop {
            let data = match self.kind() {
                T::Question => {
                    self.bump();
                    TypeData::Optional(self.push_type(base))
                }
                T::QuestionColon => {
                    self.split_first(T::Question, T::Colon);
                    self.bump();
                    TypeData::Optional(self.push_type(base))
                }
                T::LBracket if self.kind_at(1) == T::RBracket => {
                    self.bump();
                    self.bump();
                    TypeData::Array(self.push_type(base))
                }
                _ => return Ok(base),
            };
            base = self.type_node(self.span_from(start), data);
        }
    }

    /// Continues a type whose first primary was already parsed.
    fn type_after_primary(&mut self, primary: TypeNode, start: u32) -> PResult<TypeNode> {
        let postfix = self.postfix_rest(primary, start)?;
        let isect = self.intersection_rest(postfix, start)?;
        self.union_rest(isect, start)
    }

    fn primary_type(&mut self) -> PResult<TypeNode> {
        let start = self.start_at(0);
        let data = match self.kind() {
            T::Nil => {
                self.bump();
                TypeData::Nil
            }
            T::True => {
                self.bump();
                TypeData::True
            }
            T::False => {
                self.bump();
                TypeData::False
            }
            T::Function => {
                self.bump();
                TypeData::AnyFunction
            }
            T::String | T::LongString => {
                let t = self.advance();
                TypeData::String(self.string_lit(&t))
            }
            T::Number => {
                let t = self.advance();
                TypeData::Number(self.make_name(&t))
            }
            T::Name if self.is_word("typeof") && self.kind_at(1) == T::LParen => {
                self.bump();
                let open = self.advance();
                let e = self.expr()?;
                self.expect_match(T::RParen, "(", open.start)?;
                TypeData::Typeof(self.push_expr(e))
            }
            T::Name => {
                let (path, args) = self.type_ref_parts()?;
                TypeData::Ref { path, args }
            }
            T::LBrace => self.table_type()?,
            T::Lt => {
                let type_params = self.type_params_opt()?;
                let (params, vararg, _) = self.paren_group()?;
                return self.function_type_rest(type_params, params, vararg, start);
            }
            T::LParen => {
                let (params, vararg, _) = self.paren_group()?;
                if self.check(T::Arrow) {
                    return self.function_type_rest(Range::default(), params, vararg, start);
                }
                if params.len() == 1 && params[0].name.is_none() && vararg.is_none() {
                    let inner = params.into_iter().next().expect("one").ty;
                    TypeData::Paren(self.push_type(inner))
                } else {
                    return Err(self.syntax_error("'->' expected"));
                }
            }
            _ => return Err(self.syntax_error("type expected")),
        };
        Ok(self.type_node(self.span_from(start), data))
    }

    fn type_ref_parts(&mut self) -> PResult<(Range, Range)> {
        let mut path = vec![self.name()?];
        while self.check(T::Dot) {
            self.bump();
            path.push(self.name()?);
        }
        let args = if self.check(T::Lt) {
            self.type_args()?
        } else {
            Vec::new()
        };
        let path = self.names(path);
        Ok((path, push_all!(self, type_args, args)))
    }

    pub(crate) fn type_ref(&mut self) -> PResult<TypeNode> {
        let start = self.start_at(0);
        let (path, args) = self.type_ref_parts()?;
        Ok(self.type_node(self.span_from(start), TypeData::Ref { path, args }))
    }

    fn type_args(&mut self) -> PResult<Vec<TypeArgData>> {
        self.advance(); // <
        let mut args = Vec::new();
        loop {
            args.push(self.type_arg()?);
            if !self.eat(T::Comma) {
                break;
            }
        }
        self.expect_gt()?;
        Ok(args)
    }

    fn type_arg(&mut self) -> PResult<TypeArgData> {
        let start = self.start_at(0);
        if self.check(T::Name) && self.kind_at(1) == T::Ellipsis {
            let name = self.name()?;
            self.bump();
            return Ok(TypeArgData::Pack(push!(
                self,
                type_packs,
                TypePackData::Named(name)
            )));
        }
        if self.check(T::LParen) {
            let (params, vararg, span) = self.paren_group()?;
            if self.check(T::Arrow) {
                let f = self.function_type_rest(Range::default(), params, vararg, start)?;
                return Ok(TypeArgData::Type(self.push_type(f)));
            }
            if params.len() == 1 && params[0].name.is_none() && vararg.is_none() {
                let inner = params.into_iter().next().expect("one").ty;
                let inner = self.push_type(inner);
                let paren = self.type_node(span, TypeData::Paren(inner));
                let t = self.type_after_primary(paren, start)?;
                return Ok(TypeArgData::Type(self.push_type(t)));
            }
            let types = self.types(params.into_iter().map(|p| p.ty).collect());
            let vararg = opt_id(vararg, |v| push!(self, vararg_types, v));
            return Ok(TypeArgData::Pack(push!(
                self,
                type_packs,
                TypePackData::List {
                    types,
                    vararg,
                    span: sp(span),
                }
            )));
        }
        let t = self.ty()?;
        Ok(TypeArgData::Type(self.push_type(t)))
    }

    /// `( [name:] T, …, ...: V )`, shared by function types, parenthesized
    /// types and type packs.
    fn paren_group(&mut self) -> PResult<(Vec<GroupParam>, Option<VarargTypeData>, Span)> {
        let open = self.expect(T::LParen, "(")?;
        let mut params = Vec::new();
        let mut vararg = None;
        if !self.check(T::RParen) {
            loop {
                if self.check(T::Ellipsis) {
                    self.bump();
                    self.eat(T::Colon);
                    vararg = Some(self.vararg_type()?);
                    break;
                }
                let name = if self.check(T::Name) && self.kind_at(1) == T::Colon {
                    let n = self.name()?;
                    self.bump();
                    Some(n)
                } else {
                    None
                };
                let ty = self.ty()?;
                if name.is_none() && self.eat(T::Ellipsis) {
                    // `(A...)`, `(EventArgs<E>...)`: a pack expansion.
                    vararg = Some(VarargTypeData::Expand(self.push_type(ty)));
                    break;
                }
                params.push(GroupParam { name, ty });
                if !self.eat(T::Comma) {
                    break;
                }
            }
        }
        self.expect_match(T::RParen, "(", open.start)?;
        Ok((params, vararg, self.span_from(open.start)))
    }

    fn function_type_rest(
        &mut self,
        type_params: Range,
        params: Vec<GroupParam>,
        vararg: Option<VarargTypeData>,
        start: u32,
    ) -> PResult<TypeNode> {
        self.expect(T::Arrow, "->")?;
        let ret = self.ret_type()?;
        let params: Vec<ParamTypeNode> = params
            .into_iter()
            .map(|p| ParamTypeNode {
                name: p.name,
                ty: self.push_type(p.ty),
            })
            .collect();
        let params = push_all!(self, param_types, params);
        let vararg = opt_id(vararg, |v| push!(self, vararg_types, v));
        let f = push!(
            self,
            fn_types,
            FnTypeNode {
                type_params,
                params,
                vararg,
                ret,
            }
        );
        Ok(self.type_node(self.span_from(start), TypeData::Function(f)))
    }

    /// `T` or `T...` after `...:`.
    fn vararg_type(&mut self) -> PResult<VarargTypeData> {
        if self.check(T::Name) && self.kind_at(1) == T::Ellipsis {
            let name = self.name()?;
            self.bump();
            return Ok(VarargTypeData::Pack(name));
        }
        let t = self.ty()?;
        let t = self.push_type(t);
        // `...: EventArgs<E>...`: a pack expansion.
        if self.eat(T::Ellipsis) {
            return Ok(VarargTypeData::Expand(t));
        }
        Ok(VarargTypeData::Type(t))
    }

    /// A result type: its index.
    fn ret_type(&mut self) -> PResult<u32> {
        let r = self.ret_type_data()?;
        Ok(push!(self, rets, r))
    }

    fn ret_type_data(&mut self) -> PResult<RetData> {
        let start = self.start_at(0);
        if self.check(T::Name) && self.kind_at(1) == T::Ellipsis {
            let name = self.name()?;
            self.bump();
            return Ok(RetData::Pack(name));
        }
        if self.check(T::Ellipsis) {
            // `...any`: any number of results.
            let open = self.advance();
            self.eat(T::Colon);
            let vararg = self.vararg_type()?;
            let vararg = push!(self, vararg_types, vararg);
            return Ok(RetData::List {
                types: Range::default(),
                vararg,
                span: sp(self.span_from(open.start)),
            });
        }
        if self.check(T::Name)
            && self.is_word_at(1, "is")
            && self.same_line(self.end_at(0), self.start_at(1))
        {
            let name = self.name()?;
            self.bump();
            let ty = self.ty()?;
            return Ok(RetData::Predicate {
                name,
                ty: self.push_type(ty),
            });
        }
        if self.check(T::LParen) {
            let (params, vararg, span) = self.paren_group()?;
            if self.check(T::Arrow) {
                let f = self.function_type_rest(Range::default(), params, vararg, start)?;
                return Ok(RetData::Single(self.push_type(f)));
            }
            if params.len() == 1 && params[0].name.is_none() && vararg.is_none() {
                let inner = params.into_iter().next().expect("one").ty;
                let inner = self.push_type(inner);
                let paren = self.type_node(span, TypeData::Paren(inner));
                let t = self.type_after_primary(paren, start)?;
                return Ok(RetData::Single(self.push_type(t)));
            }
            let types = self.types(params.into_iter().map(|p| p.ty).collect());
            let vararg = opt_id(vararg, |v| push!(self, vararg_types, v));
            return Ok(RetData::List {
                types,
                vararg,
                span: sp(span),
            });
        }
        let ty = self.ty()?;
        if self.check(T::Ellipsis) {
            let end = self.advance();
            let span = Sp {
                start,
                end: end.end,
            };
            let ty = self.push_type(ty);
            let vararg = push!(self, vararg_types, VarargTypeData::Expand(ty));
            return Ok(RetData::List {
                types: Range::default(),
                vararg,
                span,
            });
        }
        Ok(RetData::Single(self.push_type(ty)))
    }

    fn table_type(&mut self) -> PResult<TypeData> {
        let open = self.advance();
        let mut fields = Vec::new();
        loop {
            if self.check(T::RBrace) {
                break;
            }
            let start = self.start_at(0);
            let readonly = self.is_word("readonly")
                && ((self.kind_at(1) == T::Name
                    && matches!(self.kind_at(2), T::Colon | T::QuestionColon))
                    || (self.kind_at(1) == T::LBracket
                        && matches!(self.kind_at(2), T::String | T::LongString)
                        && self.kind_at(3) == T::RBracket
                        && matches!(self.kind_at(4), T::Colon | T::QuestionColon)));
            if readonly {
                self.bump();
            }
            let field =
                if self.check(T::Name) && matches!(self.kind_at(1), T::Colon | T::QuestionColon) {
                    let name = self.name()?;
                    let optional = self.advance().kind == T::QuestionColon;
                    let ty = self.ty()?;
                    TableTypeFieldData::Named {
                        name,
                        optional,
                        readonly,
                        ty: self.push_type(ty),
                    }
                } else if self.check(T::LBracket) {
                    self.bump();
                    if matches!(self.kind(), T::String | T::LongString)
                        && self.kind_at(1) == T::RBracket
                    {
                        let t = self.advance();
                        let key = self.string_lit(&t);
                        self.bump();
                        let optional = self.eat(T::QuestionColon);
                        if !optional {
                            self.expect(T::Colon, ":")?;
                        }
                        let ty = self.ty()?;
                        TableTypeFieldData::NamedString {
                            key,
                            optional,
                            readonly,
                            ty: self.push_type(ty),
                            span: sp(self.span_from(start)),
                        }
                    } else {
                        let key = self.ty()?;
                        self.expect(T::RBracket, "]")?;
                        self.expect(T::Colon, ":")?;
                        let value = self.ty()?;
                        TableTypeFieldData::Indexer {
                            key: self.push_type(key),
                            value: self.push_type(value),
                        }
                    }
                } else {
                    let t = self.ty()?;
                    TableTypeFieldData::Array(self.push_type(t))
                };
            fields.push(field);
            if !(self.eat(T::Comma) || self.eat(T::Semi)) {
                break;
            }
        }
        self.expect_match(T::RBrace, "{", open.start)?;
        Ok(TypeData::Table(push_all!(self, table_type_fields, fields)))
    }

    // ------------------------------------------------------ declarations --

    /// A type alias: its index.
    fn type_alias(&mut self) -> PResult<u32> {
        self.advance(); // type
        let name = self.name()?;
        let type_params = self.type_params_opt()?;
        self.expect(T::Eq, "=")?;
        let ty = self.ty()?;
        let ty = self.push_type(ty);
        Ok(push!(
            self,
            aliases,
            AliasNode {
                name,
                type_params,
                ty,
            }
        ))
    }

    fn type_list_after(&mut self, word: &str) -> PResult<Range> {
        let mut out = Vec::new();
        if self.is_word(word) {
            self.bump();
            out.push(self.type_ref()?);
            while self.eat(T::Comma) {
                out.push(self.type_ref()?);
            }
        }
        Ok(self.types(out))
    }

    /// An optional `extends T`: its type's index.
    fn extends_opt(&mut self) -> PResult<u32> {
        if self.is_word("extends") {
            self.bump();
            let t = self.type_ref()?;
            Ok(self.push_type(t))
        } else {
            Ok(NONE)
        }
    }

    /// A class: its index.
    fn class_decl(&mut self) -> PResult<u32> {
        let is_abstract = self.is_word("abstract");
        if is_abstract {
            self.bump();
        }
        let opened = self.advance().start; // class
        let name = self.name()?;
        let type_params = self.type_params_opt()?;
        let extends = self.extends_opt()?;
        let implements = self.type_list_after("implements")?;
        let mut members = Vec::new();
        while !self.check(T::End) && !self.check(T::Eof) {
            let before = self.pos;
            match self.class_member() {
                Ok(m) => members.push(m),
                Err(Failed) => {
                    if self.pos == before {
                        self.bump();
                    }
                    while !matches!(self.kind(), T::End | T::Eof | T::Function | T::At)
                        && !self.is_member_start()
                    {
                        self.bump();
                    }
                }
            }
        }
        let end = self.expect_match(T::End, "class", opened)?;
        let members = push_all!(self, class_members, members);
        Ok(push!(
            self,
            classes,
            ClassNode {
                is_abstract,
                name,
                type_params,
                extends,
                implements,
                members,
                end_span: sp(self.token_span(&end)),
            }
        ))
    }

    fn is_member_start(&self) -> bool {
        self.check(T::Name)
            && (matches!(
                self.kind_at(1),
                T::Colon | T::QuestionColon | T::Name | T::LParen
            ))
    }

    /// A modifier word is a member name when followed by `:`, `?:` or `(`
    /// (spec §1.2.5).
    fn is_modifier(&self) -> bool {
        self.check(T::Name)
            && MODIFIERS.iter().any(|m| self.is_word(m))
            && !matches!(self.kind_at(1), T::Colon | T::QuestionColon | T::LParen)
    }

    fn modifier(&mut self) -> Modifier {
        let t = self.advance();
        match self.text_of(&t) {
            b"public" => Modifier::Public,
            b"private" => Modifier::Private,
            b"protected" => Modifier::Protected,
            b"static" => Modifier::Static,
            b"readonly" => Modifier::Readonly,
            b"override" => Modifier::Override,
            b"abstract" => Modifier::Abstract,
            _ => Modifier::Overload,
        }
    }

    fn class_member(&mut self) -> PResult<ClassMemberNode> {
        let start = self.start_at(0);
        let decorators = if self.check(T::At) {
            self.decorators()?
        } else {
            Range::default()
        };
        let mut modifiers = Vec::new();
        while self.is_modifier() {
            modifiers.push(self.modifier());
        }
        let signature_only = modifiers
            .iter()
            .any(|m| matches!(m, Modifier::Abstract | Modifier::Overload));
        let data = if self.check(T::Function) {
            let opened = self.advance().start;
            let name = self.name()?;
            if signature_only {
                ClassMemberData::Method {
                    name,
                    func: self.signature(false)?,
                    has_body: false,
                }
            } else {
                ClassMemberData::Method {
                    name,
                    func: self.method_body(opened)?,
                    has_body: true,
                }
            }
        } else if self.is_word("constructor") && self.kind_at(1) == T::LParen {
            let opened = self.advance().start;
            if signature_only {
                ClassMemberData::Constructor {
                    func: self.signature(false)?,
                    has_body: false,
                }
            } else {
                self.ctor_params = true;
                ClassMemberData::Constructor {
                    func: self.method_body(opened)?,
                    has_body: true,
                }
            }
        } else if self.is_word("get") && self.kind_at(1) == T::Name {
            let opened = self.advance().start;
            let name = self.name()?;
            let open = self.expect(T::LParen, "(")?;
            self.expect_match(T::RParen, "(", open.start)?;
            let ty = if self.eat(T::Colon) {
                let t = self.ty()?;
                self.push_type(t)
            } else {
                NONE
            };
            let body = self.method_block();
            self.expect_match(T::End, "get", opened)?;
            ClassMemberData::Getter { name, ty, body }
        } else if self.is_word("set") && self.kind_at(1) == T::Name {
            let opened = self.advance().start;
            let name = self.name()?;
            let open = self.expect(T::LParen, "(")?;
            let param = self.typed_name()?;
            self.expect_match(T::RParen, "(", open.start)?;
            let body = self.method_block();
            self.expect_match(T::End, "set", opened)?;
            ClassMemberData::Setter {
                name,
                param: push!(self, typed_names, param),
                body,
            }
        } else if self.check(T::Name) {
            let name = self.name()?;
            let optional = match self.kind() {
                T::QuestionColon => {
                    self.bump();
                    true
                }
                _ => {
                    self.expect(T::Colon, ":")?;
                    false
                }
            };
            let ty = self.ty()?;
            let ty = self.push_type(ty);
            let value = if self.eat(T::Eq) {
                let e = self.expr()?;
                self.push_expr(e)
            } else {
                NONE
            };
            ClassMemberData::Field {
                name,
                optional,
                ty,
                value,
            }
        } else {
            return Err(self.syntax_error("class member expected"));
        };
        let modifiers = push_all!(self, modifiers, modifiers);
        Ok(ClassMemberNode {
            decorators,
            modifiers,
            span: sp(self.span_from(start)),
            data,
        })
    }

    /// Methods are vararg only when they declare `...`.
    fn method_body(&mut self, opened: u32) -> PResult<u32> {
        self.func_body(opened)
    }

    fn method_block(&mut self) -> u32 {
        self.funcs.push(FuncCtx::default());
        let block = self.block();
        self.funcs.pop();
        block
    }

    /// An interface: its index.
    fn interface_decl(&mut self) -> PResult<u32> {
        let opened = self.advance().start; // interface
        let name = self.name()?;
        let type_params = self.type_params_opt()?;
        let extends = self.type_list_after("extends")?;
        let mut members = Vec::new();
        while !self.check(T::End) && !self.check(T::Eof) {
            members.push(self.interface_member()?);
        }
        self.expect_match(T::End, "interface", opened)?;
        let members = push_all!(self, interface_members, members);
        Ok(push!(
            self,
            interfaces,
            InterfaceNode {
                name,
                type_params,
                extends,
                members,
            }
        ))
    }

    fn interface_member(&mut self) -> PResult<InterfaceMemberData> {
        if self.check(T::Function) {
            self.bump();
            let name = self.name()?;
            return Ok(InterfaceMemberData::Method {
                name,
                sig: self.signature(true)?,
            });
        }
        let readonly = self.is_word("readonly") && self.kind_at(1) == T::Name;
        if readonly {
            self.bump();
        }
        let name = self.name()?;
        let optional = match self.kind() {
            T::QuestionColon => {
                self.bump();
                true
            }
            _ => {
                self.expect(T::Colon, ":")?;
                false
            }
        };
        let ty = self.ty()?;
        Ok(InterfaceMemberData::Field {
            readonly,
            name,
            optional,
            ty: self.push_type(ty),
        })
    }

    /// An enum: its index.
    fn enum_decl(&mut self) -> PResult<u32> {
        self.advance(); // enum
        let name = self.name()?;
        let open = self.expect(T::LBrace, "{")?;
        let mut items = Vec::new();
        while !self.check(T::RBrace) {
            let item = self.name()?;
            let value = if self.eat(T::Eq) {
                let e = self.expr()?;
                self.push_expr(e)
            } else {
                NONE
            };
            items.push(EnumItemNode { name: item, value });
            if !(self.eat(T::Comma) || self.eat(T::Semi)) {
                break;
            }
        }
        self.expect_match(T::RBrace, "{", open.start)?;
        let items = push_all!(self, enum_items, items);
        Ok(push!(self, enums, EnumNode { name, items }))
    }

    /// A `declare …`: its index.
    fn declare(&mut self) -> PResult<u32> {
        let d = self.declare_data()?;
        Ok(push!(self, declares, d))
    }

    fn declare_data(&mut self) -> PResult<DeclareData> {
        let opened = self.advance().start; // declare
        if self.eat(T::Function) {
            let (name, _) = self.func_name()?;
            let sig = self.signature(true)?;
            return Ok(DeclareData::Function { name, sig });
        }
        let word = self.advance();
        let word = String::from_utf8_lossy(self.text_of(&word)).into_owned();
        match word.as_str() {
            "global" => {
                let name = self.name()?;
                self.expect(T::Colon, ":")?;
                let ty = self.ty()?;
                Ok(DeclareData::Global {
                    name,
                    ty: self.push_type(ty),
                })
            }
            "namespace" => {
                let name = self.name()?;
                let members = self.ns_members()?;
                self.expect_match(T::End, "namespace", opened)?;
                Ok(DeclareData::Namespace { name, members })
            }
            "class" => {
                let name = self.name()?;
                let type_params = self.type_params_opt()?;
                let extends = self.extends_opt()?;
                let implements = self.type_list_after("implements")?;
                let members = self.decl_members()?;
                self.expect_match(T::End, "class", opened)?;
                Ok(DeclareData::Class {
                    name,
                    type_params,
                    extends,
                    implements,
                    members,
                })
            }
            "interface" => {
                self.pos -= 1;
                Ok(DeclareData::Interface(self.interface_decl()?))
            }
            "event" | "message" => {
                let name = self.name()?;
                let params = self.signature(true)?;
                Ok(if word == "event" {
                    DeclareData::Event { name, params }
                } else {
                    DeclareData::Message { name, params }
                })
            }
            _ => {
                // module
                let t = self.expect_string()?;
                let path = self.string_lit(&t);
                let members = self.ns_members()?;
                self.expect_match(T::End, "module", opened)?;
                Ok(DeclareData::Module { path, members })
            }
        }
    }

    fn expect_string(&mut self) -> PResult<Token> {
        if matches!(self.kind(), T::String | T::LongString) {
            Ok(self.advance())
        } else {
            Err(self.syntax_error("<string> expected"))
        }
    }

    fn ns_members(&mut self) -> PResult<Range> {
        let mut out = Vec::new();
        while !self.check(T::End) && !self.check(T::Eof) {
            let start = self.start_at(0);
            let decorators = if self.check(T::At) {
                self.decorators()?
            } else {
                Range::default()
            };
            let data = if self.eat(T::Function) {
                let name = self.name()?;
                NsMemberData::Function {
                    name,
                    sig: self.signature(true)?,
                }
            } else if self.is_word("namespace") && self.kind_at(1) == T::Name {
                let opened = self.advance().start;
                let name = self.name()?;
                let members = self.ns_members()?;
                self.expect_match(T::End, "namespace", opened)?;
                NsMemberData::Namespace { name, members }
            } else if self.is_word("class") && self.kind_at(1) == T::Name {
                let opened = self.advance().start;
                let name = self.name()?;
                let type_params = self.type_params_opt()?;
                let extends = self.extends_opt()?;
                let members = self.decl_members()?;
                self.expect_match(T::End, "class", opened)?;
                NsMemberData::Class {
                    name,
                    type_params,
                    extends,
                    members,
                }
            } else if self.is_word("interface") && self.kind_at(1) == T::Name {
                NsMemberData::Interface(self.interface_decl()?)
            } else if self.is_word("type") && self.kind_at(1) == T::Name {
                NsMemberData::TypeAlias(self.type_alias()?)
            } else if self.is_word("enum") && self.kind_at(1) == T::Name {
                NsMemberData::Enum(self.enum_decl()?)
            } else {
                let is_const = self.is_word("const") && self.kind_at(1) == T::Name;
                if is_const {
                    self.bump();
                }
                let name = self.name()?;
                self.expect(T::Colon, ":")?;
                let ty = self.ty()?;
                NsMemberData::Value {
                    is_const,
                    name,
                    ty: self.push_type(ty),
                }
            };
            out.push(NsMemberNode {
                decorators,
                span: sp(self.span_from(start)),
                data,
            });
        }
        Ok(push_all!(self, ns_members, out))
    }

    fn decl_members(&mut self) -> PResult<Range> {
        let mut out = Vec::new();
        while !self.check(T::End) && !self.check(T::Eof) {
            let start = self.start_at(0);
            let decorators = if self.check(T::At) {
                self.decorators()?
            } else {
                Range::default()
            };
            let is_static = self.is_word("static")
                && !matches!(self.kind_at(1), T::Colon | T::QuestionColon | T::LParen);
            if is_static {
                self.bump();
            }
            let data = if self.eat(T::Function) {
                let name = self.name()?;
                DeclMemberData::Function {
                    name,
                    sig: self.signature(true)?,
                }
            } else {
                let readonly = self.is_word("readonly") && self.kind_at(1) == T::Name;
                if readonly {
                    self.bump();
                }
                let name = self.name()?;
                let optional = match self.kind() {
                    T::QuestionColon => {
                        self.bump();
                        true
                    }
                    _ => {
                        self.expect(T::Colon, ":")?;
                        false
                    }
                };
                let ty = self.ty()?;
                DeclMemberData::Field {
                    readonly,
                    name,
                    optional,
                    ty: self.push_type(ty),
                }
            };
            out.push(DeclMemberNode {
                decorators,
                is_static,
                span: sp(self.span_from(start)),
                data,
            });
        }
        Ok(push_all!(self, decl_members, out))
    }

    /// An import: its index.
    fn import(&mut self) -> PResult<u32> {
        self.advance(); // import
        // `import type X from` is type-only; `import type from "x"` imports `type`.
        let type_only =
            self.is_word("type") && self.kind_at(1) == T::Name && !self.is_word_at(1, "from");
        if type_only {
            self.bump();
        }
        // `import internal a from` reads a module's locals; `import
        // internal from "x"` imports `internal`.
        let internal = !type_only
            && self.is_word("internal")
            && self.kind_at(1) == T::Name
            && !self.is_word_at(1, "from");
        if internal {
            self.advance();
        }
        let clause = if self.eat(T::Star) {
            if !self.is_word("as") {
                return Err(self.syntax_error("'as' expected"));
            }
            self.bump();
            ImportClauseData::Namespace(self.name()?)
        } else if self.is_word("_") && !matches!(self.kind_at(1), T::Comma) {
            self.bump();
            ImportClauseData::SideEffect
        } else {
            let mut specs = Vec::new();
            loop {
                let name = self.name()?;
                if self.text_at(name) == b"_" {
                    self.error_at(
                        "E0117",
                        self.sp_span(name),
                        "`import _` cannot be combined with other names",
                    );
                }
                let alias = if self.is_word("as") {
                    self.bump();
                    Some(self.name()?)
                } else {
                    None
                };
                specs.push(ImportSpecNode { name, alias });
                if !self.eat(T::Comma) {
                    break;
                }
            }
            ImportClauseData::Names(push_all!(self, import_specs, specs))
        };
        if !self.is_word("from") {
            return Err(self.syntax_error("'from' expected"));
        }
        self.bump();
        let t = self.expect_string()?;
        let from = self.string_lit(&t);
        Ok(push!(
            self,
            imports,
            ImportNode {
                clause,
                type_only,
                internal,
                from,
            }
        ))
    }

    fn export(&mut self, start: u32) -> PResult<StmtData> {
        self.advance(); // export
        let inner_start = self.start_at(0);
        let data = match self.kind() {
            T::Local => {
                self.bump();
                let mut names = vec![self.typed_name()?];
                while self.eat(T::Comma) {
                    names.push(self.typed_name()?);
                }
                let values = if self.eat(T::Eq) {
                    self.expr_list()?
                } else {
                    let span = self.span_from(start);
                    self.error_at("E0119", span, "`export local` needs an initializer");
                    Range::default()
                };
                StmtData::Local {
                    names: push_all!(self, typed_names, names),
                    values,
                }
            }
            T::Function => {
                let opened = self.advance().start;
                let (name, plain) = self.func_name()?;
                if !plain {
                    let span = self.sp_span(self.t.func_names[name as usize].span);
                    self.error_at("E0118", span, "`export function` takes a plain name");
                }
                let func = self.func_body(opened)?;
                StmtData::Function { name, func }
            }
            _ if self.is_word("const") => self.const_decl(inner_start)?,
            _ if self.is_word("class") || self.is_word("abstract") => {
                StmtData::Class(self.class_decl()?)
            }
            _ if self.is_word("interface") => StmtData::Interface(self.interface_decl()?),
            _ if self.is_word("enum") => StmtData::Enum(self.enum_decl()?),
            _ => StmtData::TypeAlias(self.type_alias()?),
        };
        let span = self.span_from(inner_start);
        let inner = push!(
            self,
            stmts,
            StmtNode {
                span: sp(span),
                decorators: Range::default(),
                data,
            }
        );
        Ok(StmtData::Export(inner))
    }
}

/// Runs `f` with enough stack for one more nesting level (deeply nested
/// input must produce an error, never a stack overflow).
fn grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, f)
}

fn decoratable(data: &StmtData) -> bool {
    matches!(
        data,
        StmtData::Function { .. }
            | StmtData::LocalFunction { .. }
            | StmtData::Local { .. }
            | StmtData::LocalDestructure { .. }
            | StmtData::Const { .. }
            | StmtData::Class(_)
            | StmtData::Interface(_)
            | StmtData::Enum(_)
            | StmtData::TypeAlias(_)
            | StmtData::Declare(_)
            | StmtData::Import(_)
            | StmtData::Export(_)
    )
}

fn is_call(data: &ExprData) -> bool {
    matches!(
        data,
        ExprData::Call { .. }
            | ExprData::Method { .. }
            | ExprData::OptCall { .. }
            | ExprData::OptMethod { .. }
    )
}
