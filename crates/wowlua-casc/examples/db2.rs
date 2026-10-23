//! `cargo run -p wowlua-casc --example db2 -- <product> <file id>`: a
//! table's size and its first records' fields.

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(Box::new(wowlua_casc::FsStorage { root }), &a[1])
        .expect("the product");
    let id: u32 = a[2].parse().expect("an id");
    println!("{}", casc.explain(id));
    let bytes = casc.file_by_id(id).expect("the file");
    println!(
        "{} bytes, {:?}",
        bytes.len(),
        String::from_utf8_lossy(&bytes[..4])
    );
    let Some(db) = wowlua_casc::db2::Db2::parse(bytes) else {
        println!("not parsed");
        return;
    };
    println!(
        "{} records, {} fields, layout {:08X}",
        db.len(),
        db.field_count(),
        db.layout_hash
    );
    for i in 0..db.len().min(3) {
        let ints: Vec<String> = (0..db.field_count())
            .map(|f| format!("{:?}", db.int(i, f)))
            .collect();
        println!(
            "#{} id {}: {} | {:?}",
            i,
            db.id(i),
            ints.join(" "),
            db.string(i, 0)
        );
    }
}
