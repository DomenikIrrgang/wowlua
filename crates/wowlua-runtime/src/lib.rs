//! The wowlua runtime (`__wl`, spec codegen §14.7): Lua code emitted as
//! `wowlua/runtime.lua`, sliced by feature: `core` (module registry,
//! `define` for plain Lua, build metadata tables) plus the slices the
//! compiled code uses.

use std::collections::BTreeSet;

const CORE: &str = include_str!("../lua/core.lua");

/// A runtime slice beyond `core`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Slice {
    /// Classes (spec §3.1).
    Class,
    /// Widget classes (spec §3.2); needs `Class`.
    Widget,
    /// Frozen enum tables in debug builds (spec §3.4.3).
    Enum,
    /// User decorators (spec §4.5).
    Decorate,
    /// The profiler (spec §7.3).
    Prof,
    /// Tracing wrappers (spec §7.4).
    Trace,
    /// Runtime type checks (spec §7.5).
    Check,
    /// Reflection metadata (spec §7.8).
    Reflect,
    /// The event, script and timer monitor (spec §7.9).
    Monitor,
    /// Error capture (spec §7.10).
    Errors,
    /// The dev console and window (spec §7.7).
    DevConsole,
}

impl Slice {
    fn source(self) -> &'static str {
        match self {
            Slice::Class => include_str!("../lua/class.lua"),
            Slice::Widget => include_str!("../lua/widget.lua"),
            Slice::Enum => include_str!("../lua/enum.lua"),
            Slice::Decorate => include_str!("../lua/decorate.lua"),
            Slice::Prof => include_str!("../lua/prof.lua"),
            Slice::Trace => include_str!("../lua/trace.lua"),
            Slice::Check => include_str!("../lua/check.lua"),
            Slice::Reflect => include_str!("../lua/reflect.lua"),
            Slice::Monitor => include_str!("../lua/monitor.lua"),
            Slice::Errors => include_str!("../lua/errors.lua"),
            Slice::DevConsole => include_str!("../lua/devconsole.lua"),
        }
    }

    fn requires(self) -> &'static [Slice] {
        match self {
            Slice::Widget => &[Slice::Class],
            _ => &[],
        }
    }
}

/// What the generated runtime contains.
#[derive(Clone, Debug, Default)]
pub struct RuntimeOptions {
    /// Every module of the addon: their export tables exist from the start,
    /// so lazy bindings can refer to modules that load later.
    pub modules: Vec<String>,
    /// Debug builds check that `require` finds loaded modules.
    pub debug: bool,
    /// The profile's `log-level` (`wowlua:log` cannot go below it).
    pub log_level: String,
    /// Entries `wowlua:log` keeps in its ring buffer.
    pub log_history: u32,
    /// `profile-errors`: profiled functions count their errors.
    pub profile_errors: bool,
    /// Dev builds: what the dev console shows about this build, as
    /// `(key, value)` strings, and the runtime version number.
    pub dev_info: Vec<(String, String)>,
    pub runtime_version: u32,
    /// Lua statements filling `wl.meta` (build metadata runtime tables).
    pub meta: Option<String>,
    /// Slices the compiled code uses.
    pub slices: BTreeSet<Slice>,
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
    if options.profile_errors {
        placeholders.push_str("wl.profErrors = true\n");
    }
    if !options.dev_info.is_empty() {
        let fields: Vec<String> = options
            .dev_info
            .iter()
            .map(|(k, v)| format!("{k} = {}", lua_string(v)))
            .collect();
        placeholders.push_str(&format!(
            "wl.devInfo = {{ {}, runtime = {} }}\n",
            fields.join(", "),
            options.runtime_version
        ));
    }
    if !options.log_level.is_empty() {
        placeholders.push_str(&format!(
            "wl.logLevel = {}\nwl.logHistory = {}\n",
            lua_string(&options.log_level),
            options.log_history
        ));
    }
    // A checkout with CRLF line endings must give the same runtime.
    let mut text = CORE
        .replace("\r\n", "\n")
        .replace("--@placeholders\n", &placeholders);
    // Slices in a fixed order, with their dependencies.
    let mut slices = options.slices.clone();
    for s in options.slices.iter() {
        slices.extend(s.requires());
    }
    for s in &slices {
        text.push_str(&s.source().replace("\r\n", "\n"));
    }
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
            slices: BTreeSet::new(),
            ..RuntimeOptions::default()
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

    /// Dev builds share one `/wl` console: the newest runtime serves it,
    /// every addon registers (spec §7.7, decision T98).
    #[test]
    fn dev_console_registry_newest_wins() {
        let lua = mlua::Lua::new();
        lua.load("SlashCmdList = {}").exec().unwrap();
        let build = |version: u32| {
            runtime(&RuntimeOptions {
                slices: [Slice::DevConsole].into_iter().collect(),
                dev_info: vec![("version".into(), "1.0.0".into())],
                runtime_version: version,
                ..RuntimeOptions::default()
            })
        };
        for (name, version) in [("Old", 5_000), ("New", 6_000), ("Older", 4_000)] {
            let ns = lua.create_table().unwrap();
            lua.load(build(version)).call::<()>((name, ns)).unwrap();
        }
        let (version, owner, order): (u32, String, String) = lua
            .load(
                r#"return WOWLUA_DEV.version, (function()
                    for n, d in pairs(WOWLUA_DEV.addons) do if d.console then return n end end
                end)(), table.concat(WOWLUA_DEV.order, ",")"#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            (version, owner.as_str(), order.as_str()),
            (6_000, "New", "Old,New,Older")
        );
    }

    #[test]
    fn classes_widgets_enums_and_decorators() {
        let lua = mlua::Lua::new();
        let code = runtime(&RuntimeOptions {
            slices: [Slice::Widget, Slice::Enum, Slice::Decorate]
                .into_iter()
                .collect(),
            ..RuntimeOptions::default()
        });
        assert!(code.contains("function wl.class("), "widget needs class");
        let ns = lua.create_table().unwrap();
        lua.load(&code).call::<()>(("Addon", ns.clone())).unwrap();
        let wl: mlua::Table = ns.get("__wowlua").unwrap();
        lua.globals().set("wl", wl).unwrap();
        lua.load(
            r##"
            local Animal = wl.class("Animal")
            function Animal.__init(self, name) self.name = name end
            function Animal:speak() return self.name .. " ..." end
            function Animal:__tostring() return "Animal(" .. self.name .. ")" end
            wl.accessor(Animal, "get", "label", function(self) return "#" .. self.name end)
            wl.accessor(Animal, "sget", "kind", function() return "animal" end)
            wl.finalize(Animal)
            local Dog = wl.class("Dog", Animal)
            function Dog.__init(self, name) Animal.__init(self, name); self.tricks = 0 end
            function Dog:speak() return Animal.speak(self) .. "!" end
            wl.finalize(Dog)
            local d = Dog("rex")
            assert(d:speak() == "rex ...!" and tostring(d) == "Animal(rex)")
            assert(d.label == "#rex" and Dog.kind == "animal")
            assert(not pcall(function() d.label = 1 end))
            assert(wl.isInstance(d, Animal) and wl.classOf(d) == Dog and not wl.isInstance({}, Dog))
            local Shape = wl.class("Shape"); wl.abstract(Shape, "area"); wl.finalize(Shape)
            assert(not pcall(Shape.new().area))
            -- widgets: frames are tables
            function CreateFrame(kind) return { kind = kind } end
            local Button = wl.widgetClass("Button")
            function Button.__ctor(cls, label)
                local self = wl.mixin(CreateFrame("Button"), cls)
                self.label = label
                return self
            end
            function Button:text() return self.label end
            wl.finalizeWidget(Button)
            local Fancy = wl.widgetClass("Fancy", Button)
            function Fancy.__ctor(cls, label)
                local self = Button.__ctor(cls, label)
                return self
            end
            function Fancy:text() return "*" .. Button.text(self) end
            function Fancy.__initFields(self) self.stars = 1 end
            wl.finalizeWidget(Fancy)
            local f = Fancy.new("ok")
            assert(f.kind == "Button" and f:text() == "*ok" and wl.isInstance(f, Button))
            local m = wl.xmlMixin(Fancy)
            local frame = {}
            for k, v in pairs(m) do frame[k] = v end
            frame:OnLoad()
            assert(frame.stars == 1 and frame.__class == Fancy)
            -- enums and decorators
            local E = wl.enum({ A = 1 }, "E")
            assert(E.A == 1 and not pcall(function() return E.B end))
            local log = {}
            local function d(tag) return function(fn) log[#log + 1] = tag; return function() return tag .. fn() end end end
            local g = wl.decorate(function() return "x" end, {}, d("1"), d("2"))
            assert(g() == "12x" and log[1] == "2")
            "##,
        )
        .exec()
        .unwrap();
    }
}
