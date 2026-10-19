//! Keybindings and the addons a test loads.
//!
//! Bindings: those an addon's `Bindings.xml` declares (`defineBinding`:
//! its name and the code to run) and the keys bound to them (`SetBinding`);
//! `pressKey(key)` runs the bound binding with "down", then "up"
//! (`runOnUp` ones only), `pressBinding(name)` runs it directly.
//!
//! Addons: `defineAddon(name, { metadata, loadOnDemand, load })` makes an
//! addon the client knows (its TOC metadata), loaded by `C_AddOns.LoadAddOn`
//! when on demand (`load` runs, then `ADDON_LOADED`). The addon folders a
//! test loads (`test.addons`, [`add_user_addon`]) are known the same way:
//! those loaded at startup get their `ADDON_LOADED` at login, before the
//! addon under test's; a load-on-demand one's files load with
//! `C_AddOns.LoadAddOn`, after its dependencies.
//!
//! Unverified: `GetBinding`'s results beyond the command and keys, and the
//! addons' reasons and security strings.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// A binding from an addon's `Bindings.xml`.
pub struct Binding {
    pub name: String,
    pub header: Option<String>,
    pub action: Value,
    pub run_on_up: bool,
}

/// An override binding: its owner, priority, key and command (`CLICK
/// name:button`, `SPELL name`, `MACRO x`, `ITEM name` or a binding's).
pub struct Override {
    pub owner: Value,
    pub priority: bool,
    pub key: String,
    pub command: String,
}

/// An addon a test defined.
pub struct Addon {
    pub name: String,
    pub metadata: Vec<(String, String)>,
    pub on_demand: bool,
    pub load: Value,
    /// An addon folder's (`test.addons`): its files, loaded on demand.
    pub files: Option<AddonFiles>,
    /// Its dependencies, loaded before it on demand.
    pub deps: Vec<String>,
    /// Loaded at startup: `ADDON_LOADED` at login.
    pub at_login: bool,
    /// The addon under test, whose `ADDON_LOADED` the login tells itself.
    pub own: bool,
}

/// An addon folder's files: its folder, TOC, addon table and whether it
/// has a `Bindings.xml`.
pub struct AddonFiles {
    pub dir: std::path::PathBuf,
    pub toc: std::path::PathBuf,
    pub table: Value,
    pub bindings: bool,
}

#[derive(Default)]
pub struct Bindings {
    pub bindings: Vec<Binding>,
    /// Key → binding name (`SetBinding`).
    pub keys: Vec<(String, String)>,
    /// Override bindings (`SetOverrideBinding*`), latest last.
    pub overrides: Vec<Override>,
    pub addons: Vec<Addon>,
    /// The addons `C_AddOns.DisableAddOn` disabled, lower-case.
    pub disabled: std::collections::HashSet<String>,
    /// `C_AddOns.SetAddonVersionCheck(false)`: out-of-date addons load.
    pub no_version_check: bool,
    /// The modified clicks (`<ModifiedClick action default>`): action,
    /// key (`SHIFT`, `ALT-CTRL`, `NONE`).
    pub modified_clicks: Vec<(String, String)>,
}

fn string(vm: &mut Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

/// `defineBinding(name, action [, { header, runOnUp }])`.
pub fn define_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let action = c.arg(2);
    widgets::keep(vm, action);
    let (header, run_on_up) = match c.arg(3) {
        Value::Table(opts) => {
            let h = field(vm, opts, "header");
            let up = field(vm, opts, "runOnUp").truthy();
            (string(vm, h), up)
        }
        _ => (None, false),
    };
    engine_mut(vm).world.bindings.bindings.push(Binding {
        name,
        header,
        action,
        run_on_up,
    });
    Ok(vec![])
}

/// An addon's `Bindings.xml`, as the client reads it at startup: each
/// `<Binding name header runOnUp>` with its body compiled as the addon's
/// code (`None`: Blizzard's, secure), a function of `keystate`.
pub fn load_bindings_xml(
    vm: &mut Vm,
    path: &std::path::Path,
    addon: Option<&str>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let root = crate::ui::xml::parse(&text).map_err(|e| format!("Bindings.xml: {e}"))?;
    let mut list = Vec::new();
    fn collect(el: &crate::ui::xml::Element, out: &mut Vec<std::rc::Rc<crate::ui::xml::Element>>) {
        for c in &el.children {
            if c.tag.eq_ignore_ascii_case("Binding") {
                out.push(c.clone());
            } else {
                collect(c, out);
            }
        }
    }
    collect(&root, &mut list);
    // The modified clicks, with their defaults.
    fn clicks(el: &crate::ui::xml::Element, out: &mut Vec<(String, String)>) {
        for c in &el.children {
            if c.tag.eq_ignore_ascii_case("ModifiedClick") {
                if let (Some(action), Some(key)) = (c.attr("action"), c.attr("default")) {
                    out.push((action.to_string(), key.to_string()));
                }
            } else {
                clicks(c, out);
            }
        }
    }
    let mut found = Vec::new();
    clicks(&root, &mut found);
    for (action, key) in found {
        let known = &mut engine_mut(vm).world.bindings.modified_clicks;
        if !known.iter().any(|(a, _)| *a == action) {
            known.push((action, key));
        }
    }
    for b in list {
        let Some(name) = b.attr("name") else {
            continue;
        };
        let src = format!(
            "{}return function(keystate) {} end",
            "\n".repeat(b.line.saturating_sub(1)),
            b.text
        );
        let chunk = format!(
            "@{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        let f = match addon {
            Some(addon) => vm.load_as(src.as_bytes(), &chunk, addon)?,
            None => vm.load(src.as_bytes(), &chunk)?,
        };
        let action = match vm.call(f, &[]) {
            Ok(r) => r.first().copied().unwrap_or(Value::Nil),
            Err(e) => {
                let msg = string(vm, e.value).unwrap_or_default();
                return Err(format!("Bindings.xml: {msg}"));
            }
        };
        widgets::keep(vm, action);
        engine_mut(vm).world.bindings.bindings.push(Binding {
            name: name.to_string(),
            // Its category (`category="BINDING_HEADER_MOVEMENT"`, what
            // `GetBinding` answers: the full probe), else its header.
            header: b
                .attr("category")
                .or_else(|| b.attr("header"))
                .map(str::to_string),
            action,
            run_on_up: b
                .attr("runOnUp")
                .is_some_and(|v| v.eq_ignore_ascii_case("true")),
        });
    }
    Ok(())
}

fn run_binding(vm: &mut Vm, name: &str, keystate: &str) {
    let found = engine(vm)
        .world
        .bindings
        .bindings
        .iter()
        .find(|b| b.name == name)
        .map(|b| (b.action, b.run_on_up));
    if let Some((action, run_on_up)) = found
        && (keystate == "down" || run_on_up)
    {
        let k = vm.str(keystate);
        vm.call_from_engine(action, &[k]);
    }
}

/// `pressBinding(name [, keystate])`.
pub fn press_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let state = vm
        .opt_str(&c, 2)?
        .map(|s| String::from_utf8_lossy(vm.str_bytes(s)).into_owned())
        .unwrap_or_else(|| "down".into());
    crate::input::as_user(vm, |vm| run_binding(vm, &name, &state));
    Ok(vec![])
}

/// `pressKey(key)`: the key pressed and released as the user
/// (`input::press_key`).
pub fn press_key(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = vm.check_str(&c, 1)?;
    let key = String::from_utf8_lossy(vm.str_bytes(key)).to_uppercase();
    crate::input::as_user(vm, |vm| crate::input::press_key(vm, &key));
    Ok(vec![])
}

/// The command a key runs: a priority override's, an override's, else the
/// key's binding (`GetBindingAction(key, true)`; unverified: the latest of
/// two owners' overrides wins).
pub fn action_for(vm: &Vm, key: &str) -> Option<String> {
    let b = &engine(vm).world.bindings;
    for priority in [true, false] {
        if let Some(o) = b
            .overrides
            .iter()
            .rev()
            .find(|o| o.priority == priority && o.key == key)
        {
            return Some(o.command.clone());
        }
    }
    b.keys
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, command)| command.clone())
}

/// Runs a key's command, "down" or "up": a binding's code, or a `CLICK`
/// binding's button (`OnClick(button, down)`; unverified: on the down and
/// the up whatever the button registered).
pub fn run_command(vm: &mut Vm, command: &str, keystate: &str) {
    if let Some(target) = command.strip_prefix("CLICK ") {
        let (name, button) = target.split_once(':').unwrap_or((target, "LeftButton"));
        let frame = vm.get_global(name);
        if let Some(id) = widgets::widget_of(vm, frame) {
            let button = vm.str(button);
            let down = Value::Bool(keystate == "down");
            for script in ["PreClick", "OnClick", "PostClick"] {
                widgets::run_script(vm, id, script, &[button, down]);
            }
        }
        return;
    }
    run_binding(vm, command, keystate);
}

/// Whether a binding command exists: a binding's (`Bindings.xml`), or a
/// `CLICK`, `SPELL`, `MACRO` or `ITEM` one (unverified).
fn known_command(vm: &Vm, command: &str) -> bool {
    ["CLICK ", "SPELL ", "MACRO ", "ITEM "]
        .iter()
        .any(|p| command.starts_with(p))
        || engine(vm)
            .world
            .bindings
            .bindings
            .iter()
            .any(|b| b.name == command)
}

/// `SetBinding(key [, command])`: whether it bound. A key in lower case or
/// an unknown command binds nothing (false); the key is kept as written,
/// its modifiers in the order given (`SHIFT-ALT-CTRL-F8` is not
/// `CTRL-SHIFT-ALT-F8`: `/stdtest engine answers` "Binding set …"). A
/// binding set tells `UPDATE_BINDINGS` at once (unverified:
/// warcraft.wiki.gg's UPDATE_BINDINGS, "fired after completion of
/// LoadBindings, SaveBindings, and SetBinding (and its derivatives)";
/// Bartender4 guards its handler against the SetBinding it calls there).
fn set_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = vm.check_str(&c, 1)?;
    let key = String::from_utf8_lossy(vm.str_bytes(key)).into_owned();
    let command = string(vm, c.arg(2));
    if key != key.to_uppercase()
        || command
            .as_deref()
            .is_some_and(|cmd| !known_command(vm, cmd))
    {
        return Ok(vec![Value::Bool(false)]);
    }
    let keys = &mut engine_mut(vm).world.bindings.keys;
    keys.retain(|(k, _)| *k != key);
    if let Some(command) = command {
        keys.push((key, command));
    }
    widgets::fire_event(vm, "UPDATE_BINDINGS", &[]);
    Ok(vec![Value::Bool(true)])
}

/// `GetBindingAction(key [, checkOverride])`: the key's command, "" for
/// none; overrides only when asked.
fn get_binding_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = vm.check_str(&c, 1)?;
    let key = String::from_utf8_lossy(vm.str_bytes(key)).to_uppercase();
    let action = if c.arg(2).truthy() {
        action_for(vm, &key)
    } else {
        engine(vm)
            .world
            .bindings
            .keys
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, b)| b.clone())
    };
    Ok(vec![vm.str(&action.unwrap_or_default())])
}

/// `C_KeyBindings.GetBindingByKey(key)`: the command the key runs, nil
/// for none (unverified: with overrides).
fn get_binding_by_key(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = vm.check_str(&c, 1)?;
    let key = String::from_utf8_lossy(vm.str_bytes(key)).to_uppercase();
    Ok(vec![match action_for(vm, &key) {
        Some(a) => vm.str(&a),
        None => Value::Nil,
    }])
}

/// Sets (or, without a command, removes) `owner`'s override of `key`.
fn set_override(vm: &mut Vm, owner: Value, priority: bool, key: String, command: Option<String>) {
    let list = &mut engine_mut(vm).world.bindings.overrides;
    list.retain(|o| !(o.owner.raw_eq(owner) && o.key == key));
    if let Some(command) = command {
        list.push(Override {
            owner,
            priority,
            key,
            command,
        });
    }
}

/// The owner, priority and key of a `SetOverrideBinding*` call.
fn override_args(
    vm: &mut Vm,
    c: &NativeCall,
) -> Result<(Value, bool, String), wowlua_vm::LuaError> {
    let key = vm.check_str(c, 3)?;
    let key = String::from_utf8_lossy(vm.str_bytes(key)).to_uppercase();
    Ok((c.arg(1), c.arg(2).truthy(), key))
}

/// `SetOverrideBinding(owner, isPriority, key [, command])`.
fn set_override_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let (owner, priority, key) = override_args(vm, &c)?;
    let command = string(vm, c.arg(4));
    set_override(vm, owner, priority, key, command);
    Ok(vec![])
}

/// `SetOverrideBindingClick(owner, isPriority, key, buttonName [,
/// mouseButton])`: `CLICK buttonName:mouseButton` (`LeftButton` unless
/// given).
fn set_override_binding_click(vm: &mut Vm, c: NativeCall) -> Ret {
    let (owner, priority, key) = override_args(vm, &c)?;
    let command = string(vm, c.arg(4)).map(|name| {
        let button = string(vm, c.arg(5)).unwrap_or_else(|| "LeftButton".into());
        format!("CLICK {name}:{button}")
    });
    set_override(vm, owner, priority, key, command);
    Ok(vec![])
}

/// `SetOverrideBindingSpell` / `Macro` / `Item`: `SPELL name`, `MACRO x`,
/// `ITEM name`.
fn set_override_with(vm: &mut Vm, c: &NativeCall, kind: &str) -> Ret {
    let (owner, priority, key) = override_args(vm, c)?;
    let what = match c.arg(4) {
        Value::Num(n) if n.fract() == 0.0 => Some(format!("{}", n as i64)),
        v => string(vm, v),
    };
    let command = what.map(|w| format!("{kind} {w}"));
    set_override(vm, owner, priority, key, command);
    Ok(vec![])
}

fn set_override_binding_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    set_override_with(vm, &c, "SPELL")
}

fn set_override_binding_macro(vm: &mut Vm, c: NativeCall) -> Ret {
    set_override_with(vm, &c, "MACRO")
}

fn set_override_binding_item(vm: &mut Vm, c: NativeCall) -> Ret {
    set_override_with(vm, &c, "ITEM")
}

/// `ClearOverrideBindings(owner)`.
fn clear_override_bindings(vm: &mut Vm, c: NativeCall) -> Ret {
    let owner = c.arg(1);
    engine_mut(vm)
        .world
        .bindings
        .overrides
        .retain(|o| !o.owner.raw_eq(owner));
    Ok(vec![])
}

fn keys_of(vm: &Vm, command: &str) -> Vec<String> {
    engine(vm)
        .world
        .bindings
        .keys
        .iter()
        .filter(|(_, b)| b == command)
        .map(|(k, _)| k.clone())
        .collect()
}

fn get_binding_key(vm: &mut Vm, c: NativeCall) -> Ret {
    let command = vm.check_str(&c, 1)?;
    let command = String::from_utf8_lossy(vm.str_bytes(command)).into_owned();
    let keys = keys_of(vm, &command);
    Ok(keys.iter().map(|k| vm.str(k)).collect())
}

fn get_num_bindings(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        engine(vm).world.bindings.bindings.len() as f64
    )])
}

/// `GetBinding(index)`: the command, its category (header), its keys.
fn get_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = vm.check_int(&c, 1)?;
    let found = usize::try_from(i - 1)
        .ok()
        .and_then(|i| engine(vm).world.bindings.bindings.get(i))
        .map(|b| (b.name.clone(), b.header.clone()));
    let Some((name, header)) = found else {
        return Ok(vec![]);
    };
    let mut out = vec![vm.str(&name), header.map_or(Value::Nil, |h| vm.str(&h))];
    for k in keys_of(vm, &name) {
        out.push(vm.str(&k));
    }
    Ok(out)
}

// ----------------------------------------------------------------- addons

/// `defineAddon(name, { metadata = { Title = …, Version = … }, loadOnDemand,
/// load })`.
pub fn define_addon(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let mut metadata = Vec::new();
    let (mut on_demand, mut load) = (false, Value::Nil);
    if let Value::Table(opts) = c.arg(2) {
        if let Value::Table(m) = field(vm, opts, "metadata") {
            let mut k = Value::Nil;
            while let Some((key, v)) = vm.heap.table(m).next(k).ok().flatten() {
                k = key;
                if let (Some(key), Some(v)) = (string(vm, key), string(vm, v)) {
                    metadata.push((key, v));
                }
            }
        }
        on_demand = field(vm, opts, "loadOnDemand").truthy();
        load = field(vm, opts, "load");
        widgets::keep(vm, load);
    }
    engine_mut(vm).world.bindings.addons.push(Addon {
        name,
        metadata,
        on_demand,
        load,
        files: None,
        deps: Vec::new(),
        at_login: false,
        own: false,
    });
    Ok(vec![])
}

/// Makes an addon folder known to the client (`test.addons`), `table` its
/// addon table: loaded at startup unless on demand (its files are the
/// caller's to load then), with or without a `Bindings.xml`.
pub fn add_user_addon(vm: &mut Vm, addon: &crate::ui::UserAddon, table: Value, bindings: bool) {
    widgets::keep(vm, table);
    engine_mut(vm).world.bindings.addons.push(Addon {
        name: addon.name.clone(),
        metadata: addon.metadata.clone(),
        on_demand: addon.on_demand,
        load: Value::Nil,
        files: Some(AddonFiles {
            dir: addon.dir.clone(),
            toc: addon.toc.clone(),
            table,
            bindings,
        }),
        deps: addon.deps.clone(),
        at_login: !addon.on_demand,
        own: false,
    });
}

/// Makes the addon under test known to the client as its TOC describes it
/// (its metadata for `C_AddOns.GetAddOnMetadata`), loaded at startup, last.
pub fn add_own_addon(vm: &mut Vm, addon: &crate::ui::UserAddon, table: Value, bindings: bool) {
    add_user_addon(vm, addon, table, bindings);
    if let Some(a) = engine_mut(vm).world.bindings.addons.last_mut() {
        a.at_login = true;
        a.own = true;
    }
}

/// The addons loaded at startup that `ADDON_LOADED` tells at login, in
/// load order: name and whether each has a `Bindings.xml`.
pub fn login_addons(vm: &Vm) -> Vec<(String, bool)> {
    engine(vm)
        .world
        .bindings
        .addons
        .iter()
        .filter(|a| a.at_login && !a.own)
        .map(|a| (a.name.clone(), a.files.as_ref().is_some_and(|f| f.bindings)))
        .collect()
}

/// The addon an `uiAddon` argument names: its name or index.
fn addon_index(vm: &mut Vm, v: Value) -> Option<usize> {
    let addons = &engine(vm).world.bindings.addons;
    match vm.hidden(v) {
        Value::Num(n) => {
            let i = n as usize;
            (i >= 1 && i <= addons.len()).then(|| i - 1)
        }
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            engine(vm)
                .world
                .bindings
                .addons
                .iter()
                .position(|a| a.name.eq_ignore_ascii_case(&name))
        }
        _ => None,
    }
}

/// `GetAddOnMetadata(name, field)`: the TOC's field, any case.
fn get_addon_metadata(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = addon_index(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let field = string(vm, c.arg(2)).unwrap_or_default();
    let addons = &engine(vm).world.bindings.addons;
    let mut value = meta_of(&addons[i], &field);
    // "Every addon gets a group set" (Blizzard's AddonList): the TOC's,
    // else one the client gives addons with similar names and
    // dependencies, else the addon's own name. Unverified: the client's
    // similarity rule (here: a dependency its name starts with).
    if value.is_none() && field.eq_ignore_ascii_case("Group") {
        let a = &addons[i];
        let lower = a.name.to_lowercase();
        let parent = a
            .deps
            .iter()
            .find(|d| lower.starts_with(&d.to_lowercase()) && lower != d.to_lowercase());
        value = Some(parent.cloned().unwrap_or_else(|| a.name.clone()));
    }
    Ok(vec![value.map_or(Value::Nil, |v| vm.str(&v))])
}

/// A TOC field of an addon, any case.
fn meta_of(a: &Addon, field: &str) -> Option<String> {
    a.metadata
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(field))
        .map(|(_, v)| v.clone())
}

/// The addon an argument names, or nothing for one not known.
fn known(vm: &mut Vm, c: &NativeCall) -> Option<usize> {
    addon_index(vm, c.arg(1))
}

/// `C_AddOns.GetAddOnName(addon)`.
fn get_addon_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = known(vm, &c) else {
        return Ok(vec![]);
    };
    let name = engine(vm).world.bindings.addons[i].name.clone();
    Ok(vec![vm.str(&name)])
}

/// `C_AddOns.GetAddOnTitle(addon)`: its `Title`, else its name (as
/// `GetAddOnInfo`'s).
fn get_addon_title(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = known(vm, &c) else {
        return Ok(vec![]);
    };
    let a = &engine(vm).world.bindings.addons[i];
    let title = meta_of(a, "Title").unwrap_or_else(|| a.name.clone());
    Ok(vec![vm.str(&title)])
}

/// `C_AddOns.GetAddOnNotes(addon)`: its `Notes`, else nil (unverified).
fn get_addon_notes(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = known(vm, &c) else {
        return Ok(vec![]);
    };
    let notes = meta_of(&engine(vm).world.bindings.addons[i], "Notes");
    Ok(vec![notes.map_or(Value::Nil, |n| vm.str(&n))])
}

/// Whether an argument names an addon the client has, and whether it is
/// one of Blizzard's: a test-defined addon, one of Blizzard's UI code
/// (loaded or loadable), or one whose `ADDON_LOADED` came (the addon under
/// test's).
fn addon_kind(vm: &mut Vm, c: &NativeCall) -> Option<bool> {
    if known(vm, c).is_some() {
        return Some(false);
    }
    let name = string(vm, c.arg(1))?;
    if crate::ui::is_blizzard_addon(vm, &name) {
        return Some(true);
    }
    engine(vm)
        .world
        .loaded_addons
        .contains(&name.to_lowercase())
        .then_some(false)
}

/// `C_AddOns.GetAddOnSecurity(addon)`: `Enum.AddOnSecurityStatus` Secure
/// (0) for Blizzard's, Insecure (1) for the others, as `GetAddOnInfo`'s
/// "INSECURE" (recorded: `Blizzard_GroupFinder_VanillaStyle` 0, StdTest
/// 1); nothing for an unknown one (unverified).
fn get_addon_security(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match addon_kind(vm, &c) {
        Some(blizzard) => vec![Value::Num(if blizzard { 0.0 } else { 1.0 })],
        None => vec![],
    })
}

/// `C_AddOns.GetAddOnDependencies(addon)`: its required dependencies.
fn get_addon_dependencies(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = known(vm, &c) else {
        return Ok(vec![]);
    };
    let deps = engine(vm).world.bindings.addons[i].deps.clone();
    Ok(deps.iter().map(|d| vm.str(d)).collect())
}

/// `C_AddOns.IsAddOnLoadable(addon)`: every known addon loads, the reason
/// empty; an unknown one is "MISSING" (recorded for StdTest, a Blizzard
/// addon and a missing one; unverified: a disabled addon's).
fn is_addon_loadable(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match addon_kind(vm, &c) {
        Some(_) => vec![Value::Bool(true), vm.str("")],
        None => vec![Value::Bool(false), vm.str("MISSING")],
    })
}

fn yes(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// `C_AddOns.EnableAllAddOns()`, `ResetAddOns()`: none disabled (the
/// session started with all enabled).
fn enable_all_addons(vm: &mut Vm, _c: NativeCall) -> Ret {
    engine_mut(vm).world.bindings.disabled.clear();
    Ok(vec![])
}

fn disable_all_addons(vm: &mut Vm, _c: NativeCall) -> Ret {
    let names: Vec<String> = engine(vm)
        .world
        .bindings
        .addons
        .iter()
        .map(|a| a.name.to_lowercase())
        .collect();
    engine_mut(vm).world.bindings.disabled.extend(names);
    Ok(vec![])
}

/// `C_AddOns.IsAddonVersionCheckEnabled()`: on unless turned off
/// (unverified: the account's setting).
fn is_version_check_enabled(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        !engine(vm).world.bindings.no_version_check,
    )])
}

fn set_version_check(vm: &mut Vm, c: NativeCall) -> Ret {
    engine_mut(vm).world.bindings.no_version_check = !c.arg(1).truthy();
    Ok(vec![])
}

fn get_num_addons(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        engine(vm).world.bindings.addons.len() as f64
    )])
}

/// `GetAddOnInfo(name)`: name, title, notes, loadable, reason, security.
fn get_addon_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = addon_index(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let a = &engine(vm).world.bindings.addons[i];
    let meta = |k: &str| {
        a.metadata
            .iter()
            .find(|(m, _)| m.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.clone())
    };
    let (name, title, notes) = (a.name.clone(), meta("Title"), meta("Notes"));
    let title = vm.str(&title.unwrap_or_else(|| name.clone()));
    Ok(vec![
        vm.str(&name),
        title,
        notes.map_or(Value::Nil, |n| vm.str(&n)),
        Value::Bool(true),
        Value::Nil,
        vm.str("INSECURE"),
    ])
}

fn is_addon_load_on_demand(vm: &mut Vm, c: NativeCall) -> Ret {
    let lod =
        addon_index(vm, c.arg(1)).is_some_and(|i| engine(vm).world.bindings.addons[i].on_demand);
    Ok(vec![Value::Bool(lod)])
}

/// Loads a test-defined addon on demand: its `load`, then `ADDON_LOADED`.
/// `None` for a name the test did not define.
pub fn load_defined(vm: &mut Vm, name: &str) -> Option<bool> {
    let i = engine(vm)
        .world
        .bindings
        .addons
        .iter()
        .position(|a| a.name.eq_ignore_ascii_case(name))?;
    let lower = name.to_lowercase();
    // One loaded at startup has its files loaded already (its
    // `ADDON_LOADED` waits for the login).
    if engine(vm).world.loaded_addons.contains(&lower)
        || engine(vm).world.bindings.addons[i].at_login
    {
        return Some(true);
    }
    let (load, real, deps) = {
        let a = &engine(vm).world.bindings.addons[i];
        (a.load, a.name.clone(), a.deps.clone())
    };
    // Marked loading first, so a dependency cycle ends.
    engine_mut(vm).world.loaded_addons.insert(lower);
    for dep in deps {
        if load_defined(vm, &dep).is_none() {
            crate::ui::load_addon(vm, &dep);
        }
    }
    if !load.is_nil() {
        vm.call_from_engine(load, &[]);
    }
    let files = engine(vm).world.bindings.addons[i]
        .files
        .as_ref()
        .map(|f| (f.dir.clone(), f.toc.clone(), f.table, f.bindings));
    let mut bindings = false;
    if let Some((dir, toc, table, has)) = files {
        bindings = has;
        // Its files' errors reach the error handler, as the client's do
        // (those the client drops stay the engine's alone).
        let before = engine(vm).world.ui.errors.len();
        crate::ui::load_other_user_addon(vm, &dir, &toc, &real, table);
        let (dropped, errors): (Vec<String>, Vec<String>) = engine_mut(vm)
            .world
            .ui
            .errors
            .drain(before..)
            .partition(|e| e.starts_with(crate::ui::DROPPED));
        engine_mut(vm).world.ui.errors.extend(dropped);
        for e in errors {
            let (handler, message) = (vm.error_handler, vm.str(&e));
            vm.call_from_engine(handler, &[message]);
        }
    }
    let n = vm.str(&real);
    widgets::fire_event(vm, "ADDON_LOADED", &[n, Value::Bool(bindings)]);
    Some(true)
}

/// `C_AddOns.DoesAddOnExist(name)`: an addon the client knows (one the
/// test loads, or a Blizzard addon of the loaded UI code). Unverified for
/// Blizzard's addons.
fn does_addon_exist(vm: &mut Vm, c: NativeCall) -> Ret {
    let known = addon_index(vm, c.arg(1)).is_some()
        || string(vm, c.arg(1)).is_some_and(|n| crate::ui::is_blizzard_addon(vm, &n));
    Ok(vec![Value::Bool(known)])
}

/// `C_AddOns.GetAddOnEnableState(name [, character])`: `All` (2) for an
/// addon the client knows and loads, `None` (0) for one disabled or not
/// known. Unverified: `Some` (enabled for other characters only), the
/// character argument.
fn get_addon_enable_state(vm: &mut Vm, c: NativeCall) -> Ret {
    let state = match addon_index(vm, c.arg(1)) {
        Some(i) => {
            let name = engine(vm).world.bindings.addons[i].name.to_lowercase();
            if engine(vm).world.bindings.disabled.contains(&name) {
                0.0
            } else {
                2.0
            }
        }
        None => 0.0,
    };
    Ok(vec![Value::Num(state)])
}

/// `C_AddOns.EnableAddOn(name)`, `DisableAddOn(name)`: the addon's state
/// for the next session (what is loaded stays loaded).
fn set_addon_enabled(vm: &mut Vm, c: NativeCall, enabled: bool) -> Ret {
    let Some(i) = addon_index(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let name = engine(vm).world.bindings.addons[i].name.to_lowercase();
    let disabled = &mut engine_mut(vm).world.bindings.disabled;
    if enabled {
        disabled.remove(&name);
    } else {
        disabled.insert(name);
    }
    Ok(vec![])
}

fn enable_addon(vm: &mut Vm, c: NativeCall) -> Ret {
    set_addon_enabled(vm, c, true)
}

fn disable_addon(vm: &mut Vm, c: NativeCall) -> Ret {
    set_addon_enabled(vm, c, false)
}

/// `C_AddOns.SaveAddOns()`: the enabled states are written at logout;
/// nothing to tell.
fn save_addons(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// `C_UIFileAsset.IsKnownFile(asset)`: a file ID, or a path the client
/// ships, is known (the engine has no list of the client's files:
/// unverified); a path into a loaded addon's folder is known if the file is
/// there (the documentation: "a locally existing loose file"), with or
/// without its extension, any case.
fn is_known_file(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(path) = string(vm, c.arg(1)) else {
        return Ok(vec![Value::Bool(!vm.hidden(c.arg(1)).is_nil())]);
    };
    let path = path.replace('\\', "/");
    let lower = path.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("interface/addons/") else {
        return Ok(vec![Value::Bool(true)]);
    };
    let folder = rest.split('/').next().unwrap_or("");
    let dir = engine(vm)
        .world
        .bindings
        .addons
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(folder))
        .and_then(|a| a.files.as_ref().map(|f| f.dir.clone()));
    let Some(dir) = dir else {
        return Ok(vec![Value::Bool(true)]);
    };
    let rel = &path["interface/addons/".len() + folder.len()..];
    let rel = rel.trim_start_matches('/');
    let found = loose_file(&dir, rel);
    Ok(vec![Value::Bool(found)])
}

/// Whether `rel` names a file in `dir`, any case, its extension optional
/// (the client finds `x` as `x.blp` or `x.tga`).
fn loose_file(dir: &std::path::Path, rel: &str) -> bool {
    let mut cur = dir.to_path_buf();
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let Ok(entries) = std::fs::read_dir(&cur) else {
            return false;
        };
        let next = entries.flatten().map(|e| e.path()).find(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.eq_ignore_ascii_case(part)
                    || (last
                        && n.rsplit_once('.')
                            .is_some_and(|(stem, _)| stem.eq_ignore_ascii_case(part)))
            })
        });
        match next {
            Some(p) => cur = p,
            None => return false,
        }
    }
    cur.is_file()
}

/// The functions with behaviour here.
fn modified_click_of(vm: &Vm, action: &str) -> Option<String> {
    engine(vm)
        .world
        .bindings
        .modified_clicks
        .iter()
        .find(|(a, _)| a == action)
        .map(|(_, k)| k.clone())
}

/// `GetModifiedClick(action)`: its key (`SHIFT`; the bindings' default).
fn get_modified_click(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(a) = c.arg(1) else {
        return Ok(vec![Value::Nil]);
    };
    let action = String::from_utf8_lossy(vm.str_bytes(a)).into_owned();
    Ok(vec![match modified_click_of(vm, &action) {
        Some(k) => vm.str(&k),
        None => Value::Nil,
    }])
}

/// `SetModifiedClick(action, key)`.
fn set_modified_click(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Str(a), Value::Str(k)) = (c.arg(1), c.arg(2)) else {
        return Ok(vec![]);
    };
    let action = String::from_utf8_lossy(vm.str_bytes(a)).into_owned();
    let key = String::from_utf8_lossy(vm.str_bytes(k)).into_owned();
    let known = &mut engine_mut(vm).world.bindings.modified_clicks;
    match known.iter_mut().find(|(a, _)| *a == action) {
        Some(entry) => entry.1 = key,
        None => known.push((action, key)),
    }
    Ok(vec![])
}

/// `GetNumModifiedClickActions()`: the bindings' modified clicks. The
/// client's answer differs from session to session (380, then 500, while
/// it lists 20: `GetModifiedClickAction(21)` and on are nil; recorded).
fn get_num_modified_click_actions(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        engine(vm).world.bindings.modified_clicks.len() as f64,
    )])
}

fn get_modified_click_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = match c.arg(1) {
        Value::Num(n) if n >= 1.0 => n as usize,
        _ => return Ok(vec![Value::Nil]),
    };
    let action = engine(vm)
        .world
        .bindings
        .modified_clicks
        .get(i - 1)
        .map(|(a, _)| a.clone());
    Ok(vec![match action {
        Some(a) => vm.str(&a),
        None => Value::Nil,
    }])
}

/// `IsModifiedClick([action])`: the action's modifiers are held (`NONE`:
/// never; no action: any modifier). Unverified: extra modifiers held,
/// `NONE` (probe queued).
fn is_modified_click(vm: &mut Vm, c: NativeCall) -> Ret {
    let held = engine(vm).world.input.modifiers;
    let key = match c.arg(1) {
        Value::Str(a) => {
            let action = String::from_utf8_lossy(vm.str_bytes(a)).into_owned();
            modified_click_of(vm, &action)
        }
        _ => return Ok(vec![Value::Bool(held.iter().any(|h| *h))]),
    };
    let Some(key) = key else {
        return Ok(vec![Value::Bool(false)]);
    };
    if key == "NONE" {
        return Ok(vec![Value::Bool(false)]);
    }
    let on = key.split('-').all(|part| match part {
        "SHIFT" => held[0],
        "CTRL" => held[1],
        "ALT" => held[2],
        _ => false,
    });
    Ok(vec![Value::Bool(on)])
}

/// A global string, if the UI's strings define it.
fn global_text(vm: &mut Vm, name: &str) -> Option<String> {
    match vm.get_global(name) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

/// A key as the client shows it (recorded, 167 keys on the Mac client):
/// the modifiers as written but META, which is `CMD_KEY_TEXT`; the key its
/// `KEY_<key>_MAC` string, else `KEY_<key>`, else itself ("CTRL-Page Up",
/// "Help" for INSERT, "F13" for PRINTSCREEN, "Num Pad 0"). Unverified: the
/// other platforms, `abbreviate`.
pub fn binding_text(vm: &mut Vm, key: &str) -> String {
    // The modifiers, then the key (which may be "-" itself: "CTRL--").
    let mut rest = key;
    let mut parts = Vec::new();
    loop {
        let found = ["ALT-", "CTRL-", "SHIFT-", "META-"]
            .into_iter()
            .find(|m| rest.starts_with(m) && rest.len() > m.len());
        let Some(m) = found else { break };
        let name = &m[..m.len() - 1];
        parts.push(if name == "META" {
            global_text(vm, "CMD_KEY_TEXT").unwrap_or_else(|| "CMD".to_string())
        } else {
            name.to_string()
        });
        rest = &rest[m.len()..];
    }
    let shown = global_text(vm, &format!("KEY_{rest}_MAC"))
        .or_else(|| global_text(vm, &format!("KEY_{rest}")))
        .unwrap_or_else(|| rest.to_string());
    parts.push(shown);
    parts.join("-")
}

/// `GetBindingText(key [, prefix, abbreviate])`: the key as shown; no
/// key the empty string (recorded).
fn get_binding_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = match c.arg(1) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        // A number: as Lua writes it (the full probe: `GetBindingText(-1)`
        // "-1").
        Value::Num(n) => wowlua_vm::number::fmt_num(n),
        _ => return Ok(vec![vm.str("")]),
    };
    let text = binding_text(vm, &key);
    Ok(vec![vm.str(&text)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("SetBinding", set_binding),
        ("GetBindingText", get_binding_text),
        ("GetModifiedClick", get_modified_click),
        ("SetModifiedClick", set_modified_click),
        ("GetNumModifiedClickActions", get_num_modified_click_actions),
        ("GetModifiedClickAction", get_modified_click_action),
        ("IsModifiedClick", is_modified_click),
        ("GetBindingAction", get_binding_action),
        ("C_KeyBindings.GetBindingByKey", get_binding_by_key),
        ("SetOverrideBinding", set_override_binding),
        ("SetOverrideBindingClick", set_override_binding_click),
        ("SetOverrideBindingSpell", set_override_binding_spell),
        ("SetOverrideBindingMacro", set_override_binding_macro),
        ("SetOverrideBindingItem", set_override_binding_item),
        ("ClearOverrideBindings", clear_override_bindings),
        ("GetBindingKey", get_binding_key),
        ("GetNumBindings", get_num_bindings),
        ("GetBinding", get_binding),
        ("C_AddOns.GetAddOnMetadata", get_addon_metadata),
        ("C_AddOns.GetNumAddOns", get_num_addons),
        ("C_AddOns.GetAddOnInfo", get_addon_info),
        ("C_AddOns.IsAddOnLoadOnDemand", is_addon_load_on_demand),
        ("C_AddOns.DoesAddOnExist", does_addon_exist),
        ("C_AddOns.GetAddOnEnableState", get_addon_enable_state),
        ("C_AddOns.EnableAddOn", enable_addon),
        ("C_AddOns.DisableAddOn", disable_addon),
        ("C_AddOns.SaveAddOns", save_addons),
        ("C_AddOns.GetAddOnName", get_addon_name),
        ("C_AddOns.GetAddOnTitle", get_addon_title),
        ("C_AddOns.GetAddOnNotes", get_addon_notes),
        ("C_AddOns.GetAddOnSecurity", get_addon_security),
        ("C_AddOns.GetAddOnDependencies", get_addon_dependencies),
        ("C_AddOns.IsAddOnLoadable", is_addon_loadable),
        // The TOC's `DefaultState` is not read (unverified: all on).
        ("C_AddOns.IsAddOnDefaultEnabled", yes),
        ("C_AddOns.EnableAllAddOns", enable_all_addons),
        ("C_AddOns.ResetAddOns", enable_all_addons),
        ("C_AddOns.DisableAllAddOns", disable_all_addons),
        (
            "C_AddOns.IsAddonVersionCheckEnabled",
            is_version_check_enabled,
        ),
        ("C_AddOns.SetAddonVersionCheck", set_version_check),
        ("C_UIFileAsset.IsKnownFile", is_known_file),
    ]
}
