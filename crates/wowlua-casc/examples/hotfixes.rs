//! `cargo run -p wowlua-casc --example hotfixes -- <product> <file id>`: a
//! table's fields (storage, bits, bytes, count) and its hotfixes' sizes.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(Box::new(wowlua_casc::FsStorage { root }), &a[1])
        .expect("the product");
    let db = wowlua_casc::db2::Db2::parse(casc.file_by_id(a[2].parse().unwrap()).unwrap()).unwrap();
    let info = db.field_info();
    let total: usize = info.iter().map(|f| f.2.clamp(1, 4) * f.3).sum();
    for (i, f) in info.iter().enumerate() {
        println!(
            "field {i}: storage {} bits {} bytes {} count {}",
            f.0, f.1, f.2, f.3
        );
    }
    let hot = casc.hotfixes(db.table_hash);
    let mut sizes: std::collections::BTreeMap<usize, usize> = Default::default();
    for (_, status, data) in hot {
        if *status == 1 {
            *sizes.entry(data.len()).or_default() += 1;
        }
    }
    println!(
        "{} hotfixes; fields without strings take {total} bytes; data sizes {:?}",
        hot.len(),
        sizes
    );
    if let Some((id, _, data)) = hot.iter().find(|h| h.1 == 1) {
        println!("first: {id} {:02x?}", &data[..data.len().min(96)]);
    }
}
