//! `cargo run -p wowlua-render --example texture -- <client folder> <path> <out.png>`:
//! a texture of the installation as PNG.

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let install = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_render::client::open_client(&install, &[a[1].clone()]).expect("the client");
    let bytes = match a[2].parse::<u32>() {
        Ok(id) => casc.file_by_id(id),
        Err(_) => casc.file_by_path(&a[2]),
    }
    .expect("the file");
    println!(
        "{} bytes, header {:?}",
        bytes.len(),
        &bytes[..20.min(bytes.len())]
    );
    let img = wowlua_casc::decode_image(&bytes).expect("an image");
    println!("{}x{}", img.width, img.height);
    std::fs::write(&a[3], wowlua_render::encode_png(&img).unwrap()).unwrap();
}
