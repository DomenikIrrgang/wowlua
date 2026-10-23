//! The scene (spec debugger §18.9, decision T309): what the screen shows,
//! as a draw list in the client's order, built from the engine's state on
//! request. Coordinates are screen pixels from the top left.

use wowlua_vm::{Value, Vm};

use crate::engine;
use crate::widgets::{self, WidgetId, stored_ref};

/// A rectangle in screen pixels: left, top, width, height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn intersect(self, o: Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        Rect {
            x: x0,
            y: y0,
            w: (x1 - x0).max(0.0),
            h: (y1 - y0).max(0.0),
        }
    }

    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

/// What a texture shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Image {
    /// A file by path (`Interface\Buttons\WHITE8X8`), as given.
    File(String),
    /// A file by its ID.
    FileId(u32),
    /// A plain color (`SetColorTexture`): the item's color is it.
    Color,
    /// A unit's portrait (`SetPortraitTexture`): its model through its
    /// portrait camera, else the flat stand-in file.
    Portrait {
        source: Box<ModelSource>,
        fallback: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Blend,
    Add,
    Mod,
    AlphaKey,
    Disable,
}

/// A mask on a texture: its image, rectangle, and its part of the image
/// (left, right, top, bottom; an atlas's region), with the atlas's name
/// for a renderer with the build's own atlases.
#[derive(Clone, Debug)]
pub struct Mask {
    pub image: Image,
    pub rect: Rect,
    pub region: [f32; 4],
    pub atlas: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Quad {
    pub widget: usize,
    pub rect: Rect,
    pub image: Image,
    /// Texture coordinates of the corners: top left, bottom left, top
    /// right, bottom right (x, y each), as `SetTexCoord`'s eight values.
    pub texcoord: [f32; 8],
    /// The atlas set (`SetAtlas`), and the corners' coordinates within
    /// its region: a renderer with the build's own atlases places them.
    pub atlas: Option<String>,
    pub atlas_texcoord: [f32; 8],
    /// Vertex color times the effective alpha (and a color texture's
    /// color).
    pub color: [f32; 4],
    pub blend: Blend,
    pub desaturated: bool,
    /// Radians, about the rectangle's center.
    pub rotation: f32,
    pub tile: (bool, bool),
    pub clip: Option<Rect>,
    pub masks: Vec<Mask>,
}

/// A run of characters of one color.
#[derive(Clone, Debug)]
pub struct Run {
    pub text: String,
    pub color: [f32; 4],
    /// Each character's left edge, pixels from the line's start.
    pub xs: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct TextLine {
    /// Where the line starts (its left edge and top).
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug)]
pub struct Text {
    pub widget: usize,
    pub rect: Rect,
    /// The font file as set (`Fonts\FRIZQT__.TTF`).
    pub font: String,
    /// The font's pixel size.
    pub px: f32,
    pub line_height: f32,
    pub outline: u8,
    pub monochrome: bool,
    pub shadow: Option<([f32; 4], f32, f32)>,
    pub lines: Vec<TextLine>,
    pub clip: Option<Rect>,
}

/// What a model shows (spec debugger §18.9.7).
#[derive(Clone, Debug, PartialEq)]
pub enum ModelSource {
    File(u32),
    Path(String),
    /// A creature display (`CreatureDisplayInfo`).
    Display(u32),
    /// A creature (`Creature`): its first display.
    Creature(u32),
    /// A character as a unit looks.
    Character(Look),
    /// An item's own model (`SetItem`).
    Item(u32),
    /// An item appearance's model (`SetItemAppearance`).
    Appearance(u32),
}

/// How a character looks: race, sex (0 male, 1 female), class, and what it
/// wears (inventory slot, item ID); items tried on (links or appearance
/// IDs) over that.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Look {
    pub race: u32,
    pub sex: u32,
    pub class: u32,
    pub items: Vec<(u32, u32)>,
    pub tried: Vec<String>,
}

/// A model placed in its frame or scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Actor {
    pub source: ModelSource,
    pub position: [f32; 3],
    /// Radians: about the up axis (a frame's facing), then pitch, roll.
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub scale: f32,
    pub alpha: f32,
    /// The animation (`AnimationData` ID, 0 standing) and its variation.
    pub animation: u32,
    pub variation: u32,
}

/// How a model is looked at.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelCamera {
    /// A model frame's: the whole model in view, toward its portrait
    /// camera by `portrait_zoom` (0 to 1), farther by `distance_scale`; a
    /// camera of the model's own when `SetCamera` chose one.
    Frame {
        portrait_zoom: f32,
        distance_scale: f32,
        camera: Option<i32>,
    },
    /// A model scene's: where it is, its axes, field of view (radians,
    /// vertical) and clip planes.
    Scene {
        position: [f32; 3],
        forward: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        fov: f32,
        near: f32,
        far: f32,
    },
}

/// A model frame or scene's light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub enabled: bool,
    /// The direction the light shines in, in the model's space.
    pub direction: [f32; 3],
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
}

/// A model frame or scene: what it draws into its rectangle.
#[derive(Clone, Debug)]
pub struct Model {
    pub widget: usize,
    pub rect: Rect,
    pub clip: Option<Rect>,
    pub alpha: f32,
    pub actors: Vec<Actor>,
    pub camera: ModelCamera,
    pub light: Light,
    /// Seconds since the session began (the animations' clock).
    pub time: f64,
}

#[derive(Clone, Debug)]
pub enum Item {
    Quad(Quad),
    Text(Text),
    Model(Model),
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub items: Vec<Item>,
}

const LAYERS: [&str; 5] = ["BACKGROUND", "BORDER", "ARTWORK", "OVERLAY", "HIGHLIGHT"];

/// Screen pixels per UIParent unit, and UIParent's height in its units.
pub fn pixel_scale(vm: &Vm) -> (f64, f64) {
    let e = engine(vm);
    let (w, h) = e.screen.ui_parent_size();
    (e.screen.width / w.max(1.0), h)
}

/// A widget's rectangle in screen pixels, if it has one.
pub fn rect_of(vm: &Vm, id: WidgetId) -> Option<Rect> {
    let (k, ui_h) = pixel_scale(vm);
    let (l, b, w, h) = widgets::bounds(vm, id, 0)?;
    Some(Rect {
        x: (l * k) as f32,
        y: ((ui_h - b - h) * k) as f32,
        w: (w * k) as f32,
        h: (h * k) as f32,
    })
}

fn first(vm: &Vm, id: WidgetId, stem: &str) -> Option<Value> {
    let props = engine(vm).world.widgets[id.0].props;
    stored_ref(vm, props, stem).and_then(|v| v.first().copied())
}

fn args(vm: &Vm, id: WidgetId, stem: &str) -> Option<Vec<Value>> {
    let props = engine(vm).world.widgets[id.0].props;
    stored_ref(vm, props, stem)
}

fn num(vm: &Vm, v: Option<&Value>, default: f64) -> f64 {
    match v.map(|v| vm.hidden(*v)) {
        Some(Value::Num(n)) => n,
        _ => default,
    }
}

fn text_of(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.heap.str_bytes(s)).into_owned()),
        Value::Num(n) => Some(format!("{n}")),
        _ => None,
    }
}

fn color(vm: &Vm, v: Option<Vec<Value>>, default: [f32; 4]) -> [f32; 4] {
    match v {
        Some(v) => [
            num(vm, v.first(), f64::from(default[0])) as f32,
            num(vm, v.get(1), f64::from(default[1])) as f32,
            num(vm, v.get(2), f64::from(default[2])) as f32,
            num(vm, v.get(3), f64::from(default[3])) as f32,
        ],
        None => default,
    }
}

thread_local! {
    /// The animations' effects for the scene being built.
    static EFFECTS: std::cell::RefCell<std::collections::HashMap<usize, crate::animations::Effect>> =
        std::cell::RefCell::default();
}

fn effect_of(id: WidgetId) -> crate::animations::Effect {
    EFFECTS.with(|e| e.borrow().get(&id.0).copied().unwrap_or_default())
}

/// Its alpha times its parents', up to one that ignores its parent's (a
/// playing alpha animation's in place of a region's own).
fn effective_alpha(vm: &Vm, id: WidgetId) -> f32 {
    let mut a = 1.0;
    let mut cur = Some(id);
    for _ in 0..256 {
        let Some(w) = cur else { break };
        let widget = &engine(vm).world.widgets[w.0];
        a *= effect_of(w).alpha.unwrap_or(widget.alpha);
        if first(vm, w, "IgnoreParentAlpha").is_some_and(|v| v.truthy()) {
            break;
        }
        cur = widget.parent;
    }
    a as f32
}

/// The rectangle everything of `id` is clipped to: the frames above it
/// that clip their children (scroll frames among them).
fn clip_of(vm: &Vm, id: WidgetId) -> Option<Rect> {
    let mut clip: Option<Rect> = None;
    let mut cur = engine(vm).world.widgets[id.0].parent;
    let mut seen = 0;
    while let Some(p) = cur {
        seen += 1;
        if seen > 256 {
            break;
        }
        let w = &engine(vm).world.widgets[p.0];
        let clips = first(vm, p, "ClipsChildren").is_some_and(|v| v.truthy())
            || widgets::is_a(engine(vm), &w.class, "ScrollFrame");
        if clips && let Some(r) = rect_of(vm, p) {
            clip = Some(match clip {
                Some(c) => c.intersect(r),
                None => r,
            });
        }
        cur = w.parent;
    }
    clip
}

fn blend_of(vm: &Vm, id: WidgetId) -> Blend {
    let set = first(vm, id, "BlendMode").and_then(|v| text_of(vm, v));
    // A button's highlight adds, unless set otherwise.
    let highlight = set.is_none()
        && engine(vm).world.widgets[id.0].parent.is_some_and(|p| {
            first(vm, p, "HighlightTexture")
                .and_then(|v| widgets::widget_of(vm, v))
                .is_some_and(|h| h == id)
        });
    if highlight {
        return Blend::Add;
    }
    match set.as_deref() {
        Some("ADD") => Blend::Add,
        Some("MOD") => Blend::Mod,
        Some("ALPHAKEY") => Blend::AlphaKey,
        Some("DISABLE") => Blend::Disable,
        _ => Blend::Blend,
    }
}

/// A texture's image, the atlas's region of its file (left, right, top,
/// bottom), whether the atlas tiles, and a color texture's color.
type ImageOf = (Image, Option<[f32; 4]>, (bool, bool), Option<[f32; 4]>);

/// A texture's image and its texture coordinates as the atlas or the
/// file gives them.
fn image_of(vm: &Vm, id: WidgetId) -> Option<ImageOf> {
    // A portrait the client draws: its flat stand-in (`facts.rs`).
    let props = engine(vm).world.widgets[id.0].props;
    if let Some(k) = vm.heap.interned("\u{0}portrait".as_bytes())
        && let Some(path) = text_of(vm, vm.heap.table(props).get(Value::Str(k)))
    {
        let look = vm
            .heap
            .interned("\u{0}portraitlook".as_bytes())
            .and_then(|k| text_of(vm, vm.heap.table(props).get(Value::Str(k))))
            .and_then(|t| decode_look(&t));
        let image = match look {
            Some(look) => Image::Portrait {
                source: Box::new(ModelSource::Character(look)),
                fallback: path,
            },
            None => Image::File(path),
        };
        return Some((image, None, (false, false), None));
    }
    if let Some(name) = first(vm, id, "Atlas").and_then(|v| text_of(vm, v))
        && let Some(a) = crate::measures::atlas(&name)
        && let Some(info) = &a.info
    {
        let c = info.coords;
        return Some((
            Image::FileId(info.file as u32),
            Some([c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32]),
            (info.tiles_horizontally, info.tiles_vertically),
            None,
        ));
    }
    if let Some(v) = first(vm, id, "Texture") {
        match vm.hidden(v) {
            Value::Str(_) => {
                let path = text_of(vm, v)?;
                if path.is_empty() {
                    return None;
                }
                return Some((Image::File(path), None, (false, false), None));
            }
            Value::Num(n) => return Some((Image::FileId(n as u32), None, (false, false), None)),
            _ => {}
        }
    }
    let c = args(vm, id, "ColorTexture")?;
    let rgba = color(vm, Some(c), [1.0; 4]);
    Some((Image::Color, None, (false, false), Some(rgba)))
}

fn texcoord_of(vm: &Vm, id: WidgetId, base: Option<[f32; 4]>) -> [f32; 8] {
    let from_lrtb = |l: f32, r: f32, t: f32, b: f32| [l, t, l, b, r, t, r, b];
    let [bl, br, bt, bb] = base.unwrap_or([0.0, 1.0, 0.0, 1.0]);
    let Some(tc) = args(vm, id, "TexCoord") else {
        return from_lrtb(bl, br, bt, bb);
    };
    let n: Vec<f32> = tc.iter().map(|v| num(vm, Some(v), 0.0) as f32).collect();
    // Within the atlas's region when it has one.
    let map = |x: f32, y: f32| [bl + x * (br - bl), bt + y * (bb - bt)];
    let corners: [[f32; 2]; 4] = if n.len() >= 8 {
        [
            map(n[0], n[1]),
            map(n[2], n[3]),
            map(n[4], n[5]),
            map(n[6], n[7]),
        ]
    } else if n.len() >= 4 {
        let (l, r, t, b) = (n[0], n[1], n[2], n[3]);
        [map(l, t), map(l, b), map(r, t), map(r, b)]
    } else {
        return from_lrtb(bl, br, bt, bb);
    };
    [
        corners[0][0],
        corners[0][1],
        corners[1][0],
        corners[1][1],
        corners[2][0],
        corners[2][1],
        corners[3][0],
        corners[3][1],
    ]
}

fn masks_of(vm: &Vm, id: WidgetId) -> Vec<Mask> {
    let props = engine(vm).world.widgets[id.0].props;
    let Some(key) = vm.heap.interned("\u{0}masks".as_bytes()) else {
        return Vec::new();
    };
    let Value::Table(t) = vm.heap.table(props).get(Value::Str(key)) else {
        return Vec::new();
    };
    let n = vm.heap.table(t).border();
    let mut out = Vec::new();
    for i in 1..=n {
        let v = vm.heap.table(t).get(Value::Num(i as f64));
        let Some(m) = widgets::widget_of(vm, v) else {
            continue;
        };
        let (Some((image, base, _, _)), Some(rect)) = (image_of(vm, m), rect_of(vm, m)) else {
            continue;
        };
        out.push(Mask {
            image,
            rect,
            region: base.unwrap_or([0.0, 1.0, 0.0, 1.0]),
            atlas: first(vm, m, "Atlas").and_then(|v| text_of(vm, v)),
        });
    }
    out
}

/// A status bar's texture (a file's or an atlas's made for it, or one set).
fn bar_texture(vm: &Vm, bar: WidgetId) -> Option<WidgetId> {
    ["StatusBarTexture", "StatusBarTextureObject"]
        .iter()
        .find_map(|k| first(vm, bar, k).and_then(|v| widgets::widget_of(vm, v)))
}

/// A status bar's texture: the part its value fills.
fn bar_fill(vm: &Vm, id: WidgetId, rect: &mut Rect, tc: &mut [f32; 8]) {
    let Some(parent) = engine(vm).world.widgets[id.0].parent else {
        return;
    };
    let is_fill = bar_texture(vm, parent).is_some_and(|t| t == id);
    if !is_fill {
        return;
    }
    let Some(bar) = engine(vm).world.bars.get(&parent.0) else {
        return;
    };
    let range = bar.max - bar.min;
    let f = if range > 0.0 {
        ((bar.value - bar.min) / range).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let vertical = first(vm, parent, "Orientation")
        .and_then(|v| text_of(vm, v))
        .is_some_and(|o| o == "VERTICAL");
    let reverse = first(vm, parent, "ReverseFill").is_some_and(|v| v.truthy());
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    if vertical {
        let h = rect.h * f;
        if !reverse {
            rect.y += rect.h - h;
            // Keep the bottom part of the image.
            for i in [1, 5] {
                tc[i] = lerp(tc[i + 2], tc[i], f);
            }
        } else {
            for i in [3, 7] {
                tc[i] = lerp(tc[i - 2], tc[i], f);
            }
        }
        rect.h = h;
    } else {
        let w = rect.w * f;
        if reverse {
            rect.x += rect.w - w;
            tc[0] = lerp(tc[4], tc[0], f);
            tc[2] = lerp(tc[6], tc[2], f);
        } else {
            tc[4] = lerp(tc[0], tc[4], f);
            tc[6] = lerp(tc[2], tc[6], f);
        }
        rect.w = w;
    }
}

fn quad_of(vm: &Vm, id: WidgetId) -> Option<Quad> {
    let (image, base, tile, plain) = image_of(vm, id)?;
    // A status bar's texture without points fills the bar.
    let mut rect = rect_of(vm, id).or_else(|| {
        let w = &engine(vm).world.widgets[id.0];
        let parent = w.parent?;
        let fills = w.points.is_empty() && bar_texture(vm, parent).is_some_and(|t| t == id);
        if fills { rect_of(vm, parent) } else { None }
    })?;
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return None;
    }
    let mut texcoord = texcoord_of(vm, id, base);
    let atlas = first(vm, id, "Atlas").and_then(|v| text_of(vm, v));
    let mut atlas_texcoord = texcoord_of(vm, id, None);
    // A flip book playing: its frame's part.
    if let Some((rows, cols, frame)) = effect_of(id).flip {
        let (col, row) = (frame % cols, frame / cols);
        let (u0, u1) = (col as f32 / cols as f32, (col + 1) as f32 / cols as f32);
        let (v0, v1) = (row as f32 / rows as f32, (row + 1) as f32 / rows as f32);
        texcoord = sub_rect(texcoord, u0, u1, v0, v1);
        atlas_texcoord = sub_rect(atlas_texcoord, u0, u1, v0, v1);
    }
    let mut fill_rect = rect;
    bar_fill(vm, id, &mut fill_rect, &mut atlas_texcoord);
    bar_fill(vm, id, &mut rect, &mut texcoord);
    let vc = color(vm, args(vm, id, "VertexColor"), [1.0; 4]);
    let pc = plain.unwrap_or([1.0; 4]);
    let a = effective_alpha(vm, id);
    Some(Quad {
        widget: id.0,
        rect,
        image,
        texcoord,
        atlas,
        atlas_texcoord,
        color: [
            vc[0] * pc[0],
            vc[1] * pc[1],
            vc[2] * pc[2],
            vc[3] * pc[3] * a,
        ],
        blend: blend_of(vm, id),
        desaturated: first(vm, id, "Desaturated").is_some_and(|v| v.truthy()),
        rotation: num(vm, first(vm, id, "Rotation").as_ref(), 0.0) as f32,
        tile,
        clip: clip_of(vm, id),
        masks: masks_of(vm, id),
    })
}

/// A named colour (`|cnNAME:`): the global colour object of that name
/// (`VISITABLE_URL_DEFAULT_CHAT_LINK_COLOR`, made from the client's
/// colours), its `r`, `g`, `b`, `a`.
fn named_color(vm: &Vm, name: &str) -> Option<[f32; 4]> {
    let k = vm.heap.interned(name.as_bytes())?;
    let Value::Table(t) = vm.heap.table(vm.globals).get(Value::Str(k)) else {
        return None;
    };
    let field = |f: &str| -> Option<f32> {
        let k = vm.heap.interned(f.as_bytes())?;
        match vm.heap.table(t).get(Value::Str(k)) {
            Value::Num(n) => Some(n as f32),
            _ => None,
        }
    };
    Some([
        field("r")?,
        field("g")?,
        field("b")?,
        field("a").unwrap_or(1.0),
    ])
}

/// A text's characters with their colors (`|cAARRGGBB … |r`, `|cnNAME:`),
/// the escapes taken out as [`crate::text::visible`] takes them.
fn colored_chars(vm: &Vm, raw: &str, base: [f32; 4]) -> Vec<(char, [f32; 4])> {
    let mut out = Vec::new();
    let mut stack: Vec<[f32; 4]> = Vec::new();
    let mut chars = raw.chars().peekable();
    let cur = |stack: &Vec<[f32; 4]>| *stack.last().unwrap_or(&base);
    while let Some(c) = chars.next() {
        if c != '|' {
            out.push((c, cur(&stack)));
            continue;
        }
        match chars.next() {
            Some('|') => out.push(('|', cur(&stack))),
            Some('c') if chars.peek() == Some(&'n') => {
                chars.next();
                let name: String = chars.by_ref().take_while(|&x| x != ':').collect();
                let c = named_color(vm, &name).unwrap_or(cur(&stack));
                stack.push([c[0], c[1], c[2], base[3] * c[3]]);
            }
            Some('c') => {
                let hex: String = (0..8).filter_map(|_| chars.next()).collect();
                let byte = |i: usize| {
                    u8::from_str_radix(hex.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255)
                };
                stack.push([
                    f32::from(byte(2)) / 255.0,
                    f32::from(byte(4)) / 255.0,
                    f32::from(byte(6)) / 255.0,
                    base[3] * f32::from(byte(0)) / 255.0,
                ]);
            }
            Some('r') => {
                stack.pop();
            }
            Some('h') => {}
            Some('n') => out.push(('\n', cur(&stack))),
            Some('H') => {
                while let Some(x) = chars.next() {
                    if x == '|' && chars.peek() == Some(&'h') {
                        chars.next();
                        break;
                    }
                }
            }
            Some('T') | Some('A') => {
                while let Some(x) = chars.next() {
                    if x == '|' && matches!(chars.peek(), Some('t') | Some('a')) {
                        chars.next();
                        break;
                    }
                }
            }
            Some(other) => {
                out.push(('|', cur(&stack)));
                out.push((other, cur(&stack)));
            }
            None => out.push(('|', cur(&stack))),
        }
    }
    out
}

/// A font string's (or an edit box's) text, laid out as the engine
/// measures it.
fn text_item(vm: &Vm, id: WidgetId, edit_box: bool) -> Option<Text> {
    text_item_in(vm, id, edit_box, None)
}

/// [`text_item`] in a given rectangle (a message frame's line), from its
/// top, left aligned.
fn text_item_in(vm: &Vm, id: WidgetId, edit_box: bool, place: Option<Rect>) -> Option<Text> {
    let w = &engine(vm).world.widgets[id.0];
    let raw = w.text.and_then(|v| text_of(vm, v))?;
    if raw.is_empty() {
        return None;
    }
    // A button's text without points of its own sits in the button's
    // middle, as the client places it.
    let mut rect = place.or_else(|| rect_of(vm, id)).or_else(|| {
        let parent = w.parent?;
        let class = &engine(vm).world.widgets[parent.0].class;
        if !w.points.is_empty() || !widgets::is_a(engine(vm), class, "Button") {
            return None;
        }
        rect_of(vm, parent)
    })?;
    if edit_box && let Some(ins) = args(vm, id, "TextInsets") {
        let (k, _) = pixel_scale(vm);
        let k = k as f32;
        let l = num(vm, ins.first(), 0.0) as f32 * k;
        let r = num(vm, ins.get(1), 0.0) as f32 * k;
        let t = num(vm, ins.get(2), 0.0) as f32 * k;
        let b = num(vm, ins.get(3), 0.0) as f32 * k;
        rect = Rect {
            x: rect.x + l,
            y: rect.y + t,
            w: (rect.w - l - r).max(0.0),
            h: (rect.h - t - b).max(0.0),
        };
    }
    let font_args = widgets::font_chain_ref(vm, id, "Font").unwrap_or_default();
    let font = font_args
        .first()
        .and_then(|v| text_of(vm, *v))
        .unwrap_or_default();
    let flags = font_args
        .get(2)
        .and_then(|v| text_of(vm, *v))
        .unwrap_or_default()
        .to_ascii_uppercase();
    let base = color(
        vm,
        widgets::font_chain_ref(vm, id, "TextColor"),
        [1.0, 1.0, 1.0, 1.0],
    );
    let alpha = effective_alpha(vm, id);
    let base = [base[0], base[1], base[2], base[3] * alpha];
    let shadow = {
        let off = widgets::font_chain_ref(vm, id, "ShadowOffset");
        let (dx, dy) = match &off {
            Some(o) => (num(vm, o.first(), 0.0), num(vm, o.get(1), 0.0)),
            None => (0.0, 0.0),
        };
        if dx != 0.0 || dy != 0.0 {
            let c = color(
                vm,
                widgets::font_chain_ref(vm, id, "ShadowColor"),
                [0.0, 0.0, 0.0, 1.0],
            );
            let (k, _) = pixel_scale(vm);
            Some((
                [c[0], c[1], c[2], c[3] * alpha],
                (dx * k) as f32,
                (-dy * k) as f32,
            ))
        } else {
            None
        }
    };
    let wraps = !edit_box
        && widgets::font_chain_ref(vm, id, "WordWrap")
            .and_then(|v| v.first().copied())
            .is_none_or(|v| v.truthy());
    let (k, _) = pixel_scale(vm);
    let units_w = f64::from(rect.w) / k;
    let lines = crate::text::wrapped_lines(vm, id, &raw, wraps.then_some(units_w));
    let first_line = crate::text::glyph_line(vm, id, "");
    // Pixel sizes in the region's own pixels; the screen's are these
    // times the pixel ratio.
    let ratio = (f64::from(crate::text::pixel(vm, id)) * k) as f32;
    let line_h = first_line.line_height_px as f32 * ratio;
    let spacing = num(vm, first(vm, id, "Spacing").as_ref(), 0.0) as f32 * k as f32;
    let justify_h = widgets::font_chain_ref(vm, id, "JustifyH")
        .and_then(|v| v.first().copied())
        .and_then(|v| text_of(vm, v))
        .unwrap_or_else(|| {
            if edit_box {
                "LEFT".into()
            } else {
                "CENTER".into()
            }
        });
    let justify_v = if place.is_some() {
        "TOP".to_string()
    } else {
        widgets::font_chain_ref(vm, id, "JustifyV")
            .and_then(|v| v.first().copied())
            .and_then(|v| text_of(vm, v))
            .unwrap_or_else(|| "MIDDLE".into())
    };
    // Lines that fit (at least one).
    let fit = if rect.h > 0.0 {
        ((rect.h + spacing) / (line_h + spacing)).floor().max(1.0) as usize
    } else {
        lines.len()
    };
    let shown: Vec<String> = lines.into_iter().take(fit.max(1)).collect();
    let total_h = shown.len() as f32 * line_h + (shown.len().saturating_sub(1)) as f32 * spacing;
    let top = match justify_v.as_str() {
        "TOP" => rect.y,
        "BOTTOM" => rect.y + rect.h - total_h,
        _ => rect.y + (rect.h - total_h) / 2.0,
    };
    let colored = colored_chars(vm, &raw, base);
    let mut cursor = 0usize;
    let mut out_lines = Vec::new();
    for (i, line) in shown.iter().enumerate() {
        // Each laid-out line's characters, in order, with their colors
        // (the spaces and new lines wrapping dropped skipped).
        let mut picked: Vec<(char, [f32; 4])> = Vec::new();
        for ch in line.chars() {
            while cursor < colored.len() && colored[cursor].0 != ch {
                cursor += 1;
            }
            if cursor < colored.len() {
                picked.push(colored[cursor]);
                cursor += 1;
            } else {
                picked.push((ch, base));
            }
        }
        let mut text: String = line.clone();
        let mut g = crate::text::glyph_line(vm, id, &text);
        let mut width = g.width_px as f32 * ratio;
        // Too wide without wrapping: cut, with an ellipsis.
        if width > rect.w + 0.5 && rect.w > 0.0 {
            let mut chars: Vec<char> = text.chars().collect();
            while !chars.is_empty() {
                chars.pop();
                let candidate: String = chars.iter().collect::<String>() + "...";
                let cg = crate::text::glyph_line(vm, id, &candidate);
                if cg.width_px as f32 * ratio <= rect.w + 0.5 {
                    text = candidate;
                    g = cg;
                    break;
                }
            }
            let kept = chars.len();
            picked.truncate(kept);
            for _ in 0..3 {
                picked.push(('.', picked.last().map_or(base, |p| p.1)));
            }
            width = g.width_px as f32 * ratio;
        }
        let x = match justify_h.as_str() {
            "LEFT" => rect.x,
            "RIGHT" => rect.x + rect.w - width,
            _ => rect.x + (rect.w - width) / 2.0,
        };
        let y = top + i as f32 * (line_h + spacing);
        let mut runs: Vec<Run> = Vec::new();
        for (j, (ch, c)) in picked.iter().enumerate() {
            let cx = g.xs.get(j).copied().unwrap_or(0.0) as f32 * ratio;
            match runs.last_mut() {
                Some(r) if r.color == *c => {
                    r.text.push(*ch);
                    r.xs.push(cx);
                }
                _ => runs.push(Run {
                    text: ch.to_string(),
                    color: *c,
                    xs: vec![cx],
                }),
            }
        }
        out_lines.push(TextLine {
            x: x.round(),
            y: y.round(),
            width,
            runs,
        });
    }
    Some(Text {
        widget: id.0,
        rect,
        font,
        px: g_px(&first_line) * ratio,
        line_height: line_h,
        outline: if flags.contains("THICKOUTLINE") {
            2
        } else if flags.contains("OUTLINE") {
            1
        } else {
            0
        },
        monochrome: flags.contains("MONOCHROME"),
        shadow,
        lines: out_lines,
        clip: clip_of(vm, id),
    })
}

/// A message frame's messages, as the client shows them: newest at the
/// bottom of the frame, each wrapped to its width, older ones above while
/// they fit. Unverified: fading (`SetTimeVisible`), the insertion mode
/// (`SetInsertMode`), the frame's scroll.
fn message_items(vm: &Vm, frame: WidgetId) -> Vec<Text> {
    let Some(list) = engine(vm).world.messages.get(&frame.0) else {
        return Vec::new();
    };
    let Some(r) = rect_of(vm, frame) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut bottom = r.y + r.h;
    for (_, v) in list.iter().rev() {
        let Some(fs) = widgets::widget_of(vm, *v) else {
            continue;
        };
        if !engine(vm).world.widgets[fs.0].shown {
            continue;
        }
        let tall = Rect {
            x: r.x,
            y: r.y,
            w: r.w,
            h: f32::MAX / 4.0,
        };
        let Some(mut t) = text_item_in(vm, fs, false, Some(tall)) else {
            continue;
        };
        let h = t.lines.len() as f32 * t.line_height;
        let top = bottom - h;
        if top < r.y - 0.5 {
            break;
        }
        let dy = top - r.y;
        for line in &mut t.lines {
            line.y += dy;
            line.x = r.x;
        }
        t.rect = Rect {
            x: r.x,
            y: top,
            w: r.w,
            h,
        };
        t.clip = Some(match t.clip {
            Some(c) => c.intersect(r),
            None => r,
        });
        out.push(t);
        bottom = top;
    }
    out.reverse();
    out
}

fn g_px(g: &crate::text::GlyphLine) -> f32 {
    g.px as f32
}

/// A region's draw layer rank and sublevel.
fn layer_of(vm: &Vm, id: WidgetId) -> (usize, i32) {
    let a = args(vm, id, "DrawLayer");
    let layer = a
        .as_ref()
        .and_then(|a| a.first().copied())
        .and_then(|v| text_of(vm, v))
        .unwrap_or_else(|| "ARTWORK".into());
    let sub = a.as_ref().map_or(0.0, |a| num(vm, a.get(1), 0.0));
    (
        LAYERS.iter().position(|l| *l == layer).unwrap_or(2),
        sub as i32,
    )
}

/// Whether a widget is a region (drawn by its frame), by its type: XML
/// builds regions as it builds frames.
fn is_region(vm: &Vm, id: WidgetId) -> bool {
    let e = engine(vm);
    let class = &e.world.widgets[id.0].class;
    ["Texture", "FontString", "Line", "MaskTexture"]
        .iter()
        .any(|k| widgets::is_a(e, class, k))
}

/// Whether a widget is a frame (drawn in the strata's order).
fn is_frame(vm: &Vm, id: WidgetId) -> bool {
    let e = engine(vm);
    let w = &e.world.widgets[id.0];
    !is_region(vm, id) && (w.frame || widgets::is_a(e, &w.class, "Frame"))
}

/// The visible frames, in draw order (back to front).
fn frames_in_order(vm: &mut Vm) -> Vec<WidgetId> {
    let count = engine(vm).world.widgets.len();
    let mut frames: Vec<(usize, f64, usize)> = Vec::new();
    for i in 0..count {
        let id = WidgetId(i);
        let w = &engine(vm).world.widgets[i];
        if w.forbidden || !is_frame(vm, id) || !widgets::visible(engine(vm), id) {
            continue;
        }
        let strata = widgets::strata_rank(vm, id);
        let level = widgets::frame_level(vm, id);
        frames.push((strata, level, i));
    }
    frames.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
    frames.into_iter().map(|(_, _, i)| WidgetId(i)).collect()
}

fn mouse_enabled(vm: &Vm, id: WidgetId) -> bool {
    match first(vm, id, "MouseEnabled") {
        Some(v) => v.truthy(),
        None => {
            let class = &engine(vm).world.widgets[id.0].class;
            widgets::is_a(engine(vm), class, "Button")
        }
    }
}

/// The part (`u0..u1`, `v0..v1` of the unit square) of texture
/// coordinates given by their corners (top left, bottom left, top right,
/// bottom right).
fn sub_rect(tc: [f32; 8], u0: f32, u1: f32, v0: f32, v1: f32) -> [f32; 8] {
    let at = |u: f32, v: f32| {
        [
            tc[0] + (tc[4] - tc[0]) * u + (tc[2] - tc[0]) * v,
            tc[1] + (tc[5] - tc[1]) * u + (tc[3] - tc[1]) * v,
        ]
    };
    let [a, b, c, d] = [at(u0, v0), at(u0, v1), at(u1, v0), at(u1, v1)];
    [a[0], a[1], b[0], b[1], c[0], c[1], d[0], d[1]]
}

fn nums(vm: &Vm, id: WidgetId, stem: &str, n: usize) -> Option<Vec<f32>> {
    let a = args(vm, id, stem)?;
    Some((0..n).map(|i| num(vm, a.get(i), 0.0) as f32).collect())
}

/// A unit's look: its race, sex, class and what it wears, as the API
/// answers them.
pub(crate) fn look_of(vm: &mut Vm, unit: &str) -> Look {
    let u = vm.str(unit);
    let call = |vm: &mut Vm, name: &str, extra: Option<Value>| -> Vec<Value> {
        let f = vm.get_global(name);
        if !matches!(f, Value::Func(_)) {
            return Vec::new();
        }
        let mut a = vec![u];
        a.extend(extra);
        vm.pcall(f, &a).unwrap_or_default()
    };
    let id_at = |v: &[Value], i: usize| match v.get(i) {
        Some(Value::Num(n)) => *n as u32,
        _ => 0,
    };
    let race = id_at(&call(vm, "UnitRace", None), 2);
    let sex = match call(vm, "UnitSex", None).first() {
        Some(Value::Num(n)) if *n == 3.0 => 1,
        _ => 0,
    };
    let class = id_at(&call(vm, "UnitClass", None), 2);
    let mut items = Vec::new();
    for slot in 1..=19u32 {
        let r = call(vm, "GetInventoryItemID", Some(Value::Num(f64::from(slot))));
        if let Some(Value::Num(n)) = r.first()
            && *n > 0.0
        {
            items.push((slot, *n as u32));
        }
    }
    Look {
        race,
        sex,
        class,
        items,
        tried: Vec::new(),
    }
}

/// A look as text (`race:sex:class:slot=item,…`), for a portrait kept
/// as the unit looked when it was set.
pub(crate) fn encode_look(l: &Look) -> String {
    let items: Vec<String> = l.items.iter().map(|(s, i)| format!("{s}={i}")).collect();
    format!("{}:{}:{}:{}", l.race, l.sex, l.class, items.join(","))
}

fn decode_look(t: &str) -> Option<Look> {
    let mut p = t.split(':');
    let race = p.next()?.parse().ok()?;
    let sex = p.next()?.parse().ok()?;
    let class = p.next()?.parse().ok()?;
    let items = p
        .next()
        .unwrap_or("")
        .split(',')
        .filter_map(|e| {
            let (s, i) = e.split_once('=')?;
            Some((s.parse().ok()?, i.parse().ok()?))
        })
        .collect();
    (race > 0).then_some(Look {
        race,
        sex,
        class,
        items,
        tried: Vec::new(),
    })
}

/// An actor of a model frame or a scene's actor widget.
fn actor_of(vm: &mut Vm, id: WidgetId, frame: bool) -> Option<Actor> {
    let (source, tried, undressed) = crate::models::source_of(vm, id)?;
    let source = match source {
        crate::models::Source::File(f) => ModelSource::File(f),
        crate::models::Source::Path(p) => ModelSource::Path(p),
        crate::models::Source::Display(d) => ModelSource::Display(d),
        crate::models::Source::Creature(c) => ModelSource::Creature(c),
        crate::models::Source::Item(i) => ModelSource::Item(i),
        crate::models::Source::Appearance(a) => ModelSource::Appearance(a),
        crate::models::Source::Unit(u) => {
            let mut look = look_of(vm, &u);
            if undressed {
                look.items.clear();
            }
            let emptied = crate::models::undressed_slots(vm, id);
            look.items.retain(|(s, _)| !emptied.contains(s));
            look.tried = tried;
            ModelSource::Character(look)
        }
    };
    let position = nums(vm, id, "Position", 3).unwrap_or(vec![0.0; 3]);
    let (yaw, scale, alpha) = if frame {
        let facing = first(vm, id, "Facing")
            .or_else(|| first(vm, id, "Rotation"))
            .map_or(0.0, |v| num(vm, Some(&v), 0.0));
        (
            facing as f32,
            num(vm, first(vm, id, "ModelScale").as_ref(), 1.0) as f32,
            num(vm, first(vm, id, "ModelAlpha").as_ref(), 1.0) as f32,
        )
    } else {
        (
            num(vm, first(vm, id, "Yaw").as_ref(), 0.0) as f32,
            num(vm, first(vm, id, "Scale").as_ref(), 1.0) as f32,
            num(vm, first(vm, id, "Alpha").as_ref(), 1.0) as f32,
        )
    };
    let anim = args(vm, id, "Animation").unwrap_or_default();
    Some(Actor {
        source,
        position: [position[0], position[1], position[2]],
        yaw,
        pitch: num(vm, first(vm, id, "Pitch").as_ref(), 0.0) as f32,
        roll: num(vm, first(vm, id, "Roll").as_ref(), 0.0) as f32,
        scale,
        alpha,
        animation: num(vm, anim.first(), 0.0).max(0.0) as u32,
        variation: num(vm, anim.get(1), 0.0).max(0.0) as u32,
    })
}

/// The light of a model frame (`SetLight`) or scene (`SetLightDirection`,
/// …); else a light from the front above (unverified: the client's
/// default light).
fn light_of(vm: &Vm, id: WidgetId) -> Light {
    let mut light = Light {
        enabled: true,
        direction: [-1.0, 0.3, -0.6],
        ambient: [0.55, 0.55, 0.55],
        diffuse: [0.75, 0.75, 0.75],
    };
    if let Some(d) = nums(vm, id, "LightDirection", 3) {
        light.direction = [d[0], d[1], d[2]];
    }
    if let Some(c) = nums(vm, id, "LightAmbientColor", 3) {
        light.ambient = [c[0], c[1], c[2]];
    }
    if let Some(c) = nums(vm, id, "LightDiffuseColor", 3) {
        light.diffuse = [c[0], c[1], c[2]];
    }
    if let Some(v) = first(vm, id, "LightVisible") {
        light.enabled = v.truthy();
    }
    light
}

/// A model frame's or model scene's model item, if it shows a model.
fn model_item(vm: &mut Vm, f: WidgetId, class: &str) -> Option<Model> {
    let e = engine(vm);
    let is_model = widgets::is_a(e, class, "Model");
    let is_scene = widgets::is_a(e, class, "ModelScene");
    if !is_model && !is_scene {
        return None;
    }
    let rect = rect_of(vm, f)?;
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return None;
    }
    let (actors, camera) = if is_model {
        let actor = actor_of(vm, f, true)?;
        // A camera the code placed (`SetCameraPosition`,
        // `SetCameraTarget`: `Model_ApplyUICamera`'s), else the frame's.
        let placed = nums(vm, f, "CameraPosition", 3).zip(nums(vm, f, "CameraTarget", 3));
        let camera = if let Some((p, t)) = placed {
            let forward = [t[0] - p[0], t[1] - p[1], t[2] - p[2]];
            ModelCamera::Scene {
                position: [p[0], p[1], p[2]],
                forward,
                right: [forward[1], -forward[0], 0.0],
                up: [0.0, 0.0, 1.0],
                fov: 0.6,
                near: 0.1,
                far: 100.0,
            }
        } else {
            ModelCamera::Frame {
                portrait_zoom: num(vm, first(vm, f, "PortraitZoom").as_ref(), 0.0) as f32,
                distance_scale: num(vm, first(vm, f, "CamDistanceScale").as_ref(), 1.0) as f32,
                camera: first(vm, f, "Camera").map(|v| num(vm, Some(&v), 0.0) as i32),
            }
        };
        (vec![actor], camera)
    } else {
        let list = widgets::actors(vm, f);
        let n = vm.heap.table(list).border();
        let mut actors = Vec::new();
        for k in 1..=n {
            let v = vm.heap.table(list).get(Value::Num(k as f64));
            let Some(a) = widgets::widget_of(vm, v) else {
                continue;
            };
            if !engine(vm).world.widgets[a.0].shown {
                continue;
            }
            if let Some(actor) = actor_of(vm, a, false) {
                actors.push(actor);
            }
        }
        if actors.is_empty() {
            return None;
        }
        let position = nums(vm, f, "CameraPosition", 3).unwrap_or(vec![5.0, 0.0, 1.0]);
        // The axes set, or those of the yaw, pitch and roll set.
        let set = args(vm, f, "CameraOrientationByAxisVectors").is_some()
            || args(vm, f, "CameraOrientationByYawPitchRoll").is_some();
        let (forward, right, up) = if set {
            let a = crate::methods3::scene_axes(vm, f);
            let v = |i: usize| [a[i][0] as f32, a[i][1] as f32, a[i][2] as f32];
            (v(0), v(1), v(2))
        } else {
            ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0])
        };
        let camera = ModelCamera::Scene {
            position: [position[0], position[1], position[2]],
            forward,
            right,
            up,
            fov: num(vm, first(vm, f, "CameraFieldOfView").as_ref(), 0.7) as f32,
            near: num(vm, first(vm, f, "CameraNearClip").as_ref(), 0.2) as f32,
            far: num(vm, first(vm, f, "CameraFarClip").as_ref(), 100.0) as f32,
        };
        (actors, camera)
    };
    Some(Model {
        widget: f.0,
        rect,
        clip: clip_of(vm, f),
        alpha: effective_alpha(vm, f),
        actors,
        camera,
        light: light_of(vm, f),
        time: crate::clock::now(vm),
    })
}

/// The scene: every visible region, back to front.
pub fn build(vm: &mut Vm) -> Scene {
    let effects = crate::animations::effects(vm);
    EFFECTS.with(|e| *e.borrow_mut() = effects);
    let scene = build_scene(vm);
    EFFECTS.with(|e| e.borrow_mut().clear());
    scene
}

fn build_scene(vm: &mut Vm) -> Scene {
    let (width, height) = {
        let s = &engine(vm).screen;
        (s.width.round() as u32, s.height.round() as u32)
    };
    let order = frames_in_order(vm);
    let hover = engine(vm).world.input.mouse_focus;
    let mut items = Vec::new();
    for f in order {
        let (children, class) = {
            let w = &engine(vm).world.widgets[f.0];
            (w.children.clone(), w.class.clone())
        };
        // A message frame's messages are laid out as its lines.
        let messages: Vec<usize> = engine(vm)
            .world
            .messages
            .get(&f.0)
            .map(|l| {
                l.iter()
                    .filter_map(|(_, v)| widgets::widget_of(vm, *v))
                    .map(|w| w.0)
                    .collect()
            })
            .unwrap_or_default();
        let mut regions: Vec<(usize, i32, usize)> = Vec::new();
        for c in children {
            let w = &engine(vm).world.widgets[c.0];
            if !w.shown || !is_region(vm, c) || messages.contains(&c.0) {
                continue;
            }
            let (layer, sub) = layer_of(vm, c);
            // The highlight layer while the mouse is over the frame.
            if layer == 4 && hover != Some(f) {
                continue;
            }
            regions.push((layer, sub, c.0));
        }
        regions.sort();
        for (_, _, r) in regions {
            let id = WidgetId(r);
            let class = engine(vm).world.widgets[r].class.clone();
            if widgets::is_a(engine(vm), &class, "FontString") {
                if let Some(t) = text_item(vm, id, false) {
                    items.push(Item::Text(t));
                }
            } else if widgets::is_a(engine(vm), &class, "MaskTexture") {
                continue;
            } else if let Some(q) = quad_of(vm, id) {
                items.push(Item::Quad(q));
            }
        }
        // A model frame's model over its regions (unverified: the
        // client's order of a model and its frame's own regions).
        if let Some(m) = model_item(vm, f, &class) {
            items.push(Item::Model(m));
        }
        if widgets::is_a(engine(vm), &class, "EditBox")
            && let Some(t) = text_item(vm, f, true)
        {
            items.push(Item::Text(t));
        }
        if !messages.is_empty() {
            items.extend(message_items(vm, f).into_iter().map(Item::Text));
        }
    }
    Scene {
        width,
        height,
        items,
    }
}

/// The topmost visible mouse-enabled frame at a point (screen pixels from
/// the top left), as a click there finds it.
pub fn hit_test(vm: &mut Vm, x: f64, y: f64) -> Option<WidgetId> {
    let order = frames_in_order(vm);
    order.into_iter().rev().find(|&f| {
        if !mouse_enabled(vm, f) {
            return false;
        }
        let Some(r) = rect_of(vm, f) else {
            return false;
        };
        let inside = r.contains(x as f32, y as f32);
        let clipped = clip_of(vm, f).is_some_and(|c| !c.contains(x as f32, y as f32));
        inside && !clipped
    })
}

/// The rectangle that holds a frame and everything drawn of it (a
/// screenshot of one frame).
pub fn extent(vm: &Vm, id: WidgetId) -> Option<Rect> {
    let mut acc: Option<Rect> = rect_of(vm, id);
    let mut stack = engine(vm).world.widgets[id.0].children.clone();
    while let Some(c) = stack.pop() {
        if !engine(vm).world.widgets[c.0].shown {
            continue;
        }
        if let Some(r) = rect_of(vm, c) {
            acc = Some(match acc {
                None => r,
                Some(a) => {
                    let x0 = a.x.min(r.x);
                    let y0 = a.y.min(r.y);
                    let x1 = (a.x + a.w).max(r.x + r.w);
                    let y1 = (a.y + a.h).max(r.y + r.h);
                    Rect {
                        x: x0,
                        y: y0,
                        w: x1 - x0,
                        h: y1 - y0,
                    }
                }
            });
        }
        stack.extend(engine(vm).world.widgets[c.0].children.iter().copied());
    }
    acc
}
