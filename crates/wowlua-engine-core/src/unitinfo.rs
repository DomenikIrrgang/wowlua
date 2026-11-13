//! What the core needs of wowlua-engine's `unitinfo`.

use wowlua_vm::Vm;

use crate::engine;

/// The player's facing, radians counter-clockwise from north, 0 to 2π:
/// `mock.moveTo`'s `facing`, north at first.
pub fn player_facing(vm: &Vm) -> f64 {
    engine(vm).world.character.position.facing
}
