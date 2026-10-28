//! Glyphs measured with the client's font files (decision T392), against
//! the client's own measurements (`wowlua-engine/data/camelot-glyphs.txt`:
//! Friz Quadrata, Arial Narrow, Skurri and Morpheus at 8 to 45 pixels, every
//! printable ASCII character), on the WoW installation of this machine;
//! skipped where there is none.

#[test]
fn the_client_fonts_measure_as_the_client_does() {
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let storage = wowlua_casc::FsStorage { root: root.clone() };
    let Some(product) = wowlua_casc::Casc::product_of_folder(&storage, "_forever_") else {
        eprintln!("no camelot client: skipped");
        return;
    };
    let casc = wowlua_casc::Casc::open(Box::new(storage), &product).unwrap();
    let data = include_str!("../../wowlua-engine/data/camelot-glyphs.txt");
    let mut fonts = std::collections::HashMap::new();
    let (mut advances, mut widths, mut total) = (0, 0, 0);
    for line in data.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if line.starts_with('#') || f.len() != 6 || f[1] != "-" {
            continue;
        }
        let (file, px): (&str, f32) = (f[0], f[2].parse().unwrap());
        let font = fonts.entry(file).or_insert_with(|| {
            let bytes = casc.file_by_path(&format!("Fonts\\{file}")).expect(file);
            fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).unwrap()
        });
        let nums = |s: &str| -> Vec<f32> { s.split(',').map(|v| v.parse().unwrap()).collect() };
        for (i, (width, advance)) in nums(f[4]).into_iter().zip(nums(f[5])).enumerate() {
            let c = char::from(32 + i as u8);
            let m = font.metrics(c, px);
            // the engine's rule (wowlua-engine's text::glyph)
            let ours_advance = m.advance_width.ceil();
            let ours_width = if c == ' ' {
                ours_advance
            } else {
                (m.xmin as f32 + m.width as f32).ceil() + 1.0
            };
            total += 1;
            advances += usize::from(ours_advance == advance);
            widths += usize::from(ours_width == width);
        }
    }
    eprintln!("{total} glyphs: {advances} advances and {widths} widths the same");
    assert!(total > 14_000, "{total}");
    assert!(
        advances * 1000 >= total * 985,
        "{advances} of {total} advances"
    );
    assert!(widths * 1000 >= total * 940, "{widths} of {total} widths");
}
