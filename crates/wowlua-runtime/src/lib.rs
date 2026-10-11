//! The wowlua runtime (`__wl`, spec codegen §14.7): Lua code emitted as
//! `wowlua/runtime.lua`, sliced by feature. Milestone M2 has the `core`
//! slice (module registry, `define` for plain Lua, build metadata tables).

const CORE: &str = include_str!("../lua/core.lua");

/// What the generated runtime contains.
#[derive(Clone, Debug, Default)]
pub struct RuntimeOptions {
    /// Every module of the addon: their export tables exist from the start,
    /// so lazy bindings can refer to modules that load later.
    pub modules: Vec<String>,
    /// Debug builds check that `require` finds loaded modules.
    pub debug: bool,
    /// Lua statements filling `wl.meta` (build metadata runtime tables).
    pub meta: Option<String>,
}

fn lua_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The text of `wowlua/runtime.lua`.
pub fn runtime(options: &RuntimeOptions) -> String {
    let mut placeholders = String::new();
    if !options.modules.is_empty() {
        let ids: Vec<String> = options.modules.iter().map(|m| lua_string(m)).collect();
        placeholders.push_str(&format!(
            "for _, id in ipairs({{ {} }}) do\n\tmodules[id] = modules[id] or {{}}\nend\n",
            ids.join(", ")
        ));
    }
    placeholders.push_str(&format!("wl.debug = {}\n", options.debug));
    // A checkout with CRLF line endings must give the same runtime.
    let mut text = CORE
        .replace("\r\n", "\n")
        .replace("--@placeholders\n", &placeholders);
    if let Some(meta) = &options.meta {
        text.push_str(meta);
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_resolve_and_debug_builds_check_load_order() {
        let lua = mlua::Lua::new();
        let code = runtime(&RuntimeOptions {
            modules: vec!["a".into(), "b".into()],
            debug: true,
            meta: None,
        });
        let ns = lua.create_table().unwrap();
        lua.load(&code).call::<()>(("Addon", ns.clone())).unwrap();
        let wl: mlua::Table = ns.get("__wowlua").unwrap();
        lua.globals().set("wl", wl).unwrap();
        lua.load(
            r#"
            local lazy = wl.require("b", true)       -- placeholder before b loads
            local a = wl.module("a"); a.x = 1
            assert(wl.require("a").x == 1)
            assert(not pcall(wl.require, "b"))       -- debug: not loaded yet
            wl.module("b").y = 2
            assert(lazy.y == 2)
            wl.define("legacy", { round = math.floor })
            assert(wl.require("legacy").round(1.5) == 1)
            "#,
        )
        .exec()
        .unwrap();
    }
}
