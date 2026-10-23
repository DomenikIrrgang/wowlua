//! `cargo run -p wowlua-casc --example m2info -- <product> <file id>`: a
//! model's chunks, sequences and whether its stand sequence's bones move.

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let casc = wowlua_casc::Casc::open(Box::new(wowlua_casc::FsStorage { root }), &a[1])
        .expect("the product");
    let id: u32 = a[2].parse().unwrap();
    let mut m = wowlua_casc::m2::Model::parse(&casc.file_by_id(id).unwrap()).unwrap();
    println!(
        "{} bones {} sequences, skeleton {:?}, {} anim files, bounds {:?}",
        m.bones.len(),
        m.sequences.len(),
        m.skeleton_file,
        m.anim_files.len(),
        m.bounds
    );
    if let Some(s) = m.skeleton_file {
        let b = casc.file_by_id(s).unwrap();
        let tags: Vec<String> = {
            let mut p = 0;
            let mut t = Vec::new();
            while p + 8 <= b.len() {
                let n = u32::from_le_bytes(b[p + 4..p + 8].try_into().unwrap()) as usize;
                t.push(format!("{}({n})", String::from_utf8_lossy(&b[p..p + 4])));
                p += 8 + n;
            }
            t
        };
        println!(
            "skel chunks {tags:?}, parent {:?}",
            wowlua_casc::m2::Model::skeleton_parent(&b)
        );
        if let Some(pid) = wowlua_casc::m2::Model::skeleton_parent(&b) {
            m.load_skeleton(&casc.file_by_id(pid).unwrap());
        }
        m.load_skeleton(&b);
        println!(
            "after skel: {} bones {} sequences {} anim files",
            m.bones.len(),
            m.sequences.len(),
            m.anim_files.len()
        );
    }
    let s = m.sequence(0).unwrap();
    println!(
        "stand seq {s}: {:?}, needs {:?}",
        m.sequences[s],
        m.needs_anim(s)
    );
    if let Some(f) = m.needs_anim(s) {
        let b = casc.file_by_id(f);
        println!(
            "anim file {f}: {:?} bytes, head {:?}",
            b.as_ref().map(Vec::len),
            b.as_ref()
                .map(|b| String::from_utf8_lossy(&b[..4]).into_owned())
        );
        if let Some(b) = b {
            m.load_anim(s, &b);
        }
    }
    let moving = m
        .bones
        .iter()
        .filter(|b| b.rotation.keys.get(s).is_some_and(|k| !k.times.is_empty()))
        .count();
    println!("bones with rotation keys in stand: {moving}");
}
