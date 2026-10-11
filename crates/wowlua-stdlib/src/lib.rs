//! The wowlua standard library (spec chapter 8): `wowlua:*` modules,
//! written in wowlua and compiled into each addon that imports them, and the
//! compiler's implicit declarations.

/// A file shipped inside the compiler.
#[derive(Clone, Copy, Debug)]
pub struct File {
    /// Its path as diagnostics show it (`<wowlua>/widgets.d.wowlua`).
    pub path: &'static str,
    pub text: &'static str,
}

/// Declarations in scope in every package (until the generated `wow-api`
/// packages of milestone M4 provide them).
pub fn declarations() -> &'static [File] {
    &[File {
        path: "<wowlua>/widgets.d.wowlua",
        text: include_str!("../decl/widgets.d.wowlua"),
    }]
}

/// A `wowlua:*` module: its wowlua source and, optionally, a declaration
/// file with the `@intrinsic` declarations of its framework decorators.
#[derive(Clone, Debug)]
pub struct Module {
    pub name: String,
    pub source: String,
    pub declaration: Option<String>,
}

/// The modules shipped with this compiler version (none yet: they arrive
/// with milestones M5 and M8).
const MODULES: &[(&str, &str, Option<&str>)] = &[];

/// A standard-library module by name (`events` for `wowlua:events`).
///
/// `WOWLUA_STDLIB_DIR` points the compiler at a directory of modules
/// (`<name>.wowlua`, `<name>.d.wowlua`) instead, for developing and testing
/// the standard library.
pub fn module(name: &str) -> Option<Module> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return None;
    }
    if let Some(dir) = std::env::var_os("WOWLUA_STDLIB_DIR") {
        let dir = std::path::PathBuf::from(dir);
        let source = std::fs::read_to_string(dir.join(format!("{name}.wowlua"))).ok()?;
        let declaration = std::fs::read_to_string(dir.join(format!("{name}.d.wowlua"))).ok();
        return Some(Module {
            name: name.to_string(),
            source,
            declaration,
        });
    }
    MODULES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(n, source, declaration)| Module {
            name: n.to_string(),
            source: source.to_string(),
            declaration: declaration.map(str::to_string),
        })
}

/// The source path of a standard-library module inside a build
/// (`@wowlua/events.wowlua`); its module ID is `wowlua:events`.
pub fn src_rel(name: &str) -> String {
    format!("@wowlua/{name}.wowlua")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_names_are_checked() {
        assert!(module("../etc").is_none());
        assert!(module("").is_none());
        assert_eq!(src_rel("events"), "@wowlua/events.wowlua");
        assert!(declarations()[0].text.contains("@widget(\"Button\")"));
    }
}
