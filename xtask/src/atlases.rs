//! `cargo xtask atlases <dir>`: every atlas of the client from its own
//! tables (`UiTextureAtlas`, `UiTextureAtlasMember`,
//! `UiTextureAtlasElement` and `UiCanvas` of build 1.60.1.70532, as CSV
//! from wago.tools:
//! `curl -o <dir>/<Table>.csv "https://wago.tools/db2/<Table>/csv?build=1.60.1.70532"`)
//! into `data/camelot-atlases-client.txt`: per element (the name as
//! `GetAtlas` spells it), its width and height, file, texture coordinates
//! (left, right, top, bottom), tiling (horizontally, vertically) and raw
//! size (decision T238).
//!
//! An element has a member per atlas texture it is in; the client takes
//! the member of the override set (`UiTextureAtlasSetID` 1, the `-c60`
//! textures) over the base set's, then the 2x canvas's over the 1x's,
//! then the first; a 2x member's size is half its pixels. Held to every
//! atlas the client was asked about (`data/camelot-atlases.txt`, 290 of
//! 291 alike; the one left, `common-sidetab-mask`, has a right coordinate
//! a float below 1 that its flags do not explain): the engine takes the
//! recorded answer where there is one.

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

struct Member {
    id: u64,
    atlas: u64,
    width: f64,
    height: f64,
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
    override_width: f64,
    override_height: f64,
    flags: u64,
    canvas: u64,
}

struct Atlas {
    file: u64,
    set: u64,
    width: f64,
    height: f64,
    canvas: u64,
}

/// A table's rows by its header's columns.
fn rows(dir: &Path, table: &str) -> Result<Vec<HashMap<String, String>>> {
    let path = dir.join(format!("{table}.csv"));
    let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()
        .context("a header")?
        .split(',')
        .map(str::to_string)
        .collect();
    Ok(lines
        .filter(|l| !l.is_empty())
        .map(|l| {
            header
                .iter()
                .cloned()
                // (a value with a space comes quoted: `"Raid Frame"`)
                .zip(l.split(',').map(|v| v.trim_matches('"').to_string()))
                .collect()
        })
        .collect())
}

fn num(row: &HashMap<String, String>, key: &str) -> Result<f64> {
    row.get(key)
        .with_context(|| format!("no {key}"))?
        .parse::<f64>()
        .with_context(|| format!("{key} not a number"))
}

/// A texture coordinate as the client keeps it: a single float.
fn coord(v: f64) -> f64 {
    f64::from(v as f32)
}

pub fn run(root: &Path, dir: &Path) -> Result<()> {
    let mut canvases: HashMap<u64, f64> = HashMap::new();
    for r in rows(dir, "UiCanvas")? {
        canvases.insert(num(&r, "ID")? as u64, num(&r, "Width")?);
    }
    let base = canvases.get(&1).copied().unwrap_or(1024.0);
    let mut atlases: HashMap<u64, Atlas> = HashMap::new();
    for r in rows(dir, "UiTextureAtlas")? {
        atlases.insert(
            num(&r, "ID")? as u64,
            Atlas {
                file: num(&r, "FileDataID")? as u64,
                set: num(&r, "UiTextureAtlasSetID")? as u64,
                width: num(&r, "AtlasWidth")?,
                height: num(&r, "AtlasHeight")?,
                canvas: num(&r, "UiCanvasID")? as u64,
            },
        );
    }
    let mut members: HashMap<u64, Vec<Member>> = HashMap::new();
    for r in rows(dir, "UiTextureAtlasMember")? {
        members
            .entry(num(&r, "UiTextureAtlasElementID")? as u64)
            .or_default()
            .push(Member {
                id: num(&r, "ID")? as u64,
                atlas: num(&r, "UiTextureAtlasID")? as u64,
                width: num(&r, "Width")?,
                height: num(&r, "Height")?,
                left: num(&r, "CommittedLeft")?,
                right: num(&r, "CommittedRight")?,
                top: num(&r, "CommittedTop")?,
                bottom: num(&r, "CommittedBottom")?,
                override_width: num(&r, "OverrideWidth")?,
                override_height: num(&r, "OverrideHeight")?,
                flags: num(&r, "CommittedFlags")? as u64,
                canvas: num(&r, "UiCanvasID")? as u64,
            });
    }
    let mut elements: Vec<(String, u64)> = Vec::new();
    for r in rows(dir, "UiTextureAtlasElement")? {
        elements.push((
            r.get("Name").context("no Name")?.clone(),
            num(&r, "ID")? as u64,
        ));
    }
    if elements.is_empty() {
        bail!("no elements in {}", dir.display());
    }
    elements.sort_by(|a, b| a.0.cmp(&b.0));
    let canvas_of = |m: &Member| -> u64 {
        if m.canvas != 0 {
            m.canvas
        } else {
            atlases.get(&m.atlas).map_or(1, |a| a.canvas)
        }
    };
    let mut out = String::from(
        "# Every atlas of WoW Forever 1.60.1 (70532) from the client's own tables\n\
         # (`cargo xtask atlases`, decision T238): the element's name, its width\n\
         # and height, file ID, left, right, top and bottom texture coordinates,\n\
         # tiling horizontally and vertically, raw width and height. Recorded\n\
         # answers (camelot-atlases.txt) take the place of these.\n",
    );
    let mut written = 0;
    for (name, id) in &elements {
        let Some(list) = members.get(id) else {
            continue;
        };
        let Some(m) = list
            .iter()
            .filter(|m| atlases.contains_key(&m.atlas))
            .min_by(|a, b| {
                let (sa, sb) = (atlases[&a.atlas].set, atlases[&b.atlas].set);
                sb.cmp(&sa)
                    .then(canvas_of(b).cmp(&canvas_of(a)))
                    .then(a.id.cmp(&b.id))
            })
        else {
            continue;
        };
        let a = &atlases[&m.atlas];
        let scale = canvases.get(&canvas_of(m)).map_or(1.0, |w| w / base);
        let width = if m.override_width > 0.0 {
            m.override_width
        } else {
            m.width
        } / scale;
        let height = if m.override_height > 0.0 {
            m.override_height
        } else {
            m.height
        } / scale;
        writeln!(
            out,
            "{name}\t{width}\t{height}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            a.file,
            coord(m.left / a.width),
            coord(m.right / a.width),
            coord(m.top / a.height),
            coord(m.bottom / a.height),
            m.flags & 4 != 0,
            m.flags & 2 != 0,
            m.width,
            m.height,
        )?;
        written += 1;
    }
    let path = root.join("crates/wowlua-engine/data/camelot-atlases-client.txt");
    std::fs::write(&path, out)?;
    println!("{written} atlases to {}", path.display());
    Ok(())
}
