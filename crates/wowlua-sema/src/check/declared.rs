//! The names the declarations give: global values, global types, event and
//! message handlers, on-demand globals, the callbacks registry and the
//! slash router. Every
//! read goes through an accessor that can record it, so a check can tell
//! what it read of a flavor's declarations (#6); the maps are private, so
//! a read that does not record does not compile.

use super::types::{DefId, Func, TypeId};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// A name a check looked up.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Root {
    Global(Box<str>),
    Type(Box<str>),
    Handler(Box<str>),
    OnDemand(Box<str>),
    CallbacksRegistry,
    SlashRouter,
}

/// What a check read: the names it looked up (with whether they were
/// found) and what it read of declarations; `opaque` when it read something
/// else of the declarations (its results cannot be reused).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reads {
    pub roots: BTreeMap<Root, bool>,
    pub defs: BTreeMap<DefId, DefRead>,
    pub opaque: bool,
}

/// What a check read of one declaration (its name and kind are always
/// part of the fingerprint): all of it, or parts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefRead {
    pub whole: bool,
    /// Every field but the member lists: params, bases, alias,
    /// abstractness, widget, nativeness, indexer, span.
    pub header: bool,
    /// Members and statics looked up by name, found or not.
    pub members: BTreeSet<Box<str>>,
    pub statics: BTreeSet<Box<str>>,
    /// Enum members looked up by name, and by position.
    pub enum_named: BTreeSet<Box<str>>,
    pub enum_at: BTreeSet<u32>,
}

/// A part of a declaration (see [`DefRead`]).
#[derive(Clone, Copy, Debug)]
pub enum Part<'a> {
    Header,
    Member(&'a str),
    Static(&'a str),
    EnumNamed(&'a str),
    EnumAt(u32),
}

#[derive(Default)]
pub struct Declared {
    globals: FxHashMap<Box<str>, TypeId>,
    global_types: FxHashMap<Box<str>, DefId>,
    handlers: FxHashMap<Box<str>, Func>,
    on_demand: FxHashMap<Box<str>, String>,
    callbacks_registry: Option<DefId>,
    slash_router: Option<DefId>,
    /// While recording, what was read.
    reads: RefCell<Option<Reads>>,
}

impl Declared {
    fn note(&self, root: impl FnOnce() -> Root, found: bool) {
        if let Some(reads) = self.reads.borrow_mut().as_mut() {
            reads.roots.insert(root(), found);
        }
    }

    /// Records that all of `def` was read.
    pub(crate) fn note_def(&self, def: DefId) {
        if let Some(reads) = self.reads.borrow_mut().as_mut() {
            reads.defs.entry(def).or_default().whole = true;
        }
    }

    /// Records that a part of `def` was read.
    pub(crate) fn note_part(&self, def: DefId, part: Part<'_>) {
        if let Some(reads) = self.reads.borrow_mut().as_mut() {
            let read = reads.defs.entry(def).or_default();
            match part {
                Part::Header => read.header = true,
                Part::Member(n) => {
                    read.members.insert(n.into());
                }
                Part::Static(n) => {
                    read.statics.insert(n.into());
                }
                Part::EnumNamed(n) => {
                    read.enum_named.insert(n.into());
                }
                Part::EnumAt(i) => {
                    read.enum_at.insert(i);
                }
            }
        }
    }

    /// Records a read of declarations that fingerprints do not cover.
    pub(crate) fn note_opaque(&self) {
        if let Some(reads) = self.reads.borrow_mut().as_mut() {
            reads.opaque = true;
        }
    }

    /// Starts recording reads.
    pub(crate) fn record(&self) {
        *self.reads.borrow_mut() = Some(Reads::default());
    }

    /// Stops recording; what was read since [`Declared::record`].
    pub(crate) fn recorded(&self) -> Option<Reads> {
        self.reads.borrow_mut().take()
    }

    pub fn global(&self, name: &str) -> Option<TypeId> {
        let found = self.globals.get(name).copied();
        self.note(|| Root::Global(name.into()), found.is_some());
        found
    }

    pub fn has_global(&self, name: &str) -> bool {
        self.global(name).is_some()
    }

    /// Every globally named type, without noting a read.
    pub fn global_type_entries(&self) -> impl Iterator<Item = (&str, DefId)> {
        self.global_types.iter().map(|(n, &d)| (&**n, d))
    }

    pub fn global_type(&self, name: &str) -> Option<DefId> {
        let found = self.global_types.get(name).copied();
        self.note(|| Root::Type(name.into()), found.is_some());
        found
    }

    pub fn handler(&self, name: &str) -> Option<&Func> {
        let found = self.handlers.get(name);
        self.note(|| Root::Handler(name.into()), found.is_some());
        found
    }

    pub fn on_demand(&self, name: &str) -> Option<&str> {
        let found = self.on_demand.get(name).map(String::as_str);
        self.note(|| Root::OnDemand(name.into()), found.is_some());
        found
    }

    pub fn callbacks_registry(&self) -> Option<DefId> {
        self.note(
            || Root::CallbacksRegistry,
            self.callbacks_registry.is_some(),
        );
        self.callbacks_registry
    }

    pub fn slash_router(&self) -> Option<DefId> {
        self.note(|| Root::SlashRouter, self.slash_router.is_some());
        self.slash_router
    }

    pub fn set_global(&mut self, name: Box<str>, ty: TypeId) {
        self.globals.insert(name, ty);
    }

    /// Declares `name` as `ty` unless it is declared already.
    pub fn set_global_if_new(&mut self, name: &str, ty: TypeId) {
        self.globals.entry(name.into()).or_insert(ty);
    }

    pub fn set_global_type(&mut self, name: Box<str>, def: DefId) {
        self.global_types.insert(name, def);
    }

    pub fn set_handler(&mut self, name: Box<str>, handler: Func) {
        self.handlers.insert(name, handler);
    }

    pub fn set_on_demand(&mut self, name: Box<str>, addon: String) {
        self.on_demand.insert(name, addon);
    }

    pub fn set_callbacks_registry(&mut self, def: Option<DefId>) {
        self.callbacks_registry = def;
    }

    pub fn set_slash_router(&mut self, def: Option<DefId>) {
        self.slash_router = def;
    }

    /// Every global value, without recording: for tests and tools that
    /// compare environments, never for checking.
    pub fn unrecorded_globals(&self) -> impl Iterator<Item = (&str, TypeId)> {
        self.globals.iter().map(|(k, &v)| (&**k, v))
    }

    /// Every global type, without recording (as `unrecorded_globals`).
    pub fn unrecorded_global_types(&self) -> impl Iterator<Item = (&str, DefId)> {
        self.global_types.iter().map(|(k, &v)| (&**k, v))
    }

    /// Every on-demand global, without recording (as `unrecorded_globals`).
    pub fn unrecorded_on_demand(&self) -> impl Iterator<Item = (&str, &str)> {
        self.on_demand.iter().map(|(k, v)| (&**k, v.as_str()))
    }

    /// The answer `root` gives, without recording (for fingerprints).
    pub(crate) fn peek(&self, root: &Root) -> Peeked<'_> {
        match root {
            Root::Global(n) => Peeked::Type(self.globals.get(n).copied()),
            Root::Type(n) => Peeked::Def(self.global_types.get(n).copied()),
            Root::Handler(n) => Peeked::Func(self.handlers.get(n)),
            Root::OnDemand(n) => Peeked::Text(self.on_demand.get(n).map(String::as_str)),
            Root::CallbacksRegistry => Peeked::Def(self.callbacks_registry),
            Root::SlashRouter => Peeked::Def(self.slash_router),
        }
    }
}

/// What a [`Root`] resolves to.
pub(crate) enum Peeked<'a> {
    Type(Option<TypeId>),
    Def(Option<DefId>),
    Func(Option<&'a Func>),
    Text(Option<&'a str>),
}
