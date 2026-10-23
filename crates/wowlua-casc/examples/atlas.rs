//! `cargo run -p wowlua-casc --example atlas -- <product> <name>`: every
//! entry of the build's atlas tables with that name.

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(Box::new(wowlua_casc::FsStorage { root }), &a[1])
        .expect("the product");
    let atlases =
        wowlua_casc::db2::Db2::parse(casc.file_by_id(897470).expect("atlases")).expect("parsed");
    let members =
        wowlua_casc::db2::Db2::parse(casc.file_by_id(897532).expect("members")).expect("parsed");
    let want = a[2].to_ascii_lowercase();
    for i in 0..members.len() {
        let name = members
            .string(i, 0)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if want.starts_with('~') && name.contains(&want[1..]) {
            println!("{name} (element {:?})", members.int(i, 9));
            continue;
        }
        if name == want {
            let fields: Vec<String> = (0..members.field_count())
                .map(|f| format!("{:?}", members.int(i, f)))
                .collect();
            let atlas = members.int(i, 2).unwrap_or(0);
            let row = (0..atlases.len()).find(|&j| atlases.id(j) == atlas);
            let af: Vec<String> = row
                .map(|j| {
                    (0..atlases.field_count())
                        .map(|f| format!("{:?}", atlases.int(j, f)))
                        .collect()
                })
                .unwrap_or_default();
            println!(
                "member {}: {} | atlas {atlas}: {}",
                members.id(i),
                fields.join(" "),
                af.join(" ")
            );
        }
    }
}
