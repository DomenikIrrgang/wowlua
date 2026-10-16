//! Syntax tree → document, StyLua-like (spec tooling §16.3). Comments are
//! taken from the tokens' trivia by source position, so none is lost: when
//! a token is printed, every comment before it that was not printed yet is.

use crate::doc::Doc;
use wowlua_syntax::Parse;
use wowlua_syntax::ast::*;
use wowlua_syntax::token::TriviaKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteStyle {
    Double,
    Single,
    Preserve,
}

struct Comment {
    start: u32,
    text: Vec<u8>,
    block: bool,
    /// On the same line as the token before it.
    trailing: bool,
    /// Line breaks between the comment and what follows it.
    newlines_after: u32,
}

pub(crate) struct Formatter<'a> {
    pub(crate) parse: &'a Parse,
    pub(crate) src: &'a [u8],
    quote: QuoteStyle,
    comments: Vec<Comment>,
    next_comment: usize,
}

fn grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, f)
}

fn hard() -> Doc {
    Doc::HardLine
}

fn txt(s: &str) -> Doc {
    Doc::text(s)
}

impl<'a> Formatter<'a> {
    pub(crate) fn new(parse: &'a Parse, src: &'a [u8], quote: QuoteStyle) -> Self {
        let lexed = &parse.lexed;
        let mut comments = Vec::new();
        for ti in 0..lexed.tokens.len() {
            let trivia = lexed.trivia_of(ti);
            let mut seen_newline = ti == 0;
            for (i, t) in trivia.iter().enumerate() {
                match t.kind {
                    TriviaKind::Newline => seen_newline = true,
                    TriviaKind::LineComment | TriviaKind::BlockComment => {
                        let mut newlines_after = 0;
                        for next in &trivia[i + 1..] {
                            match next.kind {
                                TriviaKind::Newline => newlines_after += 1,
                                TriviaKind::Whitespace => {}
                                _ => break,
                            }
                        }
                        let block = t.kind == TriviaKind::BlockComment;
                        if !block {
                            newlines_after = newlines_after.max(1);
                        }
                        let raw = &src[t.start as usize..t.end as usize];
                        let text = if block {
                            raw.to_vec()
                        } else {
                            let end = raw
                                .iter()
                                .rposition(|c| !c.is_ascii_whitespace())
                                .map_or(0, |i| i + 1);
                            raw[..end].to_vec()
                        };
                        comments.push(Comment {
                            start: t.start,
                            text,
                            block,
                            trailing: !seen_newline,
                            newlines_after,
                        });
                        // A comment right after this one on the same line
                        // trails it.
                        seen_newline = false;
                    }
                    TriviaKind::Whitespace => {}
                }
            }
        }
        Formatter {
            parse,
            src,
            quote,
            comments,
            next_comment: 0,
        }
    }

    // ------------------------------------------------------------ tokens --

    /// Start offset of the last token before `offset`.
    pub(crate) fn prev_token(&self, offset: u32) -> u32 {
        let tokens = &self.parse.lexed.tokens;
        let i = tokens.partition_point(|t| t.start < offset);
        i.checked_sub(1)
            .and_then(|i| tokens.get(i))
            .map_or(offset, |t| t.start)
    }

    /// Marks the comments before `offset` printed (text kept as it is).
    pub(crate) fn skip_comments_before(&mut self, offset: u32) {
        while self
            .comments
            .get(self.next_comment)
            .is_some_and(|c| c.start < offset)
        {
            self.next_comment += 1;
        }
    }

    /// Start offset of the first token at or after `offset`.
    pub(crate) fn next_token(&self, offset: u32) -> u32 {
        let tokens = &self.parse.lexed.tokens;
        let i = tokens.partition_point(|t| t.start < offset);
        tokens.get(i).map(|t| t.start).unwrap_or(offset)
    }

    /// Comments before `offset` that were not printed yet: trailing ones go
    /// to the end of the current line, leading ones before the token.
    pub(crate) fn comments_before(&mut self, offset: u32) -> Vec<Doc> {
        let mut parts = Vec::new();
        while let Some(c) = self.comments.get(self.next_comment) {
            if c.start >= offset {
                break;
            }
            self.next_comment += 1;
            if c.trailing && !c.block {
                parts.push(Doc::LineSuffix(spaced(&c.text)));
                parts.push(Doc::BreakParent);
            } else if c.trailing {
                parts.push(Doc::text([c.text.as_slice(), b" "].concat()));
            } else {
                parts.push(Doc::FreshLine);
                parts.push(Doc::text(&c.text));
                match c.newlines_after {
                    0 => parts.push(txt(" ")),
                    1 => parts.push(hard()),
                    _ => {
                        parts.push(hard());
                        parts.push(hard());
                    }
                }
            }
        }
        parts
    }

    /// Trailing comments (on the line of the previous token) before
    /// `offset`, attached to the end of the current line.
    pub(crate) fn trailing_before(&mut self, offset: u32) -> Doc {
        let mut parts = Vec::new();
        while let Some(c) = self.comments.get(self.next_comment) {
            if c.start >= offset || !c.trailing {
                break;
            }
            self.next_comment += 1;
            if c.block {
                parts.push(Doc::text(spaced(&c.text)));
            } else {
                parts.push(Doc::LineSuffix(spaced(&c.text)));
                parts.push(Doc::BreakParent);
            }
        }
        Doc::concat(parts)
    }

    /// A token at source `offset`, preceded by the comments before it.
    pub(crate) fn tok(&mut self, text: impl AsRef<[u8]>, offset: u32) -> Doc {
        let mut parts = self.comments_before(offset);
        parts.push(Doc::text(text));
        Doc::concat(parts)
    }

    /// Comments before `offset` placed on their own lines inside a block
    /// (before its closing keyword).
    pub(crate) fn inner_comments(&mut self, offset: u32) -> Option<Doc> {
        let start = self.next_comment;
        let mut parts = Vec::new();
        while let Some(c) = self.comments.get(self.next_comment) {
            if c.start >= offset {
                break;
            }
            self.next_comment += 1;
            if c.trailing && !c.block {
                parts.push(Doc::LineSuffix(spaced(&c.text)));
                parts.push(Doc::BreakParent);
            } else {
                parts.push(hard());
                parts.push(Doc::text(&c.text));
            }
        }
        (self.next_comment > start).then(|| Doc::concat(parts))
    }

    /// The rest of the file's comments (after the last token).
    pub(crate) fn rest(&mut self) -> Doc {
        let mut parts = Vec::new();
        while let Some(c) = self.comments.get(self.next_comment) {
            self.next_comment += 1;
            if c.trailing && !c.block {
                parts.push(Doc::LineSuffix(spaced(&c.text)));
            } else {
                parts.push(hard());
                parts.push(Doc::text(&c.text));
            }
        }
        Doc::concat(parts)
    }

    /// The source had a blank line between `prev_end` and the token at
    /// `offset` (before any leading comment; `;` tokens in between count).
    pub(crate) fn blank_line_between(&self, prev_end: u32, offset: u32) -> bool {
        let tokens = &self.parse.lexed.tokens;
        let first = tokens.partition_point(|t| t.start < prev_end);
        let last = tokens.partition_point(|t| t.start < offset);
        let mut newlines = 0;
        let count = tokens.get(first..=last).map_or(0, <[_]>::len);
        for index in first..first + count {
            for t in self.parse.lexed.trivia_of(index) {
                match t.kind {
                    TriviaKind::Newline => newlines += 1,
                    TriviaKind::Whitespace => {}
                    _ if newlines == 0 => {} // trailing comment of the previous line
                    _ => return newlines >= 2,
                }
            }
        }
        newlines >= 2
    }

    // ------------------------------------------------------------ blocks --

    pub(crate) fn chunk(&mut self, chunk: &Chunk) -> Doc {
        let body = self.stmts(&chunk.block.stmts);
        let rest = self.rest();
        Doc::concat(vec![body, rest])
    }

    fn stmts(&mut self, stmts: &[Stmt]) -> Doc {
        let mut parts = Vec::new();
        for (i, s) in stmts.iter().enumerate() {
            if i > 0 {
                parts.push(self.trailing_before(s.span.start));
                parts.push(hard());
                if self.blank_line_between(stmts[i - 1].span.end, s.span.start) {
                    parts.push(hard());
                }
            }
            if i > 0 && starts_with_paren(s) {
                parts.push(txt(";"));
            }
            parts.push(self.stmt(s));
        }
        Doc::concat(parts)
    }

    /// A block's statements and its closing keyword at `close` (`end`, …).
    pub(crate) fn block(&mut self, b: &Block, close: &str, close_offset: u32) -> Doc {
        let first = b
            .stmts
            .first()
            .map(|s| s.span.start)
            .unwrap_or(close_offset);
        let header = self.trailing_before(first);
        let body = self.stmts(&b.stmts);
        let inner = self.inner_comments(close_offset);
        let mut parts = vec![header];
        if !b.stmts.is_empty() || inner.is_some() {
            let mut indented = Vec::new();
            if !b.stmts.is_empty() {
                indented.push(hard());
                indented.push(body);
            }
            if let Some(inner) = inner {
                indented.push(inner);
            }
            parts.push(Doc::indent(Doc::concat(indented)));
        }
        parts.push(hard());
        parts.push(self.tok(close, close_offset));
        Doc::concat(parts)
    }

    fn close_of(&self, b: &Block) -> u32 {
        self.next_token(b.span.end)
    }

    pub(crate) fn stmt(&mut self, s: &Stmt) -> Doc {
        grow(|| {
            if s.decorators.is_empty() {
                self.stmt_inner(s)
            } else {
                let d = self.decorators(&s.decorators);
                Doc::concat(vec![d, self.stmt_inner(s)])
            }
        })
    }

    fn stmt_inner(&mut self, s: &Stmt) -> Doc {
        let start = s.span.start;
        match &s.kind {
            StmtKind::Local { names, values } => {
                let mut parts = vec![self.tok("local", start), txt(" "), self.names(names)];
                if !values.is_empty() {
                    let eq =
                        self.next_token(names.last().map(|n| n.name.span.end).unwrap_or(start));
                    parts.push(txt(" "));
                    parts.push(self.tok("=", eq));
                    parts.push(self.rhs(values));
                }
                Doc::concat(parts)
            }
            StmtKind::Assign { targets, values } => {
                let lhs = self.expr_list_flat(targets);
                let eq = self.next_token(targets.last().map(|t| t.span.end).unwrap_or(start));
                Doc::concat(vec![lhs, txt(" "), self.tok("=", eq), self.rhs(values)])
            }
            StmtKind::Call(e) => self.expr(e),
            StmtKind::Do(b) => {
                let d = self.tok("do", start);
                let close = self.close_of(b);
                Doc::concat(vec![d, self.block(b, "end", close)])
            }
            StmtKind::While { cond, body } => {
                let w = self.tok("while", start);
                let c = self.expr(cond);
                let d = self.tok("do", self.next_token(cond.span.end));
                let close = self.close_of(body);
                Doc::concat(vec![
                    w,
                    txt(" "),
                    c,
                    txt(" "),
                    d,
                    self.block(body, "end", close),
                ])
            }
            StmtKind::Repeat { body, cond } => {
                let r = self.tok("repeat", start);
                let close = self.close_of(body);
                let b = self.block(body, "until", close);
                Doc::concat(vec![r, b, txt(" "), self.expr(cond)])
            }
            StmtKind::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                let mut parts = vec![self.tok("if", start), txt(" "), self.expr(cond), txt(" ")];
                parts.push(self.tok("then", self.next_token(cond.span.end)));
                let mut prev = then_block;
                let mut prev_close = self.close_of(then_block);
                let mut clauses: Vec<(&Expr, &Block)> =
                    elseifs.iter().map(|(c, b)| (c, b)).collect();
                clauses.reverse();
                while let Some((c, b)) = clauses.pop() {
                    parts.push(self.block(prev, "elseif", prev_close));
                    parts.push(txt(" "));
                    parts.push(self.expr(c));
                    parts.push(txt(" "));
                    parts.push(self.tok("then", self.next_token(c.span.end)));
                    prev = b;
                    prev_close = self.close_of(b);
                }
                if let Some(e) = else_block {
                    parts.push(self.block(prev, "else", prev_close));
                    let close = self.close_of(e);
                    parts.push(self.block(e, "end", close));
                } else {
                    parts.push(self.block(prev, "end", prev_close));
                }
                Doc::concat(parts)
            }
            StmtKind::NumericFor {
                var,
                start: first,
                limit,
                step,
                body,
            } => {
                let mut parts = vec![
                    self.tok("for", start),
                    txt(" "),
                    self.typed_name(var),
                    txt(" = "),
                ];
                parts.push(self.expr(first));
                parts.push(txt(", "));
                parts.push(self.expr(limit));
                if let Some(step) = step {
                    parts.push(txt(", "));
                    parts.push(self.expr(step));
                }
                let last = step.as_ref().unwrap_or(limit);
                parts.push(txt(" "));
                parts.push(self.tok("do", self.next_token(last.span.end)));
                let close = self.close_of(body);
                parts.push(self.block(body, "end", close));
                Doc::concat(parts)
            }
            StmtKind::GenericFor { vars, exprs, body } => {
                let mut parts = vec![
                    self.tok("for", start),
                    txt(" "),
                    self.names(vars),
                    txt(" in "),
                ];
                parts.push(self.expr_list_flat(exprs));
                parts.push(txt(" "));
                let last = exprs.last().map(|e| e.span.end).unwrap_or(start);
                parts.push(self.tok("do", self.next_token(last)));
                let close = self.close_of(body);
                parts.push(self.block(body, "end", close));
                Doc::concat(parts)
            }
            StmtKind::Function { name, func } => {
                let f = self.tok("function", start);
                let n = self.func_name(name);
                Doc::concat(vec![f, txt(" "), n, self.func_body(func)])
            }
            StmtKind::LocalFunction { name, func } => {
                let l = self.tok("local", start);
                let f = self.tok("function", self.next_token(start + 1));
                let n = self.name(name);
                Doc::concat(vec![l, txt(" "), f, txt(" "), n, self.func_body(func)])
            }
            StmtKind::Return(values) => {
                let r = self.tok("return", start);
                if values.is_empty() {
                    r
                } else if values.len() == 1 {
                    // One value: no list to indent (a returned function's
                    // body is indented once).
                    Doc::concat(vec![r, txt(" "), self.expr(&values[0])])
                } else {
                    Doc::concat(vec![r, txt(" "), self.expr_list_group(values)])
                }
            }
            StmtKind::Break => self.tok("break", start),
            _ => self.wowlua_stmt(s),
        }
    }

    pub(crate) fn name(&mut self, n: &Name) -> Doc {
        self.tok(n.text.as_bytes(), n.span.start)
    }

    fn names(&mut self, names: &[TypedName]) -> Doc {
        let mut parts = Vec::new();
        for (i, n) in names.iter().enumerate() {
            if i > 0 {
                parts.push(txt(", "));
            }
            parts.push(self.typed_name(n));
        }
        Doc::concat(parts)
    }

    pub(crate) fn func_name(&mut self, name: &FuncName) -> Doc {
        let mut parts = Vec::new();
        for (i, n) in name.path.iter().enumerate() {
            if i > 0 {
                parts.push(txt("."));
            }
            parts.push(self.name(n));
        }
        if let Some(m) = &name.method {
            parts.push(txt(":"));
            parts.push(self.name(m));
        }
        Doc::concat(parts)
    }

    pub(crate) fn func_body(&mut self, f: &FuncBody) -> Doc {
        let signature = self.signature(f);
        let first = f
            .body
            .stmts
            .first()
            .map(|s| s.span.start)
            .unwrap_or(f.end_span.start);
        let header = self.trailing_before(first);
        let body = self.stmts(&f.body.stmts);
        let inner = self.inner_comments(f.end_span.start);
        let mut parts = vec![signature, header];
        if f.body.stmts.is_empty() && inner.is_none() {
            parts.push(txt(" "));
        } else {
            let mut indented = Vec::new();
            if !f.body.stmts.is_empty() {
                indented.push(hard());
                indented.push(body);
            }
            if let Some(inner) = inner {
                indented.push(inner);
            }
            parts.push(Doc::indent(Doc::concat(indented)));
            parts.push(hard());
        }
        parts.push(self.tok("end", f.end_span.start));
        Doc::concat(parts)
    }

    /// ` = values`: one value stays on the line; several break as a group.
    fn rhs(&mut self, values: &[Expr]) -> Doc {
        if values.len() == 1 {
            Doc::concat(vec![txt(" "), self.expr(&values[0])])
        } else {
            Doc::concat(vec![txt(" "), self.expr_list_group(values)])
        }
    }

    fn expr_list_flat(&mut self, es: &[Expr]) -> Doc {
        let mut parts = Vec::new();
        for (i, e) in es.iter().enumerate() {
            if i > 0 {
                parts.push(txt(", "));
            }
            parts.push(self.expr(e));
        }
        Doc::concat(parts)
    }

    fn expr_list_group(&mut self, es: &[Expr]) -> Doc {
        let mut parts = Vec::new();
        for (i, e) in es.iter().enumerate() {
            if i > 0 {
                parts.push(txt(","));
                parts.push(self.trailing_before(e.span.start));
                parts.push(Doc::Line);
            }
            parts.push(self.expr(e));
        }
        Doc::group(Doc::indent(Doc::concat(parts)))
    }

    // ------------------------------------------------------- expressions --

    pub(crate) fn expr(&mut self, e: &Expr) -> Doc {
        grow(|| self.expr_inner(e))
    }

    fn expr_inner(&mut self, e: &Expr) -> Doc {
        let start = e.span.start;
        match &e.kind {
            ExprKind::Nil => self.tok("nil", start),
            ExprKind::True => self.tok("true", start),
            ExprKind::False => self.tok("false", start),
            ExprKind::Number(n) => self.tok(n.raw.as_bytes(), start),
            ExprKind::String(s) => {
                let text = self.string(s);
                self.tok(&text, start)
            }
            ExprKind::Vararg => self.tok("...", start),
            ExprKind::Function(f) => {
                let k = self.tok("function", start);
                Doc::concat(vec![k, self.func_body(f)])
            }
            ExprKind::Table(fields) => self.table(fields, e.span),
            ExprKind::Name(n) => self.name(n),
            ExprKind::Index { obj, key } => {
                let o = self.expr(obj);
                let open = self.tok("[", self.next_token(obj.span.end));
                let k = self.expr(key);
                let close = self.tok("]", self.next_token(key.span.end));
                // `t[ [[x]] ]`: `[[` would open a long string.
                let pad = if is_long_string(key) {
                    txt(" ")
                } else {
                    txt("")
                };
                Doc::concat(vec![o, open, pad.clone(), k, pad, close])
            }
            ExprKind::Field { obj, name } => {
                let o = self.expr(obj);
                let dot = self.tok(".", self.next_token(obj.span.end));
                Doc::concat(vec![o, dot, self.name(name)])
            }
            ExprKind::Call { callee, args } => {
                let c = self.expr(callee);
                Doc::concat(vec![c, self.args(args)])
            }
            ExprKind::Method { obj, name, args } => {
                let o = self.expr(obj);
                let colon = self.tok(":", self.next_token(obj.span.end));
                let n = self.name(name);
                Doc::concat(vec![o, colon, n, self.args(args)])
            }
            ExprKind::Paren(inner) => {
                let open = self.tok("(", start);
                let i = self.expr(inner);
                let close = self.tok(")", e.span.end.saturating_sub(1));
                Doc::concat(vec![open, i, close])
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let l = self.expr(lhs);
                let o = self.tok(op.as_str(), self.next_token(lhs.span.end));
                let after = self.trailing_before(rhs.span.start);
                let r = self.expr(rhs);
                Doc::group(Doc::concat(vec![
                    l,
                    txt(" "),
                    o,
                    after,
                    Doc::indent(Doc::concat(vec![Doc::Line, r])),
                ]))
            }
            ExprKind::Unary { op, operand } => {
                let o = self.tok(op.as_str(), start);
                let inner = self.expr(operand);
                let space = match op {
                    UnOp::Not => txt(" "),
                    // `- -x` must not become a comment.
                    UnOp::Neg
                        if matches!(&operand.kind, ExprKind::Unary { op: UnOp::Neg, .. })
                            || matches!(&operand.kind, ExprKind::Number(n) if n.raw.starts_with('-')) =>
                    {
                        txt(" ")
                    }
                    _ => txt(""),
                };
                Doc::concat(vec![o, space, inner])
            }
            _ => self.wowlua_expr(e),
        }
    }

    /// Call arguments, always parenthesized. A single table or function
    /// argument, or a function as the last argument, hugs the parentheses.
    pub(crate) fn args(&mut self, args: &Args) -> Doc {
        match args {
            Args::Parens { args, span } => {
                let open = self.tok("(", span.start);
                let close_offset = span.end.saturating_sub(1);
                if args.is_empty() {
                    let inner = self.inner_comments(close_offset);
                    let close = self.tok(")", close_offset);
                    return match inner {
                        Some(inner) => Doc::concat(vec![open, Doc::indent(inner), hard(), close]),
                        None => Doc::concat(vec![open, close]),
                    };
                }
                let last_hugs = args
                    .last()
                    .is_some_and(|a| matches!(a.kind, ExprKind::Function(_) | ExprKind::Table(_)));
                if last_hugs {
                    let mut parts = vec![open];
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            parts.push(txt(", "));
                        }
                        parts.push(self.expr(a));
                    }
                    parts.push(self.tok(")", close_offset));
                    return Doc::concat(parts);
                }
                let mut parts = vec![self.trailing_before(args[0].span.start)];
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        parts.push(txt(","));
                        parts.push(self.trailing_before(a.span.start));
                        parts.push(Doc::Line);
                    }
                    parts.push(self.expr(a));
                }
                let inner = self.inner_comments(close_offset);
                let close = self.tok(")", close_offset);
                let head = parts.remove(0);
                Doc::group(Doc::concat(vec![
                    open,
                    head,
                    Doc::indent(Doc::concat(vec![
                        Doc::SoftLine,
                        Doc::concat(parts),
                        inner.unwrap_or_else(|| txt("")),
                    ])),
                    Doc::SoftLine,
                    close,
                ]))
            }
            Args::Table(e) | Args::String(e) => Doc::concat(vec![txt("("), self.expr(e), txt(")")]),
        }
    }

    fn table(&mut self, fields: &[TableField], span: wowlua_span::Span) -> Doc {
        let open = self.tok("{", span.start);
        let close_offset = span.end.saturating_sub(1);
        if fields.is_empty() {
            let inner = self.inner_comments(close_offset);
            let close = self.tok("}", close_offset);
            return match inner {
                Some(inner) => Doc::concat(vec![open, Doc::indent(inner), hard(), close]),
                None => Doc::concat(vec![open, close]),
            };
        }
        let head = self.trailing_before(field_start(&fields[0]));
        let mut parts = Vec::new();
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                parts.push(txt(","));
                parts.push(self.trailing_before(field_start(f)));
                parts.push(Doc::Line);
            }
            let doc = match f {
                TableField::Keyed { key, value, span } => {
                    let open_b = self.tok("[", span.start);
                    let k = self.expr(key);
                    let close_b = self.tok("]", self.next_token(key.span.end));
                    let eq = self.tok("=", self.next_token(self.next_token(key.span.end) + 1));
                    let pad = if is_long_string(key) {
                        txt(" ")
                    } else {
                        txt("")
                    };
                    Doc::concat(vec![
                        open_b,
                        pad.clone(),
                        k,
                        pad,
                        close_b,
                        txt(" "),
                        eq,
                        txt(" "),
                        self.expr(value),
                    ])
                }
                TableField::Named { name, value, .. } => {
                    let n = self.name(name);
                    let eq = self.tok("=", self.next_token(name.span.end));
                    Doc::concat(vec![n, txt(" "), eq, txt(" "), self.expr(value)])
                }
                TableField::Positional(value) => self.expr(value),
            };
            parts.push(doc);
        }
        let inner = self.inner_comments(close_offset);
        let close = self.tok("}", close_offset);
        Doc::group(Doc::concat(vec![
            open,
            head,
            Doc::indent(Doc::concat(vec![
                Doc::Line,
                Doc::concat(parts),
                Doc::if_break(txt(","), txt("")),
                inner.unwrap_or_else(|| txt("")),
            ])),
            Doc::Line,
            close,
        ]))
    }

    /// A string literal in the configured quote style, when that needs no
    /// new escapes.
    pub(crate) fn string(&self, s: &StringLit) -> Vec<u8> {
        let raw = &s.raw;
        if s.long || self.quote == QuoteStyle::Preserve || raw.len() < 2 {
            return raw.to_vec();
        }
        let (want, other) = match self.quote {
            QuoteStyle::Double => (b'"', b'\''),
            _ => (b'\'', b'"'),
        };
        if raw[0] != other {
            return raw.to_vec();
        }
        let inner = &raw[1..raw.len() - 1];
        let mut out = Vec::with_capacity(raw.len());
        out.push(want);
        let mut i = 0;
        while i < inner.len() {
            let c = inner[i];
            if c == b'\\' {
                match inner.get(i + 1) {
                    // `\'` in `'…'` becomes a plain quote in `"…"`.
                    Some(&n) if n == other => out.push(n),
                    Some(&n) => {
                        out.push(b'\\');
                        out.push(n);
                    }
                    None => out.push(b'\\'),
                }
                i += 2;
            } else if c == want {
                return raw.to_vec(); // would need an escape: keep the original quotes
            } else {
                out.push(c);
                i += 1;
            }
        }
        out.push(want);
        out
    }
}

fn is_long_string(e: &Expr) -> bool {
    matches!(&e.kind, ExprKind::String(s) if s.long)
}

fn spaced(text: &[u8]) -> Vec<u8> {
    [b" ".as_slice(), text].concat()
}

fn field_start(f: &TableField) -> u32 {
    match f {
        TableField::Keyed { span, .. } => span.start,
        TableField::Named { name, .. } => name.span.start,
        TableField::Positional(e) => e.span.start,
    }
}

/// The statement's first token is `(` (needs `;` before it).
fn starts_with_paren(s: &Stmt) -> bool {
    fn leftmost(e: &Expr) -> &Expr {
        match &e.kind {
            ExprKind::Index { obj, .. }
            | ExprKind::Field { obj, .. }
            | ExprKind::Method { obj, .. } => leftmost(obj),
            ExprKind::Call { callee, .. } => leftmost(callee),
            _ => e,
        }
    }
    let first = match &s.kind {
        StmtKind::Call(e) => e,
        StmtKind::Assign { targets, .. } => match targets.first() {
            Some(t) => t,
            None => return false,
        },
        _ => return false,
    };
    matches!(leftmost(first).kind, ExprKind::Paren(_))
}
