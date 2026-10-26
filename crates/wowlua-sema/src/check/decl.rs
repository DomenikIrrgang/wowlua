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
}
