//! The scopes of the file being lowered, and which kinds of binding they
//! have ever held. Plain Lua only ever declares locals, so the lookups that
//! can only find an import, a class, an enum, a constant or a `wowlua:meta`
//! binding are skipped there: a large data addon looks millions of names up.

use super::Binding;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
struct Scope {
    names: HashMap<Box<str>, Binding>,
    /// `@deprecated` bindings of this scope: their messages.
    deprecated: HashMap<Box<str>, Option<String>>,
    /// `@debugOnly` functions of this scope that the build keeps.
    debug_only: HashSet<Box<str>>,
}

/// Which kinds of binding the file has declared. Set only by
/// [`Scopes::declare`] and [`Scopes::mark_deprecated`], never cleared: a
/// `false` flag means no scope holds such a binding, so a lookup that can
/// only match that kind finds nothing.
#[derive(Clone, Copy, Default)]
pub(crate) struct Seen {
    /// Any binding but `Local`.
    pub non_local: bool,
    /// `Meta`, `MetaNamespace`.
    pub meta: bool,
    /// `Const`.
    pub consts: bool,
    /// `Intrinsic`, `IntrinsicNamespace`.
    pub intrinsics: bool,
    /// `Class`, `ImportedClass`.
    pub classes: bool,
    /// `Enum`, `ImportedEnum`.
    pub enums: bool,
    /// A `@deprecated` binding.
    pub deprecated: bool,
    /// A kept `@debugOnly` function.
    pub debug_only: bool,
}

pub(crate) struct Scopes {
    stack: Vec<Scope>,
    seen: Seen,
}

impl Scopes {
    /// The file's top-level scope.
    pub fn new() -> Self {
        Scopes {
            stack: vec![Scope::default()],
            seen: Seen::default(),
        }
    }

    pub fn push(&mut self) {
        self.stack.push(Scope::default());
    }

    pub fn pop(&mut self) {
        self.stack.pop();
    }

    pub fn seen(&self) -> Seen {
        #[cfg(debug_assertions)]
        self.check_seen();
        self.seen
    }

    /// Debug builds: every kind of binding the scopes hold has its flag set.
    #[cfg(debug_assertions)]
    fn check_seen(&self) {
        let seen = self.seen;
        for scope in &self.stack {
            assert!(scope.deprecated.is_empty() || seen.deprecated);
            for binding in scope.names.values() {
                let flag = match binding {
                    Binding::Local => true,
                    Binding::Meta(_) | Binding::MetaNamespace => seen.meta,
                    Binding::Const(_) => seen.consts,
                    Binding::Intrinsic(_) | Binding::IntrinsicNamespace(_) => seen.intrinsics,
                    Binding::Class { .. } | Binding::ImportedClass(_) => seen.classes,
                    Binding::Enum { .. } | Binding::ImportedEnum(_) => seen.enums,
                    _ => true,
                };
                assert!(flag, "{binding:?} without its flag");
                assert!(matches!(binding, Binding::Local) || seen.non_local);
            }
        }
    }

    pub fn declare(&mut self, name: &str, binding: Binding) {
        let seen = &mut self.seen;
        // No `_` arm: a new kind of binding must say which flag it sets.
        match &binding {
            Binding::Local => {}
            Binding::Meta(_) | Binding::MetaNamespace => seen.meta = true,
            Binding::Const(_) => seen.consts = true,
            Binding::Intrinsic(_) | Binding::IntrinsicNamespace(_) => seen.intrinsics = true,
            Binding::Class { .. } | Binding::ImportedClass(_) => seen.classes = true,
            Binding::Enum { .. } | Binding::ImportedEnum(_) => seen.enums = true,
            Binding::Import
            | Binding::LazyImport { .. }
            | Binding::Export
            | Binding::Hoisted
            | Binding::Removed(_)
            | Binding::ConstLocal
            | Binding::DebugOnlyRemoved => {}
        }
        seen.non_local |= !matches!(binding, Binding::Local);
        let scope = self.stack.last_mut().expect("a scope");
        if seen.debug_only {
            scope.debug_only.remove(name);
        }
        scope.names.insert(name.into(), binding);
    }

    pub fn lookup(&self, name: &str) -> Option<&Binding> {
        self.stack.iter().rev().find_map(|s| s.names.get(name))
    }

    /// The binding of `name` in the innermost scope only.
    pub fn lookup_here(&self, name: &str) -> Option<&Binding> {
        self.stack.last().and_then(|s| s.names.get(name))
    }

    /// Remembers a `@deprecated` binding of the current scope.
    pub fn mark_deprecated(&mut self, name: &str, message: Option<String>) {
        self.seen.deprecated = true;
        self.stack
            .last_mut()
            .expect("a scope")
            .deprecated
            .insert(name.into(), message);
    }

    /// Remembers a `@debugOnly` function of the current scope that the
    /// build keeps (its uses as a value are E3032 still).
    pub fn mark_debug_only(&mut self, name: &str) {
        self.seen.debug_only = true;
        self.stack
            .last_mut()
            .expect("a scope")
            .debug_only
            .insert(name.into());
    }

    /// `name` refers to a kept `@debugOnly` function.
    pub fn is_debug_only(&self, name: &str) -> bool {
        if !self.seen.debug_only {
            return false;
        }
        for scope in self.stack.iter().rev() {
            if scope.debug_only.contains(name) {
                return true;
            }
            if scope.names.contains_key(name) {
                return false;
            }
        }
        false
    }

    /// The `@deprecated` message of the binding `name` refers to, if any.
    pub fn deprecated_of(&self, name: &str) -> Option<Option<String>> {
        if !self.seen.deprecated {
            return None;
        }
        for scope in self.stack.iter().rev() {
            if scope.names.contains_key(name) || scope.deprecated.contains_key(name) {
                return scope.deprecated.get(name).cloned();
            }
        }
        None
    }
}
