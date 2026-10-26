//! Spells as the client's tables have them (spec debugger §18.10, decision
//! T391): a spell's name, rank, icon, cast time, range, cooldowns, costs
//! and description, from `SpellName`, `Spell`, `SpellMisc` and the tables
//! they point at. Values for the base difficulty (0) only.

use crate::Casc;
use crate::spelltext::{self, Caster, Data, Effect};
use crate::tables::{Row, Table};

/// The spell tables of a build, opened together.
pub struct SpellTables {
    name: Table,
    spell: Table,
    misc: Table,
    effect: Table,
    cast_times: Option<Table>,
    range: Option<Table>,
    duration: Option<Table>,
    radius: Option<Table>,
    levels: Option<Table>,
    aura: Option<Table>,
    cooldowns: Option<Table>,
    power: Option<Table>,
    shapeshift: Option<Table>,
    forms: Option<Table>,
    totems: Option<Table>,
    totem_kinds: Option<Table>,
    reagents: Option<Table>,
    equipped: Option<Table>,
    subclasses: Option<Table>,
}

/// A spell's power cost (`SpellPower`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cost {
    /// `Enum.PowerType`.
    pub power_type: i32,
    /// The least it costs.
    pub amount: u32,
    /// What it can take beyond that (a finisher's combo points).
    pub optional: u32,
    /// Of the base power, in percent (0 when a fixed amount).
    pub percent: f32,
}

impl Cost {
    /// The cost as the client's API shows it (`cost`: the most it takes),
    /// rage and runic power in whole points (the tables have tenths).
    pub fn shown(&self) -> u32 {
        self.scaled(self.amount + self.optional)
    }

    /// The least it takes, likewise (`minCost`).
    pub fn shown_min(&self) -> u32 {
        self.scaled(self.amount)
    }

    fn scaled(&self, n: u32) -> u32 {
        match self.power_type {
            1 | 6 => n / 10,
            _ => n,
        }
    }
}

/// A spell's facts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpellInfo {
    pub name: String,
    /// The rank ("Rank 2"), empty for none.
    pub subtext: String,
    pub icon: Option<u32>,
    pub cast_ms: u32,
    pub min_range: f32,
    pub max_range: f32,
    pub cooldown_ms: u32,
    pub category_cooldown_ms: u32,
    /// The global cooldown it starts.
    pub gcd_ms: u32,
    pub school_mask: u32,
    pub costs: Vec<Cost>,
    pub duration_ms: Option<i32>,
    /// Its range is the melee range (`SpellRange`'s flag 1).
    pub melee: bool,
    /// It is channeled (`SpellMisc`'s attributes 1, 0x4 or 0x40).
    pub channeled: bool,
    /// It strikes with the next melee swing (attributes 0, 0x4).
    pub next_swing: bool,
    /// It is passive (attributes 0, 0x40).
    pub passive: bool,
    /// The tools it needs (`SpellTotems`' kinds, `TotemCategory`).
    pub tools: Vec<String>,
    /// Its reagents: (item, count) (`SpellReagents`).
    pub reagents: Vec<(u32, u32)>,
    /// The kinds of item it needs equipped (`SpellEquippedItems`' class
    /// and subclasses, their names: "Daggers").
    pub equipped: Vec<String>,
    /// The forms it needs (`SpellShapeshift`'s mask, by form ID; none for
    /// one usable without).
    pub forms: Vec<String>,
}

/// The base difficulty's row of a table keyed by spell.
fn base<'a>(t: &'a Table, spell: u32) -> Option<Row<'a>> {
    let rows = t.rows_where("SpellID", spell);
    rows.iter()
        .copied()
        .find(|r| !t.has("DifficultyID") || r.int("DifficultyID") == 0)
        .or_else(|| rows.first().copied())
}

impl SpellTables {
    /// `None` when the build lacks the spell tables or their layouts.
    pub fn open(casc: &Casc) -> Option<SpellTables> {
        Some(SpellTables {
            name: Table::open(casc, "SpellName")?,
            spell: Table::open(casc, "Spell")?,
            misc: Table::open(casc, "SpellMisc")?,
            effect: Table::open(casc, "SpellEffect")?,
            cast_times: Table::open(casc, "SpellCastTimes"),
            range: Table::open(casc, "SpellRange"),
            duration: Table::open(casc, "SpellDuration"),
            radius: Table::open(casc, "SpellRadius"),
            levels: Table::open(casc, "SpellLevels"),
            aura: Table::open(casc, "SpellAuraOptions"),
            cooldowns: Table::open(casc, "SpellCooldowns"),
            power: Table::open(casc, "SpellPower"),
            shapeshift: Table::open(casc, "SpellShapeshift"),
            forms: Table::open(casc, "SpellShapeshiftForm"),
            totems: Table::open(casc, "SpellTotems"),
            totem_kinds: Table::open(casc, "TotemCategory"),
            reagents: Table::open(casc, "SpellReagents"),
            equipped: Table::open(casc, "SpellEquippedItems"),
            subclasses: Table::open(casc, "ItemSubClass"),
        })
    }

    fn duration_of(&self, misc: Option<Row<'_>>) -> Option<i32> {
        let index = misc?.int("DurationIndex");
        let d = self.duration.as_ref()?.row(index)?.signed("Duration");
        Some(d)
    }

    /// A spell's facts; `None` for a spell the tables lack.
    pub fn info(&self, id: u32) -> Option<SpellInfo> {
        let name = self.name.row(id)?.string("Name_lang")?;
        let misc = base(&self.misc, id);
        let mut info = SpellInfo {
            name,
            subtext: self
                .spell
                .row(id)
                .and_then(|r| r.string("NameSubtext_lang"))
                .unwrap_or_default(),
            duration_ms: self.duration_of(misc),
            ..Default::default()
        };
        if let Some(m) = misc {
            info.icon = Some(m.int("SpellIconFileDataID")).filter(|i| *i > 0);
            info.school_mask = m.int("SchoolMask");
            if let Some(c) = self
                .cast_times
                .as_ref()
                .and_then(|t| t.row(m.int("CastingTimeIndex")))
            {
                info.cast_ms = c.signed("Base").max(0) as u32;
            }
            if let Some(r) = self.range.as_ref().and_then(|t| t.row(m.int("RangeIndex"))) {
                info.min_range = r.floats("RangeMin").first().copied().unwrap_or(0.0);
                info.max_range = r.floats("RangeMax").first().copied().unwrap_or(0.0);
                info.melee = r.int("Flags") & 1 != 0;
            }
        }
        // The forms it needs, unless it needs none (`SpellMisc`'s
        // attributes 2, 0x80000: usable in them, recorded Moonfire's).
        info.passive =
            misc.is_some_and(|m| m.ints("Attributes").first().is_some_and(|a| a & 0x40 != 0));
        if let (Some(t), Some(kinds)) = (
            self.totems
                .as_ref()
                .and_then(|t| t.rows_where("SpellID", id).first().copied()),
            self.totem_kinds.as_ref(),
        ) {
            info.tools = t
                .ints("RequiredTotemCategoryID")
                .into_iter()
                .filter(|k| *k != 0)
                .filter_map(|k| kinds.row(k)?.string("Name_lang"))
                .collect();
        }
        if let Some(r) = self
            .reagents
            .as_ref()
            .and_then(|t| t.rows_where("SpellID", id).first().copied())
        {
            info.reagents = r
                .ints("Reagent")
                .into_iter()
                .zip(r.ints("ReagentCount"))
                .filter(|(i, n)| *i != 0 && *n != 0)
                .collect();
        }
        if let (Some(e), Some(subclasses)) = (
            self.equipped
                .as_ref()
                .and_then(|t| t.rows_where("SpellID", id).first().copied()),
            self.subclasses.as_ref(),
        ) {
            let class = e.signed("EquippedItemClass");
            let mask = e.int("EquippedItemSubclass");
            if class >= 0 {
                for bit in 0..32 {
                    if mask & (1 << bit) == 0 {
                        continue;
                    }
                    if let Some(r) = subclasses
                        .rows()
                        .find(|r| r.int("ClassID") == class as u32 && r.int("SubClassID") == bit)
                    {
                        let verbose = r.string("VerboseName_lang").unwrap_or_default();
                        info.equipped.push(if verbose.is_empty() {
                            r.string("DisplayName_lang").unwrap_or_default()
                        } else {
                            verbose
                        });
                    }
                }
            }
        }
        info.next_swing =
            misc.is_some_and(|m| m.ints("Attributes").first().is_some_and(|a| a & 0x4 != 0));
        info.channeled =
            misc.is_some_and(|m| m.ints("Attributes").get(1).is_some_and(|a| a & 0x44 != 0));
        let formless = misc.is_some_and(|m| {
            m.ints("Attributes")
                .get(2)
                .is_some_and(|a| a & 0x80000 != 0)
        });
        if !formless
            && let (Some(s), Some(forms)) = (
                self.shapeshift.as_ref().and_then(|t| base(t, id)),
                self.forms.as_ref(),
            )
        {
            for (word, mask) in s.ints("ShapeshiftMask").into_iter().enumerate() {
                for bit in 0..32 {
                    if mask & (1 << bit) != 0
                        && let Some(name) = forms
                            .row(word as u32 * 32 + bit + 1)
                            .and_then(|r| r.string("Name_lang"))
                    {
                        info.forms.push(name);
                    }
                }
            }
        }
        if let Some(c) = self.cooldowns.as_ref().and_then(|t| base(t, id)) {
            info.cooldown_ms = c.int("RecoveryTime");
            info.category_cooldown_ms = c.int("CategoryRecoveryTime");
            info.gcd_ms = c.int("StartRecoveryTime");
        }
        if let Some(t) = &self.power {
            let mut rows = t.rows_where("SpellID", id);
            rows.sort_by_key(|r| r.int("OrderIndex"));
            info.costs = rows
                .iter()
                .map(|r| Cost {
                    power_type: r.signed("PowerType"),
                    amount: r.int("ManaCost"),
                    optional: r.int("OptionalCost"),
                    percent: r.float("PowerCostPct"),
                })
                .collect();
        }
        Some(info)
    }

    /// A spell's description for `caster`, its placeholders filled in;
    /// `None` without one or where a placeholder cannot be filled exactly.
    pub fn describe(&self, id: u32, caster: &Caster) -> Option<String> {
        let text = self.spell.row(id)?.string("Description_lang")?;
        spelltext::describe(&text, id, self, caster)
    }

    /// Its description as a trainer shows it, the spell not known: an
    /// aura's live value (`$w1`) left as written (recorded: the trainers'
    /// Thorns, "causing $w1 Nature damage"; its `GetSpellDescription` 8).
    pub fn describe_unlearned(&self, id: u32, caster: &Caster) -> Option<String> {
        let text = self.spell.row(id)?.string("Description_lang")?;
        // (kept from the placeholders' parser by another mark, put back)
        let text = text.replace("$w", "\u{1}w");
        spelltext::describe(&text, id, self, caster).map(|d| d.replace('\u{1}', "$"))
    }

    /// Its aura's description (the buff's tooltip), likewise.
    pub fn describe_aura(&self, id: u32, caster: &Caster) -> Option<String> {
        let text = self.spell.row(id)?.string("AuraDescription_lang")?;
        spelltext::describe(&text, id, self, caster)
    }
}

impl spelltext::Spells for SpellTables {
    fn data(&self, id: u32) -> Option<Data> {
        self.name.row(id)?;
        let misc = base(&self.misc, id);
        let mut data = Data {
            duration_ms: self.duration_of(misc),
            ..Default::default()
        };
        if let Some(l) = self.levels.as_ref().and_then(|t| base(t, id)) {
            data.spell_level = l.int("SpellLevel");
            data.max_level = l.int("MaxLevel");
        }
        if let Some(a) = self.aura.as_ref().and_then(|t| base(t, id)) {
            data.proc_chance = a.int("ProcChance");
            data.stacks = a.int("CumulativeAura");
            data.charges = a.int("ProcCharges");
        }
        if let Some(m) = misc
            && let Some(r) = self.range.as_ref().and_then(|t| t.row(m.int("RangeIndex")))
        {
            data.max_range = r.floats("RangeMax").first().copied().unwrap_or(0.0);
        }
        for e in self.effect.rows_where("SpellID", id) {
            if e.int("DifficultyID") != 0 {
                continue;
            }
            let radius = e
                .ints("EffectRadiusIndex")
                .first()
                .copied()
                .filter(|r| *r > 0)
                .and_then(|r| Some(self.radius.as_ref()?.row(r)?.float("Radius")));
            data.effects.push(Effect {
                index: e.int("EffectIndex"),
                base: e.float("EffectBasePointsF"),
                variance: e.float("Variance"),
                per_level: e.float("EffectRealPointsPerLevel"),
                sp: e.float("EffectBonusCoefficient"),
                ap: e.float("BonusCoefficientFromAP"),
                period_ms: e.int("EffectAuraPeriod"),
                chain_targets: e.int("EffectChainTargets"),
                radius,
                // a heal, or a heal over time
                healing: e.int("Effect") == 10 || e.int("EffectAura") == 8,
                damage: e.int("Effect") == 2 || e.int("EffectAura") == 3,
            });
        }
        Some(data)
    }

    fn name(&self, id: u32) -> Option<String> {
        self.name.row(id)?.string("Name_lang")
    }

    fn description(&self, id: u32) -> Option<String> {
        self.spell.row(id)?.string("Description_lang")
    }
}
