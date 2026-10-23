//! `cargo run -p wowlua-casc --example withparticles -- <product>`: creature
//! displays whose models have particle emitters (their counts).

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(Box::new(wowlua_casc::FsStorage { root }), &a[1])
        .expect("the product");
    let mut lib = wowlua_casc::display::Library::default();
    let mut found = 0;
    for id in 1..20000 {
        let Some(d) = lib.creature_display(&casc, id) else {
            continue;
        };
        let Some(b) = casc.file_by_id(d.parts[0].file) else {
            continue;
        };
        // MD21's body: the particle emitters' count at 296.
        let md = if b.starts_with(b"MD21") {
            &b[8..]
        } else {
            &b[..]
        };
        let n = u32::from_le_bytes(md[296..300].try_into().unwrap());
        if n > 0 {
            println!("display {id}: file {} has {n} emitters", d.parts[0].file);
            found += 1;
            if found == 8 {
                break;
            }
        }
    }
}
