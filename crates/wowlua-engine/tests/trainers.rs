//! A disabled button's clicks (decision T616), the trainer's choice and
//! `TRAINER_DESCRIPTION_UPDATE` (decision T617), a hunter's Beast Training
//! (decision T618): what the engine does without Blizzard's UI.

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    call(vm, f)
}

/// Runs `src` as Blizzard's secure code runs.
fn run_secure(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "=secure").expect("compiles");
    call(vm, f)
}

/// Runs `f` as its own execution.
fn call(vm: &mut Vm, f: Value) -> Vec<String> {
    let text = |vm: &Vm, v: Value| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    };
    match vm.call_secure(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// `Click()` runs the click scripts with its button and `isDown`; on a
/// disabled button it does nothing, nor during the button's own click; the
/// user's click on a disabled button runs no click script either.
#[test]
fn a_disabled_button_ignores_clicks() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local b = CreateFrame('Button', nil, UIParent)
b:SetSize(20, 20)
b:SetPoint('CENTER')
for _, s in ipairs({ 'PreClick', 'OnClick', 'PostClick' }) do
    b:SetScript(s, function(self, button, down)
        seen[#seen + 1] = s .. ':' .. tostring(button) .. ':' .. tostring(down)
        if s == 'OnClick' then self:Click() end
    end)
end
b:Click('RightButton', true)
local enabled = table.concat(seen, ',')
seen = {}
b:Disable()
b:Click()
mock.click(b)
local disabled = #seen
b:Enable()
b:Click()
return enabled, disabled, table.concat(seen, ',')",
    );
    assert_eq!(
        r,
        [
            "PreClick:RightButton:true,OnClick:RightButton:true,PostClick:RightButton:true",
            "0",
            "PreClick:LeftButton:false,OnClick:LeftButton:false,PostClick:LeftButton:false",
        ]
    );
}

/// A trainer opened through its gossip: the NPC, its trainer `data`, the
/// option chosen.
const OPEN: &str =
    "local npc = mock.spawnUnit({ name = 'Trainer', level = 30, reaction = 'friendly' })
mock.defineGossip(npc, { text = 'hi', options = {
    { name = 'Train me.', icon = 132058, gossipOptionID = 1, opens = 'trainer' },
} })
mock.defineTrainer(npc, DATA)
npc:setDistance(3)
mock.player:interact(npc)
mock.advanceTime(1)
C_GossipInfo.SelectOption(1)
mock.advanceTime(1)
";

fn open(data: &str) -> String {
    OPEN.replace("DATA", data)
}

/// The first available service is chosen as the list comes; a change of
/// the choice sends `TRAINER_DESCRIPTION_UPDATE` at once, the same one
/// again nothing; an index past the list none.
#[test]
fn a_change_of_the_trainers_choice_is_told() {
    let mut vm = vm();
    let src = format!(
        "mock.setPlayer({{ level = 10 }})
local told = {{}}
local f = CreateFrame('Frame')
f:RegisterEvent('TRAINER_DESCRIPTION_UPDATE')
f:SetScript('OnEvent', function() told[#told + 1] = tostring(GetTrainerSelectionIndex()) end)
{}
local first = tostring(GetTrainerSelectionIndex())
SelectTrainerService(2)
local same = #told
SelectTrainerService(3)
local during = table.concat(told, ',')
SelectTrainerService(3)
SelectTrainerService(GetNumTrainerServices() + 1)
SelectTrainerService(GetNumTrainerServices() + 1)
return first, same, during, table.concat(told, ','), tostring(GetTrainerSelectionIndex())",
        open(
            "{ greeting = 'hi', services = {
    { spell = 90001, name = 'Charge', icon = 132337, level = 20, rank = 'Rank 1', cost = 100 },
    { spell = 90002, name = 'Rend', icon = 132155, level = 4, rank = 'Rank 1', cost = 100 },
    { spell = 90003, name = 'Parry', icon = 132269, level = 6, rank = 'Passive', cost = 100 },
} }"
        )
    );
    let r = run(&mut vm, &src);
    assert_eq!(r, ["2", "0", "3", "3,nil", "nil"]);
}

/// Beast Training, the hunter's spell: cast with a pet out, it opens the
/// trainer in its pet mode (its type, the pet's training points and
/// level, what the pet knows used); an addon's `BuyTrainerService` is
/// forbidden there, Blizzard's teaches the pet (its spellbook,
/// `IsSpellKnown(id, Pet)`, its points spent); the pet's leaving closes it.
#[test]
fn beast_training_teaches_the_pet() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "SEEN = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'UNIT_PET_TRAINING_POINTS', 'CHAT_MSG_SYSTEM', 'TRAINER_SHOW', 'TRAINER_UPDATE', 'TRAINER_CLOSED',
    'SPELLS_CHANGED', 'PLAYER_INTERACTION_MANAGER_FRAME_SHOW', 'ADDON_ACTION_FORBIDDEN' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, a, b)
    SEEN[#SEEN + 1] = e .. '(' .. tostring(a) .. (b and ',' .. tostring(b) or '') .. ')'
end)
mock.defineBeastTraining({ services = {
    { spell = 2649, name = 'Growl', icon = 132270, level = 1, rank = 'Rank 1', cost = 0 },
    { spell = 17253, name = 'Bite', icon = 132278, level = 8, rank = 'Rank 1', cost = 3 },
    { spell = 23099, name = 'Dash', icon = 132120, level = 30, rank = 'Rank 1', cost = 5 },
    { spell = 16827, name = 'Claw', icon = 132140, level = 1, rank = 'Rank 1', cost = 20 },
} })
-- no pet: the cast opens nothing
mock.player:useSpell(5149)
mock.advanceTime(2)
local noPet = table.concat(SEEN, ',')
local wolf = mock.spawnUnit({ name = 'Wolf', level = 20, reaction = 'friendly' })
mock.setPet(wolf, { trainingPoints = 10, spells = { { spell = 2649, name = 'Growl', subName = 'Rank 1' } } })
SEEN = {}
local before = C_Trainer.GetTrainerType()
mock.player:useSpell(5149)
mock.advanceTime(2)
local opened = table.concat(SEEN, ',')
SEEN = {}
local out = {}
for i = 1, GetNumTrainerServices() do
    out[i] = select(2, GetTrainerServiceInfo(i))
end
local total, used = C_PetInfo.GetPetTrainingPoints()
BuyTrainerService(2)
mock.advanceTime(1)
return before, noPet, opened, C_Trainer.GetTrainerType(), table.concat(out, ','),
    tostring(GetTrainerSelectionIndex()), total .. '/' .. used,
    tostring(C_SpellBook.IsSpellKnown(17253, Enum.SpellBookSpellBank.Pet)),
    tostring(C_SpellBook.IsSpellKnown(2649, Enum.SpellBookSpellBank.Pet)), table.concat(SEEN, ',')",
    );
    assert_eq!(
        r,
        [
            "0",
            "",
            "TRAINER_SHOW(nil),TRAINER_UPDATE(nil)",
            "3",
            "used,available,unavailable,available",
            "2",
            "10/0",
            "false",
            "true",
            "ADDON_ACTION_FORBIDDEN(MyAddon,BuyTrainerService())",
        ]
    );
    // Blizzard's Train: the unaffordable and the unavailable nothing, Bite
    // taught; the pet dismissed closes the window.
    let r = run_secure(
        &mut vm,
        "SEEN = {}
BuyTrainerService(4)
BuyTrainerService(3)
mock.advanceTime(1)
local soon = table.concat(SEEN, ',')
BuyTrainerService(2)
mock.advanceTime(1)
local total, used = C_PetInfo.GetPetTrainingPoints()
local out = {}
for i = 1, GetNumTrainerServices() do
    out[i] = select(2, GetTrainerServiceInfo(i))
end
local taught = table.concat(SEEN, ',')
local pet = tostring(C_SpellBook.IsSpellKnown(17253, Enum.SpellBookSpellBank.Pet))
local mine = tostring(C_SpellBook.IsSpellKnown(17253))
SEEN = {}
mock.setPet(nil)
return soon, taught, total .. '/' .. used, pet, mine, table.concat(out, ','), table.concat(SEEN, ','),
    C_Trainer.GetTrainerType()",
    );
    assert_eq!(
        r,
        [
            "",
            "CHAT_MSG_SYSTEM(Your pet has learned a new ability: |cff71d5ff|Hspell:17253:0|h[Bite]|h|r.,),\
UNIT_PET_TRAINING_POINTS(pet),SPELLS_CHANGED(nil),TRAINER_UPDATE(nil)",
            "10/3",
            "true",
            "false",
            "used,used,unavailable,available",
            "TRAINER_CLOSED(nil),SPELLS_CHANGED(nil)",
            "0",
        ]
    );
}

/// `mock.setPet`'s training points: a change tells
/// `UNIT_PET_TRAINING_POINTS("pet")`; without a pet, none.
#[test]
fn the_pets_training_points() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = 0
local f = CreateFrame('Frame')
f:RegisterEvent('UNIT_PET_TRAINING_POINTS')
f:SetScript('OnEvent', function(_, _, unit) seen = seen + 1 end)
local wolf = mock.spawnUnit({ name = 'Wolf', level = 20, reaction = 'friendly' })
mock.setPet(wolf)
local none = seen
mock.setPet(wolf, { trainingPoints = 12, trainingPointsUsed = 4 })
local t, u = C_PetInfo.GetPetTrainingPoints()
mock.setPet(wolf, { happiness = 2 })
local same = seen
mock.setPet(nil)
local t0, u0 = C_PetInfo.GetPetTrainingPoints()
return none, t .. '/' .. u, same, t0 .. '/' .. u0",
    );
    assert_eq!(r, ["0", "12/4", "1", "0/0"]);
}
