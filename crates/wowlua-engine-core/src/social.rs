//! What the core needs of wowlua-engine's `social`.

use wowlua_vm::Vm;

use crate::{engine, units};

/// The player's realm as `GetRealmName` gives it: the world's, else the
/// engine's placeholder "Realm".
pub fn player_realm_name(vm: &Vm) -> String {
    engine(vm).units.list[units::PLAYER.0]
        .realm
        .clone()
        .unwrap_or_else(|| "Realm".into())
}

/// The player's realm as `GetNormalizedRealmName` gives it (no spaces or
/// hyphens).
pub fn player_realm(vm: &Vm) -> String {
    player_realm_name(vm).replace([' ', '-'], "")
}
