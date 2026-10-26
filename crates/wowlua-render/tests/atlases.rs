//! Atlases as the API answers them from the client's tables (decision
//! T472), against the engine's snapshot of the same build's tables
//! (`wowlua-engine/data/camelot-atlases-client.txt`) and the answers the
//! client gave (`camelot-atlases.txt`), on the WoW installation of this
//! machine; skipped where there is none.

fn lines(text: &str) -> Vec<Vec<&str>> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').collect())
        .collect()
}

#[test]
fn the_client_atlases_answer_as_recorded() {
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let storage = wowlua_casc::FsStorage { root };
    let Some(product) = wowlua_casc::Casc::product_of_folder(&storage, "_classic_beta_") else {
        eprintln!("no camelot client: skipped");
        return;
    };
    let casc = wowlua_casc::Casc::open(Box::new(storage), &product).unwrap();
    let ours = wowlua_casc::atlas::read_api(&casc);
    let check = |text: &str, label: &str| -> (usize, Vec<String>) {
        let mut wrong = Vec::new();
        let rows = lines(text);
        for f in &rows {
            let Some(a) = ours.get(&f[0].to_ascii_lowercase()) else {
                wrong.push(format!("{label} {}: none", f[0]));
                continue;
            };
            let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok());
            let near = |x: f64, y: Option<f64>| y.is_none_or(|y| (x - y).abs() < 1e-6);
            let mut same = near(a.width, num(1)) && near(a.height, num(2));
            if f.len() >= 10 {
                same &= near(f64::from(a.file), num(3))
                    && (0..4).all(|i| near(a.coords[i], num(4 + i)))
                    && (a.tiles_horizontally == (f[8] == "true"))
                    && (a.tiles_vertically == (f[9] == "true"));
            }
            if f.len() >= 12 {
                same &= near(a.raw.0, num(10)) && near(a.raw.1, num(11));
            }
            if !same {
                wrong.push(format!("{label} {}: {a:?}", f.join(" ")));
            }
        }
        (rows.len(), wrong)
    };
    let (n, snapshot) = check(
        include_str!("../../wowlua-engine/data/camelot-atlases-client.txt"),
        "snapshot",
    );
    let (r, recorded) = check(
        include_str!("../../wowlua-engine/data/camelot-atlases.txt"),
        "recorded",
    );
    eprintln!(
        "{n} snapshot atlases, {} differ; {r} recorded, {} differ",
        snapshot.len(),
        recorded.len()
    );
    assert!(n > 15_000, "{n}");
    assert!(
        snapshot.is_empty(),
        "{}",
        snapshot[..snapshot.len().min(10)].join("\n")
    );
    // common-sidetab-mask: its right coordinate a float below 1 that its
    // flags do not explain (decision T238)
    let unexplained: Vec<&String> = recorded
        .iter()
        .filter(|w| !w.contains("common-sidetab-mask"))
        .collect();
    assert!(unexplained.is_empty(), "{unexplained:?}");
}
