//! `cargo run -p wowlua-casc --example uicameras -- <product> <name part>`:
//! the UI cameras whose names have the part.

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(Box::new(wowlua_casc::FsStorage { root }), &a[1])
        .expect("the product");
    let t = wowlua_casc::tables::Table::open(&casc, "UiCamera").expect("the table");
    for r in t.rows() {
        let name = r.string("Name").unwrap_or_default();
        if name.contains(&a[2]) {
            println!(
                "{} {name}: pos {:?} look {:?} up {:?} anim {} frame {} flags {}",
                r.id(),
                r.floats("Pos"),
                r.floats("LookAt"),
                r.floats("Up"),
                r.signed("AnimID"),
                r.signed("AnimFrame"),
                r.int("Flags")
            );
        }
    }
}
