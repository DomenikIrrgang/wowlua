//! The player's stats as the client reports them (decision: modeled
//! profile fields): attributes, damage, ratings, regeneration, experience,
//! honor, talents, forms, appearances, the spellbook, professions, skills,
//! reputations, currencies and the quest log, each the full answer of the
//! client function that reports it.
//! A world profile's `stats` (recorded by `/stdtest snapshot`) sets them,
//! and tests change them with `mock.player:setStats`. The snapshot records
//! the same keys (`crates/wowlua-runtime/wowlua/snapshot.wowlua`; a test keeps the two
//! lists together).
//!
//! The default character's stats are stand-ins (no recording of it yet):
//! empty values, a whole damage multiplier, a two-second swing, no combat
//! ratings, spell power or item level; one talent group; no transmog.

use std::collections::BTreeMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, units, widgets};

/// How a stat's function is asked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ask {
    /// `f(unit)`: the player's.
    Unit,
    /// `f()`.
    Plain,
    /// `f(unit, i)`: by index (an attribute, a school).
    UnitIndex,
    /// `f(i)`: by index (a rating, a school).
    Index,
    /// `f(stat, value)`: by attribute, for the attribute's effective value
    /// (as the character sheet asks it).
    ForStat,
    /// `f(location)`: by transmog location, `slotID * 100 + type * 10 +
    /// modification` (Blizzard's lookup key).
    Location,
    /// `f(slot, bank)`: by the player's spellbook slot (the pet's bank:
    /// nothing recorded yet).
    Slot,
    /// `f(flyoutID, slot)`: by `flyoutID * 100 + slot`.
    FlyoutSlot,
}

/// A stat: its key in a profile, the function that reports it, how it is
/// asked, and the event that tells of a change (unverified).
pub struct Stat {
    pub key: &'static str,
    pub function: &'static str,
    pub ask: Ask,
    pub event: Option<&'static str>,
}

const fn stat(
    key: &'static str,
    function: &'static str,
    ask: Ask,
    event: Option<&'static str>,
) -> Stat {
    Stat {
        key,
        function,
        ask,
        event,
    }
}

use Ask::*;

/// Every stat (the client functions the character sheet computes from).
pub const STATS: &[Stat] = &[
    stat("stat", "UnitStat", UnitIndex, Some("UNIT_STATS")),
    stat(
        "resistance",
        "UnitResistance",
        UnitIndex,
        Some("UNIT_RESISTANCES"),
    ),
    stat("armor", "UnitArmor", Unit, Some("UNIT_RESISTANCES")),
    stat("damage", "UnitDamage", Unit, Some("UNIT_DAMAGE")),
    stat(
        "rangedDamage",
        "UnitRangedDamage",
        Unit,
        Some("UNIT_RANGEDDAMAGE"),
    ),
    stat(
        "attackSpeed",
        "UnitAttackSpeed",
        Unit,
        Some("UNIT_ATTACK_SPEED"),
    ),
    stat(
        "attackPower",
        "UnitAttackPower",
        Unit,
        Some("UNIT_ATTACK_POWER"),
    ),
    stat(
        "rangedAttackPower",
        "UnitRangedAttackPower",
        Unit,
        Some("UNIT_RANGED_ATTACK_POWER"),
    ),
    stat("defense", "UnitDefenseSkill", Unit, Some("UNIT_DEFENSE")),
    stat("hpPerStamina", "UnitHPPerStamina", Unit, None),
    stat(
        "spellHaste",
        "UnitSpellHaste",
        Unit,
        Some("UNIT_SPELL_HASTE"),
    ),
    stat("movement", "GetUnitSpeed", Unit, None),
    stat("xp", "UnitXP", Unit, Some("PLAYER_XP_UPDATE")),
    stat("xpMax", "UnitXPMax", Unit, Some("PLAYER_XP_UPDATE")),
    stat("honor", "UnitHonor", Unit, Some("HONOR_XP_UPDATE")),
    stat("honorMax", "UnitHonorMax", Unit, Some("HONOR_XP_UPDATE")),
    stat(
        "honorLevel",
        "UnitHonorLevel",
        Unit,
        Some("HONOR_LEVEL_UPDATE"),
    ),
    stat(
        "restedXP",
        "GetXPExhaustion",
        Plain,
        Some("UPDATE_EXHAUSTION"),
    ),
    stat(
        "restState",
        "GetRestState",
        Plain,
        Some("UPDATE_EXHAUSTION"),
    ),
    stat(
        "xpDisabled",
        "IsXPUserDisabled",
        Plain,
        Some("PLAYER_XP_UPDATE"),
    ),
    stat(
        "expertise",
        "GetExpertise",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "spellHealing",
        "GetSpellBonusHealing",
        Plain,
        Some("SPELL_POWER_CHANGED"),
    ),
    stat("spellPenetration", "GetSpellPenetration", Plain, None),
    stat(
        "spellHit",
        "GetSpellHitModifier",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "spellCrit",
        "GetSpellCritChance",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "shieldBlock",
        "GetShieldBlock",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "rangedHaste",
        "GetRangedHaste",
        Plain,
        Some("UNIT_RANGEDDAMAGE"),
    ),
    stat(
        "rangedCrit",
        "GetRangedCritChance",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "rangedHit",
        "GetRangedHitModifier",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat("crit", "GetCritChance", Plain, Some("COMBAT_RATING_UPDATE")),
    stat("hit", "GetHitModifier", Plain, Some("COMBAT_RATING_UPDATE")),
    stat(
        "offhandHit",
        "GetOffhandHitModifier",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "meleeHaste",
        "GetMeleeHaste",
        Plain,
        Some("UNIT_ATTACK_SPEED"),
    ),
    stat("mastery", "GetMasteryEffect", Plain, Some("MASTERY_UPDATE")),
    stat("powerRegen", "GetPowerRegen", Plain, None),
    stat("manaRegen", "GetManaRegen", Plain, None),
    stat("manaRegenFromSpirit", "GetManaRegenFromSpirit", Plain, None),
    stat("healthRegen", "GetHealthRegen", Plain, None),
    stat(
        "healthRegenFromSpirit",
        "GetHealthRegenFromSpirit",
        Plain,
        None,
    ),
    stat(
        "block",
        "GetBlockChance",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "dodge",
        "GetDodgeChance",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "dodgeFromAttribute",
        "GetDodgeChanceFromAttribute",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "parry",
        "GetParryChance",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "resilience",
        "GetModResilienceDamageReduction",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat("lifesteal", "GetLifesteal", Plain, Some("LIFESTEAL_UPDATE")),
    stat("avoidance", "GetAvoidance", Plain, Some("AVOIDANCE_UPDATE")),
    stat("speed", "GetSpeed", Plain, Some("SPEED_UPDATE")),
    stat(
        "itemLevel",
        "GetAverageItemLevel",
        Plain,
        Some("PLAYER_AVG_ITEM_LEVEL_UPDATE"),
    ),
    stat(
        "armorPenetration",
        "GetArmorPenetration",
        Plain,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "spellPowerByAttackPower",
        "GetOverrideSpellPowerByAP",
        Plain,
        Some("SPELL_POWER_CHANGED"),
    ),
    stat(
        "attackPowerBySpellPower",
        "GetOverrideAPBySpellPower",
        Plain,
        Some("SPELL_POWER_CHANGED"),
    ),
    // Talents.
    stat(
        "specGroups",
        "GetNumSpecGroups",
        Plain,
        Some("PLAYER_TALENT_UPDATE"),
    ),
    stat(
        "activeSpecGroup",
        "C_SpecializationInfo.GetActiveSpecGroup",
        Plain,
        Some("ACTIVE_TALENT_GROUP_CHANGED"),
    ),
    stat(
        "specialization",
        "C_SpecializationInfo.GetSpecialization",
        Plain,
        Some("PLAYER_SPECIALIZATION_CHANGED"),
    ),
    stat(
        "specializationInfo",
        "C_SpecializationInfo.GetSpecializationInfo",
        Index,
        Some("PLAYER_SPECIALIZATION_CHANGED"),
    ),
    stat(
        "combatConfig",
        "C_SpecializationInfo.GetCombatConfigIDForSpecGroup",
        Index,
        Some("PLAYER_TALENT_UPDATE"),
    ),
    stat(
        "talentUI",
        "C_SpecializationInfo.CanPlayerUseTalentUI",
        Plain,
        Some("PLAYER_TALENT_UPDATE"),
    ),
    stat(
        "talentSpecUI",
        "C_SpecializationInfo.CanPlayerUseTalentSpecUI",
        Plain,
        Some("PLAYER_TALENT_UPDATE"),
    ),
    stat(
        "earnedTalentPoint",
        "C_SpecializationInfo.HasPlayerEarnedATalentPoint",
        Plain,
        Some("PLAYER_TALENT_UPDATE"),
    ),
    // Shapeshift forms (a paladin's auras too).
    stat(
        "shapeshiftForms",
        "GetNumShapeshiftForms",
        Plain,
        Some("UPDATE_SHAPESHIFT_FORMS"),
    ),
    stat(
        "shapeshiftForm",
        "GetShapeshiftForm",
        Plain,
        Some("UPDATE_SHAPESHIFT_FORM"),
    ),
    stat(
        "shapeshiftFormID",
        "GetShapeshiftFormID",
        Plain,
        Some("UPDATE_SHAPESHIFT_FORM"),
    ),
    stat(
        "shapeshiftFormInfo",
        "GetShapeshiftFormInfo",
        Index,
        Some("UPDATE_SHAPESHIFT_FORMS"),
    ),
    // The cooldown viewer's spells (by category) and group buffs.
    stat(
        "cooldownViewerSet",
        "C_CooldownViewer.GetCooldownViewerCategorySet",
        Index,
        Some("COOLDOWN_VIEWER_DATA_LOADED"),
    ),
    stat(
        "groupBuffs",
        "C_CooldownViewer.GetGroupBuffItems",
        Plain,
        Some("COOLDOWN_VIEWER_DATA_LOADED"),
    ),
    // The spellbook: its skill lines (tabs) and the player's items by slot.
    stat(
        "spellBookSkillLines",
        "C_SpellBook.GetNumSpellBookSkillLines",
        Plain,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "spellBookSkillLine",
        "C_SpellBook.GetSpellBookSkillLineInfo",
        Index,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "classSkillLine",
        "C_SpellBook.GetClassSkillLineInfo",
        Plain,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "hasPetSpells",
        "C_SpellBook.HasPetSpells",
        Plain,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "spellBookItem",
        "C_SpellBook.GetSpellBookItemInfo",
        Slot,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "spellBookItemName",
        "C_SpellBook.GetSpellBookItemName",
        Slot,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "spellBookItemTexture",
        "C_SpellBook.GetSpellBookItemTexture",
        Slot,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "spellBookItemType",
        "C_SpellBook.GetSpellBookItemType",
        Slot,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "spellBookItemPassive",
        "C_SpellBook.IsSpellBookItemPassive",
        Slot,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "spellBookItemLowRank",
        "C_SpellBook.IsSpellBookItemLowRank",
        Slot,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "spellBookItemLevel",
        "C_SpellBook.GetSpellBookItemLevelLearned",
        Slot,
        Some("SPELLS_CHANGED"),
    ),
    stat(
        "spellBookItemLink",
        "C_SpellBook.GetSpellBookItemLink",
        Slot,
        Some("SPELLS_CHANGED"),
    ),
    // Professions.
    stat(
        "professions",
        "GetProfessions",
        Plain,
        Some("SKILL_LINES_CHANGED"),
    ),
    stat(
        "professionInfo",
        "GetProfessionInfo",
        Index,
        Some("SKILL_LINES_CHANGED"),
    ),
    // Skills (by line), reputations (by faction), currencies (by line).
    stat(
        "skillLines",
        "C_SkillInfo.GetNumSkillLines",
        Plain,
        Some("SKILL_LINES_CHANGED"),
    ),
    stat(
        "skillLine",
        "C_SkillInfo.GetSkillLineInfo",
        Index,
        Some("SKILL_LINES_CHANGED"),
    ),
    stat(
        "selectedSkill",
        "C_SkillInfo.GetSelectedSkill",
        Plain,
        Some("SKILL_LINES_CHANGED"),
    ),
    stat(
        "factions",
        "C_Reputation.GetNumFactions",
        Plain,
        Some("UPDATE_FACTION"),
    ),
    stat(
        "faction",
        "C_Reputation.GetFactionDataByIndex",
        Index,
        Some("UPDATE_FACTION"),
    ),
    stat(
        "selectedFaction",
        "C_Reputation.GetSelectedFaction",
        Plain,
        Some("UPDATE_FACTION"),
    ),
    stat(
        "watchedFaction",
        "C_Reputation.GetWatchedFactionData",
        Plain,
        Some("UPDATE_FACTION"),
    ),
    stat(
        "currencyListSize",
        "C_CurrencyInfo.GetCurrencyListSize",
        Plain,
        Some("CURRENCY_DISPLAY_UPDATE"),
    ),
    stat(
        "currencyListInfo",
        "C_CurrencyInfo.GetCurrencyListInfo",
        Index,
        Some("CURRENCY_DISPLAY_UPDATE"),
    ),
    stat(
        "backpackCurrency",
        "C_CurrencyInfo.GetBackpackCurrencyInfo",
        Index,
        Some("CURRENCY_DISPLAY_UPDATE"),
    ),
    // The quest log (by entry; by quest ID).
    stat(
        "questLogEntries",
        "C_QuestLog.GetNumQuestLogEntries",
        Plain,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questLogInfo",
        "C_QuestLog.GetInfo",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questCompletionText",
        "GetQuestLogCompletionText",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "selectedQuest",
        "C_QuestLog.GetSelectedQuest",
        Plain,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questWatches",
        "C_QuestLog.GetNumQuestWatches",
        Plain,
        Some("QUEST_WATCH_LIST_CHANGED"),
    ),
    stat(
        "questWatchID",
        "C_QuestLog.GetQuestIDForQuestWatchIndex",
        Index,
        Some("QUEST_WATCH_LIST_CHANGED"),
    ),
    stat(
        "questLogIndex",
        "C_QuestLog.GetLogIndexForQuestID",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questObjectives",
        "C_QuestLog.GetQuestObjectives",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questComplete",
        "C_QuestLog.IsComplete",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questWatchType",
        "C_QuestLog.GetQuestWatchType",
        Index,
        Some("QUEST_WATCH_LIST_CHANGED"),
    ),
    stat(
        "questTag",
        "C_QuestLog.GetQuestTagInfo",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questTitle",
        "C_QuestLog.GetTitleForQuestID",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    // The spellbook items' cooldowns, auto-cast and flyouts.
    stat(
        "spellBookItemCooldown",
        "C_SpellBook.GetSpellBookItemCooldown",
        Slot,
        Some("SPELL_UPDATE_COOLDOWN"),
    ),
    stat(
        "spellBookItemAutoCast",
        "C_SpellBook.GetSpellBookItemAutoCast",
        Slot,
        Some("PET_BAR_UPDATE"),
    ),
    stat(
        "spellBookItemFlyoutMember",
        "C_SpellBook.IsSpellBookItemLooseFlyoutMember",
        Slot,
        Some("SPELLS_CHANGED"),
    ),
    // The spellbook's flyouts: each one's name, description, slot count and
    // whether it is known; each slot's spell, its override, known, name,
    // slot spell (`/stdtest engine snapshot`).
    stat("flyoutInfo", "GetFlyoutInfo", Index, Some("SPELLS_CHANGED")),
    stat(
        "flyoutSlot",
        "GetFlyoutSlotInfo",
        FlyoutSlot,
        Some("SPELLS_CHANGED"),
    ),
    // Each logged quest's details (by quest ID; the special item by log
    // index).
    stat(
        "questFailed",
        "C_QuestLog.IsFailed",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questElite",
        "C_QuestLog.IsEliteQuest",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questDifficulty",
        "C_QuestLog.GetQuestDifficultyLevel",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questTheme",
        "C_QuestLog.GetQuestDetailsTheme",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questTimeAllowed",
        "C_QuestLog.GetTimeAllowed",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questDisabled",
        "C_QuestLog.IsQuestDisabledForSession",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questSequenced",
        "IsQuestSequenced",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questClassification",
        "C_QuestInfoSystem.GetQuestClassification",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questContentDifficulty",
        "C_PlayerInfo.GetContentDifficultyQuestForPlayer",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questGroup",
        "C_LFGList.CanCreateQuestGroup",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    stat(
        "questSpecialItem",
        "GetQuestLogSpecialItemInfo",
        Index,
        Some("QUEST_LOG_UPDATE"),
    ),
    // Each faction's kind (by faction ID).
    stat(
        "factionParagon",
        "C_Reputation.IsFactionParagonForCurrentPlayer",
        Index,
        Some("UPDATE_FACTION"),
    ),
    stat(
        "factionMajor",
        "C_Reputation.IsMajorFaction",
        Index,
        Some("UPDATE_FACTION"),
    ),
    stat(
        "factionAccountWide",
        "C_Reputation.IsAccountWideReputation",
        Index,
        Some("UPDATE_FACTION"),
    ),
    stat(
        "factionFriendship",
        "C_GossipInfo.GetFriendshipReputation",
        Index,
        Some("UPDATE_FACTION"),
    ),
    // The cooldown viewer's entries (by cooldown ID).
    stat(
        "cooldownViewerInfo",
        "C_CooldownViewer.GetCooldownViewerCooldownInfo",
        Index,
        Some("COOLDOWN_VIEWER_DATA_LOADED"),
    ),
    // The equipped gear's appearances.
    stat(
        "transmogSlot",
        "C_Transmog.GetSlotVisualInfo",
        Location,
        Some("TRANSMOGRIFY_UPDATE"),
    ),
    stat(
        "combatRating",
        "GetCombatRating",
        Index,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "combatRatingBonus",
        "GetCombatRatingBonus",
        Index,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "spellDamage",
        "GetSpellBonusDamage",
        Index,
        Some("SPELL_POWER_CHANGED"),
    ),
    stat(
        "versatility",
        "GetVersatilityBonus",
        Index,
        Some("COMBAT_RATING_UPDATE"),
    ),
    stat(
        "attackPowerForStat",
        "GetAttackPowerForStat",
        ForStat,
        Some("UNIT_STATS"),
    ),
    stat(
        "rangedAttackPowerForStat",
        "GetRangedAttackPowerForStat",
        ForStat,
        Some("UNIT_STATS"),
    ),
    stat(
        "critFromStat",
        "GetCritChanceFromStat",
        ForStat,
        Some("UNIT_STATS"),
    ),
    stat(
        "spellCritFromStat",
        "GetSpellCritChanceFromStat",
        ForStat,
        Some("UNIT_STATS"),
    ),
];

/// A recorded value.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    /// A structure (`{ baseSourceID = 0, … }`).
    Table(Vec<(Answer, Answer)>),
}

/// A stat's answer: one, or one per index.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    One(Vec<Answer>),
    ByIndex(BTreeMap<i64, Vec<Answer>>),
}

/// The player's stats, by key.
pub type Stats = BTreeMap<&'static str, Entry>;

/// The player's stats, or its pet's.
fn stats_of(vm: &Vm, pet: bool) -> &Stats {
    if pet {
        &engine(vm).world.pets.stats
    } else {
        &engine(vm).world.character.stats
    }
}

/// The default character's (stand-ins, see the module's note).
pub fn defaults() -> Stats {
    let mut s = Stats::new();
    let n = Answer::Num;
    s.insert(
        "damage",
        Entry::One(vec![n(0.0), n(0.0), n(0.0), n(0.0), n(0.0), n(0.0), n(1.0)]),
    );
    s.insert(
        "rangedDamage",
        Entry::One(vec![n(0.0), n(0.0), n(0.0), n(0.0), n(0.0), n(1.0)]),
    );
    s.insert(
        "attackSpeed",
        Entry::One(vec![n(2.0), Answer::Nil, Answer::Nil]),
    );
    // No combat ratings (documented nilable; the sheet adds to them).
    let none: BTreeMap<i64, Vec<Answer>> = (1..=31).map(|i| (i, vec![n(0.0)])).collect();
    s.insert("combatRating", Entry::ByIndex(none.clone()));
    s.insert("combatRatingBonus", Entry::ByIndex(none));
    // No spell power (nilable too).
    let schools: BTreeMap<i64, Vec<Answer>> = (1..=7).map(|i| (i, vec![n(0.0)])).collect();
    s.insert("spellDamage", Entry::ByIndex(schools));
    // Overall, equipped and PvP (undocumented).
    s.insert("itemLevel", Entry::One(vec![n(0.0), n(0.0), n(0.0)]));
    // One talent group, active.
    s.insert("specGroups", Entry::One(vec![n(1.0)]));
    s.insert("activeSpecGroup", Entry::One(vec![n(1.0)]));
    // Not rested.
    s.insert("restedXP", Entry::One(vec![Answer::Nil]));
    s.insert(
        "restState",
        Entry::One(vec![n(2.0), Answer::Str("Normal".into()), n(1.0)]),
    );
    s
}

fn answer_of(vm: &Vm, v: Value) -> Answer {
    match vm.hidden(v) {
        Value::Bool(b) => Answer::Bool(b),
        Value::Num(n) => Answer::Num(n),
        Value::Str(s) => Answer::Str(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        Value::Table(t) => Answer::Table(
            vm.heap
                .table(t)
                .entries()
                .map(|(k, v)| (answer_of(vm, k), answer_of(vm, v)))
                .collect(),
        ),
        _ => Answer::Nil,
    }
}

fn value_of(vm: &mut Vm, a: &Answer) -> Value {
    match a {
        Answer::Nil => Value::Nil,
        Answer::Bool(b) => Value::Bool(*b),
        Answer::Num(n) => Value::Num(*n),
        Answer::Str(s) => vm.str(s),
        // A fresh table each time (callers change what they get).
        Answer::Table(entries) => {
            let t = vm.new_table();
            for (k, v) in entries {
                let k = value_of(vm, k);
                let v = value_of(vm, v);
                let _ = vm.heap.table_mut(t).set(k, v);
            }
            Value::Table(t)
        }
    }
}

/// A list of values (`{ 67, 67, 22, 0 }`; with nils, its length in `n`),
/// or one value.
fn answers_of(vm: &mut Vm, v: Value) -> Vec<Answer> {
    match v {
        Value::Table(t) => {
            let key = vm.str("n");
            let n = match vm.heap.table(t).get(key) {
                Value::Num(n) if n >= 0.0 => n as usize,
                _ => vm.heap.table(t).border(),
            };
            // A structure: the one value.
            if n == 0 && vm.heap.table(t).entries().next().is_some() {
                return vec![answer_of(vm, v)];
            }
            (1..=n)
                .map(|i| answer_of(vm, vm.heap.table(t).get(Value::Num(i as f64))))
                .collect()
        }
        other => vec![answer_of(vm, other)],
    }
}

/// The answers for the indexed stat in `t` (`{ [1] = {…}, [2] = {…} }`).
fn by_index(vm: &mut Vm, t: TableRef) -> BTreeMap<i64, Vec<Answer>> {
    let entries: Vec<(Value, Value)> = vm.heap.table(t).entries().collect();
    entries
        .into_iter()
        .filter_map(|(k, v)| match k {
            Value::Num(i) if i.fract() == 0.0 => Some((i as i64, answers_of(vm, v))),
            _ => None,
        })
        .collect()
}

/// `mock.player:setStats(stats)`: sets the stats given (by key; a value
/// or the function's answer as a list; indexed stats by index), each
/// change told by its event (unverified). The player's pet's too
/// (`mock.setPet`): the unit's own stats only (unverified).
pub fn control_set_stats(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let pet = me != units::PLAYER;
    if pet && engine(vm).units.pet != Some(me) {
        return Err(vm.error("unit:setStats: only the player and its pet have that"));
    }
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("unit:setStats: a table of stats expected"));
    };
    let mut events: Vec<&'static str> = Vec::new();
    let entries: Vec<(Value, Value)> = vm.heap.table(t).entries().collect();
    for (k, v) in entries {
        let key = match k {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => return Err(vm.error("unit:setStats: stats are named by their keys")),
        };
        let Some(stat) = STATS.iter().find(|s| s.key == key) else {
            return Err(vm.error(&format!("unit:setStats: no stat {key:?}")));
        };
        if pet && !matches!(stat.ask, Unit | UnitIndex) {
            return Err(vm.error(&format!("unit:setStats: {key} is the player's")));
        }
        let entry = match (stat.ask, v) {
            (UnitIndex | Index | ForStat | Location | Slot | FlyoutSlot, Value::Table(inner)) => {
                let mut map = match stats_of(vm, pet).get(stat.key) {
                    Some(Entry::ByIndex(m)) => m.clone(),
                    _ => BTreeMap::new(),
                };
                map.extend(by_index(vm, inner));
                Entry::ByIndex(map)
            }
            (UnitIndex | Index | ForStat | Location | Slot | FlyoutSlot, _) => {
                return Err(vm.error(&format!("unit:setStats: {key} is by index ({{ [1] = … }})")));
            }
            (_, v) => Entry::One(answers_of(vm, v)),
        };
        if pet {
            engine_mut(vm).world.pets.stats.insert(stat.key, entry);
        } else {
            engine_mut(vm).world.character.stats.insert(stat.key, entry);
        }
        if let Some(e) = stat.event
            && !events.contains(&e)
        {
            events.push(e);
        }
    }
    for event in events {
        let unit_event = event.starts_with("UNIT_") || event.ends_with("XP_UPDATE");
        if unit_event {
            // For each token naming the player (`targettarget` too:
            // `/stdtest engine scenario combat`), or the pet.
            units::fire_unit(vm, event, me, &[]);
        } else {
            widgets::fire_event(vm, event, &[]);
        }
    }
    Ok(vec![])
}

/// The player's experience, gained `amount`: `UnitXP` grows,
/// `PLAYER_XP_UPDATE` for each of the player's tokens; past the level's
/// end, a defined level gained (`levels.rs`; unverified without one).
pub fn gain_xp(vm: &mut Vm, amount: f64) {
    let now = match engine(vm).world.character.stats.get("xp") {
        Some(Entry::One(a)) => a.first().and_then(|a| match a {
            Answer::Num(n) => Some(*n),
            _ => None,
        }),
        _ => None,
    }
    .unwrap_or(0.0);
    if crate::levels::reaches(vm, now + amount) {
        crate::levels::level_up(vm, now + amount);
        return;
    }
    engine_mut(vm)
        .world
        .character
        .stats
        .insert("xp", Entry::One(vec![Answer::Num(now + amount)]));
    units::fire_unit(vm, "PLAYER_XP_UPDATE", units::PLAYER, &[]);
}

/// Whether the spellbook has an ability by its name (`"Fireball (Rank
/// 1)"`: its name and subtext; `"Parry"`: its name).
pub fn spellbook_has_named(vm: &Vm, ability: &str) -> bool {
    let Some(Entry::ByIndex(names)) = engine(vm).world.character.stats.get("spellBookItemName")
    else {
        return false;
    };
    names.values().any(|n| match n.as_slice() {
        [Answer::Str(name), rest @ ..] => {
            let sub = match rest.first() {
                Some(Answer::Str(s)) if !s.is_empty() => Some(s.as_str()),
                _ => None,
            };
            ability == name || sub.is_some_and(|sub| ability == format!("{name} ({sub})"))
        }
        _ => false,
    })
}

/// A skill line's rank (`C_SkillInfo.GetSkillLineInfo`'s entry with that
/// `skillID`).
/// The active spec group's combat config
/// (`C_SpecializationInfo.GetCombatConfigIDForSpecGroup`), if the
/// character's stats have it.
pub fn active_combat_config(vm: &Vm) -> Option<f64> {
    let stats = &engine(vm).world.character.stats;
    let group = match stats.get("activeSpecGroup") {
        Some(Entry::One(v)) => match v.first() {
            Some(Answer::Num(n)) => *n as i64,
            _ => 1,
        },
        _ => 1,
    };
    match stats.get("combatConfig") {
        Some(Entry::ByIndex(by)) => match by.get(&group).and_then(|v| v.first()) {
            Some(Answer::Num(n)) => Some(crate::traits::player_config(vm, *n)),
            _ => None,
        },
        _ => None,
    }
}

/// `C_SpecializationInfo.GetCombatConfigIDForSpecGroup(group)`: the
/// stats' config of the spec group, as the talent model has it
/// (`traits::player_config`); none for a group without one.
fn combat_config(vm: &mut Vm, c: NativeCall) -> Ret {
    let mut values = answer(vm, c)?;
    if let Some(Value::Num(n)) = values.first().copied() {
        values[0] = Value::Num(crate::traits::player_config(vm, n));
    }
    Ok(values)
}

pub fn skill_rank(vm: &Vm, skill: i64) -> Option<f64> {
    let Some(Entry::ByIndex(lines)) = engine(vm).world.character.stats.get("skillLine") else {
        return None;
    };
    lines.values().flatten().find_map(|a| match a {
        Answer::Table(fields) if table_num(fields, "skillID") == Some(skill as f64) => {
            table_num(fields, "rank")
        }
        _ => None,
    })
}

fn table_num(fields: &[(Answer, Answer)], key: &str) -> Option<f64> {
    fields.iter().find_map(|(k, v)| match (k, v) {
        (Answer::Str(k), Answer::Num(n)) if k == key => Some(*n),
        _ => None,
    })
}

/// Skill lines raised to `rank`: their entries, and a profession's
/// (`GetProfessionInfo`'s rank, third, for its line, seventh).
pub fn raise_skill_lines(vm: &mut Vm, skills: &[i64], rank: f64) {
    let stats = &mut engine_mut(vm).world.character.stats;
    if let Some(Entry::ByIndex(lines)) = stats.get_mut("skillLine") {
        for a in lines.values_mut().flatten() {
            if let Answer::Table(fields) = a
                && table_num(fields, "skillID").is_some_and(|id| skills.contains(&(id as i64)))
            {
                for (k, v) in fields.iter_mut() {
                    if matches!(k, Answer::Str(k) if k == "rank") {
                        *v = Answer::Num(rank);
                    }
                }
            }
        }
    }
    if let Some(Entry::ByIndex(professions)) = stats.get_mut("professionInfo") {
        for info in professions.values_mut() {
            if matches!(info.get(6), Some(Answer::Num(id)) if skills.contains(&(*id as i64)))
                && info.len() > 2
            {
                info[2] = Answer::Num(rank);
            }
        }
    }
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// A transmog location's key: `slotID * 100 + type * 10 + modification`.
fn location_key(vm: &mut Vm, v: Value) -> Option<f64> {
    let Value::Table(t) = v else {
        return None;
    };
    let mut get = |k: &str| {
        let key = vm.str(k);
        num(vm.heap.table(t).get(key))
    };
    Some(
        get("slotID")? * 100.0
            + get("type").unwrap_or(0.0) * 10.0
            + get("modification").unwrap_or(0.0),
    )
}

/// The documented empty values of `name` (no stat for this call).
fn empty(vm: &mut Vm, name: &str) -> Vec<Value> {
    // The client's answer with nothing there, if the full probe settled it.
    if let Some(answer) = crate::blanks::answer(name) {
        return crate::recorded::parse_answer(vm, answer);
    }
    let api = engine(vm).api;
    match api.functions.iter().find(|f| f.name == name) {
        Some(f) => crate::defaults(vm, api, &f.doc),
        None => Vec::new(),
    }
}

/// A flyout's name, from the spellbook item that holds it (its
/// `actionID`).
fn flyout_name(vm: &Vm, id: f64) -> Option<String> {
    let Some(Entry::ByIndex(items)) = engine(vm).world.character.stats.get("spellBookItem") else {
        return None;
    };
    items.values().find_map(|answer| match answer.first() {
        Some(Answer::Table(fields)) => {
            let field = |name: &str| {
                fields.iter().find_map(|(k, v)| match k {
                    Answer::Str(k) if k == name => Some(v),
                    _ => None,
                })
            };
            let flyout = matches!(field("itemType"), Some(Answer::Num(t)) if *t == 4.0)
                && matches!(field("actionID"), Some(Answer::Num(a)) if *a == id);
            match field("name") {
                Some(Answer::Str(n)) if flyout => Some(n.clone()),
                _ => None,
            }
        }
        _ => None,
    })
}

/// A cooldown viewer category's set without `allowUnlearned`: only the
/// cooldowns the player knows (their recorded info's `isKnown`), in order
/// (the full probe, 1.60.1: `GetCooldownViewerCategorySet(0)`, `(0,
/// false)` and `(0, nil)` 4 of the 12 `(0, true)` gives, every category
/// as its recorded entries' `isKnown` says).
fn known_cooldowns(vm: &Vm, values: Vec<Answer>) -> Vec<Answer> {
    let Some(Entry::ByIndex(info)) = engine(vm).world.character.stats.get("cooldownViewerInfo")
    else {
        return values;
    };
    let known = |id: &Answer| {
        let Answer::Num(id) = id else {
            return false;
        };
        matches!(info.get(&(*id as i64)).and_then(|a| a.first()), Some(Answer::Table(fields))
        if fields.iter().any(|(k, v)| {
            matches!(k, Answer::Str(k) if k == "isKnown") && *v == Answer::Bool(true)
        }))
    };
    values
        .into_iter()
        .map(|v| match v {
            Answer::Table(entries) => {
                let mut ids: Vec<(f64, Answer)> = entries
                    .into_iter()
                    .filter_map(|(k, v)| match k {
                        Answer::Num(i) => Some((i, v)),
                        _ => None,
                    })
                    .collect();
                ids.sort_by(|a, b| a.0.total_cmp(&b.0));
                Answer::Table(
                    ids.into_iter()
                        .map(|(_, v)| v)
                        .filter(known)
                        .enumerate()
                        .map(|(i, v)| (Answer::Num(i as f64 + 1.0), v))
                        .collect(),
                )
            }
            other => other,
        })
        .collect()
}

/// A stat's function: the player's recorded answer.
fn answer(vm: &mut Vm, c: NativeCall) -> Ret {
    // The function called: its index in the API, or (undocumented) name.
    let name = match c.upvals.first().copied() {
        Some(Value::Num(i)) => engine(vm)
            .api
            .functions
            .get(i as usize)
            .map(|f| f.name.to_string()),
        Some(Value::Str(s)) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    };
    let Some(stat) = name.and_then(|n| STATS.iter().find(|s| s.function == n)) else {
        return Ok(vec![]);
    };
    // An inspected unit's or pet's groups: none (the full probe:
    // `GetNumSpecGroups(isInspect)` 0 with any argument, 1 without).
    if stat.function == "GetNumSpecGroups" && !c.arg(1).is_nil() {
        return Ok(vec![Value::Num(0.0)]);
    }
    // A spellbook item's link with a glyph: nothing (the full probe:
    // glyph 1, 1.5 and 2147483647 nothing, -1, 0 and nil the link;
    // unverified: a glyph the spell takes).
    if stat.function == "C_SpellBook.GetSpellBookItemLink"
        && matches!(c.arg(3), Value::Num(g) if g > 0.0)
    {
        return Ok(vec![]);
    }
    if stat.key == "specializationInfo"
        && let Some(values) = other_specialization(vm, &c)
    {
        return Ok(values);
    }
    // The unit asked about: the player, or its pet (its own stats).
    let asked = units::unit_arg(vm, &c, 1);
    let pet = matches!(stat.ask, Unit | UnitIndex)
        && asked.is_some()
        && asked == engine(vm).units.pet
        && asked != Some(units::PLAYER);
    let player = match stat.ask {
        Unit | UnitIndex => asked == Some(units::PLAYER) || pet,
        _ => true,
    };
    let index = match stat.ask {
        // An enumeration's index not a number reads as 0 (the full probe:
        // `UnitResistance("player", "x")` answered as 0's armor).
        UnitIndex => num(c.arg(2)).or(matches!(c.arg(2), Value::Str(_)).then_some(0.0)),
        Index | ForStat => num(c.arg(1)),
        Location => location_key(vm, c.arg(1)),
        FlyoutSlot => num(c.arg(1)).zip(num(c.arg(2))).map(|(f, i)| f * 100.0 + i),
        // The player's bank (0) only.
        Slot if matches!(c.arg(2), Value::Num(b) if b == 0.0) => num(c.arg(1)),
        _ => None,
    };
    // An index is truncated (the full probe, 1.60.1: the spellbook's item
    // and skill line 1.5 answered as 1).
    let index = index.map(f64::trunc);
    // The cooldown viewer's category, an enumeration's value: a non-number
    // read as 0 (the full probe: `GetCooldownViewerCategorySet("x", true)`
    // answered category 0's).
    let index = match stat.key {
        "cooldownViewerSet" => index.or(Some(0.0)),
        _ => index,
    };
    // A spellbook item's link with a glyph above 0: none (the full probe,
    // 1.60.1: glyph 1, 1.5 and 2147483647 nothing, -1 the link).
    if stat.key == "spellBookItemLink" && matches!(c.arg(3), Value::Num(g) if g > 0.0) {
        return Ok(vec![]);
    }
    let found = if player {
        let ch = &engine(vm).world.character;
        let stats = if pet {
            &engine(vm).world.pets.stats
        } else {
            &ch.stats
        };
        match (stats.get(stat.key), index) {
            (Some(Entry::One(v)), None) => Some(v.clone()),
            // An index with a fraction as its whole part (the full probe:
            // `GetCombatRating(1.5)`, `C_SpellBook.GetSpellBookItemInfo(1.5,
            // 0)` answered as with 1).
            (Some(Entry::ByIndex(m)), Some(i)) => {
                let v = m.get(&(i.trunc() as i64)).cloned();
                // For the attribute's effective value only (as the sheet
                // asks it); unverified for others.
                if stat.ask == ForStat {
                    let effective = match ch.stats.get("stat") {
                        Some(Entry::ByIndex(s)) => s.get(&(i as i64)).and_then(|a| a.get(1)),
                        _ => None,
                    };
                    match (effective, num(c.arg(2))) {
                        (Some(Answer::Num(e)), Some(value)) if *e == value => v,
                        // Another value: the recorded one's rate per point,
                        // the value taken whole, none below 0 (the full
                        // probe: strength 67 recorded 134, and 1 and 1.5
                        // answer 2, 2147483647 4294967294, -1 and 0 0;
                        // unverified for a rate not whole).
                        (Some(Answer::Num(e)), Some(value)) if *e > 0.0 => match v.as_deref() {
                            Some([Answer::Num(at)]) => {
                                Some(vec![Answer::Num(at / e * value.trunc().max(0.0))])
                            }
                            _ => None,
                        },
                        _ => None,
                    }
                } else {
                    v
                }
            }
            _ => None,
        }
    } else {
        None
    };
    let found = match found {
        Some(values) if stat.key == "cooldownViewerSet" && !c.arg(2).truthy() => {
            Some(known_cooldowns(vm, values))
        }
        other => other,
    };
    match found {
        Some(values) => Ok(values.iter().map(|a| value_of(vm, a)).collect()),
        // A slot without a recorded appearance: none (stand-in).
        None if stat.ask == Location && player => {
            let api = engine(vm).api;
            Ok(
                match api.functions.iter().find(|f| f.name == stat.function) {
                    Some(f) => crate::documented_values(vm, api, &f.doc),
                    None => Vec::new(),
                },
            )
        }
        // A flyout not recorded: its spellbook item's name, no slots
        // (unverified; the snapshot records flyouts since this).
        None if stat.key == "flyoutInfo" && player => {
            let id = num(c.arg(1));
            let name = id.and_then(|id| flyout_name(vm, id));
            crate::note_default(vm, stat.function, &c, 1);
            // One the spellbook lacks: no name, no slots, unknown (the
            // client knows every flyout; unverified).
            Ok(match name {
                Some(n) => vec![vm.str(&n), vm.str(""), Value::Num(0.0), Value::Bool(true)],
                None => vec![vm.str(""), vm.str(""), Value::Num(0.0), Value::Bool(false)],
            })
        }
        // No forms without a snapshot's (the function is undocumented, so
        // without a documented empty answer).
        None if stat.key == "shapeshiftForms" => Ok(vec![Value::Num(0.0)]),
        None => Ok(match unrecorded(vm, stat.key, index) {
            Some(values) => values,
            None => empty(vm, stat.function),
        }),
    }
}

/// A quest's or faction's stat for one the profile does not hold, as the
/// client answers any (the full probe, every number asked): a quest's
/// classification `Normal` (7), its difficulty for the player `Fair`
/// (`Enum.RelativeContentDifficulty`, 2); a faction's friendship (an ID
/// from 1) a friendship of none, all empty. Unverified: a real quest's or
/// faction's own (game data the engine lacks).
fn unrecorded(vm: &mut Vm, key: &str, index: Option<f64>) -> Option<Vec<Value>> {
    Some(match key {
        // Even without a quest (`GetQuestClassification(nil, 1)`: 7).
        "questClassification" => vec![Value::Num(7.0)],
        "questContentDifficulty" => vec![Value::Num(2.0)],
        "factionFriendship" if index? >= 1.0 => {
            let t = vm.new_table();
            for (k, v) in [
                ("friendshipFactionID", Value::Num(0.0)),
                ("maxRep", Value::Num(0.0)),
                ("reaction", vm.str("")),
                ("reactionThreshold", Value::Num(0.0)),
                ("reversedColor", Value::Bool(false)),
                ("standing", Value::Num(0.0)),
                ("text", vm.str("")),
                ("texture", Value::Num(0.0)),
            ] {
                let key = vm.str(k);
                let _ = vm.heap.table_mut(t).set(key, v);
            }
            vec![Value::Table(t)]
        }
        _ => return None,
    })
}

/// `C_SpecializationInfo.GetSpecializationInfo(index, isInspect, isPet,
/// …)` for another than the player's recorded specs (the full probe,
/// 1.60.1): inspecting nobody, or an index without a spec, gives the empty
/// spec (ID 0, no texts, recommended); the pet's first spec is the
/// stable's "Default" (`C_StableInfo.GetAvailablePetSpecInfos`' 1493),
/// asked even without a pet. Unverified: a hunter's pet's other specs.
fn other_specialization(vm: &mut Vm, c: &NativeCall) -> Option<Vec<Value>> {
    let empty = || {
        vec![
            Value::Num(0.0),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Num(0.0),
            Value::Nil,
            Value::Num(0.0),
            Value::Bool(true),
        ]
    };
    let index = num(c.arg(1)).map(f64::trunc);
    if c.arg(2).truthy() {
        return Some(empty());
    }
    if c.arg(3).truthy() {
        if index != Some(1.0) {
            return Some(empty());
        }
        return Some(vec![
            Value::Num(1493.0),
            vm.str("Default"),
            vm.str("I'm a pet!"),
            Value::Num(6642732.0),
            vm.str("DAMAGER"),
            Value::Num(2.0),
            Value::Num(0.0),
            Value::Nil,
            Value::Num(0.0),
            Value::Bool(true),
        ]);
    }
    let recorded = match engine(vm).world.character.stats.get("specializationInfo") {
        Some(Entry::ByIndex(m)) => index.is_some_and(|i| m.contains_key(&(i as i64))),
        _ => true,
    };
    (!recorded).then(empty)
}

/// The player's spellbook slot holding `spell` (the recorded items'
/// `spellID`).
pub fn spellbook_slot_of(vm: &Vm, spell: f64) -> Option<i64> {
    let Some(Entry::ByIndex(items)) = engine(vm).world.character.stats.get("spellBookItem") else {
        return None;
    };
    items
        .iter()
        .find_map(|(slot, answer)| match answer.first() {
            Some(Answer::Table(fields)) => fields
                .iter()
                .any(|(k, v)| {
                    matches!(k, Answer::Str(k) if k == "spellID")
                        && matches!(v, Answer::Num(n) if *n == spell)
                })
                .then_some(*slot),
            _ => None,
        })
}

/// A spell's cooldown running when the snapshot was taken (its spellbook
/// item's `spellBookItemCooldown`): start and duration on the client's
/// clock, which the engine's is.
pub fn recorded_cooldown(vm: &Vm, spell: f64) -> Option<(f64, f64)> {
    let slot = spellbook_slot_of(vm, spell)?;
    let Some(Entry::ByIndex(items)) = engine(vm)
        .world
        .character
        .stats
        .get("spellBookItemCooldown")
    else {
        return None;
    };
    let Some(Answer::Table(fields)) = items.get(&slot)?.first() else {
        return None;
    };
    let get = |name: &str| {
        fields.iter().find_map(|(k, v)| match (k, v) {
            (Answer::Str(k), Answer::Num(n)) if k == name => Some(*n),
            _ => None,
        })
    };
    let (start, duration) = (get("startTime")?, get("duration")?);
    // Its time left at the snapshot, counted from the snapshot's moment
    // when the test told it (`snapshotTaken`: the engine's clock need not
    // be the client's; recorded: a warrior's spell off its cooldown 59.6 s
    // into the quest's recording, as the snapshot's time left said).
    let start = match (
        engine(vm).world.snapshot_at,
        get("timeUntilEndOfStartRecovery"),
    ) {
        (Some(at), Some(left)) => at + left - duration,
        _ => start,
    };
    (duration > 0.0).then_some((start, duration))
}

/// The recorded spellbook item of `spell` (its `GetSpellBookItemInfo`).
pub fn spellbook_item(vm: &mut Vm, spell: f64) -> Option<Value> {
    let slot = spellbook_slot_of(vm, spell)?;
    let Some(Entry::ByIndex(items)) = engine(vm).world.character.stats.get("spellBookItem") else {
        return None;
    };
    let answer = items.get(&slot)?.first()?.clone();
    Some(value_of(vm, &answer))
}

/// The recorded spellbook item in the player's `slot`.
pub fn spellbook_item_at(vm: &mut Vm, slot: i64) -> Option<Value> {
    let Some(Entry::ByIndex(items)) = engine(vm).world.character.stats.get("spellBookItem") else {
        return None;
    };
    let answer = items.get(&slot)?.first()?.clone();
    Some(value_of(vm, &answer))
}

/// `C_SpellBook.FindSpellBookSlotForSpell(spellID, …)`: its slot and bank
/// (the player's); nothing for a spell not in the book.
///
/// A spell's name finds its spell (the full probe, 1.60.1: "Attack" slot 1).
fn find_spellbook_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    let spell = match c.arg(1) {
        v @ Value::Str(_) => crate::spellinfo::spell_of(vm, v),
        v => num(v),
    };
    Ok(match spell.and_then(|s| spellbook_slot_of(vm, s)) {
        Some(slot) => vec![Value::Num(slot as f64), Value::Num(0.0)],
        None => vec![],
    })
}

/// `C_SpellBook.IsSpellInSpellBook(spellID [, bank])`: in the player's
/// book (unverified: overrides, the pet's bank).
pub(crate) fn is_spell_in_spellbook(vm: &mut Vm, c: NativeCall) -> Ret {
    let pet = matches!(c.arg(2), Value::Num(b) if b == 1.0);
    let found = !pet
        && num(c.arg(1))
            .and_then(|s| spellbook_slot_of(vm, s))
            .is_some();
    Ok(vec![Value::Bool(found)])
}

/// A recorded table's field.
fn field_of<'a>(answer: &'a Answer, key: &str) -> Option<&'a Answer> {
    match answer {
        Answer::Table(fields) => fields
            .iter()
            .find(|(k, _)| matches!(k, Answer::Str(k) if k == key))
            .map(|(_, v)| v),
        _ => None,
    }
}

/// The quest at a log index (its recorded `C_QuestLog.GetInfo`'s ID), and
/// its recorded objectives.
fn quest_objectives(vm: &Vm, log_index: f64) -> Option<Vec<Answer>> {
    let stats = &engine(vm).world.character.stats;
    let Some(Entry::ByIndex(log)) = stats.get("questLogInfo") else {
        return None;
    };
    let info = log.get(&(log_index as i64))?.first()?;
    let Answer::Num(quest) = field_of(info, "questID")? else {
        return None;
    };
    let quest = *quest;
    let Some(Entry::ByIndex(objectives)) = stats.get("questObjectives") else {
        return None;
    };
    match objectives.get(&(quest as i64))?.first()? {
        Answer::Table(entries) => {
            let mut list: Vec<(f64, Answer)> = entries
                .iter()
                .filter_map(|(k, v)| match k {
                    Answer::Num(i) => Some((*i, v.clone())),
                    _ => None,
                })
                .collect();
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
            Some(list.into_iter().map(|(_, v)| v).collect())
        }
        _ => None,
    }
}

/// `C_QuestLog.GetQuestIDForLogIndex(logIndex)`: the entry's quest ID (from
/// the recorded log; a header's is 0).
fn quest_id_for_log_index(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(c.arg(1)).and_then(|i| {
        let Some(Entry::ByIndex(log)) = engine(vm).world.character.stats.get("questLogInfo") else {
            return None;
        };
        match field_of(log.get(&(i as i64))?.first()?, "questID")? {
            Answer::Num(id) => Some(*id),
            _ => None,
        }
    });
    // No entry there: 0 (the full probe: index 0 and 2147483647).
    Ok(vec![Value::Num(id.unwrap_or(0.0))])
}

/// `GetNumQuestLeaderBoards(logIndex)`: the quest's objectives, derived
/// from the recorded log and objectives (as the client gave them for every
/// recorded entry; `quest_objectives_derive_as_the_client_gave_them`).
fn num_quest_leader_boards(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = num(c.arg(1))
        .and_then(|i| quest_objectives(vm, i))
        .map_or(0, |o| o.len());
    Ok(vec![Value::Num(n as f64)])
}

/// `GetQuestLogLeaderBoard(i, logIndex)`: the objective's text, type and
/// whether it is finished (derived as `GetNumQuestLeaderBoards`).
fn quest_log_leader_board(vm: &mut Vm, c: NativeCall) -> Ret {
    // No such objective: three nils (the full probe).
    let none = || Ok(vec![Value::Nil; 3]);
    let (Some(i), Some(log)) = (num(c.arg(1)), num(c.arg(2))) else {
        return none();
    };
    let Some(objective) =
        quest_objectives(vm, log).and_then(|o| o.get((i as usize).wrapping_sub(1)).cloned())
    else {
        return none();
    };
    Ok(["text", "type", "finished"]
        .iter()
        .map(|k| field_of(&objective, k).map_or(Value::Nil, |a| value_of(vm, a)))
        .collect())
}

/// The level talents open at (the client's answer below it: "This feature
/// becomes available at level 5.", the default character's at level 1;
/// every recorded character above it: available).
const TALENT_LEVEL: f64 = 5.0;

/// `C_SpecializationInfo.CanPlayerUseTalentUI()` and
/// `CanPlayerUseTalentSpecUI()`: from level 5, as the client's answer
/// says (the stat a profile recorded is the level it was recorded at).
fn talent_ui(vm: &mut Vm, _c: NativeCall) -> Ret {
    let level = engine(vm).units.list[crate::units::PLAYER.0].level;
    Ok(if level >= TALENT_LEVEL {
        vec![Value::Bool(true), vm.str("")]
    } else {
        let why = vm.str("This feature becomes available at level 5.");
        vec![Value::Bool(false), why]
    })
}

/// A global table's field (`Constants.LevelConstsExposed`, say).
fn global_path(vm: &mut Vm, path: &[&str]) -> Value {
    let mut v = vm.get_global(path[0]);
    for k in &path[1..] {
        let Value::Table(t) = v else {
            return Value::Nil;
        };
        let key = vm.str(k);
        v = vm.heap.table(t).get(key);
    }
    v
}

/// The level the player earns talent points from: the client's first
/// talent level (`Constants.LevelConstsExposed.MIN_TALENT_LEVEL`, 10),
/// lowered by the legacy adventure tree's "Talented" ranks, at least 1
/// (Blizzard's camelot micro menu reckons it so; the ranks' descriptions,
/// recorded: "You gain talent points every level starting at level 9
/// instead of starting at level 10", down to 5).
pub(crate) fn talent_unlock_level(vm: &mut Vm) -> f64 {
    let num = |v: Value, d: f64| match v {
        Value::Num(n) => n,
        _ => d,
    };
    let first = global_path(vm, &["Constants", "LevelConstsExposed", "MIN_TALENT_LEVEL"]);
    let first = num(first, 10.0);
    let tree = global_path(
        vm,
        &["Constants", "LegacyConsts", "LEGACY_TREE_ADVENTURE_ID"],
    );
    let node = global_path(
        vm,
        &[
            "Constants",
            "LegacyConsts",
            "LEGACY_TREE_ADVENTURE_TALENTED_NODE_ID",
        ],
    );
    let config_of = global_path(vm, &["C_Traits", "GetConfigIDByTreeID"]);
    let node_info = global_path(vm, &["C_Traits", "GetNodeInfo"]);
    let rank = (|| {
        if tree.is_nil() || node.is_nil() || config_of.is_nil() || node_info.is_nil() {
            return None;
        }
        let config = *vm.call(config_of, &[tree]).ok()?.first()?;
        if config.is_nil() {
            return None;
        }
        let Value::Table(info) = *vm.call(node_info, &[config, node]).ok()?.first()? else {
            return None;
        };
        let key = vm.str("activeRank");
        match vm.heap.table(info).get(key) {
            Value::Num(n) => Some(n),
            _ => None,
        }
    })()
    .unwrap_or(0.0);
    (first - rank).max(1.0)
}

/// The most talent points a character earns (the ranks' descriptions,
/// recorded: "you still may not have more than 51 total talent points").
const MAX_TALENT_POINTS: f64 = 51.0;

/// The talent points the player has earned: one a level from the unlock
/// level, at most 51 (decision T330).
pub(crate) fn talent_points(vm: &mut Vm) -> f64 {
    let level = engine(vm).units.list[crate::units::PLAYER.0].level;
    let unlock = talent_unlock_level(vm);
    (level - unlock + 1.0).clamp(0.0, MAX_TALENT_POINTS)
}

/// `C_SpecializationInfo.HasPlayerEarnedATalentPoint()`: whether the
/// player's level has reached the talent unlock level (the recorded
/// answer is the level it was recorded at).
fn earned_talent_point(vm: &mut Vm, _c: NativeCall) -> Ret {
    let level = engine(vm).units.list[crate::units::PLAYER.0].level;
    let unlock = talent_unlock_level(vm);
    Ok(vec![Value::Bool(level >= unlock)])
}

/// `C_SkillInfo.SetSelectedSkill(index)`: the skill line `GetSelectedSkill`
/// answers from then (Blizzard's skills window selects its first skill so,
/// and shows its details; unverified: an index naming no line, and whether
/// a change is told).
fn set_selected_skill(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(n) = vm.hidden(c.arg(1)) else {
        return Ok(vec![]);
    };
    engine_mut(vm)
        .world
        .character
        .stats
        .insert("selectedSkill", Entry::One(vec![Answer::Num(n.trunc())]));
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    STATS
        .iter()
        .map(|s| (s.function, answer as NativeFn))
        .chain([
            (
                "C_SpellBook.FindSpellBookSlotForSpell",
                find_spellbook_slot as NativeFn,
            ),
            ("C_SpellBook.IsSpellInSpellBook", is_spell_in_spellbook),
            ("C_SkillInfo.SetSelectedSkill", set_selected_skill),
            ("GetNumQuestLeaderBoards", num_quest_leader_boards),
            ("C_QuestLog.GetQuestIDForLogIndex", quest_id_for_log_index),
            ("GetQuestLogLeaderBoard", quest_log_leader_board),
            ("C_SpecializationInfo.CanPlayerUseTalentUI", talent_ui),
            ("C_SpecializationInfo.CanPlayerUseTalentSpecUI", talent_ui),
            (
                "C_SpecializationInfo.HasPlayerEarnedATalentPoint",
                earned_talent_point,
            ),
            (
                "C_SpecializationInfo.GetCombatConfigIDForSpecGroup",
                combat_config,
            ),
        ])
        .collect()
}
