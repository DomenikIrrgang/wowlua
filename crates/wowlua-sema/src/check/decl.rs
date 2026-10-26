//! Checks of a class declaration as a whole (spec chapter 3): the
//! interfaces it implements, and the scripts its widget methods handle.

use super::body::Checker;
use super::env::DefKind;
use super::types::*;
use wowlua_syntax::view::*;

impl Checker<'_, '_> {
    /// `implements I`: the class's public members satisfy `I` (E3006,
    /// spec §3.1.8), each one there with a type that fits.
    pub(crate) fn check_implements(&mut self, c: ClassDecl<'_>, instance: TypeId) {
        let class = c.name().text();
        for t in c.implements() {
            let scope = self.scope.clone();
            let it = self.env.resolve(t, &scope);
            let shown = self.env.display(it);
            let (iface, args) = match self.env.types.get(it).clone() {
                Ty::Named(d, args) if self.env.def(d).kind == DefKind::Interface => (d, args),
                Ty::Error | Ty::Any => continue,
                _ => {
                    let msg =
                        format!("class `{class}` implements `{shown}`, which is not an interface");
                    self.report("E3006", t.span(), msg);
                    continue;
                }
            };
            for name in self.interface_member_names(iface) {
                let Some((m, _)) = self.env.instance_member(iface, &args, &name) else {
                    continue;
                };
                let want = match m.kind {
                    super::env::MemberKind::Method => self.env.with_self(m.ty, instance),
                    _ => m.ty,
                };
                match self.env.field_of(instance, &name) {
                    Some((have, _)) => {
                        if !self.env.assignable(have, want, self.mode) {
                            let msg = format!(
                                "class `{class}` does not satisfy `{shown}`: its `{name}` is `{}`, `{shown}` needs `{}`",
                                self.env.display(have),
                                self.env.display(want)
                            );
                            self.report("E3006", t.span(), msg);
                        }
                    }
                    None if m.optional
                        || (self.mode == super::query::Mode::Basic
                            && self.env.types.is_optional(m.ty)) => {}
                    None => {
                        let msg = format!(
                            "class `{class}` does not satisfy `{shown}`: it has no public `{name}`"
                        );
                        self.report("E3006", t.span(), msg);
                    }
                }
            }
        }
    }

    /// An interface's member names, those of the interfaces it extends
    /// included, in declaration order.
    fn interface_member_names(&self, iface: DefId) -> Vec<Box<str>> {
        let mut names: Vec<Box<str>> = Vec::new();
        let mut stack = vec![iface];
        let mut seen = rustc_hash::FxHashSet::default();
        while let Some(d) = stack.pop() {
            if !seen.insert(d) {
                continue;
            }
            for n in self.env.def(d).members.keys() {
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
            for &b in self.env.def(d).bases.iter().rev() {
                if let Ty::Named(bd, _) = self.env.types.get(b) {
                    stack.push(*bd);
                }
            }
        }
        names
    }

    /// `@script(name, opts?)` on a widget class's method (spec §3.2.1): the
    /// widget type has a script `name` (E2001), and the method, with the
    /// frame as `self`, takes the arguments that script is called with
    /// (E2001), as `SetScript` (or, with `{ hook = true }`, `HookScript`)
    /// would check it.
    pub(crate) fn check_scripts(&mut self, def: DefId, instance: TypeId, c: ClassDecl<'_>) {
        if self.env.def(def).widget.is_none() {
            return;
        }
        // The native type the chain ends in.
        let mut native = None;
        let mut d = def;
        for _ in 0..64 {
            let Some(&b) = self.env.def(d).bases.first() else {
                break;
            };
            let Ty::Named(bd, _) = self.env.types.get(b).clone() else {
                break;
            };
            if self.env.def(bd).native {
                native = Some(bd);
                break;
            }
            d = bd;
        }
        let Some(native) = native else { return };
        let widget = self.env.def(native).name.clone();
        let names = self.env.script_names(native);
        for m in c.members() {
            let ClassMemberKind::Method { name: method, .. } = m.kind() else {
                continue;
            };
            if m.modifiers().contains(&Modifier::Static) {
                continue;
            }
            for d in m.decorators().iter().filter(|d| d.is_named("script")) {
                let args = d.args().map(|a| a.exprs());
                let Some(first) = args.and_then(|a| a.first()) else {
                    continue;
                };
                let ExprKind::String(lit) = first.kind() else {
                    continue;
                };
                let script = String::from_utf8_lossy(&lit.value()).into_owned();
                let hook = args.and_then(|a| a.get(1)).is_some_and(|o| match o.kind() {
                    ExprKind::Table(fields) => fields.iter().any(|f| {
                        matches!(f, TableField::Named { name, value, .. }
                            if name.text() == "hook" && matches!(value.kind(), ExprKind::True))
                    }),
                    _ => false,
                });
                let installer = if hook { "HookScript" } else { "SetScript" };
                // The handler type of the script's own signature, if the
                // declarations give one.
                let handler = self.script_handler(native, instance, installer, &script);
                if handler.is_none() && !names.is_empty() && !names.contains(script.as_str()) {
                    let msg = format!("`{script}` is not a script of `{widget}`");
                    self.diagnostics.push(
                        wowlua_diagnostics::Diagnostic::error("E2001", msg)
                            .with_label(first.span(), format!("no script of `{widget}`"))
                            .with_help(format!(
                                "the scripts of `{widget}` are those its `{installer}` takes; check the name's spelling"
                            )),
                    );
                    continue;
                }
                let Some(handler) = handler else { continue };
                let Some(declared) = self.env.def(def).members.get(method.text()).map(|m| m.ty)
                else {
                    continue;
                };
                // (as declared: a decorator's result is not what the frame
                // calls)
                let declared = self
                    .env
                    .decorated_methods
                    .get(&(def, false, method.text().into()))
                    .copied()
                    .unwrap_or(declared);
                let actual = self.env.with_self(declared, instance);
                if !self.env.assignable(actual, handler, self.mode) {
                    let msg = format!(
                        "`{}` cannot handle `{script}`: it is called with `{}`",
                        self.env.display(actual),
                        self.env.display(handler)
                    );
                    self.report("E2001", method.span(), msg);
                }
            }
        }
    }

    /// The handler type `installer` (`SetScript`, `HookScript`) of the
    /// native widget type gives the script `name`, with the frame as `Self`.
    fn script_handler(
        &mut self,
        native: DefId,
        instance: TypeId,
        installer: &str,
        name: &str,
    ) -> Option<TypeId> {
        let (m, _) = self
            .env
            .instance_member_as(native, &[], installer, Some(instance))?;
        let sigs: Vec<TypeId> = match self.env.types.get(m.ty).clone() {
            Ty::Intersection(parts) => parts.to_vec(),
            _ => vec![m.ty],
        };
        for sig in sigs {
            let Ty::Func(f) = self.env.types.get(sig).clone() else {
                continue;
            };
            let (Some(script), Some(handler)) = (f.params.first(), f.params.get(1)) else {
                continue;
            };
            if matches!(self.env.types.get(script.ty), Ty::Str(s) if s.as_ref() == name) {
                return Some(self.env.types.non_nil(handler.ty));
            }
        }
        None
    }
}
