//! The emitter: the only component that produces Lua text (spec compiler
//! §13.7). It handles parenthesization by Lua 5.1 priorities, statement
//! separators, line modes (spec codegen §14.5), comments and source-map
//! mappings.

use crate::ir::*;

/// How emitted statements map to lines (spec codegen §14.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineMode {
    /// Every token with a source position stays on its source line.
    Preserve,
    /// Every statement on its own line, no indentation, no blank lines.
    Statement,
    /// Indented, with the source's blank lines between statements.
    Pretty,
}

/// Which source comments are emitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentMode {
    All,
    /// Only license comments (spec codegen §14.8.6).
    License,
    None,
}

#[derive(Clone, Debug)]
pub struct EmitOptions {
    pub line_mode: LineMode,
    pub comments: CommentMode,
    pub indent: String,
}

impl Default for EmitOptions {
    fn default() -> Self {
        EmitOptions {
            line_mode: LineMode::Preserve,
            comments: CommentMode::All,
            indent: "\t".into(),
        }
    }
}

/// An output position (1-based line, 0-based byte column) and the source
/// position it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub out_line: u32,
    pub out_col: u32,
    pub src: Pos,
}

#[derive(Clone, Debug, Default)]
pub struct Emitted {
    pub text: Vec<u8>,
    pub mappings: Vec<Mapping>,
    /// Tokens that could not be placed on their source line (preserve mode).
    pub drift: u32,
}

/// Emits `chunk` as Lua 5.1 source.
pub fn emit(chunk: &Chunk, options: &EmitOptions) -> Emitted {
    let comments = chunk
        .comments
        .iter()
        .filter(|c| match options.comments {
            CommentMode::All => true,
            CommentMode::License => c.license,
            CommentMode::None => false,
        })
        .collect();
    let mut e = Emitter {
        opts: options,
        out: Vec::new(),
        line: 1,
        col: 0,
        depth: 0,
        comments,
        next_comment: 0,
        at_line_start: true,
        need_newline: false,
        pending_space: false,
        last: Last::None,
        last_src_line: 0,
        mappings: Vec::new(),
        drift: 0,
    };
    e.stmts(&chunk.block.stmts);
    e.flush_comments_before(Pos::new(u32::MAX, 0));
    if !e.out.is_empty() && !e.at_line_start {
        e.out.push(b'\n');
    }
    Emitted {
        text: e.out,
        mappings: e.mappings,
        drift: e.drift,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Last {
    None,
    /// Ends with a letter, digit or `_` (names).
    Word,
    /// A keyword: readable output puts a space before `(`, `{`, quotes, …
    Keyword,
    /// A numeral (a following `.` or letter would join it).
    Number,
    /// Punctuation; the byte is its last character.
    Symbol(u8),
}

struct Emitter<'a> {
    opts: &'a EmitOptions,
    out: Vec<u8>,
    line: u32,
    col: u32,
    depth: u32,
    comments: Vec<&'a Comment>,
    next_comment: usize,
    at_line_start: bool,
    /// A line comment was written: the next token must start a new line.
    need_newline: bool,
    pending_space: bool,
    last: Last,
    last_src_line: u32,
    mappings: Vec<Mapping>,
    drift: u32,
}

fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Counts line breaks like Lua 5.1 (`\r\n` and `\n\r` count once).
fn lua_lines(text: &[u8]) -> (u32, usize) {
    let mut lines = 0;
    let mut last_start = 0;
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        if c == b'\n' || c == b'\r' {
            if let Some(&n) = text.get(i + 1)
                && (n == b'\n' || n == b'\r')
                && n != c
            {
                i += 1;
            }
            lines += 1;
            last_start = i + 1;
        }
        i += 1;
    }
    (lines, last_start)
}

/// Runs `f` with enough stack for one more nesting level.
fn grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, f)
}

impl<'a> Emitter<'a> {
    fn preserve(&self) -> bool {
        self.opts.line_mode == LineMode::Preserve
    }

    fn newline(&mut self) {
        self.out.push(b'\n');
        self.line += 1;
        self.col = 0;
        self.at_line_start = true;
        self.need_newline = false;
        self.pending_space = false;
    }

    fn goto_line(&mut self, target: u32) {
        if self.line > target {
            self.drift += 1;
        }
        while self.line < target {
            self.newline();
        }
    }

    fn write_raw(&mut self, text: &[u8]) {
        self.out.extend_from_slice(text);
        let (lines, last_start) = lua_lines(text);
        if lines > 0 {
            self.line += lines;
            self.col = (text.len() - last_start) as u32;
        } else {
            self.col += text.len() as u32;
        }
        self.at_line_start = false;
    }

    fn indent(&mut self) {
        if self.opts.line_mode != LineMode::Statement {
            for _ in 0..self.depth {
                let indent = self.opts.indent.clone();
                self.write_raw(indent.as_bytes());
            }
            self.at_line_start = true;
        }
    }

    /// Lexing requires a space between the last token and `next`.
    fn needs_space(&self, next: &[u8]) -> bool {
        let Some(&first) = next.first() else {
            return false;
        };
        match self.last {
            Last::None => false,
            Last::Word => is_word_byte(first),
            Last::Keyword => is_word_byte(first) || b"({\"'[-#".contains(&first),
            Last::Number => is_word_byte(first) || first == b'.',
            Last::Symbol(c) => match c {
                b'-' => first == b'-',
                b'.' => first == b'.' || first.is_ascii_digit(),
                b'[' => first == b'[' || first == b'=',
                b'=' | b'<' | b'>' | b'~' => first == b'=',
                _ => false,
            },
        }
    }

    fn space(&mut self) {
        self.pending_space = true;
    }

    /// Writes one token, placing it on its source line in preserve mode.
    fn token(&mut self, text: &[u8], pos: Pos) {
        if pos.is_some() {
            self.flush_comments_before(pos);
            if self.preserve() {
                self.goto_line(pos.line);
            }
        }
        if self.need_newline {
            self.newline();
        }
        if self.at_line_start {
            if self.col == 0 {
                self.indent();
            }
        } else if self.pending_space || self.needs_space(text) {
            self.write_raw(b" ");
        }
        self.pending_space = false;
        if pos.is_some() {
            self.mappings.push(Mapping {
                out_line: self.line,
                out_col: self.col,
                src: pos,
            });
            self.last_src_line = pos.line;
        }
        self.write_raw(text);
        self.last = match text.last() {
            Some(&c) if is_word_byte(c) => Last::Word,
            Some(&c) => Last::Symbol(c),
            None => self.last,
        };
    }

    /// A keyword token.
    fn kw(&mut self, text: &str, pos: Pos) {
        self.token(text.as_bytes(), pos);
        self.last = Last::Keyword;
    }

    fn number(&mut self, text: &str, pos: Pos) {
        self.token(text.as_bytes(), pos);
        self.last = Last::Number;
    }

    fn flush_comments_before(&mut self, pos: Pos) {
        while let Some(c) = self.comments.get(self.next_comment).copied() {
            if c.pos >= pos {
                break;
            }
            self.next_comment += 1;
            self.comment(c);
        }
    }

    /// Comments on the source line where the last statement ended.
    fn flush_trailing(&mut self, before: Pos) {
        while let Some(c) = self.comments.get(self.next_comment).copied() {
            if c.pos.line != self.last_src_line || (before.is_some() && c.pos >= before) {
                break;
            }
            self.next_comment += 1;
            self.comment(c);
        }
    }

    fn comment(&mut self, c: &Comment) {
        if self.preserve() {
            self.goto_line(c.pos.line);
        }
        if self.need_newline {
            self.newline();
        }
        if self.at_line_start {
            if self.col == 0 {
                self.indent();
            }
        } else {
            self.write_raw(b" ");
        }
        self.write_raw(&c.text);
        if c.block {
            self.pending_space = true;
            self.last = Last::Symbol(b']');
        } else {
            self.need_newline = true;
            self.last = Last::None;
        }
    }

    /// Starts a new line for a statement (statement and pretty modes).
    fn line_break(&mut self, pos: Pos, prev_end_line: u32) {
        if self.preserve() {
            return;
        }
        if !self.at_line_start || self.need_newline {
            self.newline();
        }
        if self.opts.line_mode == LineMode::Pretty
            && pos.is_some()
            && prev_end_line > 0
            && pos.line > prev_end_line + 1
            && !self.out.is_empty()
        {
            self.newline();
        }
    }

    // ------------------------------------------------------------ blocks --

    fn stmts(&mut self, stmts: &[Stmt]) {
        let mut prev_end = 0;
        for (i, s) in stmts.iter().enumerate() {
            if i > 0 || !self.out.is_empty() {
                self.line_break(s.pos, prev_end);
            }
            if i > 0 && starts_with_paren(s) {
                // `a = b (f)()` would be a call; `;` ends the statement.
                self.token(b";", Pos::NONE);
            }
            self.stmt(s);
            prev_end = self.last_src_line;
            if !self.preserve() {
                let next = stmts.get(i + 1).map(|n| n.pos).unwrap_or(Pos::NONE);
                self.flush_trailing(next);
            }
        }
    }

    /// The statements of a nested block and its closing keyword.
    fn block(&mut self, b: &Block, close: &str) {
        self.depth += 1;
        self.stmts(&b.stmts);
        if !self.preserve() {
            self.flush_trailing(b.end);
        }
        self.depth -= 1;
        if !b.stmts.is_empty() || self.need_newline {
            self.line_break(b.end, 0);
        }
        self.kw(close, b.end);
    }

    fn stmt(&mut self, s: &Stmt) {
        grow(|| self.stmt_inner(s))
    }

    fn stmt_inner(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Local { names, values } => {
                self.kw("local", s.pos);
                self.idents(names);
                if !values.is_empty() {
                    self.space();
                    self.token(b"=", Pos::NONE);
                    self.space();
                    self.exprs(values);
                }
            }
            StmtKind::Assign { targets, values } => {
                self.exprs(targets);
                self.space();
                self.token(b"=", Pos::NONE);
                self.space();
                self.exprs(values);
            }
            StmtKind::Call(e) => self.expr(e),
            StmtKind::Do(b) => {
                self.kw("do", s.pos);
                self.block(b, "end");
            }
            StmtKind::While { cond, do_pos, body } => {
                self.kw("while", s.pos);
                self.expr(cond);
                self.kw("do", *do_pos);
                self.block(body, "end");
            }
            StmtKind::Repeat { body, cond } => {
                self.kw("repeat", s.pos);
                self.block(body, "until");
                self.expr(cond);
            }
            StmtKind::If {
                clauses,
                else_block,
            } => {
                for (i, clause) in clauses.iter().enumerate() {
                    if i == 0 {
                        self.kw("if", s.pos);
                    }
                    self.expr(&clause.cond);
                    self.kw("then", clause.then_pos);
                    let body = &clause.body;
                    let close = if i + 1 < clauses.len() {
                        "elseif"
                    } else if else_block.is_some() {
                        "else"
                    } else {
                        "end"
                    };
                    self.block(body, close);
                }
                if let Some(b) = else_block {
                    self.block(b, "end");
                }
            }
            StmtKind::NumericFor {
                var,
                start,
                limit,
                step,
                do_pos,
                body,
            } => {
                self.kw("for", s.pos);
                self.ident(var);
                self.space();
                self.token(b"=", Pos::NONE);
                self.space();
                self.expr(start);
                self.comma();
                self.expr(limit);
                if let Some(step) = step {
                    self.comma();
                    self.expr(step);
                }
                self.kw("do", *do_pos);
                self.block(body, "end");
            }
            StmtKind::GenericFor {
                vars,
                exprs,
                do_pos,
                body,
            } => {
                self.kw("for", s.pos);
                self.idents(vars);
                self.kw("in", Pos::NONE);
                self.exprs(exprs);
                self.kw("do", *do_pos);
                self.block(body, "end");
            }
            StmtKind::Function { path, method, func } => {
                self.token(b"function", s.pos);
                for (i, part) in path.iter().enumerate() {
                    if i > 0 {
                        self.token(b".", Pos::NONE);
                    }
                    self.ident(part);
                }
                if let Some(m) = method {
                    self.token(b":", Pos::NONE);
                    self.ident(m);
                }
                self.func(func);
            }
            StmtKind::LocalFunction { name, func } => {
                self.kw("local", s.pos);
                self.token(b"function", Pos::NONE);
                self.ident(name);
                self.func(func);
            }
            StmtKind::Return(values) => {
                self.kw("return", s.pos);
                self.exprs(values);
            }
            StmtKind::Break => self.kw("break", s.pos),
        }
    }

    fn func(&mut self, f: &Func) {
        self.token(b"(", f.pos);
        for (i, p) in f.params.iter().enumerate() {
            if i > 0 {
                self.comma();
            }
            self.ident(p);
        }
        if f.vararg {
            if !f.params.is_empty() {
                self.comma();
            }
            self.token(b"...", Pos::NONE);
        }
        self.token(b")", Pos::NONE);
        self.block(&f.body, "end");
    }

    fn comma(&mut self) {
        self.comma_at(Pos::NONE);
    }

    fn comma_at(&mut self, pos: Pos) {
        self.token(b",", pos);
        self.space();
    }

    fn ident(&mut self, id: &Ident) {
        self.token(id.name.as_bytes(), id.pos);
    }

    fn idents(&mut self, ids: &[Ident]) {
        for (i, id) in ids.iter().enumerate() {
            if i > 0 {
                self.comma();
            }
            self.ident(id);
        }
    }

    fn exprs(&mut self, es: &[Expr]) {
        for (i, e) in es.iter().enumerate() {
            if i > 0 {
                self.comma_at(es[i - 1].sep);
            }
            self.expr(e);
        }
    }

    // ------------------------------------------------------- expressions --

    fn expr(&mut self, e: &Expr) {
        grow(|| self.expr_inner(e))
    }

    fn expr_inner(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Nil => self.token(b"nil", e.pos),
            ExprKind::True => self.token(b"true", e.pos),
            ExprKind::False => self.token(b"false", e.pos),
            ExprKind::Number(n) => self.number(n, e.pos),
            ExprKind::String(raw) => self.token(raw, e.pos),
            ExprKind::Vararg => self.token(b"...", e.pos),
            ExprKind::Function(f) => {
                self.token(b"function", e.pos);
                self.func(f);
            }
            ExprKind::Table { fields, close } => {
                self.token(b"{", e.pos);
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        self.comma_at(field_value(&fields[i - 1]).sep);
                    }
                    match field {
                        Field::Keyed { open, key, value } => {
                            self.token(b"[", *open);
                            self.expr(key);
                            self.token(b"]", Pos::NONE);
                            self.space();
                            self.token(b"=", Pos::NONE);
                            self.space();
                            self.expr(value);
                        }
                        Field::Named { name, value } => {
                            self.ident(name);
                            self.space();
                            self.token(b"=", Pos::NONE);
                            self.space();
                            self.expr(value);
                        }
                        Field::Positional(value) => self.expr(value),
                    }
                }
                self.token(b"}", *close);
            }
            ExprKind::Name(id) => self.ident(id),
            ExprKind::Index { obj, key } => {
                self.prefix(obj);
                self.token(b"[", Pos::NONE);
                self.expr(key);
                self.token(b"]", Pos::NONE);
            }
            ExprKind::Field { obj, name } => {
                self.prefix(obj);
                self.token(b".", Pos::NONE);
                self.ident(name);
            }
            ExprKind::Call { callee, args } => {
                self.prefix(callee);
                self.args(args);
            }
            ExprKind::Method { obj, name, args } => {
                self.prefix(obj);
                self.token(b":", Pos::NONE);
                self.ident(name);
                self.args(args);
            }
            ExprKind::Paren { inner, close } => {
                self.token(b"(", e.pos);
                self.expr(inner);
                self.token(b")", *close);
            }
            ExprKind::Binary {
                op,
                op_pos,
                lhs,
                rhs,
            } => {
                let (left, right) = op.priority();
                // `a p b q c` groups as `a p (b q c)` when q's left
                // priority exceeds p's right priority.
                let lhs_parens = match &lhs.kind {
                    ExprKind::Binary { op: p, .. } => left > p.priority().1,
                    ExprKind::Unary { .. } => left > UNARY_PRIORITY,
                    _ => false,
                };
                let rhs_parens = match &rhs.kind {
                    ExprKind::Binary { op: p, .. } => p.priority().0 <= right,
                    _ => false,
                };
                self.operand(lhs, lhs_parens);
                self.space();
                self.token(op.as_str().as_bytes(), *op_pos);
                if matches!(op, BinOp::And | BinOp::Or) {
                    self.last = Last::Keyword;
                }
                self.space();
                self.operand(rhs, rhs_parens);
            }
            ExprKind::Unary { op, operand } => {
                self.token(op.as_str().as_bytes(), e.pos);
                if *op == UnOp::Not {
                    self.last = Last::Keyword;
                }
                let parens = match &operand.kind {
                    ExprKind::Binary { op: p, .. } => p.priority().0 <= UNARY_PRIORITY,
                    _ => false,
                };
                self.operand(operand, parens);
            }
        }
    }

    fn operand(&mut self, e: &Expr, parens: bool) {
        if parens {
            self.token(b"(", Pos::NONE);
            self.expr(e);
            self.token(b")", Pos::NONE);
        } else {
            self.expr(e);
        }
    }

    /// The object of a call, index or field access must be a prefix
    /// expression; anything else gets parentheses.
    fn prefix(&mut self, e: &Expr) {
        let is_prefix = matches!(
            e.kind,
            ExprKind::Name(_)
                | ExprKind::Index { .. }
                | ExprKind::Field { .. }
                | ExprKind::Call { .. }
                | ExprKind::Method { .. }
                | ExprKind::Paren { .. }
        );
        self.operand(e, !is_prefix);
    }

    fn args(&mut self, args: &Args) {
        match args {
            Args::Parens { args, close } => {
                self.token(b"(", Pos::NONE);
                self.exprs(args);
                self.token(b")", *close);
            }
            Args::Table(e) | Args::String(e) => self.expr(e),
        }
    }
}

fn field_value(field: &Field) -> &Expr {
    match field {
        Field::Keyed { value, .. } | Field::Named { value, .. } | Field::Positional(value) => value,
    }
}

/// The statement's first token is `(`.
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
    // A parenthesized expression, or a non-prefix object, which `prefix`
    // wraps in parentheses.
    !matches!(leftmost(first).kind, ExprKind::Name(_))
}
