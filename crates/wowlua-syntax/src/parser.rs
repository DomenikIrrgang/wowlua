//! Hand-written recursive-descent parser with Pratt-style expression parsing
//! (spec compiler §13.5.2). Accepts exactly what Lua 5.1 accepts for pure Lua
//! input, plus the wowlua grammar of spec appendix A. Errors are recovered at
//! statement boundaries, so one run reports several independent errors.

use crate::ast::*;
use crate::lexer::Lexed;
use crate::literal;
use crate::token::{Token, TokenKind};
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

pub(crate) struct Parser<'a> {
    src: &'a [u8],
    file: FileId,
    pub(crate) lexed: Lexed,
    pos: usize,
    next_id: u32,
    pub(crate) diagnostics: Vec<Diagnostic>,
    depth: u32,
    funcs: Vec<FuncCtx>,
    /// Nesting of blocks inside the chunk (0 = top level).
    block_depth: u32,
    /// Position of the last error, to avoid reporting twice at one token.
    last_error_at: Option<usize>,
    /// The fields of the tables being parsed, innermost last. A finished
    /// table moves its own into a list of exactly their size: a list grown
    /// by pushing keeps up to half of its room unused, which adds up to
    /// gigabytes in the tables of a large data addon.
    field_stack: Vec<TableField>,
    /// The same for the expression lists being parsed.
    expr_stack: Vec<Expr>,
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

impl<'a> Parser<'a> {
    pub(crate) fn new(src: &'a [u8], file: FileId, lexed: Lexed) -> Self {
        Parser {
            src,
            file,
            lexed,
            pos: 0,
            next_id: 0,
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
        }
    }

    // ------------------------------------------------------------ tokens --

    fn tok(&self, ahead: usize) -> &Token {
        let tokens = &self.lexed.tokens;
        &tokens[(self.pos + ahead).min(tokens.len() - 1)]
    }

    fn kind(&self) -> TokenKind {
        self.tok(0).kind
    }

    fn kind_at(&self, ahead: usize) -> TokenKind {
        self.tok(ahead).kind
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
        t.kind == T::Name && self.text_of(t) == word.as_bytes()
    }

    fn advance(&mut self) -> Token {
        let t = *self.tok(0);
        if t.kind != T::Eof {
            self.pos += 1;
        }
        t
    }

    fn prev(&self) -> &Token {
        &self.lexed.tokens[self.pos.saturating_sub(1)]
    }

    fn span_from(&self, start: u32) -> Span {
        let end = if self.pos == 0 {
            start
        } else {
            self.prev().end
        };
        Span {
            file: self.file,
            start,
            end: end.max(start),
        }
    }

    fn token_span(&self, t: &Token) -> Span {
        Span {
            file: self.file,
            start: t.start,
            end: t.end,
        }
    }

    fn id(&mut self) -> NodeId {
        self.next_id += 1;
        NodeId(self.next_id)
    }

    fn check(&self, kind: TokenKind) -> bool {
        self.kind() == kind
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
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
                let text = String::from_utf8_lossy(self.text_of(t));
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

    /// Lua's `check_match`: `close` closing `open` from `line`.
    fn expect_match(&mut self, close: TokenKind, open: &str, line: u32) -> PResult<Token> {
        if self.check(close) {
            return Ok(self.advance());
        }
        let what = close.describe();
        if line == self.tok(0).line {
            Err(self.syntax_error(format!("{what} expected")))
        } else {
            Err(self.syntax_error(format!(
                "{what} expected (to close '{open}' at line {line})"
            )))
        }
    }

    fn name(&mut self) -> PResult<Name> {
        if self.check(T::Name) {
            let t = self.advance();
            Ok(self.make_name(&t))
        } else {
            Err(self.syntax_error("<name> expected"))
        }
    }

    /// A decorator name may be a reserved word (`@if`).
    fn decorator_name(&mut self) -> PResult<Name> {
        if self.kind().is_keyword() {
            let t = self.advance();
            return Ok(self.make_name(&t));
        }
        self.name()
    }

    fn make_name(&self, t: &Token) -> Name {
        Name {
            text: String::from_utf8_lossy(self.text_of(t)).into(),
            span: self.token_span(t),
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

    pub(crate) fn chunk(&mut self) -> Chunk {
        let directives = self.directives();
        let start = self.tok(0).start;
        let mut stmts = Vec::new();
        loop {
            let block = self.block();
            stmts.extend(block.stmts);
            if self.check(T::Eof) {
                break;
            }
            let _ = self.syntax_error("'<eof>' expected");
            self.advance();
        }
        debug_assert!(self.field_stack.is_empty() && self.expr_stack.is_empty());
        let span = Span {
            file: self.file,
            start: 0,
            end: self.src.len() as u32,
        };
        Chunk {
            directives,
            block: Block {
                stmts,
                span: Span {
                    file: self.file,
                    start,
                    end: self.src.len() as u32,
                },
            },
            span,
        }
    }

    /// `--!name` comments before the first token (spec §1.1.1).
    fn directives(&self) -> Vec<Directive> {
        let first = self.tok(0);
        let mut out = Vec::new();
        for trivia in self.lexed.trivia_of(first) {
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

    fn block(&mut self) -> Block {
        grow(|| self.block_inner())
    }

    fn block_inner(&mut self) -> Block {
        let start = self.tok(0).start;
        let mut stmts = Vec::new();
        if self.enter_level().is_err() {
            self.skip_to_block_end();
            return Block {
                stmts,
                span: self.span_from(start),
            };
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
                    let is_continue = matches!(stmt.kind, StmtKind::Continue);
                    stmts.push(stmt);
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
                    stmts.push(Stmt {
                        id: self.id(),
                        span: self.span_from(self.lexed.tokens[before].start),
                        decorators: Vec::new(),
                        kind: StmtKind::Error,
                    });
                }
            }
        }
        self.block_depth -= 1;
        self.leave_level();
        Block {
            stmts,
            span: self.span_from(start),
        }
    }

    /// Skips to the next statement boundary after an error.
    fn recover(&mut self, before: usize) {
        let error_line = self.tok(0).line;
        if self.pos == before {
            self.advance();
        }
        loop {
            // A name or `(` starting a later line likely starts a statement.
            if self.tok(0).line > error_line && matches!(self.kind(), T::Name | T::LParen | T::At) {
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
                    self.advance();
                    return;
                }
                _ => {
                    self.advance();
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
            self.advance();
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
            || (next.kind == T::LParen && next.line == self.tok(0).end_line);
        !follows
    }

    fn statement(&mut self) -> PResult<Stmt> {
        let start = self.tok(0).start;
        let decorators = if self.check(T::At) {
            self.decorators()?
        } else {
            Vec::new()
        };
        let decorated = !decorators.is_empty();
        let kind = match self.kind() {
            T::If => self.if_stat()?,
            T::While => self.while_stat()?,
            T::Do => {
                let line = self.advance().line;
                let block = self.block();
                self.expect_match(T::End, "do", line)?;
                StmtKind::Do(block)
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
                    self.error_at("E0106", span, "no loop to break");
                }
                StmtKind::Break
            }
            T::Name => {
                if let Some(kind) = self.contextual_stat(start)? {
                    kind
                } else {
                    self.expr_stat()?
                }
            }
            _ => self.expr_stat()?,
        };
        let span = self.span_from(start);
        if decorated && !decoratable(&kind) {
            self.error_at(
                "E0109",
                span,
                "a decorator must be followed by a declaration (function, local, const, class, …)",
            );
        }
        Ok(Stmt {
            id: self.id(),
            span,
            decorators,
            kind,
        })
    }

    /// Statements introduced by contextual keywords (spec §1.2.5).
    fn contextual_stat(&mut self, start: u32) -> PResult<Option<StmtKind>> {
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
            return Ok(Some(StmtKind::Continue));
        }
        if (self.is_word("class") && next_is_name)
            || (self.is_word("abstract") && self.is_word_at(1, "class"))
        {
            place(self, "E0115", "`class`");
            return Ok(Some(StmtKind::Class(Box::new(self.class_decl()?))));
        }
        if self.is_word("interface") && next_is_name {
            return Ok(Some(StmtKind::Interface(Box::new(self.interface_decl()?))));
        }
        if self.is_word("enum") && next_is_name {
            place(self, "E0115", "`enum`");
            return Ok(Some(StmtKind::Enum(Box::new(self.enum_decl()?))));
        }
        if self.is_word("type") && next_is_name {
            return Ok(Some(StmtKind::TypeAlias(Box::new(self.type_alias()?))));
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
            return Ok(Some(StmtKind::Declare(Box::new(self.declare()?))));
        }
        if self.is_word("import") && (next_is_name || next == T::Star) {
            place(self, "E0108", "`import`");
            return Ok(Some(StmtKind::Import(Box::new(self.import()?))));
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

    fn decorators(&mut self) -> PResult<Vec<Decorator>> {
        let mut out = Vec::new();
        while self.check(T::At) {
            let at = self.advance();
            let mut path = vec![self.decorator_name()?];
            while self.check(T::Dot) {
                self.advance();
                path.push(self.decorator_name()?);
            }
            // Arguments must start on the decorator's line.
            let same_line = self.tok(0).line == self.prev().end_line;
            let args = if same_line
                && matches!(
                    self.kind(),
                    T::LParen | T::LBrace | T::String | T::LongString
                ) {
                Some(self.call_args()?)
            } else {
                None
            };
            out.push(Decorator {
                path,
                args,
                span: self.span_from(at.start),
            });
        }
        Ok(out)
    }

    fn if_stat(&mut self) -> PResult<StmtKind> {
        let line = self.advance().line;
        let cond = self.expr()?;
        self.expect(T::Then, "then")?;
        let then_block = self.block();
        let mut elseifs = Vec::new();
        let mut else_block = None;
        loop {
            match self.kind() {
                T::Elseif => {
                    self.advance();
                    let c = self.expr()?;
                    self.expect(T::Then, "then")?;
                    elseifs.push((c, self.block()));
                }
                T::Else => {
                    self.advance();
                    else_block = Some(self.block());
                    self.expect_match(T::End, "if", line)?;
                    break;
                }
                _ => {
                    self.expect_match(T::End, "if", line)?;
                    break;
                }
            }
        }
        Ok(StmtKind::If {
            cond,
            then_block,
            elseifs,
            else_block,
        })
    }

    fn loop_body(&mut self) -> (Block, bool) {
        self.func().loops.push(LoopCtx {
            has_continue: false,
        });
        let block = self.block();
        let ctx = self.func().loops.pop().expect("loop context");
        (block, ctx.has_continue)
    }

    fn while_stat(&mut self) -> PResult<StmtKind> {
        let line = self.advance().line;
        let cond = self.expr()?;
        self.expect(T::Do, "do")?;
        let (body, _) = self.loop_body();
        self.expect_match(T::End, "while", line)?;
        Ok(StmtKind::While { cond, body })
    }

    fn repeat_stat(&mut self) -> PResult<StmtKind> {
        let line = self.advance().line;
        let (body, has_continue) = self.loop_body();
        self.expect_match(T::Until, "repeat", line)?;
        let cond = self.expr()?;
        if has_continue {
            let locals = top_level_locals(&body);
            if let Some(name) = first_name_use(&cond, &locals) {
                self.error_at(
                    "E0107",
                    name.span,
                    format!(
                        "`continue` cannot be used in this `repeat` loop: its `until` condition uses the body local `{}`",
                        name.text
                    ),
                );
            }
        }
        Ok(StmtKind::Repeat { body, cond })
    }

    fn typed_name(&mut self) -> PResult<TypedName> {
        let name = self.name()?;
        let ty = if self.eat(T::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        Ok(TypedName { name, ty })
    }

    fn for_stat(&mut self) -> PResult<StmtKind> {
        let line = self.advance().line;
        let first = self.typed_name()?;
        match self.kind() {
            T::Eq => {
                self.advance();
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
                self.expect_match(T::End, "for", line)?;
                Ok(StmtKind::NumericFor {
                    var: first,
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
                self.expect_match(T::End, "for", line)?;
                Ok(StmtKind::GenericFor { vars, exprs, body })
            }
            _ => Err(self.syntax_error("'=' or 'in' expected")),
        }
    }

    fn func_name(&mut self) -> PResult<FuncName> {
        let start = self.tok(0).start;
        let mut path = vec![self.name()?];
        while self.check(T::Dot) {
            self.advance();
            path.push(self.name()?);
        }
        let method = if self.eat(T::Colon) {
            Some(self.name()?)
        } else {
            None
        };
        Ok(FuncName {
            path,
            method,
            span: self.span_from(start),
        })
    }

    fn function_stat(&mut self) -> PResult<StmtKind> {
        let line = self.advance().line;
        let name = self.func_name()?;
        let func = self.func_body(line)?;
        Ok(StmtKind::Function {
            name,
            func: Box::new(func),
        })
    }

    fn local_stat(&mut self) -> PResult<StmtKind> {
        self.advance();
        if self.check(T::Function) {
            let line = self.advance().line;
            let name = self.name()?;
            let func = self.func_body(line)?;
            return Ok(StmtKind::LocalFunction {
                name,
                func: Box::new(func),
            });
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
            Vec::new()
        };
        Ok(StmtKind::Local { names, values })
    }

    /// `local { a, b = 1 }: T = e` (spec §1.3.3)
    fn destructure(&mut self) -> PResult<StmtKind> {
        let open = self.advance();
        let mut fields = Vec::new();
        loop {
            let name = self.name()?;
            let default = if self.eat(T::Eq) {
                Some(self.expr()?)
            } else {
                None
            };
            fields.push(DestructField { name, default });
            if !self.eat(T::Comma) || self.check(T::RBrace) {
                break;
            }
        }
        self.expect_match(T::RBrace, "{", open.line)?;
        let ty = if self.eat(T::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        let value = if self.eat(T::Eq) {
            Some(self.expr()?)
        } else {
            None
        };
        Ok(StmtKind::LocalDestructure { fields, ty, value })
    }

    fn const_decl(&mut self, start: u32) -> PResult<StmtKind> {
        self.advance(); // const
        let name = self.name()?;
        let ty = if self.eat(T::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        if self.check(T::Comma) {
            let span = self.token_span(&self.tok(0).clone());
            self.error_at("E0112", span, "`const` declares exactly one name");
            while self.eat(T::Comma) {
                self.typed_name()?;
            }
        }
        let value = if self.eat(T::Eq) {
            Some(self.expr()?)
        } else {
            let span = self.span_from(start);
            self.error_at("E0112", span, "`const` needs an initializer");
            None
        };
        Ok(StmtKind::Const { name, ty, value })
    }

    fn return_stat(&mut self) -> PResult<StmtKind> {
        self.advance();
        if is_block_follow(self.kind()) || self.check(T::Semi) {
            return Ok(StmtKind::Return(Vec::new()));
        }
        Ok(StmtKind::Return(self.expr_list()?))
    }

    fn expr_stat(&mut self) -> PResult<StmtKind> {
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
            return Ok(StmtKind::Assign { targets, values });
        }
        if let Some(op) = compound_op(self.kind()) {
            self.check_target(&first)?;
            self.advance();
            let value = self.expr()?;
            return Ok(StmtKind::Compound {
                target: first,
                op,
                value,
            });
        }
        if is_call(&first.kind) {
            return Ok(StmtKind::Call(first));
        }
        Err(self.syntax_error("syntax error"))
    }

    fn check_target(&mut self, target: &Expr) -> PResult<()> {
        if contains_optional(target) {
            self.error_at(
                "E0113",
                target.span,
                "an optional chain cannot be assigned to",
            );
            return Err(Failed);
        }
        match target.kind {
            ExprKind::Name(_) | ExprKind::Index { .. } | ExprKind::Field { .. } => Ok(()),
            _ => Err(self.syntax_error("syntax error")),
        }
    }

    // ------------------------------------------------------- expressions --

    fn expr_list(&mut self) -> PResult<Vec<Expr>> {
        // Taken off the stack on every path, so that a list that fails
        // leaves nothing behind for the list it is nested in.
        let mark = self.expr_stack.len();
        let result = self.expr_items();
        let list = self.expr_stack.drain(mark..).collect();
        result.map(|()| list)
    }

    fn expr_items(&mut self) -> PResult<()> {
        let first = self.expr()?;
        self.expr_stack.push(first);
        while self.eat(T::Comma) {
            let e = self.expr()?;
            self.expr_stack.push(e);
        }
        Ok(())
    }

    pub(crate) fn expr(&mut self) -> PResult<Expr> {
        self.subexpr(0)
    }

    fn subexpr(&mut self, limit: u8) -> PResult<Expr> {
        self.enter_level()?;
        let result = grow(|| self.subexpr_inner(limit));
        self.leave_level();
        result
    }

    fn subexpr_inner(&mut self, limit: u8) -> PResult<Expr> {
        let start = self.tok(0).start;
        let mut lhs = if let Some(op) = unary_op(self.kind()) {
            self.advance();
            let operand = self.subexpr(UNARY_PRIORITY)?;
            Expr {
                id: self.id(),
                span: self.span_from(start),
                kind: ExprKind::Unary {
                    op,
                    operand: Box::new(operand),
                },
            }
        } else {
            self.simple_expr()?
        };
        while let Some(op) = binary_op(self.kind()) {
            let (left, right) = op.priority();
            if left <= limit {
                break;
            }
            let op_token = self.advance();
            let rhs = self.subexpr(right)?;
            self.check_coalesce_mix(op, &lhs, &rhs, &op_token);
            lhs = Expr {
                id: self.id(),
                span: self.span_from(start),
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            };
        }
        Ok(lhs)
    }

    /// `??` and `and`/`or` cannot be mixed without parentheses (E0110).
    fn check_coalesce_mix(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, op_token: &Token) {
        let is = |e: &Expr, ops: &[BinOp]| matches!(&e.kind, ExprKind::Binary { op, .. } if ops.contains(op));
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

    fn simple_expr(&mut self) -> PResult<Expr> {
        let start = self.tok(0).start;
        let kind = match self.kind() {
            T::Number => {
                let t = self.advance();
                ExprKind::Number(self.number_lit(&t))
            }
            T::String | T::LongString => {
                let t = self.advance();
                ExprKind::String(self.string_lit(&t))
            }
            T::Nil => {
                self.advance();
                ExprKind::Nil
            }
            T::True => {
                self.advance();
                ExprKind::True
            }
            T::False => {
                self.advance();
                ExprKind::False
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
                ExprKind::Vararg
            }
            T::LBrace => self.table()?,
            T::Function => {
                let line = self.advance().line;
                ExprKind::Function(Box::new(self.func_body(line)?))
            }
            T::InterpStart => self.interp()?,
            _ => {
                let e = self.suffixed_expr()?;
                return self.casts(e, start);
            }
        };
        let e = Expr {
            id: self.id(),
            span: self.span_from(start),
            kind,
        };
        self.casts(e, start)
    }

    /// `e :: T` (spec §1.4.3).
    fn casts(&mut self, mut e: Expr, start: u32) -> PResult<Expr> {
        while self.check(T::ColonColon) {
            self.advance();
            let ty = self.ty()?;
            e = Expr {
                id: self.id(),
                span: self.span_from(start),
                kind: ExprKind::Cast {
                    expr: Box::new(e),
                    ty: Box::new(ty),
                },
            };
        }
        Ok(e)
    }

    fn number_lit(&self, t: &Token) -> NumberLit {
        let raw = self.text_of(t);
        NumberLit {
            raw: String::from_utf8_lossy(raw).into(),
            value: crate::lexer::number_value(raw).ok(),
        }
    }

    fn string_lit(&self, t: &Token) -> StringLit {
        let raw = self.text_of(t);
        let long = t.kind == T::LongString;
        let value = if long {
            literal::long_string_value(raw)
        } else {
            literal::short_string_value(raw)
        };
        StringLit {
            raw: raw.into(),
            value: value.into(),
            long,
        }
    }

    fn primary_expr(&mut self) -> PResult<Expr> {
        let start = self.tok(0).start;
        match self.kind() {
            T::Name => {
                let t = self.advance();
                let name = self.make_name(&t);
                Ok(Expr {
                    id: self.id(),
                    span: name.span,
                    kind: ExprKind::Name(name),
                })
            }
            T::LParen => {
                let line = self.advance().line;
                let inner = self.expr()?;
                self.expect_match(T::RParen, "(", line)?;
                Ok(Expr {
                    id: self.id(),
                    span: self.span_from(start),
                    kind: ExprKind::Paren(Box::new(inner)),
                })
            }
            _ => Err(self.syntax_error("unexpected symbol")),
        }
    }

    fn suffixed_expr(&mut self) -> PResult<Expr> {
        let start = self.tok(0).start;
        let mut e = self.primary_expr()?;
        loop {
            let kind = match self.kind() {
                T::Dot => {
                    self.advance();
                    let name = self.name()?;
                    ExprKind::Field {
                        obj: Box::new(e),
                        name,
                    }
                }
                T::LBracket => {
                    self.advance();
                    let key = self.expr()?;
                    self.expect(T::RBracket, "]")?;
                    ExprKind::Index {
                        obj: Box::new(e),
                        key: Box::new(key),
                    }
                }
                T::Colon => {
                    self.advance();
                    let name = self.name()?;
                    let args = self.call_args()?;
                    ExprKind::Method {
                        obj: Box::new(e),
                        name,
                        args: Box::new(args),
                    }
                }
                T::LParen | T::LBrace | T::String | T::LongString => {
                    let args = self.call_args()?;
                    ExprKind::Call {
                        callee: Box::new(e),
                        args,
                    }
                }
                T::QuestionDot => {
                    self.advance();
                    match self.kind() {
                        T::LBracket => {
                            self.advance();
                            let key = self.expr()?;
                            self.expect(T::RBracket, "]")?;
                            ExprKind::OptIndex {
                                obj: Box::new(e),
                                key: Box::new(key),
                            }
                        }
                        T::LParen => {
                            let args = self.paren_args()?;
                            ExprKind::OptCall {
                                callee: Box::new(e),
                                args,
                            }
                        }
                        _ => {
                            let name = self.name()?;
                            ExprKind::OptField {
                                obj: Box::new(e),
                                name,
                            }
                        }
                    }
                }
                T::QuestionColon => {
                    self.advance();
                    let name = self.name()?;
                    let args = self.call_args()?;
                    ExprKind::OptMethod {
                        obj: Box::new(e),
                        name,
                        args: Box::new(args),
                    }
                }
                _ => return Ok(e),
            };
            e = Expr {
                id: self.id(),
                span: self.span_from(start),
                kind,
            };
        }
    }

    fn paren_args(&mut self) -> PResult<Args> {
        let open = self.advance();
        let args = if self.check(T::RParen) {
            Vec::new()
        } else {
            self.expr_list()?
        };
        self.expect_match(T::RParen, "(", open.line)?;
        Ok(Args::Parens {
            args,
            span: self.span_from(open.start),
        })
    }

    fn call_args(&mut self) -> PResult<Args> {
        match self.kind() {
            T::LParen => {
                let t = *self.tok(0);
                if t.line != self.prev().end_line {
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
                let start = self.tok(0).start;
                let kind = self.table()?;
                Ok(Args::Table(Box::new(Expr {
                    id: self.id(),
                    span: self.span_from(start),
                    kind,
                })))
            }
            T::String | T::LongString => {
                let t = self.advance();
                let lit = self.string_lit(&t);
                Ok(Args::String(Box::new(Expr {
                    id: self.id(),
                    span: self.token_span(&t),
                    kind: ExprKind::String(lit),
                })))
            }
            _ => Err(self.syntax_error("function arguments expected")),
        }
    }

    fn table(&mut self) -> PResult<ExprKind> {
        let open = self.advance();
        // As in `expr_list`, the fields come off the stack on every path.
        let mark = self.field_stack.len();
        let result = self.table_fields(open.line);
        let fields = self.field_stack.drain(mark..).collect();
        result.map(|()| ExprKind::Table(fields))
    }

    fn table_fields(&mut self, line: u32) -> PResult<()> {
        loop {
            if self.check(T::RBrace) {
                break;
            }
            let start = self.tok(0).start;
            let field = match self.kind() {
                T::LBracket => {
                    self.advance();
                    let key = self.expr()?;
                    self.expect(T::RBracket, "]")?;
                    self.expect(T::Eq, "=")?;
                    let value = self.expr()?;
                    TableField::Keyed {
                        key: Box::new(key),
                        value,
                        span: self.span_from(start),
                    }
                }
                T::Name if self.kind_at(1) == T::Eq => {
                    let name = self.name()?;
                    self.advance();
                    let value = self.expr()?;
                    TableField::Named {
                        name,
                        value,
                        span: self.span_from(start),
                    }
                }
                _ => TableField::Positional(self.expr()?),
            };
            self.field_stack.push(field);
            if !(self.eat(T::Comma) || self.eat(T::Semi)) {
                break;
            }
        }
        self.expect_match(T::RBrace, "{", line)?;
        Ok(())
    }

    /// A backtick string (spec §1.2.4).
    fn interp(&mut self) -> PResult<ExprKind> {
        let open = self.advance();
        let mut parts = Vec::new();
        loop {
            match self.kind() {
                T::InterpText => {
                    let t = self.advance();
                    let raw = self.text_of(&t);
                    parts.push(InterpPart::Text {
                        raw: raw.into(),
                        value: literal::interp_text_value(raw).into(),
                        span: self.token_span(&t),
                    });
                }
                T::InterpOpen => {
                    let line = self.advance().line;
                    parts.push(InterpPart::Expr(self.expr()?));
                    self.expect_match(T::InterpClose, "{", line)?;
                }
                T::InterpEnd => {
                    self.advance();
                    break;
                }
                _ => {
                    return Err(self.syntax_error(format!(
                        "'`' expected (to close '`' at line {})",
                        open.line
                    )));
                }
            }
        }
        Ok(ExprKind::Interp(parts))
    }

    // --------------------------------------------------------- functions --

    /// `[<T>] ( params ) [: ret] block end`
    fn func_body(&mut self, line: u32) -> PResult<FuncBody> {
        let start = self.tok(0).start;
        let type_params = self.type_params_opt()?;
        let params_start = self.tok(0).start;
        let open = self.expect(T::LParen, "(")?;
        let (params, vararg) = self.param_list(false)?;
        self.expect_match(T::RParen, "(", open.line)?;
        let params_span = self.span_from(params_start);
        let ret = if self.eat(T::Colon) {
            Some(self.ret_type()?)
        } else {
            None
        };
        self.funcs.push(FuncCtx {
            vararg: vararg.is_some(),
            loops: Vec::new(),
        });
        let body = self.block();
        self.funcs.pop();
        let end = self.expect_match(T::End, "function", line)?;
        Ok(FuncBody {
            type_params,
            params,
            vararg,
            ret,
            body,
            params_span,
            end_span: self.token_span(&end),
            span: self.span_from(start),
        })
    }

    /// A signature without body: `[<T>] ( params ) [: ret]`.
    fn signature(&mut self, declaration: bool) -> PResult<FuncBody> {
        let start = self.tok(0).start;
        let type_params = self.type_params_opt()?;
        let params_start = self.tok(0).start;
        let open = self.expect(T::LParen, "(")?;
        let (params, vararg) = self.param_list(declaration)?;
        self.expect_match(T::RParen, "(", open.line)?;
        let params_span = self.span_from(params_start);
        let ret = if self.eat(T::Colon) {
            Some(self.ret_type()?)
        } else {
            None
        };
        let span = self.span_from(start);
        Ok(FuncBody {
            type_params,
            params,
            vararg,
            ret,
            body: Block {
                stmts: Vec::new(),
                span: Span {
                    file: self.file,
                    start: span.end,
                    end: span.end,
                },
            },
            params_span,
            end_span: Span {
                file: self.file,
                start: span.end,
                end: span.end,
            },
            span,
        })
    }

    fn param_list(&mut self, declaration: bool) -> PResult<(Vec<Param>, Option<Vararg>)> {
        let mut params = Vec::new();
        if self.check(T::RParen) {
            return Ok((params, None));
        }
        loop {
            let decorators = if declaration && self.check(T::At) {
                self.decorators()?
            } else {
                Vec::new()
            };
            if self.check(T::Ellipsis) {
                let t = self.advance();
                let ty = if self.eat(T::Colon) {
                    Some(self.vararg_type()?)
                } else {
                    None
                };
                let vararg = Vararg {
                    span: self.span_from(t.start),
                    ty,
                };
                return Ok((params, Some(vararg)));
            }
            let name = match self.kind() {
                T::Name => self.name()?,
                _ => return Err(self.syntax_error("<name> or '...' expected")),
            };
            let ty = if self.eat(T::Colon) {
                Some(self.ty()?)
            } else {
                None
            };
            let default = if !declaration && self.eat(T::Eq) {
                Some(self.expr()?)
            } else {
                None
            };
            params.push(Param {
                decorators,
                name,
                ty,
                default,
            });
            if !self.eat(T::Comma) {
                return Ok((params, None));
            }
        }
    }

    // ------------------------------------------------------------- types --

    /// Splits a `>>`, `>=`, `>>=` (or `?:`) token at its first character, in
    /// type context (spec §1.2.1).
    fn split_first(&mut self, first: TokenKind, rest: TokenKind) {
        let t = self.lexed.tokens[self.pos];
        let mut head = t;
        head.kind = first;
        head.end = t.start + 1;
        head.end_line = t.line;
        let mut tail = t;
        tail.kind = rest;
        tail.start = t.start + 1;
        tail.trivia_start = t.trivia_end;
        self.lexed.tokens[self.pos] = head;
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
        self.advance();
        Ok(())
    }

    fn type_params_opt(&mut self) -> PResult<Vec<TypeParam>> {
        if !self.check(T::Lt) {
            return Ok(Vec::new());
        }
        self.advance();
        let mut params = Vec::new();
        loop {
            let name = self.name()?;
            if self.eat(T::Ellipsis) {
                params.push(TypeParam {
                    name,
                    constraint: None,
                    pack: true,
                });
            } else {
                let constraint = if self.is_word("extends") {
                    self.advance();
                    Some(self.ty()?)
                } else {
                    None
                };
                params.push(TypeParam {
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
        Ok(params)
    }

    pub(crate) fn ty(&mut self) -> PResult<Type> {
        self.enter_level()?;
        let start = self.tok(0).start;
        let result = grow(|| {
            self.eat(T::Pipe);
            let first = self.intersection()?;
            self.union_rest(first, start)
        });
        self.leave_level();
        result
    }

    fn union_rest(&mut self, first: Type, start: u32) -> PResult<Type> {
        if !self.check(T::Pipe) {
            return Ok(first);
        }
        let mut members = vec![first];
        while self.eat(T::Pipe) {
            members.push(self.intersection()?);
        }
        Ok(Type {
            id: self.id(),
            span: self.span_from(start),
            kind: TypeKind::Union(members),
        })
    }

    fn intersection(&mut self) -> PResult<Type> {
        let start = self.tok(0).start;
        let first = self.postfix_type()?;
        self.intersection_rest(first, start)
    }

    fn intersection_rest(&mut self, first: Type, start: u32) -> PResult<Type> {
        if !self.check(T::Amp) {
            return Ok(first);
        }
        let mut members = vec![first];
        while self.eat(T::Amp) {
            members.push(self.postfix_type()?);
        }
        Ok(Type {
            id: self.id(),
            span: self.span_from(start),
            kind: TypeKind::Intersection(members),
        })
    }

    fn postfix_type(&mut self) -> PResult<Type> {
        let start = self.tok(0).start;
        let base = self.primary_type()?;
        self.postfix_rest(base, start)
    }

    fn postfix_rest(&mut self, mut base: Type, start: u32) -> PResult<Type> {
        loop {
            let kind = match self.kind() {
                T::Question => {
                    self.advance();
                    TypeKind::Optional(Box::new(base))
                }
                T::QuestionColon => {
                    self.split_first(T::Question, T::Colon);
                    self.advance();
                    TypeKind::Optional(Box::new(base))
                }
                T::LBracket if self.kind_at(1) == T::RBracket => {
                    self.advance();
                    self.advance();
                    TypeKind::Array(Box::new(base))
                }
                _ => return Ok(base),
            };
            base = Type {
                id: self.id(),
                span: self.span_from(start),
                kind,
            };
        }
    }

    /// Continues a type whose first primary was already parsed.
    fn type_after_primary(&mut self, primary: Type, start: u32) -> PResult<Type> {
        let postfix = self.postfix_rest(primary, start)?;
        let isect = self.intersection_rest(postfix, start)?;
        self.union_rest(isect, start)
    }

    fn primary_type(&mut self) -> PResult<Type> {
        let start = self.tok(0).start;
        let kind = match self.kind() {
            T::Nil => {
                self.advance();
                TypeKind::Nil
            }
            T::True => {
                self.advance();
                TypeKind::True
            }
            T::False => {
                self.advance();
                TypeKind::False
            }
            T::Function => {
                self.advance();
                TypeKind::AnyFunction
            }
            T::String | T::LongString => {
                let t = self.advance();
                TypeKind::String(self.string_lit(&t))
            }
            T::Number => {
                let t = self.advance();
                TypeKind::Number(self.number_lit(&t))
            }
            T::Name if self.is_word("typeof") && self.kind_at(1) == T::LParen => {
                self.advance();
                let open = self.advance();
                let e = self.expr()?;
                self.expect_match(T::RParen, "(", open.line)?;
                TypeKind::Typeof(Box::new(e))
            }
            T::Name => {
                let (path, args) = self.type_ref_parts()?;
                TypeKind::Ref { path, args }
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
                    return self.function_type_rest(Vec::new(), params, vararg, start);
                }
                if params.len() == 1 && params[0].name.is_none() && vararg.is_none() {
                    let inner = params.into_iter().next().expect("one").ty;
                    TypeKind::Paren(Box::new(inner))
                } else {
                    return Err(self.syntax_error("'->' expected"));
                }
            }
            _ => return Err(self.syntax_error("type expected")),
        };
        Ok(Type {
            id: self.id(),
            span: self.span_from(start),
            kind,
        })
    }

    fn type_ref_parts(&mut self) -> PResult<(Vec<Name>, Vec<TypeArg>)> {
        let mut path = vec![self.name()?];
        while self.check(T::Dot) {
            self.advance();
            path.push(self.name()?);
        }
        let args = if self.check(T::Lt) {
            self.type_args()?
        } else {
            Vec::new()
        };
        Ok((path, args))
    }

    pub(crate) fn type_ref(&mut self) -> PResult<Type> {
        let start = self.tok(0).start;
        let (path, args) = self.type_ref_parts()?;
        Ok(Type {
            id: self.id(),
            span: self.span_from(start),
            kind: TypeKind::Ref { path, args },
        })
    }

    fn type_args(&mut self) -> PResult<Vec<TypeArg>> {
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

    fn type_arg(&mut self) -> PResult<TypeArg> {
        let start = self.tok(0).start;
        if self.check(T::Name) && self.kind_at(1) == T::Ellipsis {
            let name = self.name()?;
            self.advance();
            return Ok(TypeArg::Pack(TypePack::Named(name)));
        }
        if self.check(T::LParen) {
            let (params, vararg, span) = self.paren_group()?;
            if self.check(T::Arrow) {
                let f = self.function_type_rest(Vec::new(), params, vararg, start)?;
                return Ok(TypeArg::Type(f));
            }
            if params.len() == 1 && params[0].name.is_none() && vararg.is_none() {
                let inner = params.into_iter().next().expect("one").ty;
                let paren = Type {
                    id: self.id(),
                    span,
                    kind: TypeKind::Paren(Box::new(inner)),
                };
                return Ok(TypeArg::Type(self.type_after_primary(paren, start)?));
            }
            let types = params.into_iter().map(|p| p.ty).collect();
            return Ok(TypeArg::Pack(TypePack::List {
                types,
                vararg: vararg.map(Box::new),
                span,
            }));
        }
        Ok(TypeArg::Type(self.ty()?))
    }

    /// `( [name:] T, …, ...: V )`, shared by function types, parenthesized
    /// types and type packs.
    fn paren_group(&mut self) -> PResult<(Vec<ParamType>, Option<VarargType>, Span)> {
        let open = self.expect(T::LParen, "(")?;
        let mut params = Vec::new();
        let mut vararg = None;
        if !self.check(T::RParen) {
            loop {
                if self.check(T::Ellipsis) {
                    self.advance();
                    self.eat(T::Colon);
                    vararg = Some(self.vararg_type()?);
                    break;
                }
                let name = if self.check(T::Name) && self.kind_at(1) == T::Colon {
                    let n = self.name()?;
                    self.advance();
                    Some(n)
                } else {
                    None
                };
                let ty = self.ty()?;
                if name.is_none() && self.eat(T::Ellipsis) {
                    // `(A...)`, `(EventArgs<E>...)`: a pack expansion.
                    vararg = Some(VarargType::Expand(ty));
                    break;
                }
                params.push(ParamType { name, ty });
                if !self.eat(T::Comma) {
                    break;
                }
            }
        }
        self.expect_match(T::RParen, "(", open.line)?;
        Ok((params, vararg, self.span_from(open.start)))
    }

    fn function_type_rest(
        &mut self,
        type_params: Vec<TypeParam>,
        params: Vec<ParamType>,
        vararg: Option<VarargType>,
        start: u32,
    ) -> PResult<Type> {
        self.expect(T::Arrow, "->")?;
        let ret = self.ret_type()?;
        Ok(Type {
            id: self.id(),
            span: self.span_from(start),
            kind: TypeKind::Function(Box::new(FunctionType {
                type_params,
                params,
                vararg,
                ret,
            })),
        })
    }

    /// `T` or `T...` after `...:`.
    fn vararg_type(&mut self) -> PResult<VarargType> {
        if self.check(T::Name) && self.kind_at(1) == T::Ellipsis {
            let name = self.name()?;
            self.advance();
            return Ok(VarargType::Pack(name));
        }
        Ok(VarargType::Type(self.ty()?))
    }

    fn ret_type(&mut self) -> PResult<RetType> {
        let start = self.tok(0).start;
        if self.check(T::Name) && self.kind_at(1) == T::Ellipsis {
            let name = self.name()?;
            self.advance();
            return Ok(RetType::Pack(name));
        }
        if self.check(T::Ellipsis) {
            // `...any`: any number of results.
            let open = self.advance();
            self.eat(T::Colon);
            let vararg = self.vararg_type()?;
            return Ok(RetType::List {
                types: Vec::new(),
                vararg: Some(Box::new(vararg)),
                span: self.span_from(open.start),
            });
        }
        if self.check(T::Name)
            && self.is_word_at(1, "is")
            && self.tok(1).line == self.tok(0).end_line
        {
            let name = self.name()?;
            self.advance();
            let ty = self.ty()?;
            return Ok(RetType::Predicate { name, ty });
        }
        if self.check(T::LParen) {
            let (params, vararg, span) = self.paren_group()?;
            if self.check(T::Arrow) {
                let f = self.function_type_rest(Vec::new(), params, vararg, start)?;
                return Ok(RetType::Single(f));
            }
            if params.len() == 1 && params[0].name.is_none() && vararg.is_none() {
                let inner = params.into_iter().next().expect("one").ty;
                let paren = Type {
                    id: self.id(),
                    span,
                    kind: TypeKind::Paren(Box::new(inner)),
                };
                return Ok(RetType::Single(self.type_after_primary(paren, start)?));
            }
            let types = params.into_iter().map(|p| p.ty).collect();
            return Ok(RetType::List {
                types,
                vararg: vararg.map(Box::new),
                span,
            });
        }
        let ty = self.ty()?;
        if self.check(T::Ellipsis) {
            let end = self.advance();
            let span = Span {
                file: self.file,
                start,
                end: end.end,
            };
            return Ok(RetType::List {
                types: Vec::new(),
                vararg: Some(Box::new(VarargType::Expand(ty))),
                span,
            });
        }
        Ok(RetType::Single(ty))
    }

    fn table_type(&mut self) -> PResult<TypeKind> {
        let open = self.advance();
        let mut fields = Vec::new();
        loop {
            if self.check(T::RBrace) {
                break;
            }
            let start = self.tok(0).start;
            let readonly = self.is_word("readonly")
                && self.kind_at(1) == T::Name
                && matches!(self.kind_at(2), T::Colon | T::QuestionColon);
            if readonly {
                self.advance();
            }
            let field =
                if self.check(T::Name) && matches!(self.kind_at(1), T::Colon | T::QuestionColon) {
                    let name = self.name()?;
                    let optional = self.advance().kind == T::QuestionColon;
                    let ty = self.ty()?;
                    TableTypeField::Named {
                        name,
                        optional,
                        readonly,
                        ty,
                    }
                } else if self.check(T::LBracket) {
                    self.advance();
                    if matches!(self.kind(), T::String | T::LongString)
                        && self.kind_at(1) == T::RBracket
                    {
                        let t = self.advance();
                        let key = self.string_lit(&t);
                        self.advance();
                        self.expect(T::Colon, ":")?;
                        let ty = self.ty()?;
                        TableTypeField::NamedString {
                            key,
                            ty,
                            span: self.span_from(start),
                        }
                    } else {
                        let key = self.ty()?;
                        self.expect(T::RBracket, "]")?;
                        self.expect(T::Colon, ":")?;
                        let value = self.ty()?;
                        TableTypeField::Indexer { key, value }
                    }
                } else {
                    TableTypeField::Array(self.ty()?)
                };
            fields.push(field);
            if !(self.eat(T::Comma) || self.eat(T::Semi)) {
                break;
            }
        }
        self.expect_match(T::RBrace, "{", open.line)?;
        Ok(TypeKind::Table(fields))
    }

    // ------------------------------------------------------ declarations --

    fn type_alias(&mut self) -> PResult<TypeAlias> {
        self.advance(); // type
        let name = self.name()?;
        let type_params = self.type_params_opt()?;
        self.expect(T::Eq, "=")?;
        let ty = self.ty()?;
        Ok(TypeAlias {
            name,
            type_params,
            ty,
        })
    }

    fn type_list_after(&mut self, word: &str) -> PResult<Vec<Type>> {
        let mut out = Vec::new();
        if self.is_word(word) {
            self.advance();
            out.push(self.type_ref()?);
            while self.eat(T::Comma) {
                out.push(self.type_ref()?);
            }
        }
        Ok(out)
    }

    fn class_decl(&mut self) -> PResult<ClassDecl> {
        let is_abstract = self.is_word("abstract");
        if is_abstract {
            self.advance();
        }
        let line = self.advance().line; // class
        let name = self.name()?;
        let type_params = self.type_params_opt()?;
        let extends = if self.is_word("extends") {
            self.advance();
            Some(self.type_ref()?)
        } else {
            None
        };
        let implements = self.type_list_after("implements")?;
        let mut members = Vec::new();
        while !self.check(T::End) && !self.check(T::Eof) {
            let before = self.pos;
            match self.class_member() {
                Ok(m) => members.push(m),
                Err(Failed) => {
                    if self.pos == before {
                        self.advance();
                    }
                    while !matches!(self.kind(), T::End | T::Eof | T::Function | T::At)
                        && !self.is_member_start()
                    {
                        self.advance();
                    }
                }
            }
        }
        let end = self.expect_match(T::End, "class", line)?;
        Ok(ClassDecl {
            is_abstract,
            name,
            type_params,
            extends,
            implements,
            members,
            end_span: self.token_span(&end),
        })
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

    fn class_member(&mut self) -> PResult<ClassMember> {
        let start = self.tok(0).start;
        let decorators = if self.check(T::At) {
            self.decorators()?
        } else {
            Vec::new()
        };
        let mut modifiers = Vec::new();
        while self.is_modifier() {
            modifiers.push(self.modifier());
        }
        let signature_only = modifiers
            .iter()
            .any(|m| matches!(m, Modifier::Abstract | Modifier::Overload));
        let kind = if self.check(T::Function) {
            let line = self.advance().line;
            let name = self.name()?;
            if signature_only {
                ClassMemberKind::Method {
                    name,
                    func: Box::new(self.signature(false)?),
                    has_body: false,
                }
            } else {
                ClassMemberKind::Method {
                    name,
                    func: Box::new(self.method_body(line)?),
                    has_body: true,
                }
            }
        } else if self.is_word("constructor") && self.kind_at(1) == T::LParen {
            let line = self.advance().line;
            if signature_only {
                ClassMemberKind::Constructor {
                    func: Box::new(self.signature(false)?),
                    has_body: false,
                }
            } else {
                ClassMemberKind::Constructor {
                    func: Box::new(self.method_body(line)?),
                    has_body: true,
                }
            }
        } else if self.is_word("get") && self.kind_at(1) == T::Name {
            let line = self.advance().line;
            let name = self.name()?;
            let open = self.expect(T::LParen, "(")?;
            self.expect_match(T::RParen, "(", open.line)?;
            let ty = if self.eat(T::Colon) {
                Some(self.ty()?)
            } else {
                None
            };
            let body = self.method_block();
            self.expect_match(T::End, "get", line)?;
            ClassMemberKind::Getter { name, ty, body }
        } else if self.is_word("set") && self.kind_at(1) == T::Name {
            let line = self.advance().line;
            let name = self.name()?;
            let open = self.expect(T::LParen, "(")?;
            let param = self.typed_name()?;
            self.expect_match(T::RParen, "(", open.line)?;
            let body = self.method_block();
            self.expect_match(T::End, "set", line)?;
            ClassMemberKind::Setter { name, param, body }
        } else if self.check(T::Name) {
            let name = self.name()?;
            let optional = match self.kind() {
                T::QuestionColon => {
                    self.advance();
                    true
                }
                _ => {
                    self.expect(T::Colon, ":")?;
                    false
                }
            };
            let ty = self.ty()?;
            let value = if self.eat(T::Eq) {
                Some(self.expr()?)
            } else {
                None
            };
            ClassMemberKind::Field {
                name,
                optional,
                ty,
                value,
            }
        } else {
            return Err(self.syntax_error("class member expected"));
        };
        Ok(ClassMember {
            decorators,
            modifiers,
            span: self.span_from(start),
            kind,
        })
    }

    /// Methods are vararg only when they declare `...`.
    fn method_body(&mut self, line: u32) -> PResult<FuncBody> {
        self.func_body(line)
    }

    fn method_block(&mut self) -> Block {
        self.funcs.push(FuncCtx::default());
        let block = self.block();
        self.funcs.pop();
        block
    }

    fn interface_decl(&mut self) -> PResult<InterfaceDecl> {
        let line = self.advance().line; // interface
        let name = self.name()?;
        let type_params = self.type_params_opt()?;
        let extends = self.type_list_after("extends")?;
        let mut members = Vec::new();
        while !self.check(T::End) && !self.check(T::Eof) {
            members.push(self.interface_member()?);
        }
        self.expect_match(T::End, "interface", line)?;
        Ok(InterfaceDecl {
            name,
            type_params,
            extends,
            members,
        })
    }

    fn interface_member(&mut self) -> PResult<InterfaceMember> {
        if self.check(T::Function) {
            self.advance();
            let name = self.name()?;
            return Ok(InterfaceMember::Method {
                name,
                sig: Box::new(self.signature(false)?),
            });
        }
        let readonly = self.is_word("readonly") && self.kind_at(1) == T::Name;
        if readonly {
            self.advance();
        }
        let name = self.name()?;
        let optional = match self.kind() {
            T::QuestionColon => {
                self.advance();
                true
            }
            _ => {
                self.expect(T::Colon, ":")?;
                false
            }
        };
        let ty = self.ty()?;
        Ok(InterfaceMember::Field {
            readonly,
            name,
            optional,
            ty,
        })
    }

    fn enum_decl(&mut self) -> PResult<EnumDecl> {
        self.advance(); // enum
        let name = self.name()?;
        let open = self.expect(T::LBrace, "{")?;
        let mut items = Vec::new();
        while !self.check(T::RBrace) {
            let item = self.name()?;
            let value = if self.eat(T::Eq) {
                Some(self.expr()?)
            } else {
                None
            };
            items.push(EnumItem { name: item, value });
            if !(self.eat(T::Comma) || self.eat(T::Semi)) {
                break;
            }
        }
        self.expect_match(T::RBrace, "{", open.line)?;
        Ok(EnumDecl { name, items })
    }

    fn declare(&mut self) -> PResult<Declare> {
        let line = self.advance().line; // declare
        if self.eat(T::Function) {
            let name = self.func_name()?;
            let sig = self.signature(true)?;
            return Ok(Declare::Function {
                name,
                sig: Box::new(sig),
            });
        }
        let word = self.advance();
        let word = String::from_utf8_lossy(self.text_of(&word)).into_owned();
        match word.as_str() {
            "global" => {
                let name = self.name()?;
                self.expect(T::Colon, ":")?;
                let ty = self.ty()?;
                Ok(Declare::Global { name, ty })
            }
            "namespace" => {
                let name = self.name()?;
                let members = self.ns_members()?;
                self.expect_match(T::End, "namespace", line)?;
                Ok(Declare::Namespace { name, members })
            }
            "class" => {
                let name = self.name()?;
                let type_params = self.type_params_opt()?;
                let extends = if self.is_word("extends") {
                    self.advance();
                    Some(self.type_ref()?)
                } else {
                    None
                };
                let implements = self.type_list_after("implements")?;
                let members = self.decl_members()?;
                self.expect_match(T::End, "class", line)?;
                Ok(Declare::Class {
                    name,
                    type_params,
                    extends,
                    implements,
                    members,
                })
            }
            "interface" => {
                self.pos -= 1;
                Ok(Declare::Interface(self.interface_decl()?))
            }
            "event" | "message" => {
                let name = self.name()?;
                let params = Box::new(self.signature(true)?);
                Ok(if word == "event" {
                    Declare::Event { name, params }
                } else {
                    Declare::Message { name, params }
                })
            }
            _ => {
                // module
                let t = self.expect_string()?;
                let path = self.string_lit(&t);
                let members = self.ns_members()?;
                self.expect_match(T::End, "module", line)?;
                Ok(Declare::Module { path, members })
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

    fn ns_members(&mut self) -> PResult<Vec<NsMember>> {
        let mut out = Vec::new();
        while !self.check(T::End) && !self.check(T::Eof) {
            let start = self.tok(0).start;
            let decorators = if self.check(T::At) {
                self.decorators()?
            } else {
                Vec::new()
            };
            let kind = if self.eat(T::Function) {
                let name = self.name()?;
                NsMemberKind::Function {
                    name,
                    sig: Box::new(self.signature(true)?),
                }
            } else if self.is_word("namespace") && self.kind_at(1) == T::Name {
                let line = self.advance().line;
                let name = self.name()?;
                let members = self.ns_members()?;
                self.expect_match(T::End, "namespace", line)?;
                NsMemberKind::Namespace { name, members }
            } else if self.is_word("class") && self.kind_at(1) == T::Name {
                let line = self.advance().line;
                let name = self.name()?;
                let type_params = self.type_params_opt()?;
                let extends = if self.is_word("extends") {
                    self.advance();
                    Some(self.type_ref()?)
                } else {
                    None
                };
                let members = self.decl_members()?;
                self.expect_match(T::End, "class", line)?;
                NsMemberKind::Class {
                    name,
                    type_params,
                    extends,
                    members,
                }
            } else if self.is_word("interface") && self.kind_at(1) == T::Name {
                NsMemberKind::Interface(self.interface_decl()?)
            } else if self.is_word("type") && self.kind_at(1) == T::Name {
                NsMemberKind::TypeAlias(self.type_alias()?)
            } else if self.is_word("enum") && self.kind_at(1) == T::Name {
                NsMemberKind::Enum(self.enum_decl()?)
            } else {
                let is_const = self.is_word("const") && self.kind_at(1) == T::Name;
                if is_const {
                    self.advance();
                }
                let name = self.name()?;
                self.expect(T::Colon, ":")?;
                let ty = self.ty()?;
                NsMemberKind::Value { is_const, name, ty }
            };
            out.push(NsMember {
                decorators,
                span: self.span_from(start),
                kind,
            });
        }
        Ok(out)
    }

    fn decl_members(&mut self) -> PResult<Vec<DeclMember>> {
        let mut out = Vec::new();
        while !self.check(T::End) && !self.check(T::Eof) {
            let start = self.tok(0).start;
            let decorators = if self.check(T::At) {
                self.decorators()?
            } else {
                Vec::new()
            };
            let is_static = self.is_word("static")
                && !matches!(self.kind_at(1), T::Colon | T::QuestionColon | T::LParen);
            if is_static {
                self.advance();
            }
            let kind = if self.eat(T::Function) {
                let name = self.name()?;
                DeclMemberKind::Function {
                    name,
                    sig: Box::new(self.signature(true)?),
                }
            } else {
                let readonly = self.is_word("readonly") && self.kind_at(1) == T::Name;
                if readonly {
                    self.advance();
                }
                let name = self.name()?;
                let optional = match self.kind() {
                    T::QuestionColon => {
                        self.advance();
                        true
                    }
                    _ => {
                        self.expect(T::Colon, ":")?;
                        false
                    }
                };
                let ty = self.ty()?;
                DeclMemberKind::Field {
                    readonly,
                    name,
                    optional,
                    ty,
                }
            };
            out.push(DeclMember {
                decorators,
                is_static,
                span: self.span_from(start),
                kind,
            });
        }
        Ok(out)
    }

    fn import(&mut self) -> PResult<Import> {
        self.advance(); // import
        // `import type X from` is type-only; `import type from "x"` imports `type`.
        let type_only =
            self.is_word("type") && self.kind_at(1) == T::Name && !self.is_word_at(1, "from");
        if type_only {
            self.advance();
        }
        let clause = if self.eat(T::Star) {
            if !self.is_word("as") {
                return Err(self.syntax_error("'as' expected"));
            }
            self.advance();
            ImportClause::Namespace(self.name()?)
        } else if self.is_word("_") && !matches!(self.kind_at(1), T::Comma) {
            self.advance();
            ImportClause::SideEffect
        } else {
            let mut specs = Vec::new();
            loop {
                let name = self.name()?;
                if name.text.as_ref() == "_" {
                    self.error_at(
                        "E0117",
                        name.span,
                        "`import _` cannot be combined with other names",
                    );
                }
                let alias = if self.is_word("as") {
                    self.advance();
                    Some(self.name()?)
                } else {
                    None
                };
                specs.push(ImportSpec { name, alias });
                if !self.eat(T::Comma) {
                    break;
                }
            }
            ImportClause::Names(specs)
        };
        if !self.is_word("from") {
            return Err(self.syntax_error("'from' expected"));
        }
        self.advance();
        let t = self.expect_string()?;
        let from = self.string_lit(&t);
        Ok(Import {
            clause,
            type_only,
            from,
        })
    }

    fn export(&mut self, start: u32) -> PResult<StmtKind> {
        self.advance(); // export
        let inner_start = self.tok(0).start;
        let kind = match self.kind() {
            T::Local => {
                self.advance();
                let mut names = vec![self.typed_name()?];
                while self.eat(T::Comma) {
                    names.push(self.typed_name()?);
                }
                let values = if self.eat(T::Eq) {
                    self.expr_list()?
                } else {
                    let span = self.span_from(start);
                    self.error_at("E0119", span, "`export local` needs an initializer");
                    Vec::new()
                };
                StmtKind::Local { names, values }
            }
            T::Function => {
                let line = self.advance().line;
                let name = self.func_name()?;
                if name.path.len() > 1 || name.method.is_some() {
                    self.error_at("E0118", name.span, "`export function` takes a plain name");
                }
                let func = self.func_body(line)?;
                StmtKind::Function {
                    name,
                    func: Box::new(func),
                }
            }
            _ if self.is_word("const") => self.const_decl(inner_start)?,
            _ if self.is_word("class") || self.is_word("abstract") => {
                StmtKind::Class(Box::new(self.class_decl()?))
            }
            _ if self.is_word("interface") => StmtKind::Interface(Box::new(self.interface_decl()?)),
            _ if self.is_word("enum") => StmtKind::Enum(Box::new(self.enum_decl()?)),
            _ => StmtKind::TypeAlias(Box::new(self.type_alias()?)),
        };
        let inner = Stmt {
            id: self.id(),
            span: self.span_from(inner_start),
            decorators: Vec::new(),
            kind,
        };
        Ok(StmtKind::Export(Box::new(inner)))
    }
}

/// Runs `f` with enough stack for one more nesting level (deeply nested
/// input must produce an error, never a stack overflow).
fn grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, f)
}

fn decoratable(kind: &StmtKind) -> bool {
    matches!(
        kind,
        StmtKind::Function { .. }
            | StmtKind::LocalFunction { .. }
            | StmtKind::Local { .. }
            | StmtKind::LocalDestructure { .. }
            | StmtKind::Const { .. }
            | StmtKind::Class(_)
            | StmtKind::Interface(_)
            | StmtKind::Enum(_)
            | StmtKind::TypeAlias(_)
            | StmtKind::Declare(_)
            | StmtKind::Import(_)
            | StmtKind::Export(_)
    )
}

fn is_call(kind: &ExprKind) -> bool {
    matches!(
        kind,
        ExprKind::Call { .. }
            | ExprKind::Method { .. }
            | ExprKind::OptCall { .. }
            | ExprKind::OptMethod { .. }
    )
}

/// The expression is or contains (along its prefix chain) an optional access.
fn contains_optional(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::OptField { .. }
        | ExprKind::OptIndex { .. }
        | ExprKind::OptMethod { .. }
        | ExprKind::OptCall { .. } => true,
        ExprKind::Field { obj, .. } | ExprKind::Index { obj, .. } => contains_optional(obj),
        ExprKind::Call { callee, .. } => contains_optional(callee),
        ExprKind::Method { obj, .. } => contains_optional(obj),
        _ => false,
    }
}

/// Names declared at the top level of `block` (visible in `until`).
fn top_level_locals(block: &Block) -> Vec<Box<str>> {
    let mut out = Vec::new();
    for stmt in &block.stmts {
        match &stmt.kind {
            StmtKind::Local { names, .. } => out.extend(names.iter().map(|n| n.name.text.clone())),
            StmtKind::LocalFunction { name, .. } | StmtKind::Const { name, .. } => {
                out.push(name.text.clone())
            }
            StmtKind::LocalDestructure { fields, .. } => {
                out.extend(fields.iter().map(|f| f.name.text.clone()))
            }
            _ => {}
        }
    }
    out
}

/// The first use of one of `names` in `e` (approximate: ignores shadowing
/// inside nested functions).
fn first_name_use<'e>(e: &'e Expr, names: &[Box<str>]) -> Option<&'e Name> {
    let mut found = None;
    crate::visit::walk_expr(e, &mut |x: &'e Expr| {
        if found.is_none()
            && let ExprKind::Name(n) = &x.kind
            && names.contains(&n.text)
        {
            found = Some(n);
        }
    });
    found
}
