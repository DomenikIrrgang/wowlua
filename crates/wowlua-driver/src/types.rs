//! `wowlua types convert`: LuaLS `---@` annotations → `.d.wowlua`
//! declarations (spec package manager §10.9).

// Only a computer has this (spec M14, decision T394).
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};

/// Converts the annotated `.lua` file, or every `.lua` file under the
/// directory, into one declaration file `<out_dir>/<name>.d.wowlua`.
/// Returns the written path and the number of converted input files.
pub fn convert(path: &Path, out_dir: &Path) -> Result<(PathBuf, usize), String> {
    let mut inputs: Vec<PathBuf> = if path.is_dir() {
        walkdir::WalkDir::new(path)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .map(|e| e.into_path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")))
            .collect()
    } else if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        return Err(format!("`{}` does not exist", path.display()));
    };
    inputs.sort();
    if inputs.is_empty() {
        return Err(format!("no `.lua` files in `{}`", path.display()));
    }
    let mut files = Vec::new();
    for p in &inputs {
        let bytes = std::fs::read(p).map_err(|e| format!("cannot read `{}`: {e}", p.display()))?;
        files.push((
            p.display().to_string(),
            String::from_utf8_lossy(&bytes).into_owned(),
        ));
    }
    let decls = wowlua_types_gen::luals::convert(&files);
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "types".into());
    let source = path
        .file_name()
        .map_or_else(|| name.clone(), |n| n.to_string_lossy().into_owned());
    let header =
        format!("Converted from the LuaLS annotations of `{source}` by `wowlua types convert`.");
    let text = decls.emit(&header);
    // The output must parse; a converter bug is reported, not written.
    let mut db = wowlua_span::SourceDb::new();
    let file = db.add("out.d.wowlua", text.clone());
    let parse = wowlua_syntax::parse(text.as_bytes(), file);
    if let Some(d) = parse.diagnostics.iter().find(|d| d.is_error()) {
        return Err(format!(
            "internal error: the converted declarations do not parse: {}",
            d.message
        ));
    }
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create `{}`: {e}", out_dir.display()))?;
    let out = out_dir.join(format!("{name}.d.wowlua"));
    std::fs::write(&out, text).map_err(|e| format!("cannot write `{}`: {e}", out.display()))?;
    Ok((out, inputs.len()))
}
