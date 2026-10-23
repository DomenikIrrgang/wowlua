//! `cargo run -p wowlua-casc --example humanoids -- <product>`: the first
//! creature displays with extra display info (humanoids).

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(Box::new(wowlua_casc::FsStorage { root }), &a[1])
        .expect("the product");
    let t = wowlua_casc::tables::Table::open(&casc, "CreatureDisplayInfo").expect("the table");
    let ids: Vec<(u32, u32)> = t
        .rows()
        .filter(|r| r.int("ExtendedDisplayInfoID") != 0)
        .map(|r| (r.id(), r.int("ExtendedDisplayInfoID")))
        .take(12)
        .collect();
    println!("{ids:?}");
}
