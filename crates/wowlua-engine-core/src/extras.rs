//! What the core needs of wowlua-engine's `extras`.

use crate::hooks;

pub fn silent_unchecked(doc: &serde_json::Value) -> bool {
    (hooks::get().silent_unchecked)(doc)
}

pub fn silent_checked(doc: &serde_json::Value) -> bool {
    (hooks::get().silent_checked)(doc)
}
