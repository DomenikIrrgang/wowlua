//! Flavor guards in shared flavor mode (spec §6.5): an API that is not
//! available in every selected flavor must be used inside a guard
//! (`flavor == "…"`, `WOW_PROJECT_ID == …`, or an existence check).

use super::body::Checker;
use std::collections::{BTreeSet, HashMap, HashSet};
use wowlua_syntax::ast::*;

/// The flavors of a shared-mode build and where each API exists.
#[derive(Clone, Debug, Default)]
pub struct SharedFlavors {
    /// The selected flavors.
    pub flavors: BTreeSet<String>,
    /// `WOW_PROJECT_ID` values → flavor.
    pub project_ids: HashMap<u32, String>,
    /// `WOW_PROJECT_*` constant names → value.
    pub project_constants: HashMap<String, u32>,
    /// APIs (`Name`, `Namespace.Member`) not declared by every flavor →
    /// the flavors that declare them.
    pub availability: HashMap<String, BTreeSet<String>>,
}

/// What guards establish at a point of a function.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guard {
    /// The flavors the code can run in (`None`: all selected).
    pub flavors: Option<BTreeSet<String>>,
    /// APIs checked for existence.
    pub present: HashSet<String>,
}

impl Guard {
    /// Two paths meet.
    pub fn join(&self, other: &Guard) -> Guard {
        let flavors = match (&self.flavors, &other.flavors) {
            (Some(a), Some(b)) => Some(a.union(b).cloned().collect()),
            _ => None,
        };
        Guard {
            flavors,
            present: self.present.intersection(&other.present).cloned().collect(),
        }
    }
}

impl Checker<'_> {
    /// The API path of an expression (`C_EditMode`, `C_EditMode.GetLayouts`)
    /// when it names a global.
    fn api_path(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Name(n) if self.lookup_var(&n.text).is_none() => Some(n.text.to_string()),
            ExprKind::Field { obj, name } => {
                self.api_path(obj).map(|p| format!("{p}.{}", name.text))
            }
            ExprKind::Paren(inner) => self.api_path(inner),
            _ => None,
        }
    }

    /// Records the existence tests of a condition (the condition itself,
    /// operands of `not`, `and`, `or`, and `== nil`/`~= nil`).
    pub fn mark_probes(&mut self, cond: &Expr) {
        match &cond.kind {
            ExprKind::Paren(inner) => self.mark_probes(inner),
            ExprKind::Unary {
                op: UnOp::Not,
                operand,
            } => self.mark_probes(operand),
            ExprKind::Binary {
                op: BinOp::And | BinOp::Or,
                lhs,
                rhs,
            } => {
                self.mark_probes(lhs);
                self.mark_probes(rhs);
            }
            ExprKind::Binary {
                op: BinOp::Eq | BinOp::Ne,
                lhs,
                rhs,
            } => {
                if matches!(rhs.kind, ExprKind::Nil) {
                    self.mark_probes(lhs);
                } else if matches!(lhs.kind, ExprKind::Nil) {
                    self.mark_probes(rhs);
                }
            }
            _ => {
                if self.api_path(cond).is_some() {
                    self.probes.insert(cond.span);
                }
            }
        }
    }

    /// The flavor a comparison operand names (`"standard"`, a
    /// `WOW_PROJECT_*` constant or number).
    fn flavor_of(&self, e: &Expr, shared: &SharedFlavors, project: bool) -> Option<String> {
        match &e.kind {
            ExprKind::String(s) if !project => Some(String::from_utf8_lossy(&s.value).into_owned()),
            ExprKind::Number(n) if project => n
                .value
                .and_then(|v| shared.project_ids.get(&(v as u32)).cloned()),
            ExprKind::Name(n) if project => shared
                .project_constants
                .get(n.text.as_ref())
                .and_then(|v| shared.project_ids.get(v).cloned()),
            _ => None,
        }
    }

    /// Guards where `cond` is true and where it is false.
    pub fn narrow_guard(&self, cond: &Expr, g: &Guard) -> (Guard, Guard) {
        match &cond.kind {
            ExprKind::Paren(inner) => self.narrow_guard(inner, g),
            ExprKind::Unary {
                op: UnOp::Not,
                operand,
            } => {
                let (t, f) = self.narrow_guard(operand, g);
                (f, t)
            }
            ExprKind::Binary {
                op: BinOp::And,
                lhs,
                rhs,
            } => {
                let (lt, lf) = self.narrow_guard(lhs, g);
                let (rt, rf) = self.narrow_guard(rhs, &lt);
                (rt, lf.join(&rf))
            }
            ExprKind::Binary {
                op: BinOp::Or,
                lhs,
                rhs,
            } => {
                let (lt, lf) = self.narrow_guard(lhs, g);
                let (rt, rf) = self.narrow_guard(rhs, &lf);
                (lt.join(&rt), rf)
            }
            ExprKind::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                lhs,
                rhs,
            } => {
                let is_flavor = |e: &Expr| matches!(&e.kind, ExprKind::Name(n) if self.meta_flavor.contains(n.text.as_ref()));
                let is_project = |e: &Expr| matches!(&e.kind, ExprKind::Name(n) if n.text.as_ref() == "WOW_PROJECT_ID" && self.lookup_var("WOW_PROJECT_ID").is_none());
                let shared = self.shared.as_ref();
                let named = shared.and_then(|shared| {
                    if is_flavor(lhs) {
                        self.flavor_of(rhs, shared, false)
                    } else if is_flavor(rhs) {
                        self.flavor_of(lhs, shared, false)
                    } else if is_project(lhs) {
                        self.flavor_of(rhs, shared, true)
                    } else if is_project(rhs) {
                        self.flavor_of(lhs, shared, true)
                    } else {
                        None
                    }
                });
                let (Some(shared), Some(flavor)) = (shared, named) else {
                    // `C_X ~= nil` / `C_X == nil`: an existence check.
                    let tested = match (&lhs.kind, &rhs.kind) {
                        (_, ExprKind::Nil) => Some(&**lhs),
                        (ExprKind::Nil, _) => Some(&**rhs),
                        _ => None,
                    };
                    return match tested {
                        Some(t) => {
                            let (yes, no) = self.narrow_guard(t, g);
                            if *op == BinOp::Ne {
                                (yes, no)
                            } else {
                                (no, yes)
                            }
                        }
                        None => (g.clone(), g.clone()),
                    };
                };
                let current = g.flavors.clone().unwrap_or_else(|| shared.flavors.clone());
                let mut yes = g.clone();
                yes.flavors = Some(current.iter().filter(|f| **f == flavor).cloned().collect());
                let mut no = g.clone();
                no.flavors = Some(current.iter().filter(|f| **f != flavor).cloned().collect());
                if *op == BinOp::Eq {
                    (yes, no)
                } else {
                    (no, yes)
                }
            }
            _ => match self.api_path(cond) {
                // An existence check.
                Some(path) => {
                    let mut yes = g.clone();
                    let mut prefix = String::new();
                    for part in path.split('.') {
                        if !prefix.is_empty() {
                            prefix.push('.');
                        }
                        prefix.push_str(part);
                        yes.present.insert(prefix.clone());
                    }
                    (yes, g.clone())
                }
                None => (g.clone(), g.clone()),
            },
        }
    }

    /// A use of an API: E2030 when some flavor the code can run in lacks
    /// it and no guard covers it.
    pub fn check_available(&mut self, path: &str, span: wowlua_span::Span) {
        let Some(shared) = &self.shared else { return };
        let Some(have) = shared.availability.get(path) else {
            return;
        };
        if self.guard.present.contains(path) {
            return;
        }
        // A present namespace limits the flavors to those that have it.
        let mut active = self
            .guard
            .flavors
            .clone()
            .unwrap_or_else(|| shared.flavors.clone());
        for p in &self.guard.present {
            if path.starts_with(&format!("{p}."))
                && let Some(with) = shared.availability.get(p)
            {
                active.retain(|f| with.contains(f));
            }
        }
        let missing: Vec<&String> = active.iter().filter(|f| !have.contains(*f)).collect();
        if missing.is_empty() {
            return;
        }
        let list = missing
            .iter()
            .map(|f| format!("`{f}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let msg = format!(
            "`{path}` is not available in {list}; guard it with a flavor check or an existence check"
        );
        self.report("E2030", span, msg);
    }
}
