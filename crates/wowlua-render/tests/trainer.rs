//! Trainer services' spells from the client's tables (decision T471):
//! their tooltips (without the rank, as trainers show them) and icons as
//! the engine builds them from the client's spells, against the trainers
//! the client recorded (`wowlua-engine/data/camelot-trainers.txt`), on the
//! WoW installation of this machine; skipped where there is none. The
//! character's numbers (level, spell power, base mana, haste) are the
//! test player's, so those count apart.

use wowlua_vm::{Value, Vm};

fn text(vm: &Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => n.to_string(),
        _ => String::new(),
    }
}

#[test]
fn the_client_spells_answer_as_the_trainers_recorded() {
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let storage = wowlua_casc::FsStorage { root };
    let Some(product) = wowlua_casc::Casc::product_of_folder(&storage, "_forever_") else {
        eprintln!("no camelot client: skipped");
        return;
    };
    let casc = std::sync::Arc::new(wowlua_casc::Casc::open(Box::new(storage), &product).unwrap());
    let data = std::sync::Arc::new(wowlua_render::client::CascClientData::new(Box::new(
        move || Some(casc.clone()),
    )));
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").unwrap();
    wowlua_engine::clientdata::set(&mut vm, data);
    let f = vm
        .load_as(
            b"local t, id = ...
local out = {}
for i, l in ipairs(t and t.lines or {}) do
    out[#out + 1] = (l.leftText or '') .. '|' .. (l.rightText or '')
end
return table.concat(out, '\\n'), C_Spell.GetSpellTexture(id) or 0",
            "@t.lua",
            "T",
        )
        .unwrap();
    let services = wowlua_engine::trainer::recorded_services();
    let (mut lines_same, mut desc_same, mut icon_same, mut numbers) = (0, 0, 0, 0);
    let mut wrong = Vec::new();
    for wowlua_engine::trainer::RecordedService {
        spell,
        lines,
        description,
        icon,
    } in &services
    {
        let (tooltip, description_ours) =
            wowlua_engine::trainer::client_service(&mut vm, *spell).unwrap_or_default();
        let tip = tooltip.first().copied().unwrap_or(Value::Nil);
        let r = vm.call(f, &[tip, Value::Num(*spell as f64)]).unwrap();
        let mut ours: Vec<String> = r.iter().map(|v| text(&vm, *v)).collect();
        ours.insert(1, description_ours);
        let theirs = lines
            .iter()
            .map(|(l, r)| format!("{l}|{r}"))
            .collect::<Vec<_>>()
            .join("\n");
        // The character's numbers aside (its level, spell power, base mana,
        // haste): the same lines.
        // (a weapon a spell needs: the probing characters wielded theirs,
        // this player none)
        let shape = |s: &str| -> String {
            s.lines()
                .filter(|l| {
                    !l.starts_with("Requires") || l.contains("Form") || l.contains("Stance")
                })
                .map(|l| {
                    let l: String = l
                        .chars()
                        .map(|c| {
                            if c.is_ascii_digit() || c == ',' || c == '.' {
                                '#'
                            } else {
                                c
                            }
                        })
                        .collect();
                    let mut out = String::new();
                    for c in l.chars() {
                        if !(c == '#' && out.ends_with('#')) {
                            out.push(c);
                        }
                    }
                    // a cost in percent of the base mana: the character's
                    if out == "# Mana|" || out.starts_with("# Mana|") {
                        out.replacen("# Mana", "", 1)
                    } else {
                        out
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        if ours[0] == theirs {
            lines_same += 1;
        } else if shape(&ours[0]) == shape(&theirs) {
            numbers += 1;
        } else {
            let (a, b) = (shape(&ours[0]), shape(&theirs));
            let (a, b): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
            let i = (0..a.len().max(b.len()))
                .find(|i| a.get(*i) != b.get(*i))
                .unwrap_or(0);
            let first = |l: &[&str]| {
                l.get(i)
                    .map(|s| s.chars().take(60).collect::<String>())
                    .unwrap_or_default()
            };
            wrong.push(format!(
                "{spell} line {i}: {:?} / {:?}",
                first(&a),
                first(&b)
            ));
        }
        desc_same += usize::from(ours[1] == *description);
        if ours[1] != *description && std::env::var_os("SHOW_DESC").is_some() {
            eprintln!(
                "DESC {spell}\n  ours:   {}\n  theirs: {description}",
                ours[1]
            );
        }
        icon_same += usize::from(ours[2] == icon.to_string());
    }
    let n = services.len();
    eprintln!(
        "{n} services: tooltips {lines_same} the same, {numbers} but the character's numbers, {} else; descriptions {desc_same}, icons {icon_same} the same\n{}",
        wrong.len(),
        wrong
            .iter()
            .filter(|w| !w.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    assert!(n > 1700);
    assert_eq!(icon_same, n, "every icon the client's");
    // The rest: ranges the tables do not tell (Maul's none, Raptor Strike's,
    // a friend's and an enemy's), costs in health, reagents and levels the
    // character had.
    assert!(
        lines_same + numbers >= 1380,
        "{lines_same} + {numbers} of {n}"
    );
}
