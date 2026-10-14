//! The wowlua runtime is written in wowlua (`crates/wowlua-runtime/wowlua`)
//! and compiled into the Lua the compiler embeds (`…/lua`), which stays
//! committed: the compiler cannot compile its own runtime while it is built.
//! Each slice is compiled on its own with the runtime package's `runtime`
//! profile; slices after `core` see its locals through
//! `decl/shared.d.wowlua`. Warnings fail the generation.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// The generated slices: `(path of the .lua file, content)`.
pub fn generate(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    let package = root.join("crates/wowlua-runtime").canonicalize()?;
    let mut session = wowlua_driver::Session::new();
    let options = wowlua_driver::LoadOptions {
        cwd: package.clone(),
        skip_dependencies: true,
        ..Default::default()
    };
    let Some(project) = wowlua_driver::load_project(&mut session, &options) else {
        bail!("{}", render(&session));
    };
    let mut sources: Vec<PathBuf> = std::fs::read_dir(package.join("wowlua"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "wowlua"))
        .collect();
    sources.sort();
    let shared = package.join("decl/shared.d.wowlua");
    let mut out = Vec::new();
    for source in sources {
        let slice = source.file_stem().unwrap().to_string_lossy().into_owned();
        let mut files = vec![source.clone()];
        if slice != "core" {
            files.push(shared.clone());
        }
        let mut request = wowlua_driver::BuildRequest::new("runtime");
        request.write = false;
        request.only = Some(wowlua_driver::Only {
            files,
            project_declarations: true,
        });
        let built = wowlua_driver::build(&mut session, &project, &request);
        project.apply_lints(&mut session);
        if session
            .diagnostics
            .iter()
            .any(|d| d.is_error() || d.severity == wowlua_diagnostics::Severity::Warning)
        {
            bail!(
                "the runtime slice `{slice}` does not compile cleanly:\n{}",
                render(&session)
            );
        }
        let Some(built) = built else {
            bail!("the runtime slice `{slice}` did not build");
        };
        let lua = wowlua_driver::build::lua_outputs(&built);
        let [(_, bytes)] = lua.as_slice() else {
            bail!("the runtime slice `{slice}` has {} outputs", lua.len());
        };
        out.push((
            package.join("lua").join(format!("{slice}.lua")),
            String::from_utf8(bytes.clone())?,
        ));
    }
    Ok(out)
}

fn render(session: &wowlua_driver::Session) -> String {
    let renderer = wowlua_diagnostics::Renderer::new(
        wowlua_diagnostics::MessageFormat::Short,
        wowlua_diagnostics::ColorChoice::Never,
    );
    session
        .diagnostics
        .iter()
        .map(|d| renderer.render_to_string(&session.db, d))
        .collect::<Vec<_>>()
        .join("\n")
}
