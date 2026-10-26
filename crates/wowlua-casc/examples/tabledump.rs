//! `cargo run -p wowlua-casc --example tabledump -- <product> <table> <col>…`:
//! a table's records' columns, tab-separated (the ID first).

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(Box::new(wowlua_casc::FsStorage { root }), &a[1])
        .expect("the product");
    let table = wowlua_casc::tables::Table::open(&casc, &a[2]).expect("the table");
    for row in table.rows() {
        let cols: Vec<String> = a[3..]
            .iter()
            .map(|c| {
                if c.ends_with("_lang") || c == "InternalName" {
                    row.string(c).unwrap_or_default()
                } else {
                    row.signed(c).to_string()
                }
            })
            .collect();
        println!("{}\t{}", row.id(), cols.join("\t"));
    }
}
