//! `cargo run -p wowlua-casc --example itemfile -- <product> <item ID> [out]`:
//! an item's model files (and the first written to `out`).

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(Box::new(wowlua_casc::FsStorage { root }), &a[1])
        .expect("the product");
    let mut lib = wowlua_casc::display::Library::default();
    let d = lib
        .item_model(
            &casc,
            wowlua_casc::character::Worn::Item(a[2].parse().unwrap()),
        )
        .expect("the item's models");
    let files: Vec<u32> = d.parts.iter().map(|p| p.file).collect();
    println!("{files:?}");
    if let Some(out) = a.get(3) {
        std::fs::write(out, casc.file_by_id(files[0]).unwrap()).unwrap();
    }
}
