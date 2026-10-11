//! Lowering: wowlua syntax tree → Lua 5.1 IR (spec compiler §13.3.1, stage
//! 14). Milestone M1 lowers every Lua 5.1 construct and erases type-only
//! syntax (annotations, casts, `type`, `interface`, `declare`, `import type`);
//! other wowlua syntax reports the milestone that implements it.

use std::collections::BTreeMap;
use wowlua_diagnostics::Diagnostic;
use wowlua_luair::ir;
use wowlua_luair::ir::Pos;
use wowlua_span::Span;
use wowlua_syntax::Parse;
use wowlua_syntax::ast::*;
use wowlua_syntax::features::{Feature, features};
use wowlua_syntax::token::TriviaKind;

/// Lowers a parsed file without errors. `src` is the file's bytes.
pub fn lower(parse: &Parse, src: &[u8]) -> Result<ir::Chunk, Vec<Diagnostic>> {
    let mut unsupported: BTreeMap<Feature, (Span, usize)> = BTreeMap::new();
    for (feature, span) in features(&parse.chunk) {
        if !feature.is_type_only() {
            unsupported.entry(feature).or_insert((span, 0)).1 += 1;
        }
    }
    if !unsupported.is_empty() {
        let mut first: Vec<(Feature, Span, usize)> = unsupported
            .into_iter()
            .map(|(f, (span, n))| (f, span, n))
            .collect();
        first.sort_by_key(|(_, span, _)| span.start);
        return Err(first
            .into_iter()
            .map(|(feature, span, count)| {
                let more = if count > 1 {
                    format!(" (used {count} times in this file)")
                } else {
                    String::new()
                };
                Diagnostic::plain_error(format!(
                    "not available yet: {} (arrives with milestone {}){more}",
                    feature.describe(),
                    feature.milestone()
                ))
                .with_label(span, "needs a newer wowlua compiler")
                .with_note("this compiler version builds plain Lua 5.1 and erases type annotations")
            })
            .collect());
    }
    let lowerer = Lowerer { parse, src };
    Ok(ir::Chunk {
        block: lowerer.chunk_block(&parse.chunk.block),
        comments: lowerer.comments(),
    })
}

fn grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, f)
}

struct Lowerer<'a> {
    parse: &'a Parse,
    src: &'a [u8],
}

const LICENSE_WORDS: &[&str] = &["license", "copyright", "spdx-license-identifier"];

impl<'a> Lowerer<'a> {
    fn pos(&self, offset: u32) -> Pos {
        let lexed = &self.parse.lexed;
        Pos::new(lexed.line_of(offset), lexed.column_of(offset))
    }

    fn span_pos(&self, span: Span) -> Pos {
        self.pos(span.start)
    }

    /// Position of the first token at or after `offset` (a block's closing
    /// keyword).
    fn next_token_pos(&self, offset: u32) -> Pos {
        let tokens = &self.parse.lexed.tokens;
        let i = tokens.partition_point(|t| t.start < offset);
        match tokens.get(i) {
            Some(t) => Pos::new(t.line, self.parse.lexed.column_of(t.start)),
            None => Pos::NONE,
        }
    }

    /// The source comments; the leading block is a license header when it
    /// mentions a license (spec codegen §14.8.6).
    fn comments(&self) -> Vec<ir::Comment> {
        let lexed = &self.parse.lexed;
        let first = lexed.tokens.first().map(|t| t.trivia_end).unwrap_or(0) as usize;
        let mut out = Vec::new();
        let mut leading_license = false;
        for (i, trivia) in lexed.trivia.iter().enumerate() {
            let block = match trivia.kind {
                TriviaKind::LineComment => false,
                TriviaKind::BlockComment => true,
                _ => continue,
            };
            let text = self.src[trivia.start as usize..trivia.end as usize].to_vec();
            let lower = String::from_utf8_lossy(&text).to_ascii_lowercase();
            if i < first && LICENSE_WORDS.iter().any(|w| lower.contains(w)) {
                leading_license = true;
            }
            out.push((
                i < first,
                ir::Comment {
                    pos: Pos::new(trivia.line, lexed.column_of(trivia.start)),
                    license: text.starts_with(b"--[[!"),
                    text,
                    block,
                },
            ));
        }
        out.into_iter()
            .map(|(leading, mut c)| {
                c.license |= leading && leading_license;
                c
            })
            .collect()
    }

    fn chunk_block(&self, b: &Block) -> ir::Block {
        ir::Block {
            stmts: self.stmts(&b.stmts),
            end: Pos::NONE,
        }
    }

    fn block(&self, b: &Block) -> ir::Block {
        ir::Block {
            stmts: self.stmts(&b.stmts),
            end: self.next_token_pos(b.span.end),
        }
    }

    fn stmts(&self, stmts: &[Stmt]) -> Vec<ir::Stmt> {
        stmts.iter().filter_map(|s| self.stmt(s)).collect()
    }

    fn ident(&self, n: &Name) -> ir::Ident {
        ir::Ident::new(n.text.to_string(), self.span_pos(n.span))
    }

    fn stmt(&self, s: &Stmt) -> Option<ir::Stmt> {
        grow(|| self.stmt_inner(s))
    }

    fn stmt_inner(&self, s: &Stmt) -> Option<ir::Stmt> {
        use ir::StmtKind as K;
        let kind = match &s.kind {
            StmtKind::Assign { targets, values } => K::Assign {
                targets: self.exprs(targets),
                values: self.exprs(values),
            },
            StmtKind::Call(e) => K::Call(self.expr(e)),
            StmtKind::Do(b) => K::Do(self.block(b)),
            StmtKind::While { cond, body } => K::While {
                cond: self.expr(cond),
                do_pos: self.next_token_pos(cond.span.end),
                body: self.block(body),
            },
            StmtKind::Repeat { body, cond } => K::Repeat {
                body: self.block(body),
                cond: self.expr(cond),
            },
            StmtKind::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                let clause = |c: &Expr, b: &Block| ir::IfClause {
                    cond: self.expr(c),
                    then_pos: self.next_token_pos(c.span.end),
                    body: self.block(b),
                };
                let mut clauses = vec![clause(cond, then_block)];
                for (c, b) in elseifs {
                    clauses.push(clause(c, b));
                }
                K::If {
                    clauses,
                    else_block: else_block.as_ref().map(|b| self.block(b)),
                }
            }
            StmtKind::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => K::NumericFor {
                var: self.ident(&var.name),
                start: self.expr(start),
                limit: self.expr(limit),
                step: step.as_ref().map(|e| self.expr(e)),
                do_pos: self.next_token_pos(step.as_ref().unwrap_or(limit).span.end),
                body: self.block(body),
            },
            StmtKind::GenericFor { vars, exprs, body } => K::GenericFor {
                vars: vars.iter().map(|v| self.ident(&v.name)).collect(),
                exprs: self.exprs(exprs),
                do_pos: exprs
                    .last()
                    .map(|e| self.next_token_pos(e.span.end))
                    .unwrap_or(Pos::NONE),
                body: self.block(body),
            },
            StmtKind::Function { name, func } => K::Function {
                path: name.path.iter().map(|n| self.ident(n)).collect(),
                method: name.method.as_ref().map(|n| self.ident(n)),
                func: self.func(func),
            },
            StmtKind::LocalFunction { name, func } => K::LocalFunction {
                name: self.ident(name),
                func: self.func(func),
            },
            StmtKind::Local { names, values } => K::Local {
                names: names.iter().map(|n| self.ident(&n.name)).collect(),
                values: self.exprs(values),
            },
            StmtKind::Return(values) => K::Return(self.exprs(values)),
            StmtKind::Break => K::Break,
            // Type-only declarations are erased.
            StmtKind::TypeAlias(_)
            | StmtKind::Interface(_)
            | StmtKind::Declare(_)
            | StmtKind::Import(_)
            | StmtKind::Export(_) => return None,
            // Rejected before lowering (`features`) or by the parser.
            StmtKind::Compound { .. }
            | StmtKind::LocalDestructure { .. }
            | StmtKind::Const { .. }
            | StmtKind::Continue
            | StmtKind::Class(_)
            | StmtKind::Enum(_)
            | StmtKind::Error => return None,
        };
        Some(ir::Stmt {
            pos: self.span_pos(s.span),
            kind,
        })
    }

    fn func(&self, f: &FuncBody) -> ir::Func {
        ir::Func {
            pos: self.span_pos(f.params_span),
            params: f.params.iter().map(|p| self.ident(&p.name)).collect(),
            vararg: f.vararg.is_some(),
            body: ir::Block {
                stmts: self.stmts(&f.body.stmts),
                end: self.span_pos(f.end_span),
            },
        }
    }

    /// A list of expressions, with the positions of the separators.
    fn exprs(&self, es: &[Expr]) -> Vec<ir::Expr> {
        let mut out: Vec<ir::Expr> = es.iter().map(|e| self.expr(e)).collect();
        for (i, e) in es.iter().enumerate().take(es.len().saturating_sub(1)) {
            out[i].sep = self.next_token_pos(e.span.end);
        }
        out
    }

    fn close_pos(&self, span: Span) -> Pos {
        self.pos(span.end.saturating_sub(1))
    }

    fn args(&self, a: &Args) -> ir::Args {
        match a {
            Args::Parens { args, span } => ir::Args::Parens {
                args: self.exprs(args),
                close: self.close_pos(*span),
            },
            Args::Table(e) => ir::Args::Table(Box::new(self.expr(e))),
            Args::String(e) => ir::Args::String(Box::new(self.expr(e))),
        }
    }

    fn expr(&self, e: &Expr) -> ir::Expr {
        grow(|| self.expr_inner(e))
    }

    fn expr_inner(&self, e: &Expr) -> ir::Expr {
        use ir::ExprKind as K;
        let pos = self.span_pos(e.span);
        let kind = match &e.kind {
            ExprKind::Nil => K::Nil,
            ExprKind::True => K::True,
            ExprKind::False => K::False,
            ExprKind::Number(n) => K::Number(n.raw.to_string()),
            ExprKind::String(s) => K::String(s.raw.to_vec()),
            ExprKind::Vararg => K::Vararg,
            ExprKind::Function(f) => K::Function(Box::new(self.func(f))),
            ExprKind::Table(fields) => {
                let mut lowered: Vec<ir::Field> = fields
                    .iter()
                    .map(|f| match f {
                        TableField::Keyed { key, value, span } => ir::Field::Keyed {
                            open: self.span_pos(*span),
                            key: self.expr(key),
                            value: self.expr(value),
                        },
                        TableField::Named { name, value, .. } => ir::Field::Named {
                            name: self.ident(name),
                            value: self.expr(value),
                        },
                        TableField::Positional(v) => ir::Field::Positional(self.expr(v)),
                    })
                    .collect();
                for (i, f) in fields
                    .iter()
                    .enumerate()
                    .take(fields.len().saturating_sub(1))
                {
                    let end = match f {
                        TableField::Keyed { value, .. }
                        | TableField::Named { value, .. }
                        | TableField::Positional(value) => value.span.end,
                    };
                    let sep = self.next_token_pos(end);
                    match &mut lowered[i] {
                        ir::Field::Keyed { value, .. }
                        | ir::Field::Named { value, .. }
                        | ir::Field::Positional(value) => value.sep = sep,
                    }
                }
                K::Table {
                    fields: lowered,
                    close: self.close_pos(e.span),
                }
            }
            ExprKind::Name(n) => K::Name(self.ident(n)),
            ExprKind::Index { obj, key } => K::Index {
                obj: Box::new(self.expr(obj)),
                key: Box::new(self.expr(key)),
            },
            ExprKind::Field { obj, name } => K::Field {
                obj: Box::new(self.expr(obj)),
                name: self.ident(name),
            },
            ExprKind::Call { callee, args } => K::Call {
                callee: Box::new(self.expr(callee)),
                args: self.args(args),
            },
            ExprKind::Method { obj, name, args } => K::Method {
                obj: Box::new(self.expr(obj)),
                name: self.ident(name),
                args: self.args(args),
            },
            ExprKind::Paren(inner) => K::Paren {
                inner: Box::new(self.expr(inner)),
                close: self.close_pos(e.span),
            },
            ExprKind::Binary { op, lhs, rhs } => K::Binary {
                op: lua_binop(*op),
                op_pos: self.next_token_pos(lhs.span.end),
                lhs: Box::new(self.expr(lhs)),
                rhs: Box::new(self.expr(rhs)),
            },
            ExprKind::Unary { op, operand } => K::Unary {
                op: match op {
                    UnOp::Neg => ir::UnOp::Neg,
                    UnOp::Not => ir::UnOp::Not,
                    _ => ir::UnOp::Len,
                },
                operand: Box::new(self.expr(operand)),
            },
            // A cast has no runtime effect.
            ExprKind::Cast { expr, .. } => return self.expr(expr),
            // Rejected before lowering.
            ExprKind::Interp(_)
            | ExprKind::OptField { .. }
            | ExprKind::OptIndex { .. }
            | ExprKind::OptMethod { .. }
            | ExprKind::OptCall { .. }
            | ExprKind::Error => K::Nil,
        };
        ir::Expr::new(kind, pos)
    }
}

fn lua_binop(op: BinOp) -> ir::BinOp {
    match op {
        BinOp::Add => ir::BinOp::Add,
        BinOp::Sub => ir::BinOp::Sub,
        BinOp::Mul => ir::BinOp::Mul,
        BinOp::Div => ir::BinOp::Div,
        BinOp::Mod => ir::BinOp::Mod,
        BinOp::Pow => ir::BinOp::Pow,
        BinOp::Concat => ir::BinOp::Concat,
        BinOp::Eq => ir::BinOp::Eq,
        BinOp::Ne => ir::BinOp::Ne,
        BinOp::Lt => ir::BinOp::Lt,
        BinOp::Le => ir::BinOp::Le,
        BinOp::Gt => ir::BinOp::Gt,
        BinOp::Ge => ir::BinOp::Ge,
        BinOp::And => ir::BinOp::And,
        // `??` and the bitwise operators were rejected before lowering.
        _ => ir::BinOp::Or,
    }
}
