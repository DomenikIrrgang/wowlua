//! Reflection metadata (spec §7.8): `@reflect` classes and functions (every
//! class with `reflection = "all"`) register what the source declares in the
//! runtime's weak-keyed registry; enums register their declaration order
//! when the package uses `wowlua:reflect`.

use super::*;

impl Lowerer<'_> {
    /// `{ name = value, … }`.
    fn record(&self, fields: Vec<(&str, ir::Expr)>, pos: Pos) -> ir::Expr {
        lit(
            ir::ExprKind::Table {
                fields: fields
                    .into_iter()
                    .map(|(n, value)| ir::Field::Named {
                        name: ident(n, pos),
                        value,
                    })
                    .collect(),
                close: Pos::NONE,
            },
            pos,
        )
    }

    fn list(&self, items: Vec<ir::Expr>, pos: Pos) -> ir::Expr {
        lit(
            ir::ExprKind::Table {
                fields: items.into_iter().map(ir::Field::Positional).collect(),
                close: Pos::NONE,
            },
            pos,
        )
    }

    fn boolean(&self, b: bool, pos: Pos) -> ir::Expr {
        lit(
            if b {
                ir::ExprKind::True
            } else {
                ir::ExprKind::False
            },
            pos,
        )
    }

    /// A type annotation as written, on one line (`"string[]"`).
    fn type_text(&self, t: Option<&Type>) -> String {
        match t {
            Some(t) => {
                let text =
                    String::from_utf8_lossy(&self.src[t.span.start as usize..t.span.end as usize]);
                text.split_whitespace().collect::<Vec<_>>().join(" ")
            }
            None => "any".into(),
        }
    }

    /// Decorator infos: name and constant arguments (`"<expr>"` otherwise).
    fn decorator_infos(&self, ds: &[Decorator], pos: Pos) -> ir::Expr {
        let items = ds
            .iter()
            .map(|d| {
                let name = d
                    .path
                    .iter()
                    .map(|n| n.text.as_ref())
                    .collect::<Vec<_>>()
                    .join(".");
                let args = d
                    .args
                    .as_ref()
                    .map(|a| {
                        a.exprs()
                            .into_iter()
                            .map(|e| match &e.kind {
                                ExprKind::String(s) => {
                                    self.string(&String::from_utf8_lossy(&s.value), pos)
                                }
                                ExprKind::Number(n) => {
                                    lit(ir::ExprKind::Number(n.raw.to_string()), pos)
                                }
                                ExprKind::True => self.boolean(true, pos),
                                ExprKind::False => self.boolean(false, pos),
                                _ => self.string("<expr>", pos),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.record(
                    vec![
                        ("name", self.string(&name, pos)),
                        ("args", self.list(args, pos)),
                    ],
                    pos,
                )
            })
            .collect();
        self.list(items, pos)
    }

    fn visibility(&self, m: &ClassMember) -> &'static str {
        if m.modifiers.contains(&Modifier::Private) {
            "private"
        } else if m.modifiers.contains(&Modifier::Protected) {
            "protected"
        } else {
            "public"
        }
    }

    fn params_info(&self, f: &FuncBody, pos: Pos) -> ir::Expr {
        let items = f
            .params
            .iter()
            .map(|p| {
                let optional = p.default.is_some()
                    || matches!(p.ty.as_ref().map(|t| &t.kind), Some(TypeKind::Optional(_)));
                self.record(
                    vec![
                        ("name", self.string(&p.name.text, pos)),
                        ("type", self.string(&self.type_text(p.ty.as_ref()), pos)),
                        ("optional", self.boolean(optional, pos)),
                    ],
                    pos,
                )
            })
            .collect();
        self.list(items, pos)
    }

    fn returns_text(&self, f: &FuncBody) -> String {
        match &f.ret {
            None => "()".into(),
            Some(RetType::Single(t)) => self.type_text(Some(t)),
            Some(r) => {
                let span = match r {
                    RetType::List { span, .. } => *span,
                    RetType::Pack(n) => n.span,
                    RetType::Predicate { name, ty } => Span {
                        file: name.span.file,
                        start: name.span.start,
                        end: ty.span.end,
                    },
                    RetType::Single(t) => t.span,
                };
                let text =
                    String::from_utf8_lossy(&self.src[span.start as usize..span.end as usize]);
                text.split_whitespace().collect::<Vec<_>>().join(" ")
            }
        }
    }

    /// A signature as written: `(a: number, b: string?): boolean`.
    fn signature_text(&self, f: &FuncBody) -> String {
        let end = match &f.ret {
            None => f.params_span.end,
            Some(RetType::Single(t)) | Some(RetType::Predicate { ty: t, .. }) => t.span.end,
            Some(RetType::List { span, .. }) => span.end,
            Some(RetType::Pack(n)) => n.span.end,
        };
        let text = String::from_utf8_lossy(&self.src[f.params_span.start as usize..end as usize]);
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// `__wl.reflectClass(Cls, { fields = …, methods = …, … })`.
    pub(crate) fn class_meta(
        &mut self,
        c: &ClassDecl,
        class_decorators: &[&Decorator],
        cls: ir::Expr,
        pos: Pos,
    ) -> ir::Stmt {
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut props: Vec<(String, ir::Expr)> = Vec::new();
        for m in &c.members {
            let is_static = m.modifiers.contains(&Modifier::Static);
            let vis = self.string(self.visibility(m), pos);
            let decorators = self.decorator_infos(&m.decorators, pos);
            match &m.kind {
                ClassMemberKind::Field {
                    name,
                    optional,
                    ty,
                    value,
                } => fields.push(self.record(
                    vec![
                        ("name", self.string(&name.text, pos)),
                        ("type", self.string(&self.type_text(Some(ty)), pos)),
                        ("visibility", vis),
                        (
                            "optional",
                            self.boolean(
                                *optional || matches!(ty.kind, TypeKind::Optional(_)),
                                pos,
                            ),
                        ),
                        ("static", self.boolean(is_static, pos)),
                        (
                            "readonly",
                            self.boolean(m.modifiers.contains(&Modifier::Readonly), pos),
                        ),
                        ("hasDefault", self.boolean(value.is_some(), pos)),
                        ("decorators", decorators),
                    ],
                    pos,
                )),
                ClassMemberKind::Method { name, func, .. } => {
                    if m.modifiers.contains(&Modifier::Overload) {
                        continue;
                    }
                    let overloads: Vec<ir::Expr> = c
                        .members
                        .iter()
                        .filter_map(|o| match &o.kind {
                            ClassMemberKind::Method {
                                name: on, func: of, ..
                            } if on.text == name.text
                                && o.modifiers.contains(&Modifier::Overload) =>
                            {
                                Some(self.string(&self.signature_text(of), pos))
                            }
                            _ => None,
                        })
                        .collect();
                    methods.push(self.record(
                        vec![
                            ("name", self.string(&name.text, pos)),
                            ("params", self.params_info(func, pos)),
                            ("returns", self.string(&self.returns_text(func), pos)),
                            ("visibility", vis),
                            ("static", self.boolean(is_static, pos)),
                            (
                                "abstract",
                                self.boolean(m.modifiers.contains(&Modifier::Abstract), pos),
                            ),
                            ("overloads", self.list(overloads, pos)),
                            ("decorators", decorators),
                        ],
                        pos,
                    ));
                }
                ClassMemberKind::Getter { name, ty, .. } => {
                    let t = self.type_text(ty.as_ref());
                    props.push((
                        name.text.to_string(),
                        self.record(
                            vec![
                                ("name", self.string(&name.text, pos)),
                                ("type", self.string(&t, pos)),
                                ("visibility", vis),
                                ("static", self.boolean(is_static, pos)),
                                ("hasGetter", self.boolean(true, pos)),
                                ("hasSetter", self.boolean(has_setter(c, &name.text), pos)),
                                ("decorators", decorators),
                            ],
                            pos,
                        ),
                    ));
                }
                ClassMemberKind::Setter { name, param, .. } if !has_getter(c, &name.text) => {
                    let t = self.type_text(param.ty.as_ref());
                    props.push((
                        name.text.to_string(),
                        self.record(
                            vec![
                                ("name", self.string(&name.text, pos)),
                                ("type", self.string(&t, pos)),
                                ("visibility", vis),
                                ("static", self.boolean(is_static, pos)),
                                ("hasGetter", self.boolean(false, pos)),
                                ("hasSetter", self.boolean(true, pos)),
                                ("decorators", decorators),
                            ],
                            pos,
                        ),
                    ));
                }
                _ => {}
            }
        }
        let interfaces: Vec<ir::Expr> = c
            .implements
            .iter()
            .map(|t| self.string(&self.type_text(Some(t)), pos))
            .collect();
        let own: Vec<Decorator> = class_decorators.iter().map(|d| (*d).clone()).collect();
        let meta = self.record(
            vec![
                ("name", self.string(&c.name.text, pos)),
                ("decorators", self.decorator_infos(&own, pos)),
                ("fields", self.list(fields, pos)),
                ("methods", self.list(methods, pos)),
                (
                    "properties",
                    self.list(props.into_iter().map(|(_, p)| p).collect(), pos),
                ),
                ("interfaces", self.list(interfaces, pos)),
            ],
            pos,
        );
        self.slices.insert(Slice::Reflect);
        let call_ir = self.wl_call("reflectClass", vec![cls, meta], pos);
        stmt(ir::StmtKind::Call(call_ir), pos)
    }

    /// `__wl.reflectEnum(E, { "Up", "Down", … })`: the declaration order.
    pub(crate) fn enum_order(&mut self, names: &[String], target: ir::Expr, pos: Pos) -> ir::Stmt {
        let order = self.list(names.iter().map(|n| self.string(n, pos)).collect(), pos);
        self.slices.insert(Slice::Reflect);
        let call_ir = self.wl_call("reflectEnum", vec![target, order], pos);
        stmt(ir::StmtKind::Call(call_ir), pos)
    }

    /// The metadata of a `@reflect` function: `{ name, location, params,
    /// returns }` (registered by the instrumentation pass, after wrappers).
    pub(crate) fn function_meta(&mut self, f: &FuncBody, name: &str) -> ir::Expr {
        let pos = self.span_pos(f.params_span);
        let location = format!("{}:{}", self.ctx.file, pos.line);
        self.record(
            vec![
                ("name", self.string(name, pos)),
                ("location", self.string(&location, pos)),
                ("params", self.params_info(f, pos)),
                ("returns", self.string(&self.returns_text(f), pos)),
            ],
            pos,
        )
    }
}

fn has_setter(c: &ClassDecl, name: &str) -> bool {
    c.members.iter().any(
        |m| matches!(&m.kind, ClassMemberKind::Setter { name: n, .. } if n.text.as_ref() == name),
    )
}

fn has_getter(c: &ClassDecl, name: &str) -> bool {
    c.members.iter().any(
        |m| matches!(&m.kind, ClassMemberKind::Getter { name: n, .. } if n.text.as_ref() == name),
    )
}
