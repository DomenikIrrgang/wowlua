//! `cargo run -p wowlua-casc --example spelldesc -- <spell id>…`: a spell's
//! description as the client's tables have it, with its effects' values.
use wowlua_casc::tables::Table;
fn main() {
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(
        Box::new(wowlua_casc::FsStorage { root }),
        "wow_classic_beta",
    )
    .unwrap();
    let spell = Table::open(&casc, "Spell").unwrap();
    let misc = Table::open(&casc, "SpellMisc").unwrap();
    let effects = Table::open(&casc, "SpellEffect").unwrap();
    let durations = Table::open(&casc, "SpellDuration").unwrap();
    let auras = Table::open(&casc, "SpellAuraOptions").unwrap();
    for a in std::env::args().skip(1) {
        let id: u32 = a.parse().unwrap();
        let desc = spell.row(id).and_then(|r| r.string("Description_lang"));
        println!("== {id}: {desc:?}");
        for m in misc.rows_where("SpellID", id) {
            let d = durations
                .row(m.int("DurationIndex"))
                .map(|r| (r.signed("Duration"), r.signed("MaxDuration")));
            println!(
                "   misc diff {} duration {:?} cast {} range {}",
                m.int("DifficultyID"),
                d,
                m.int("CastingTimeIndex"),
                m.int("RangeIndex")
            );
        }
        for a in auras.rows_where("SpellID", id) {
            println!(
                "   aura stacks {} proc {} charges {}",
                a.int("CumulativeAura"),
                a.int("ProcChance"),
                a.int("ProcCharges")
            );
        }
        for e in effects.rows_where("SpellID", id) {
            println!(
                "   eff #{} effect {} aura {} base {} var {} perlvl {} period {} amp {} chain {} radius {:?} misc {:?} trig {} ppr {}",
                e.int("EffectIndex"),
                e.int("Effect"),
                e.int("EffectAura"),
                e.float("EffectBasePointsF"),
                e.float("Variance"),
                e.float("EffectRealPointsPerLevel"),
                e.int("EffectAuraPeriod"),
                e.float("EffectAmplitude"),
                e.int("EffectChainTargets"),
                e.ints("EffectRadiusIndex"),
                e.ints("EffectMiscValue"),
                e.int("EffectTriggerSpell"),
                e.float("EffectPointsPerResource")
            );
        }
    }
}
