//! What the core needs of the scene (wowlua-engine's `scene` builds it).

use wowlua_vm::{Value, Vm};

use crate::hooks;

pub fn look_of(vm: &mut Vm, unit: &str) -> Look {
    (hooks::get().look_of)(vm, unit)
}

/// What a model shows (spec debugger §18.9.7).
#[derive(Clone, Debug, PartialEq)]
pub enum ModelSource {
    File(u32),
    Path(String),
    /// A creature display (`CreatureDisplayInfo`).
    Display(u32),
    /// A creature (`Creature`): its first display.
    Creature(u32),
    /// A character as a unit looks.
    Character(Look),
    /// An item's own model (`SetItem`).
    Item(u32),
    /// An item appearance's model (`SetItemAppearance`).
    Appearance(u32),
}

/// How a character looks: race, sex (0 male, 1 female), class, and what it
/// wears (inventory slot, item ID); items tried on (links or appearance
/// IDs) over that.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Look {
    pub race: u32,
    pub sex: u32,
    pub class: u32,
    pub items: Vec<(u32, u32)>,
    pub tried: Vec<String>,
}

pub const LAYERS: [&str; 5] = ["BACKGROUND", "BORDER", "ARTWORK", "OVERLAY", "HIGHLIGHT"];

/// A draw layer's name as kept: a known one in capitals, whatever case it
/// was given in, as `GetDrawLayer` answers (decision T597); others as given.
pub fn layer_name(vm: &mut Vm, v: Value) -> Value {
    let known = match v {
        Value::Str(s) => {
            let name = vm.heap.str_bytes(s);
            LAYERS
                .iter()
                .find(|l| l.as_bytes().eq_ignore_ascii_case(name))
        }
        _ => None,
    };
    known.map_or(v, |l| vm.str(l))
}

/// A look as text (`race:sex:class:slot=item,…`), for a portrait kept
/// as the unit looked when it was set.
pub fn encode_look(l: &Look) -> String {
    let items: Vec<String> = l.items.iter().map(|(s, i)| format!("{s}={i}")).collect();
    format!("{}:{}:{}:{}", l.race, l.sex, l.class, items.join(","))
}
