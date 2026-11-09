//! What the game's own functions cost, estimated for profiles (decision
//! T611): the emulator's natives run no Lua, so a profile would count
//! `CreateFrame` and `SetPoint` as free while in the client they are the
//! expensive part of building a UI. Each native call is weighted, in Lua
//! instructions' worth: a fixed cost for any call, more for the known heavy
//! ones (making frames and regions, anchoring and sizing, which lay out
//! again, text and fonts, textures and atlases, aura queries). The weights
//! are estimates no measure in the client has confirmed yet (unverified):
//! they rank work, they do not time it.

use wowlua_vm::{Value, Vm};

/// Any native call: about what a cheap C function costs next to a Lua
/// instruction.
pub const CALL: u64 = 5;

/// Each template a new frame or region inherits (`CreateFrame`'s
/// `template`, a region's `inherits`): its XML's regions and children.
pub const TEMPLATE: u64 = 1000;

/// The heavy natives' weights, by name (a method's for every widget type
/// that has it).
pub const WEIGHTS: &[(&str, u64)] = &[
    // Making frames and regions.
    ("CreateFrame", 2000),
    ("CreateTexture", 300),
    ("CreateMaskTexture", 300),
    ("CreateFontString", 400),
    ("CreateLine", 300),
    ("CreateAnimationGroup", 200),
    ("CreateAnimation", 100),
    // Anchoring, sizing and parenting: the layout runs again.
    ("SetPoint", 50),
    ("SetAllPoints", 60),
    ("ClearAllPoints", 30),
    ("ClearPoint", 30),
    ("SetSize", 40),
    ("SetWidth", 30),
    ("SetHeight", 30),
    ("SetScale", 60),
    ("SetParent", 80),
    // Showing and hiding: the frame and its children.
    ("Show", 40),
    ("Hide", 40),
    ("SetShown", 40),
    // Reading a laid-out rectangle lays it out first.
    ("GetWidth", 20),
    ("GetHeight", 20),
    ("GetSize", 20),
    ("GetRect", 20),
    ("GetLeft", 20),
    ("GetRight", 20),
    ("GetTop", 20),
    ("GetBottom", 20),
    ("GetCenter", 20),
    // Text: shaping and measuring it, fonts.
    ("SetText", 80),
    ("SetFormattedText", 100),
    ("SetFont", 300),
    ("SetFontObject", 150),
    ("GetStringWidth", 40),
    ("GetStringHeight", 40),
    // Textures and atlases.
    ("SetTexture", 100),
    ("SetAtlas", 100),
    ("SetColorTexture", 30),
    ("SetStatusBarTexture", 100),
    ("SetTexCoord", 20),
    // Scripts and events.
    ("SetScript", 20),
    ("HookScript", 30),
    ("RegisterEvent", 30),
    ("RegisterUnitEvent", 40),
    ("UnregisterEvent", 30),
    // Auras: a table per aura.
    ("GetAuraDataByIndex", 60),
    ("GetAuraDataByAuraInstanceID", 50),
    ("GetAuraDataBySlot", 50),
    ("GetAuraSlots", 40),
    ("GetUnitAuras", 200),
    ("UnitAura", 60),
    ("UnitBuff", 60),
    ("UnitDebuff", 60),
];

/// The estimated cost of a native call: its weight (or [`CALL`]), and
/// [`TEMPLATE`] for each template a frame or region made inherits.
pub fn native_cost(vm: &Vm, name: &str, args: &[Value]) -> u64 {
    let weight = WEIGHTS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(CALL, |(_, w)| *w);
    // `CreateFrame(type, name, parent, template)`,
    // `frame:CreateTexture(name, layer, inherits)`: the fourth value.
    let inherits = match name {
        "CreateFrame" | "CreateTexture" | "CreateFontString" | "CreateMaskTexture" => {
            match args.get(3) {
                Some(Value::Str(s)) => vm
                    .heap
                    .str_bytes(*s)
                    .split(|&b| b == b',')
                    .filter(|t| t.iter().any(|b| !b.is_ascii_whitespace()))
                    .count() as u64,
                _ => 0,
            }
        }
        _ => 0,
    };
    weight + TEMPLATE * inherits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_add_to_what_makes_a_frame_or_region() {
        let mut vm = Vm::wow();
        let two = vm.str("BackdropTemplate, SecureActionButtonTemplate");
        let none = vm.str(" ");
        let args = |t: Value| [Value::Nil, Value::Nil, Value::Nil, t];
        assert_eq!(native_cost(&vm, "CreateFrame", &args(Value::Nil)), 2000);
        assert_eq!(
            native_cost(&vm, "CreateFrame", &args(two)),
            2000 + 2 * TEMPLATE
        );
        assert_eq!(native_cost(&vm, "CreateFrame", &args(none)), 2000);
        assert_eq!(
            native_cost(&vm, "CreateTexture", &args(two)),
            300 + 2 * TEMPLATE
        );
        // Only frames and regions inherit.
        assert_eq!(native_cost(&vm, "SetPoint", &args(two)), 50);
        assert_eq!(native_cost(&vm, "UnitHealth", &[]), CALL);
    }
}
