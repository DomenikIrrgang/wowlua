//! UI XML into widgets, as the client builds it: virtual elements become
//! templates (`inherits`, `CreateFrame`'s template), intrinsic ones frame
//! types; other elements become widgets, their templates applied first,
//! then their own attributes, mixins, children and scripts; `OnLoad` runs
//! once a widget and its children are built.
//!
//! Unverified (no recording): the order within an element (attributes,
//! mixins, key values, children, scripts), `$parent` naming the nearest
//! named ancestor, script parameter names, and which attributes call which
//! method.

use std::path::Path;
use std::rc::Rc;

use wowlua_vm::{TableRef, Value, Vm};

use super::xml::Element;
use crate::widgets::{self, WidgetId};
use crate::{engine, engine_mut};

/// Region tags and the class they make.
const REGIONS: &[(&str, &str)] = &[
    ("Texture", "Texture"),
    ("FontString", "FontString"),
    ("MaskTexture", "MaskTexture"),
    ("Line", "Line"),
    // A model scene's actor template (`<Actor virtual="true">`).
    ("Actor", "ModelSceneFrameActor"),
];

/// Child elements that are a texture the widget uses, and its setter.
const TEXTURE_SLOTS: &[(&str, &str)] = &[
    ("NormalTexture", "SetNormalTexture"),
    ("PushedTexture", "SetPushedTexture"),
    ("DisabledTexture", "SetDisabledTexture"),
    ("HighlightTexture", "SetHighlightTexture"),
    ("CheckedTexture", "SetCheckedTexture"),
    ("DisabledCheckedTexture", "SetDisabledCheckedTexture"),
    ("ThumbTexture", "SetThumbTexture"),
    ("BarTexture", "SetStatusBarTexture"),
    // A color select's.
    ("ColorWheelTexture", "SetColorWheelTexture"),
    ("ColorWheelThumbTexture", "SetColorWheelThumbTexture"),
    ("ColorValueTexture", "SetColorValueTexture"),
    ("ColorValueThumbTexture", "SetColorValueThumbTexture"),
    ("ColorAlphaTexture", "SetColorAlphaTexture"),
    ("ColorAlphaThumbTexture", "SetColorAlphaThumbTexture"),
];

/// Font object slots (`<NormalFont style="GameFontNormal"/>`).
const FONT_SLOTS: &[(&str, &str)] = &[
    ("NormalFont", "SetNormalFontObject"),
    ("HighlightFont", "SetHighlightFontObject"),
    ("DisabledFont", "SetDisabledFontObject"),
];

/// Boolean attributes that call a method with `true`.
const FLAG_ATTRS: &[(&str, &str)] = &[
    ("enableMouse", "EnableMouse"),
    ("enableMouseClicks", "SetMouseClickEnabled"),
    ("enableMouseMotion", "SetMouseMotionEnabled"),
    ("enableKeyboard", "EnableKeyboard"),
    ("movable", "SetMovable"),
    ("resizable", "SetResizable"),
    ("toplevel", "SetToplevel"),
    ("clampedToScreen", "SetClampedToScreen"),
    ("dontSavePosition", "SetDontSavePosition"),
    ("autoFocus", "SetAutoFocus"),
    ("multiLine", "SetMultiLine"),
    ("wordwrap", "SetWordWrap"),
    ("setToFinalAlpha", "SetToFinalAlpha"),
    ("propagateKeyboardInput", "SetPropagateKeyboardInput"),
    ("obeyStepOnDrag", "SetObeyStepOnDrag"),
    ("reverseFill", "SetReverseFill"),
    ("rotatesTexture", "SetRotatesTexture"),
];

/// String and number attributes passed to a setter.
const VALUE_ATTRS: &[(&str, &str)] = &[
    ("frameStrata", "SetFrameStrata"),
    ("frameLevel", "SetFrameLevel"),
    ("justifyH", "SetJustifyH"),
    ("justifyV", "SetJustifyV"),
    ("file", "SetTexture"),
    ("atlas", "SetAtlas"),
    ("letters", "SetMaxLetters"),
    ("alpha", "SetAlpha"),
    ("scale", "SetScale"),
    ("orientation", "SetOrientation"),
    ("valueStep", "SetValueStep"),
    ("stepsPerPage", "SetStepsPerPage"),
    // Animations.
    ("duration", "SetDuration"),
    ("order", "SetOrder"),
    ("startDelay", "SetStartDelay"),
    ("endDelay", "SetEndDelay"),
    ("smoothing", "SetSmoothing"),
    ("fromAlpha", "SetFromAlpha"),
    ("toAlpha", "SetToAlpha"),
    ("looping", "SetLooping"),
    ("childKey", "SetChildKey"),
    ("targetKey", "SetTargetKey"),
];

/// Parameters of inline script bodies (`...` after them, always).
fn script_params(script: &str) -> &'static str {
    match script {
        "OnEvent" => "self, event, ...",
        "OnUpdate" => "self, elapsed, ...",
        "OnClick" | "PreClick" | "PostClick" => "self, button, down, ...",
        "OnEnter" | "OnLeave" => "self, motion, ...",
        "OnMouseDown" | "OnMouseUp" | "OnDragStart" => "self, button, ...",
        "OnMouseWheel" => "self, delta, ...",
        "OnValueChanged" => "self, value, userInput, ...",
        "OnTextChanged" => "self, userInput, ...",
        "OnSizeChanged" => "self, width, height, ...",
        "OnAttributeChanged" => "self, name, value, ...",
        "OnKeyDown" | "OnKeyUp" => "self, key, ...",
        "OnChar" => "self, text, ...",
        _ => "self, ...",
    }
}

/// A Lua error's message.
fn message(vm: &mut Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        other => vm
            .tostring(other)
            .ok()
            .and_then(|s| match s {
                Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
                _ => None,
            })
            .unwrap_or_else(|| "?".into()),
    }
}

/// Where the loader is: the file it reads, for chunk names and errors.
pub struct Context<'a> {
    pub file: &'a str,
}

impl Context<'_> {
    fn error(&self, vm: &mut Vm, line: usize, msg: String) {
        let e = format!("{}:{line}: {msg}", self.file);
        engine_mut(vm).world.ui.errors.push(e);
    }
}

/// The class an element's tag makes, if it is a widget or region.
pub fn class_of(vm: &Vm, tag: &str) -> Option<String> {
    if let Some((_, class)) = REGIONS.iter().find(|(t, _)| t.eq_ignore_ascii_case(tag)) {
        return Some(class.to_string());
    }
    let e = engine(vm);
    if let Some((base, _)) = e.world.ui.intrinsics.get(&tag.to_ascii_lowercase()) {
        return Some(base.clone());
    }
    // Animation groups and animations (`<Alpha>`, `<Translation>`).
    if let Some(class) = crate::widgets::canonical_class(e, tag)
        && (class == "AnimationGroup" || crate::widgets::is_a(e, &class, "Animation"))
    {
        return Some(class);
    }
    e.api
        .classes
        .iter()
        .find(|(_, c)| {
            c.widget
                .as_deref()
                .is_some_and(|w| w.eq_ignore_ascii_case(tag))
        })
        .map(|(name, _)| name.clone())
}

/// A top-level widget element of a `<Ui>`: a template if virtual (and a
/// frame type if intrinsic), else a widget.
pub fn top_level(vm: &mut Vm, cx: &Context, el: &Rc<Element>) {
    let Some(class) = class_of(vm, &el.tag) else {
        return;
    };
    if el.flag("virtual") || el.flag("intrinsic") {
        if let Some(name) = el.attr("name") {
            let key = name.to_ascii_lowercase();
            let ui = &mut engine_mut(vm).world.ui;
            if el.flag("intrinsic") {
                ui.intrinsics.insert(key, (class, el.clone()));
            } else {
                ui.templates.insert(key, el.clone());
            }
        }
        return;
    }
    let parent = el.attr("parent").and_then(|p| {
        let g = vm.get_global(p);
        widgets::widget_of(vm, g)
    });
    build(vm, cx, el, parent);
}

/// The name an element gives its widget, `$parent` replaced by the nearest
/// named ancestor's name.
fn resolve_name(vm: &Vm, name: &str, parent: Option<WidgetId>) -> String {
    if !name.contains("$parent") && !name.contains("$Parent") {
        return name.to_string();
    }
    let e = engine(vm);
    let mut cur = parent;
    let mut pname = String::new();
    while let Some(p) = cur {
        if let Some(n) = &e.world.widgets[p.0].name {
            pname = n.clone();
            break;
        }
        cur = e.world.widgets[p.0].parent;
    }
    name.replace("$parent", &pname).replace("$Parent", &pname)
}

/// Builds the widget an element describes under `parent`, then runs its
/// `OnLoad`.
pub fn build(
    vm: &mut Vm,
    cx: &Context,
    el: &Rc<Element>,
    parent: Option<WidgetId>,
) -> Option<Value> {
    let class = class_of(vm, &el.tag)?;
    let name = el.attr("name").map(|n| resolve_name(vm, n, parent));
    // The client's UIParent is the screen the engine already has.
    let existing = name
        .as_deref()
        .filter(|n| *n == "UIParent")
        .map(|n| vm.get_global(n))
        .filter(|v| widgets::widget_of(vm, *v).is_some());
    let frame = match existing {
        Some(f) => f,
        None => widgets::create(vm, &class, name, parent),
    };
    let id = widgets::widget_of(vm, frame)?;
    engine_mut(vm).world.widgets[id.0].frame = true;
    // An intrinsic type's own element comes first.
    let intrinsic = engine(vm)
        .world
        .ui
        .intrinsics
        .get(&el.tag.to_ascii_lowercase())
        .map(|(_, e)| e.clone());
    if let Some(i) = intrinsic {
        apply_as_intrinsic(vm, cx, frame, id, &i);
    }
    apply(vm, cx, frame, id, el);
    widgets::run_script(vm, id, "OnLoad", &[]);
    // Built visible: shown (unverified that the client runs OnShow here;
    // Blizzard's PersonalResourceDisplay sets itself up in OnShow).
    if widgets::visible(engine(vm), id) {
        widgets::run_script(vm, id, "OnShow", &[]);
    }
    Some(frame)
}

/// Applies a template by name to a widget (`CreateFrame`'s template, or
/// `inherits`); false if there is no such template.
pub fn apply_template(vm: &mut Vm, frame: Value, name: &str) -> bool {
    apply_template_in(vm, frame, name, Phase::Own)
        && apply_template_in(vm, frame, name, Phase::Children)
}

fn apply_template_in(vm: &mut Vm, frame: Value, name: &str, phase: Phase) -> bool {
    let Some(t) = engine(vm)
        .world
        .ui
        .templates
        .get(&name.to_ascii_lowercase())
        .cloned()
    else {
        return false;
    };
    let Some(id) = widgets::widget_of(vm, frame) else {
        return false;
    };
    let file = engine(vm)
        .world
        .ui
        .template_files
        .get(&name.to_ascii_lowercase())
        .cloned();
    let cx = Context {
        file: file.as_deref().unwrap_or("?"),
    };
    let env = engine(vm)
        .world
        .ui
        .template_envs
        .get(&name.to_ascii_lowercase())
        .copied();
    let globals = vm.globals;
    if let Some(env) = env {
        vm.globals = env;
    }
    // A template's scripts are its addon's code (or Blizzard's).
    let addon = engine(vm)
        .world
        .ui
        .template_addons
        .get(&name.to_ascii_lowercase())
        .cloned();
    let code_of = std::mem::replace(&mut engine_mut(vm).world.ui.code_of, addon);
    apply_in(vm, &cx, frame, id, &t, phase);
    engine_mut(vm).world.ui.code_of = code_of;
    vm.globals = globals;
    true
}

/// An intrinsic frame type's element onto a frame `CreateFrame` made.
pub fn apply_intrinsic(vm: &mut Vm, frame: Value, el: &Rc<Element>) {
    let Some(id) = widgets::widget_of(vm, frame) else {
        return;
    };
    let cx = Context { file: "?" };
    apply_as_intrinsic(vm, &cx, frame, id, el);
}

/// An intrinsic type's element: its scripts stay, whatever the widget sets.
fn apply_as_intrinsic(vm: &mut Vm, cx: &Context, frame: Value, id: WidgetId, el: &Rc<Element>) {
    let was = std::mem::replace(&mut engine_mut(vm).world.ui.applying_intrinsic, true);
    apply(vm, cx, frame, id, el);
    engine_mut(vm).world.ui.applying_intrinsic = was;
}

/// Calls `frame:method(args…)`, if the widget has the method; an error is
/// the loader's.
fn call_method(vm: &mut Vm, cx: &Context, line: usize, frame: Value, method: &str, args: &[Value]) {
    let key = vm.str(method);
    let f = match vm.index(frame, key, None) {
        Ok(f) if !f.is_nil() => f,
        _ => return,
    };
    let mut call = vec![frame];
    call.extend_from_slice(args);
    if let Err(e) = vm.call(f, &call) {
        let msg = message(vm, e.value);
        cx.error(vm, line, format!("{method}: {msg}"));
    }
}

fn table_of(v: Value) -> Option<TableRef> {
    match v {
        Value::Table(t) => Some(t),
        _ => None,
    }
}

/// Which part of an element `apply_in` applies: its attributes, mixins and
/// key values (the whole template chain's, first), or its children and
/// scripts (a child's `OnLoad` sees the parent's key values: Blizzard's
/// `ButtonControllerMixin` reads `atlasName` of a derived template).
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Own,
    Children,
}

/// An element's attributes, mixins, children and scripts onto a widget,
/// after the templates it inherits.
fn apply(vm: &mut Vm, cx: &Context, frame: Value, id: WidgetId, el: &Rc<Element>) {
    apply_in(vm, cx, frame, id, el, Phase::Own);
    apply_in(vm, cx, frame, id, el, Phase::Children);
}

fn apply_in(vm: &mut Vm, cx: &Context, frame: Value, id: WidgetId, el: &Rc<Element>, phase: Phase) {
    let is_font_string = engine(vm).world.widgets[id.0].class == "FontString";
    for t in el.attr("inherits").unwrap_or("").split(',') {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        // A font string inherits a font object's font.
        let g = vm.get_global(t);
        let is_font = widgets::widget_of(vm, g)
            .is_some_and(|w| engine(vm).world.widgets[w.0].class == "Font");
        if is_font && (is_font_string || el.tag.eq_ignore_ascii_case("EditBox")) {
            if phase == Phase::Own {
                call_method(vm, cx, el.line, frame, "SetFontObject", &[g]);
            }
            continue;
        }
        if !apply_template_in(vm, frame, t, phase) && phase == Phase::Own {
            engine_mut(vm)
                .world
                .warnings
                .push(format!("{}:{}: no template \"{t}\"", cx.file, el.line));
        }
    }
    if phase == Phase::Children {
        for child in el
            .children
            .iter()
            .filter(|c| !c.tag.eq_ignore_ascii_case("KeyValues"))
        {
            apply_child(vm, cx, frame, id, child);
        }
        // A button's text, once its font string is there.
        if let Some(text) = el.attr("text")
            && engine(vm).world.widgets[id.0].class != "FontString"
        {
            let g = vm.get_global(text);
            let v = if matches!(g, Value::Str(_)) {
                g
            } else {
                vm.str(text)
            };
            call_method(vm, cx, el.line, frame, "SetText", &[v]);
        }
        return;
    }
    let table = table_of(frame).expect("a widget is a table");
    for attr in ["mixin", "secureMixin"] {
        for m in el.attr(attr).unwrap_or("").split(',') {
            let m = m.trim();
            if m.is_empty() {
                continue;
            }
            match vm.get_global(m) {
                Value::Table(src) => {
                    let mut k = Value::Nil;
                    while let Some((key, v)) = vm.heap.table(src).next(k).ok().flatten() {
                        let _ = vm.heap.table_mut(table).set(key, v);
                        k = key;
                    }
                }
                _ => cx.error(vm, el.line, format!("mixin {m} is not a table")),
            }
        }
    }
    // `<Mixins><Mixin key="X" [source="secure"]/></Mixins>`: from the
    // globals, or the secure environment's (partitions and secure
    // delegates not modelled).
    for list in el
        .children
        .iter()
        .filter(|c| c.tag.eq_ignore_ascii_case("Mixins"))
    {
        for m in &list.children {
            let Some(key) = m.attr("key") else { continue };
            let secure = m
                .attr("source")
                .is_some_and(|s| s.eq_ignore_ascii_case("secure"));
            let globals = vm.globals;
            if secure && let Some(env) = engine(vm).world.ui.secure_env {
                vm.globals = env;
            }
            let src = vm.get_global(key);
            vm.globals = globals;
            match src {
                Value::Table(src) => {
                    let mut k = Value::Nil;
                    while let Some((key, v)) = vm.heap.table(src).next(k).ok().flatten() {
                        let _ = vm.heap.table_mut(table).set(key, v);
                        k = key;
                    }
                }
                _ => cx.error(vm, m.line, format!("mixin {key} is not a table")),
            }
        }
    }
    // A template's `parent` for a widget its element gave none
    // (`ContainerFrameTemplate`'s `ContainerFrameContainer`).
    if let Some(p) = el.attr("parent")
        && engine(vm).world.widgets[id.0].parent.is_none()
    {
        let g = vm.get_global(p);
        if widgets::widget_of(vm, g).is_some() {
            call_method(vm, cx, el.line, frame, "SetParent", &[g]);
        }
    }
    if let Some(key) = el.attr("parentKey")
        && let Some(p) = engine(vm).world.widgets[id.0].parent
    {
        let pt = engine(vm).world.widgets[p.0].table;
        let k = vm.str(key);
        let _ = vm.heap.table_mut(pt).set(k, frame);
    }
    if let Some(key) = el.attr("parentArray")
        && let Some(p) = engine(vm).world.widgets[id.0].parent
    {
        let pt = engine(vm).world.widgets[p.0].table;
        let k = vm.str(key);
        let list = match vm.heap.table(pt).get(k) {
            Value::Table(t) => t,
            _ => {
                let t = vm.new_table();
                let _ = vm.heap.table_mut(pt).set(k, Value::Table(t));
                t
            }
        };
        let n = vm.heap.table(list).border() as f64 + 1.0;
        let _ = vm.heap.table_mut(list).set(Value::Num(n), frame);
    }
    if el.flag("hidden") {
        engine_mut(vm).world.widgets[id.0].shown = false;
    }
    if el.flag("protected") {
        engine_mut(vm).world.widgets[id.0].protected = true;
    }
    if let Some(n) = el.number("id") {
        call_method(vm, cx, el.line, frame, "SetID", &[Value::Num(n)]);
    }
    if el.flag("setAllPoints") {
        call_method(vm, cx, el.line, frame, "SetAllPoints", &[]);
    }
    for (attr, method) in FLAG_ATTRS {
        if el.flag(attr) {
            call_method(vm, cx, el.line, frame, method, &[Value::Bool(true)]);
        }
    }
    for (attr, method) in VALUE_ATTRS {
        if let Some(v) = el.attr(attr) {
            let v = match v.trim().parse::<f64>() {
                Ok(n) => Value::Num(n),
                Err(_) => vm.str(v),
            };
            call_method(vm, cx, el.line, frame, method, &[v]);
        }
    }
    // A slider's or a status bar's range, then its value.
    if el.attr("minValue").is_some() || el.attr("maxValue").is_some() {
        let (min, max) = (
            el.number("minValue").unwrap_or(0.0),
            el.number("maxValue").unwrap_or(0.0),
        );
        call_method(
            vm,
            cx,
            el.line,
            frame,
            "SetMinMaxValues",
            &[Value::Num(min), Value::Num(max)],
        );
    }
    if let Some(v) = el.number("defaultValue") {
        call_method(vm, cx, el.line, frame, "SetValue", &[Value::Num(v)]);
    }
    // `font`: a font object's name (`GameFontHighlightRight`), else a file.
    if let Some(font) = el.attr("font")
        && !el.tag.eq_ignore_ascii_case("Font")
    {
        let g = vm.get_global(font);
        if widgets::widget_of(vm, g).is_some_and(|w| engine(vm).world.widgets[w.0].class == "Font")
        {
            call_method(vm, cx, el.line, frame, "SetFontObject", &[g]);
        } else {
            let height = el
                .child("FontHeight")
                .and_then(|h| h.child("AbsValue"))
                .and_then(|v| v.number("val"))
                .unwrap_or(12.0);
            let (file, flags) = (vm.str(font), vm.str(&super::xml_font_flags(el)));
            call_method(
                vm,
                cx,
                el.line,
                frame,
                "SetFont",
                &[file, Value::Num(height), flags],
            );
        }
    }
    if let Some(text) = el.attr("text")
        && engine(vm).world.widgets[id.0].class == "FontString"
    {
        // A global string's name or the text itself. A font string's is
        // set as XML sets it, before its font may come (a button's
        // `NormalFont` follows its `ButtonText`).
        let g = vm.get_global(text);
        let v = if matches!(g, Value::Str(_)) {
            g
        } else {
            vm.str(text)
        };
        crate::widgets::store_text(vm, id, Some(v));
    }
    for child in el
        .children
        .iter()
        .filter(|c| c.tag.eq_ignore_ascii_case("KeyValues"))
    {
        apply_child(vm, cx, frame, id, child);
    }
}

/// A `<Size>`'s or `<Offset>`'s dimensions: `x`/`y` or an `<AbsDimension>`.
/// Whether a widget shows text (a font object, a font string).
fn is_text(vm: &Vm, id: WidgetId) -> bool {
    let class = &engine(vm).world.widgets[id.0].class;
    class == "Font" || class == "FontString"
}

fn dims(el: &Element) -> (Option<f64>, Option<f64>) {
    let d = el.child("AbsDimension").map(|d| d.as_ref()).unwrap_or(el);
    (d.number("x"), d.number("y"))
}

/// The widget `relativeKey="$parent.$parent.Key"` names, from `frame`.
fn relative_key(vm: &mut Vm, frame: Value, path: &str) -> Value {
    let mut cur = frame;
    for part in path.split('.') {
        cur = if part.eq_ignore_ascii_case("$parent") {
            match widgets::widget_of(vm, cur).and_then(|w| engine(vm).world.widgets[w.0].parent) {
                Some(p) => Value::Table(engine(vm).world.widgets[p.0].table),
                None => return Value::Nil,
            }
        } else {
            let k = vm.str(part);
            match vm.index(cur, k, None) {
                Ok(v) => v,
                Err(_) => return Value::Nil,
            }
        };
        if cur.is_nil() {
            return cur;
        }
    }
    cur
}

fn apply_child(vm: &mut Vm, cx: &Context, frame: Value, id: WidgetId, child: &Rc<Element>) {
    let tag = child.tag.as_str();
    match tag.to_ascii_lowercase().as_str() {
        "size" => {
            let (x, y) = dims(child);
            match (x, y) {
                (Some(x), Some(y)) => {
                    call_method(
                        vm,
                        cx,
                        child.line,
                        frame,
                        "SetSize",
                        &[Value::Num(x), Value::Num(y)],
                    );
                }
                (Some(x), None) => {
                    call_method(vm, cx, child.line, frame, "SetWidth", &[Value::Num(x)])
                }
                (None, Some(y)) => {
                    call_method(vm, cx, child.line, frame, "SetHeight", &[Value::Num(y)])
                }
                (None, None) => {}
            }
        }
        "anchors" => {
            for a in &child.children {
                let point = a.attr("point").unwrap_or("TOPLEFT");
                let rel = if let Some(key) = a.attr("relativeKey") {
                    relative_key(vm, frame, key)
                } else if let Some(name) = a.attr("relativeTo") {
                    let parent = engine(vm).world.widgets[id.0].parent;
                    let name = resolve_name(vm, name, parent);
                    vm.get_global(&name)
                } else {
                    match engine(vm).world.widgets[id.0].parent {
                        Some(p) => Value::Table(engine(vm).world.widgets[p.0].table),
                        None => Value::Nil,
                    }
                };
                let rp = a.attr("relativePoint").unwrap_or(point).to_string();
                let (mut x, mut y) = (a.number("x"), a.number("y"));
                if let Some(o) = a.child("Offset") {
                    let (ox, oy) = dims(o);
                    x = x.or(ox);
                    y = y.or(oy);
                }
                let p = vm.str(point);
                let rp = vm.str(&rp);
                call_method(
                    vm,
                    cx,
                    a.line,
                    frame,
                    "SetPoint",
                    &[
                        p,
                        rel,
                        rp,
                        Value::Num(x.unwrap_or(0.0)),
                        Value::Num(y.unwrap_or(0.0)),
                    ],
                );
            }
        }
        "layers" => {
            for layer in &child.children {
                for region in &layer.children {
                    build(vm, cx, region, Some(id));
                }
            }
        }
        "frames" => {
            for f in &child.children {
                build(vm, cx, f, Some(id));
            }
        }
        "animations" => {
            for g in &child.children {
                build(vm, cx, g, Some(id));
            }
        }
        // A font's or font string's text color and shadow.
        "color" if is_text(vm, id) => {
            let rgba = super::xml_color(vm, child);
            call_method(vm, cx, child.line, frame, "SetTextColor", &rgba);
        }
        "shadow" if is_text(vm, id) => {
            let (x, y) = match child.child("Offset") {
                Some(o) => dims(o),
                None => (child.number("x"), child.number("y")),
            };
            call_method(
                vm,
                cx,
                child.line,
                frame,
                "SetShadowOffset",
                &[Value::Num(x.unwrap_or(0.0)), Value::Num(y.unwrap_or(0.0))],
            );
            let rgba = match child.child("Color") {
                Some(c) => super::xml_color(vm, c),
                None => [
                    Value::Num(0.0),
                    Value::Num(0.0),
                    Value::Num(0.0),
                    Value::Num(1.0),
                ],
            };
            call_method(vm, cx, child.line, frame, "SetShadowColor", &rgba);
        }
        "barcolor" => {
            let rgba = super::xml_color(vm, child);
            call_method(vm, cx, child.line, frame, "SetStatusBarColor", &rgba);
        }
        "scrollchild" => {
            for f in &child.children {
                if let Some(c) = build(vm, cx, f, Some(id)) {
                    call_method(vm, cx, f.line, frame, "SetScrollChild", &[c]);
                }
            }
        }
        "keyvalues" => {
            let table = table_of(frame).expect("a widget is a table");
            for kv in &child.children {
                let Some(key) = kv.attr("key") else { continue };
                let raw = kv.attr("value").unwrap_or("");
                let v = match kv.attr("type").map(str::to_ascii_lowercase).as_deref() {
                    Some("number") => raw.trim().parse().map(Value::Num).unwrap_or(Value::Nil),
                    Some("boolean") => Value::Bool(raw.eq_ignore_ascii_case("true")),
                    Some("global") => global_path(vm, raw.trim()),
                    Some("nil") => Value::Nil,
                    _ => vm.str(raw),
                };
                let k = match kv.attr("keyType").map(str::to_ascii_lowercase).as_deref() {
                    Some("number") => key.trim().parse().map(Value::Num).unwrap_or(Value::Nil),
                    _ => vm.str(key),
                };
                if !k.is_nil() {
                    let _ = vm.heap.table_mut(table).set(k, v);
                }
            }
        }
        "attributes" => {
            let attributes = engine(vm).world.widgets[id.0].attributes;
            for a in &child.children {
                let Some(name) = a.attr("name") else { continue };
                let raw = a.attr("value").unwrap_or("");
                let v = match a.attr("type").map(str::to_ascii_lowercase).as_deref() {
                    Some("number") => raw.trim().parse().map(Value::Num).unwrap_or(Value::Nil),
                    Some("boolean") => Value::Bool(raw.eq_ignore_ascii_case("true")),
                    Some("global") => vm.get_global(raw),
                    Some("nil") => Value::Nil,
                    _ => vm.str(raw),
                };
                let k = vm.heap.intern(name.to_lowercase().as_bytes());
                vm.heap.table_mut(attributes).set_str(k, v);
            }
        }
        "scripts" => {
            for s in &child.children {
                set_script(vm, cx, id, s);
            }
        }
        "buttontext" => {
            if let Some(fs) = build_as(vm, cx, child, "FontString", Some(id)) {
                call_method(vm, cx, child.line, frame, "SetFontString", &[fs]);
            }
        }
        // A frame's own font (an edit box's, a message frame's): the
        // element's font object, font and justification. Unverified: its
        // nested height, color and shadow.
        "fontstring"
            if [
                "EditBox",
                "MessageFrame",
                "ScrollingMessageFrame",
                "SimpleHTML",
            ]
            .iter()
            .any(|c| {
                let class = engine(vm).world.widgets[id.0].class.clone();
                crate::widgets::is_a(engine(vm), &class, c)
            }) =>
        {
            if let Some(inherits) = child.attr("inherits") {
                let font = vm.get_global(inherits);
                if !font.is_nil() {
                    call_method(vm, cx, child.line, frame, "SetFontObject", &[font]);
                }
            }
            if let Some(file) = child.attr("font") {
                let file = vm.str(file);
                let size = child
                    .attr("size")
                    .and_then(|s| s.parse::<f64>().ok())
                    .map_or(Value::Num(12.0), Value::Num);
                let flags = vm.str(child.attr("outline").unwrap_or(""));
                call_method(vm, cx, child.line, frame, "SetFont", &[file, size, flags]);
            }
            // SimpleHTML justifies by text type (unverified for this
            // element).
            let html = engine(vm).world.widgets[id.0].class == "SimpleHTML";
            for (attr, method) in [("justifyH", "SetJustifyH"), ("justifyV", "SetJustifyV")] {
                if html {
                    break;
                }
                if let Some(v) = child.attr(attr) {
                    let v = vm.str(v);
                    call_method(vm, cx, child.line, frame, method, &[v]);
                }
            }
        }
        _ if engine(vm).world.widgets[id.0].class == "AnimationGroup"
            && class_of(vm, tag)
                .is_some_and(|c| crate::widgets::is_a(engine(vm), &c, "Animation")) =>
        {
            build(vm, cx, child, Some(id));
        }
        _ => {
            if let Some((_, method)) = TEXTURE_SLOTS
                .iter()
                .find(|(t, _)| t.eq_ignore_ascii_case(tag))
            {
                if let Some(tex) = build_as(vm, cx, child, "Texture", Some(id)) {
                    call_method(vm, cx, child.line, frame, method, &[tex]);
                }
            } else if let Some((_, method)) =
                FONT_SLOTS.iter().find(|(t, _)| t.eq_ignore_ascii_case(tag))
                && let Some(style) = child.attr("style")
            {
                let font = vm.get_global(style);
                if !font.is_nil() {
                    call_method(vm, cx, child.line, frame, method, &[font]);
                }
            }
        }
    }
}

/// Builds an element under a tag of its own (`<NormalTexture>`) as a
/// region of `class`.
fn build_as(
    vm: &mut Vm,
    cx: &Context,
    el: &Rc<Element>,
    class: &str,
    parent: Option<WidgetId>,
) -> Option<Value> {
    let name = el.attr("name").map(|n| resolve_name(vm, n, parent));
    let region = widgets::create(vm, class, name, parent);
    let id = widgets::widget_of(vm, region)?;
    apply(vm, cx, region, id, el);
    Some(region)
}

/// Compiles UI code: as the loading addon's ([`Registry::code_of`]), or
/// as Blizzard's.
///
/// [`Registry::code_of`]: super::Registry::code_of
pub(crate) fn load_chunk(vm: &mut Vm, src: &[u8], chunk: &str) -> Result<Value, String> {
    match engine(vm).world.ui.code_of.clone() {
        Some(addon) => vm.load_as(src, chunk, &addon),
        None => vm.load(src, chunk),
    }
}

/// Compiles Lua for a script handler; an error is the loader's.
fn compile(vm: &mut Vm, cx: &Context, line: usize, src: &str) -> Option<Value> {
    let chunk = format!("@{}", cx.file);
    match load_chunk(vm, src.as_bytes(), &chunk) {
        Ok(f) => match vm.call(f, &[]) {
            Ok(r) => r.first().copied(),
            Err(e) => {
                let msg = message(vm, e.value);
                cx.error(vm, line, msg);
                None
            }
        },
        Err(e) => {
            cx.error(vm, line, e);
            None
        }
    }
}

/// One `<Scripts>` child: a handler from `function`, `method` or the body,
/// replacing the script or, with `inherit`, running with it (an
/// intrinsic's `intrinsicOrder` handlers are kept apart).
fn set_script(vm: &mut Vm, cx: &Context, id: WidgetId, s: &Element) {
    let name = s.tag.as_str();
    let handler = if let Some(f) = s.attr("function") {
        let g = vm.get_global(f);
        if g.is_nil() {
            return;
        }
        g
    } else if let Some(m) = s.attr("method") {
        let src = format!("return function(self, ...) return self[{m:?}](self, ...) end");
        match compile(vm, cx, s.line, &src) {
            Some(f) => f,
            None => return,
        }
    } else if !s.text.trim().is_empty() {
        // Lines as in the file.
        let src = format!(
            "{}return function({}) {} end",
            "\n".repeat(s.line.saturating_sub(1)),
            script_params(name),
            s.text
        );
        match compile(vm, cx, s.line, &src) {
            Some(f) => f,
            None => return,
        }
    } else {
        return;
    };
    // An intrinsic type's own handlers stay, whatever the widget sets
    // (`ScrollingMessageFrame`'s `OnPreLoad`, before the frame's OnLoad).
    // Without an order, before (`DropdownButton`'s `OnLoad_Intrinsic`;
    // unverified).
    let in_intrinsic = engine(vm).world.ui.applying_intrinsic;
    if let Some(order) = s
        .attr("intrinsicOrder")
        .or(in_intrinsic.then_some("precall"))
    {
        let props = engine(vm).world.widgets[id.0].props;
        let key = vm.str(&widgets::intrinsic_key(&order.to_ascii_lowercase(), name));
        let _ = vm.heap.table_mut(props).set(key, handler);
        engine_mut(vm).world.widgets[id.0].intrinsic_scripts = true;
        return;
    }
    let order = s.attr("inherit").map(str::to_ascii_lowercase);
    let scripts = engine(vm).world.widgets[id.0].scripts;
    let key = vm.heap.intern(name.as_bytes());
    let existing = vm.heap.table(scripts).get_str(key);
    // `inherit="prepend"`: the inherited handler first, then this one;
    // `"append"`: this one, then the inherited (Blizzard's chat edit box
    // sets `chatFrame` in an appended OnLoad its inherited OnLoad reads).
    let f = match (order.as_deref(), existing) {
        (Some("prepend"), Value::Func(_)) => chain(vm, existing, handler),
        (Some("append"), Value::Func(_)) => chain(vm, handler, existing),
        _ => handler,
    };
    vm.heap.table_mut(scripts).set_str(key, f);
    crate::widgets::key_script_set(vm, id, name);
}

/// A handler running `a`, then `b`.
fn chain(vm: &mut Vm, a: Value, b: Value) -> Value {
    let src = "local a, b = ...\nreturn function(...) a(...) return b(...) end";
    match vm.load(src.as_bytes(), "=[engine]") {
        Ok(f) => vm
            .call(f, &[a, b])
            .ok()
            .and_then(|r| r.first().copied())
            .unwrap_or(b),
        Err(_) => b,
    }
}

/// Loads a Lua file of Blizzard's code as secure code, with the addon's
/// name and table as its arguments.
pub fn run_lua(vm: &mut Vm, path: &Path, chunk: &str, addon: &str, addon_table: Value) {
    let src = match std::fs::read(path) {
        Ok(s) => s,
        // A TOC's file that is not there: skipped, as the client skips it
        // (it only logs; unverified wording).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            engine_mut(vm)
                .world
                .warnings
                .push(format!("{chunk}: no such file"));
            return;
        }
        Err(e) => {
            engine_mut(vm).world.ui.errors.push(format!("{chunk}: {e}"));
            return;
        }
    };
    let f = match load_chunk(vm, &src, &format!("@{chunk}")) {
        Ok(f) => f,
        Err(e) => {
            engine_mut(vm).world.ui.errors.push(e);
            return;
        }
    };
    let name = vm.str(addon);
    match vm.call_secure(f, &[name, addon_table]) {
        // A file's result is its module (`import(".Name")`).
        Ok(results) => {
            if let Some(&v) = results.first()
                && !v.is_nil()
            {
                crate::widgets::keep(vm, v);
                let key = module_key(chunk.strip_suffix(".lua").unwrap_or(chunk));
                engine_mut(vm).world.ui.modules.insert(key, v);
            }
        }
        Err(e) => {
            let msg = message(vm, e.value);
            engine_mut(vm).world.ui.errors.push(msg);
        }
    }
}

/// A module's key: its path without the extension, `/`-separated,
/// lower-case.
pub fn module_key(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

/// `import(".Name")`, `import(".Dir.Name")`: the result of the file
/// `Name.lua` (in `Dir`) next to the calling file, which loaded before.
/// Unverified: absolute names, and the error for a module not there.
pub fn import(vm: &mut Vm, c: wowlua_vm::NativeCall) -> wowlua_vm::stdlib::Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let source = vm
        .caller_source(1)
        .map(|s| String::from_utf8_lossy(&s).into_owned())
        .unwrap_or_default();
    let file = source
        .strip_prefix('@')
        .unwrap_or(&source)
        .replace('\\', "/");
    let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
    let rel = name.strip_prefix('.').unwrap_or(&name).replace('.', "/");
    let key = module_key(&format!("{dir}/{rel}"));
    match engine(vm).world.ui.modules.get(&key).copied() {
        Some(v) => Ok(vec![v]),
        None => Err(vm.error(&format!("import: no module '{name}'"))),
    }
}

/// `C_XMLUtil.GetTemplateInfo(name)`: a template's widget type, size (its
/// own or inherited `<Size>`), key values, what it inherits and where it is
/// defined; nothing for a name that is no template.
pub fn template_info(vm: &mut Vm, c: wowlua_vm::NativeCall) -> wowlua_vm::stdlib::Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).to_ascii_lowercase();
    let Some(el) = engine(vm).world.ui.templates.get(&name).cloned() else {
        return Ok(vec![Value::Nil]);
    };
    // The size: this template's, else the first inherited one's.
    fn size_of(vm: &Vm, el: &Element, depth: usize) -> Option<(f64, f64)> {
        if let Some(s) = el.child("Size") {
            let (x, y) = dims(s);
            return Some((x.unwrap_or(0.0), y.unwrap_or(0.0)));
        }
        if depth > 16 {
            return None;
        }
        el.attr("inherits")?.split(',').find_map(|t| {
            let t = engine(vm)
                .world
                .ui
                .templates
                .get(&t.trim().to_ascii_lowercase())?;
            size_of(vm, t, depth + 1)
        })
    }
    let (w, h) = size_of(vm, &el, 0).unwrap_or((0.0, 0.0));
    let class = class_of(vm, &el.tag).unwrap_or_else(|| el.tag.clone());
    let file = engine(vm)
        .world
        .ui
        .template_files
        .get(&name)
        .cloned()
        .unwrap_or_default();
    let t = vm.new_table();
    let kvs = vm.new_table();
    if let Some(list) = el.child("KeyValues") {
        for (i, kv) in list.children.iter().enumerate() {
            let entry = vm.new_table();
            for (field, v) in [
                ("key", kv.attr("key").unwrap_or("")),
                ("keyType", kv.attr("keyType").unwrap_or("string")),
                ("type", kv.attr("type").unwrap_or("string")),
                ("value", kv.attr("value").unwrap_or("")),
            ] {
                let k = vm.str(field);
                let v = vm.str(v);
                let _ = vm.heap.table_mut(entry).set(k, v);
            }
            let _ = vm
                .heap
                .table_mut(kvs)
                .set(Value::Num((i + 1) as f64), Value::Table(entry));
        }
    }
    let fields = [
        ("type", vm.str(&class)),
        ("width", Value::Num(w)),
        ("height", Value::Num(h)),
        ("keyValues", Value::Table(kvs)),
        ("sourceLocation", vm.str(&file)),
    ];
    for (k, v) in fields {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, v);
    }
    if let Some(inherits) = el.attr("inherits") {
        let k = vm.str("inherits");
        let v = vm.str(inherits);
        let _ = vm.heap.table_mut(t).set(k, v);
    }
    Ok(vec![Value::Table(t)])
}

/// A global by its dotted path (`MoneyDenominationDisplayType.Copper`).
fn global_path(vm: &mut Vm, path: &str) -> Value {
    let mut parts = path.split('.');
    let mut v = vm.get_global(parts.next().unwrap_or(""));
    for part in parts {
        let Value::Table(t) = v else {
            return Value::Nil;
        };
        let k = vm.str(part);
        v = vm.heap.table(t).get(k);
    }
    v
}
