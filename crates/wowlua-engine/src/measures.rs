//! What regions measure and hold beyond their setters: a font string's
//! lines and height (on the engine's text model: 6 units a character,
//! wrapped at the string's laid out width), a scroll frame's ranges, a
//! texture's atlas, the minimap's zoom, a scroll frame's offsets, a
//! button's font string. Unverified: the text model's
//! metrics against the client's fonts.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::engine;
use crate::widgets::{self, MethodFn, WidgetId};

/// The string laid out at its laid out width, with its word wrap and its
/// maximum lines, as the client does it (`text.rs`, decisions T237, T282);
/// none without text.
fn laid_out(vm: &mut Vm, id: WidgetId) -> Option<crate::text::Layout> {
    let text = match engine(vm).world.widgets[id.0].text {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    let width = widgets::laid_out_size(vm, id).0;
    Some(crate::text::layout_in(
        vm,
        id,
        &text,
        (width > 0.0).then_some(width),
    ))
}

/// `GetNumLines()`: every line the text breaks into at its width, also
/// those it doesn't show (decision T282).
fn get_num_lines(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let n = laid_out(vm, id).map_or(0, |l| l.lines);
    Ok(vec![Value::Num(n as f64)])
}

/// `IsTruncated()`: lines it doesn't show, or a word cut without non-space
/// wrap (decision T282).
fn is_truncated(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let t = laid_out(vm, id).is_some_and(|l| l.truncated);
    Ok(vec![Value::Bool(t)])
}

/// `GetStringHeight()`: the lines it shows at the font's line height on the
/// screen (`text.rs`, decisions T237, T282), the spacing between them.
fn get_string_height(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let n = laid_out(vm, id).map_or(0, |l| l.shown) as f64;
    let size = crate::text::line_height(vm, id);
    let props = engine(vm).world.widgets[id.0].props;
    let spacing = match widgets::stored_args(vm, props, "Spacing").and_then(|v| v.first().copied())
    {
        Some(Value::Num(s)) => s,
        _ => 0.0,
    };
    let h = if n > 0.0 {
        n * size + (n - 1.0) * spacing
    } else {
        0.0
    };
    Ok(vec![Value::Num(h)])
}

/// A scroll frame's child's size beyond its own (0 without a child).
fn scroll_range(vm: &mut Vm, id: WidgetId, vertical: bool) -> f64 {
    let props = engine(vm).world.widgets[id.0].props;
    let child = widgets::stored_args(vm, props, "ScrollChild")
        .and_then(|v| v.first().copied())
        .and_then(|v| widgets::widget_of(vm, v));
    let Some(child) = child else {
        return 0.0;
    };
    let (cw, ch) = widgets::laid_out_size(vm, child);
    let (w, h) = widgets::laid_out_size(vm, id);
    if vertical {
        (ch - h).max(0.0)
    } else {
        (cw - w).max(0.0)
    }
}

fn get_vertical_scroll_range(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(scroll_range(vm, id, true))])
}

fn get_horizontal_scroll_range(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(scroll_range(vm, id, false))])
}

/// An atlas the client was seen to have: its name as the client spells
/// it, its width and height where known, and, where the client was asked
/// (`C_Texture.GetAtlasInfo`), its file, texture coordinates (left, right,
/// top, bottom) and tiling (`data/camelot-atlases.txt`).
pub struct Atlas {
    pub name: &'static str,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub info: Option<AtlasInfo>,
}

/// An atlas's info as the client gives it.
pub struct AtlasInfo {
    pub file: f64,
    pub coords: [f64; 4],
    pub tiles_horizontally: bool,
    pub tiles_vertically: bool,
    /// Its size in the file's pixels (`rawSize`), from the client's tables.
    pub raw: Option<(f64, f64)>,
}

/// An atlas line: name, width, height, then (when given) file, left,
/// right, top, bottom, tiling horizontally and vertically, raw width and
/// height.
fn atlas_line(l: &'static str) -> (String, Atlas) {
    let f: Vec<&str> = l.split('\t').collect();
    let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok());
    let info = (f.len() >= 10).then(|| AtlasInfo {
        file: num(3).unwrap_or(0.0),
        coords: [
            num(4).unwrap_or(0.0),
            num(5).unwrap_or(1.0),
            num(6).unwrap_or(0.0),
            num(7).unwrap_or(1.0),
        ],
        tiles_horizontally: f[8] == "true",
        tiles_vertically: f[9] == "true",
        raw: num(10).zip(num(11)),
    });
    (
        f[0].to_ascii_lowercase(),
        Atlas {
            name: f[0],
            width: num(1),
            height: num(2),
            info,
        },
    )
}

/// The atlas named `name` (any case), if the client has it: every atlas of
/// its tables (`data/camelot-atlases-client.txt`, decision T238), an
/// answer recorded from the client in its place (`data/camelot-atlases.txt`;
/// a size recorded alone keeps the tables' file and coordinates).
pub fn atlas(vm: &wowlua_vm::Vm, name: &str) -> Option<&'static Atlas> {
    // The installation's tables first (decision T472).
    if let Some((client, a)) = crate::clientdata::atlas(vm, name) {
        type Built = std::sync::Mutex<std::collections::HashMap<(usize, String), &'static Atlas>>;
        static BUILT: std::sync::OnceLock<Built> = std::sync::OnceLock::new();
        let mut built = BUILT
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = (client, name.to_ascii_lowercase());
        if let Some(a) = built.get(&key) {
            return Some(a);
        }
        let atlas: &'static Atlas = Box::leak(Box::new(Atlas {
            name: Box::leak(name.to_string().into_boxed_str()),
            width: Some(a.width),
            height: Some(a.height),
            info: Some(AtlasInfo {
                file: f64::from(a.file),
                coords: a.coords,
                tiles_horizontally: a.tiles_horizontally,
                tiles_vertically: a.tiles_vertically,
                raw: Some(a.raw),
            }),
        }));
        built.insert(key, atlas);
        return Some(atlas);
    }
    snapshot_atlas(name)
}

/// The atlas named `name` in the engine's snapshot of the client's tables
/// (and the recorded answers), without the installation.
pub fn snapshot_atlas(name: &str) -> Option<&'static Atlas> {
    static TABLE: std::sync::OnceLock<std::collections::HashMap<String, Atlas>> =
        std::sync::OnceLock::new();
    let lines = |text: &'static str| {
        text.lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .map(atlas_line)
    };
    let table = TABLE.get_or_init(|| {
        let mut table: std::collections::HashMap<String, Atlas> =
            lines(include_str!("../data/camelot-atlases-client.txt")).collect();
        for (key, mut recorded) in lines(include_str!("../data/camelot-atlases.txt")) {
            if let Some(client) = table.remove(&key) {
                recorded.width = recorded.width.or(client.width);
                recorded.height = recorded.height.or(client.height);
                let raw = client.info.as_ref().and_then(|i| i.raw);
                match &mut recorded.info {
                    Some(info) => info.raw = info.raw.or(raw),
                    None => recorded.info = client.info,
                }
            }
            table.insert(key, recorded);
        }
        table
    });
    table.get(&name.to_ascii_lowercase())
}

/// A texture's atlas set (`SetAtlas(atlas, useAtlasSize)`, a button's
/// `SetXAtlas`): with `useAtlasSize`, the texture takes the atlas's size
/// where it is known (SuuUI probe trees: the window frames' metal corners
/// 95 by 95 and 95 by 100, their edges 95 deep, the slots' gear frames 55
/// by 55, set so; a side not recorded keeps the size it had, unverified).
pub fn apply_atlas_size(vm: &mut Vm, id: WidgetId, args: &[Value]) {
    if !args.get(1).is_some_and(|v| v.truthy()) {
        return;
    }
    let Some(Value::Str(name)) = args.first().copied() else {
        return;
    };
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    if let Some(a) = atlas(vm, &name) {
        let r = &mut crate::engine_mut(vm).world.widgets[id.0];
        r.width = a.width.unwrap_or(r.width);
        r.height = a.height.unwrap_or(r.height);
    }
}

/// `SetAtlas(atlas [, useAtlasSize, …])`: kept, sizing the texture as
/// [`apply_atlas_size`] does.
fn set_atlas(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let args: Vec<Value> = c.args.iter().skip(1).copied().collect();
    // An atlas the client does not have leaves the texture as it was
    // (Blizzard's PvP rank ring: its XML's `pvpqueue-sidebar-honorbar-
    // background`, then `SetAtlas("UI-Character-Info-Honor-Bar")`, which
    // build 70532 lacks; in game the first shows). Camelot's, whose every
    // atlas the engine knows.
    if engine(vm).flavor == "camelot"
        && let Some(Value::Str(name)) = args.first().copied()
    {
        let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
        if !name.is_empty() && atlas(vm, &name).is_none() {
            return Ok(vec![]);
        }
    }
    widgets::store(vm, id, "Atlas", &args);
    apply_atlas_size(vm, id, &args);
    Ok(vec![])
}

/// `GetAtlas()`: the atlas set, else nil; spelled as the client spells it
/// when recorded (SuuUI probe trees: the gamepad prompts, set as
/// `gamepad-xbox1-trigger-lt-normal`, read back
/// `Gamepad-Xbox1-Trigger-LT-Normal`).
fn get_atlas(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let atlas_set = widgets::stored_args(vm, props, "Atlas").and_then(|v| v.first().copied());
    let Some(Value::Str(s)) = atlas_set else {
        return Ok(vec![Value::Nil]);
    };
    let given = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    Ok(vec![match atlas(vm, &given) {
        Some(a) if a.name != given => vm.str(a.name),
        _ => Value::Str(s),
    }])
}

/// The file a texture shows, by ID, as `GetTexture()` and
/// `GetTextureFileID()` answer: one value. A file ID set is itself (0 none,
/// recorded); a path set is its file's ID as `GetFileIDFromPath` gives it,
/// so a game file's own ID (recorded: `Interface\\Buttons\\WHITE8X8`
/// 130871, `Interface\\Icons\\INV_Misc_QuestionMark` 134400), an addon's
/// file the loose file's ID, nil for a file there is not (recorded:
/// `"NoSuchFile"`, `""`); an atlas's, its file's (unverified); nothing set,
/// a color, or another value nil (recorded: `nil`, `{}`).
fn texture_file(vm: &mut Vm, id: WidgetId) -> Value {
    let props = engine(vm).world.widgets[id.0].props;
    let set = widgets::stored_args(vm, props, "Texture").and_then(|v| v.first().copied());
    let answer = match set.map(|v| vm.hidden(v)) {
        Some(Value::Num(n)) if n != 0.0 => Value::Num(n),
        Some(Value::Str(s)) => {
            let path = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            crate::misc::file_id_of_path(vm, &path).map_or(Value::Nil, Value::Num)
        }
        Some(_) => Value::Nil,
        None => {
            let atlas_set =
                widgets::stored_args(vm, props, "Atlas").and_then(|v| v.first().copied());
            match atlas_set {
                Some(Value::Str(s)) => atlas(vm, &String::from_utf8_lossy(vm.str_bytes(s)))
                    .and_then(|a| a.info.as_ref())
                    .filter(|info| info.file > 0.0)
                    .map_or(Value::Nil, |info| Value::Num(info.file)),
                _ => Value::Nil,
            }
        }
    };
    // A secret file answers its file secretly.
    if matches!(set, Some(Value::Secret(_))) && !answer.is_nil() {
        return vm.secret_wrap(answer);
    }
    answer
}

/// `GetTexture()`: [`texture_file`].
fn get_texture(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![texture_file(vm, id)])
}

/// `GetTextureFileID()`: [`texture_file`].
fn get_texture_file_id(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![texture_file(vm, id)])
}

/// `EditBox:GetTextInsets()`: what `SetTextInsets` set, else none
/// (unverified: a fresh edit box's, not recorded).
fn get_text_insets(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    if let Some(v) = widgets::stored_args(vm, props, "TextInsets") {
        return Ok(v);
    }
    Ok(vec![Value::Num(0.0); 4])
}

/// `Minimap:GetZoomLevels()`: 6 (zooms 0 to 5). Unverified: the long-known
/// count, not recorded on this client.
fn get_zoom_levels(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(6.0)])
}

/// A texture's masks (`AddMaskTexture`), in the order added, kept in its
/// properties.
fn masks(vm: &mut Vm, id: WidgetId) -> wowlua_vm::TableRef {
    let props = engine(vm).world.widgets[id.0].props;
    let key = vm.str("\u{0}masks");
    if let Value::Table(t) = vm.heap.table(props).get(key) {
        return t;
    }
    let t = vm.new_table();
    let _ = vm.heap.table_mut(props).set(key, Value::Table(t));
    t
}

fn mask_list(vm: &mut Vm, id: WidgetId) -> Vec<Value> {
    let t = masks(vm, id);
    let n = vm.heap.table(t).border();
    (1..=n)
        .map(|i| vm.heap.table(t).get(Value::Num(i as f64)))
        .collect()
}

fn set_mask_list(vm: &mut Vm, id: WidgetId, list: &[Value]) {
    let t = vm.new_table();
    for (i, v) in list.iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), *v);
    }
    let props = engine(vm).world.widgets[id.0].props;
    let key = vm.str("\u{0}masks");
    let _ = vm.heap.table_mut(props).set(key, Value::Table(t));
}

/// The same mask: the same object's table.
fn same(a: Value, b: Value) -> bool {
    matches!((a, b), (Value::Table(x), Value::Table(y)) if x == y)
}

/// `AddMaskTexture(mask)`: added once (unverified: a second add).
fn add_mask_texture(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let mut list = mask_list(vm, id);
    let mask = c.arg(2);
    if !mask.is_nil() && !list.iter().any(|m| same(*m, mask)) {
        list.push(mask);
        set_mask_list(vm, id, &list);
    }
    Ok(vec![])
}

fn remove_mask_texture(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let mut list = mask_list(vm, id);
    let mask = c.arg(2);
    list.retain(|m| !same(*m, mask));
    set_mask_list(vm, id, &list);
    Ok(vec![])
}

fn get_num_mask_textures(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(mask_list(vm, id).len() as f64)])
}

/// `GetMaskTexture(index)`: the mask added `index`th.
fn get_mask_texture(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let list = mask_list(vm, id);
    let i = match c.arg(2) {
        Value::Num(n) if n >= 1.0 => n as usize,
        _ => return Ok(vec![Value::Nil]),
    };
    Ok(vec![list.get(i - 1).copied().unwrap_or(Value::Nil)])
}

/// `GetHorizTile()`, `GetVertTile()`: what `SetHorizTile`/`SetVertTile`
/// set, off at first (unverified: a fresh texture's, not recorded).
fn get_tile(vm: &mut Vm, id: WidgetId, stem: &str) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let on = widgets::stored_args(vm, props, stem)
        .and_then(|v| v.first().copied())
        .is_some_and(|v| v.truthy());
    Ok(vec![Value::Bool(on)])
}

fn get_horiz_tile(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    get_tile(vm, id, "HorizTile")
}

fn get_vert_tile(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    get_tile(vm, id, "VertTile")
}

/// Whether a texture snaps to the pixel grid: what `SetSnapToPixelGrid`
/// set, else on (Blizzard's code only ever turns it off:
/// `snapToPixelGrid="false"`, `SetSnapToPixelGrid(false)`; unverified in
/// the client).
pub fn snaps_to_pixel_grid(vm: &Vm, id: WidgetId) -> bool {
    let props = engine(vm).world.widgets[id.0].props;
    widgets::stored_ref(vm, props, "SnapToPixelGrid")
        .and_then(|v| v.first().copied())
        .is_none_or(|v| v.truthy())
}

fn is_snapping_to_pixel_grid(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(snaps_to_pixel_grid(vm, id))])
}

/// `Minimap:GetZoom()`: the zoom set, 0 at first.
fn get_zoom(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let zoom = widgets::stored_args(vm, props, "Zoom").and_then(|v| v.first().copied());
    Ok(vec![
        zoom.filter(|v| matches!(v, Value::Num(_)))
            .unwrap_or(Value::Num(0.0)),
    ])
}

/// `Minimap:SetZoom(zoom)`: the zoom kept; a change sets the
/// `minimapZoom` CVar (`CVAR_UPDATE("minimapZoom", zoom)`), then
/// `MINIMAP_UPDATE_ZOOM()` (recorded twice, scenario tracking: the zoom
/// buttons, in then out).
fn set_zoom(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Value::Num(zoom) = c.arg(2) else {
        return Ok(vec![]);
    };
    let props = engine(vm).world.widgets[id.0].props;
    let old = widgets::stored_args(vm, props, "Zoom")
        .and_then(|v| v.first().copied())
        .and_then(|v| match v {
            Value::Num(n) => Some(n),
            _ => None,
        })
        .unwrap_or(0.0);
    widgets::store(vm, id, "Zoom", &[Value::Num(zoom)]);
    let class = engine(vm).world.widgets[id.0].class.clone();
    let minimap = widgets::lineage(engine(vm), &class)
        .iter()
        .any(|c| c.eq_ignore_ascii_case("Minimap"));
    if minimap && zoom != old {
        let text = format!("{}", zoom as i64);
        crate::framescript::set_cvar_text(vm, "minimapZoom", text, true)?;
        widgets::fire_event(vm, "MINIMAP_UPDATE_ZOOM", &[]);
    }
    Ok(vec![])
}

/// `GetVerticalScroll()` / `GetHorizontalScroll()`: the offset set, 0 at
/// first.
fn scroll_offset(vm: &mut Vm, id: WidgetId, stem: &str) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let v = widgets::stored_args(vm, props, stem).and_then(|v| v.first().copied());
    Ok(vec![
        v.filter(|v| matches!(v, Value::Num(_)))
            .unwrap_or(Value::Num(0.0)),
    ])
}

fn get_vertical_scroll(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    scroll_offset(vm, id, "VerticalScroll")
}

fn get_horizontal_scroll(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    scroll_offset(vm, id, "HorizontalScroll")
}

/// `SetVerticalScroll(offset)` / `SetHorizontalScroll(offset)`: the offset
/// kept; a change runs `OnVerticalScroll(offset)` /
/// `OnHorizontalScroll(offset)` at once, as Blizzard's scroll templates
/// rely on (unverified: whether setting the same offset runs it too).
fn set_scroll(vm: &mut Vm, id: WidgetId, c: NativeCall, stem: &str, script: &str) -> Ret {
    let offset = match vm.hidden(c.arg(2)) {
        Value::Num(n) => n,
        _ => return Err(vm.arg_error(2, "number expected")),
    };
    let props = engine(vm).world.widgets[id.0].props;
    let old = widgets::stored_args(vm, props, stem)
        .and_then(|v| v.first().copied())
        .and_then(|v| match v {
            Value::Num(n) => Some(n),
            _ => None,
        })
        .unwrap_or(0.0);
    widgets::store(vm, id, stem, &[Value::Num(offset)]);
    if offset != old {
        widgets::run_script(vm, id, script, &[Value::Num(offset)]);
    }
    Ok(vec![])
}

fn set_vertical_scroll(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_scroll(vm, id, c, "VerticalScroll", "OnVerticalScroll")
}

fn set_horizontal_scroll(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_scroll(vm, id, c, "HorizontalScroll", "OnHorizontalScroll")
}

/// `Button:GetFontString()`: the font string set (`SetFontString`, a
/// template's `<ButtonText>`), else nil.
fn get_font_string(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let fs = widgets::stored_args(vm, props, "FontString")
        .and_then(|v| v.first().copied())
        .filter(|v| widgets::widget_of(vm, *v).is_some());
    Ok(vec![fs.unwrap_or(Value::Nil)])
}

pub fn methods() -> Vec<(&'static str, MethodFn)> {
    vec![
        ("GetNumLines", get_num_lines),
        ("IsTruncated", is_truncated),
        ("GetStringHeight", get_string_height),
        ("GetVerticalScrollRange", get_vertical_scroll_range),
        ("GetHorizontalScrollRange", get_horizontal_scroll_range),
        ("GetAtlas", get_atlas),
        ("SetAtlas", set_atlas),
        ("GetZoom", get_zoom),
        ("SetZoom", set_zoom),
        ("GetZoomLevels", get_zoom_levels),
        ("GetTexture", get_texture),
        ("GetTextureFileID", get_texture_file_id),
        ("GetTextInsets", get_text_insets),
        ("AddMaskTexture", add_mask_texture),
        ("RemoveMaskTexture", remove_mask_texture),
        ("GetNumMaskTextures", get_num_mask_textures),
        ("GetMaskTexture", get_mask_texture),
        ("IsSnappingToPixelGrid", is_snapping_to_pixel_grid),
        ("GetHorizTile", get_horiz_tile),
        ("GetVertTile", get_vert_tile),
        ("GetVerticalScroll", get_vertical_scroll),
        ("GetHorizontalScroll", get_horizontal_scroll),
        ("SetVerticalScroll", set_vertical_scroll),
        ("SetHorizontalScroll", set_horizontal_scroll),
        ("GetFontString", get_font_string),
    ]
}
