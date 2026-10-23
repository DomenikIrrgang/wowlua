//! `cargo run -p wowlua-casc --example dump -- <product> <path>`: a file's
//! size and header, and its image's first pixels.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(Box::new(wowlua_casc::FsStorage { root }), &args[1])
        .expect("the product");
    let b = casc.file_by_path(&args[2]).expect("the file");
    println!("{} bytes, header {:?}", b.len(), &b[..b.len().min(24)]);
    if let Some(img) = wowlua_casc::decode_image(&b) {
        println!(
            "{}x{} first {:?}",
            img.width,
            img.height,
            &img.pixels[..16.min(img.pixels.len())]
        );
    }
}
