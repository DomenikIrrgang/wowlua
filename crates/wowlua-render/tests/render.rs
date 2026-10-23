//! The renderer on the engine's scene: what is drawn where (spec debugger
//! §18.11). Skipped where there is no GPU adapter, software ones included.

use wowlua_engine::scene;
use wowlua_render::{NoAssets, Renderer};
use wowlua_vm::Vm;

fn vm(src: &str) -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "standard").expect("the engine");
    let f = vm.load(src.as_bytes(), "@ui.lua").expect("compiles");
    vm.call(f, &[]).expect("runs");
    vm
}

fn px(img: &wowlua_render::Rgba, x: f32, y: f32) -> [u8; 4] {
    let o = ((y as u32 * img.width + x as u32) * 4) as usize;
    img.pixels[o..o + 4].try_into().unwrap()
}

#[test]
fn draws_textures_text_and_placeholders_where_the_layout_puts_them() {
    let mut r = match Renderer::new_blocking() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let mut vm = vm("local f = CreateFrame('Frame', 'Box', UIParent)
f:SetSize(200, 100)
f:SetPoint('TOPLEFT', 10, -10)
local bg = f:CreateTexture(nil, 'BACKGROUND')
bg:SetAllPoints()
bg:SetColorTexture(1, 0, 0, 1)
local half = f:CreateTexture(nil, 'ARTWORK')
half:SetPoint('TOPLEFT')
half:SetSize(100, 50)
half:SetColorTexture(0, 0, 1, 0.5)
local missing = f:CreateTexture(nil, 'ARTWORK')
missing:SetPoint('BOTTOMRIGHT')
missing:SetSize(40, 40)
missing:SetTexture('Interface\\\\Nowhere\\\\Nothing')
local t = f:CreateFontString(nil, 'OVERLAY')
t:SetFont('Fonts\\\\FRIZQT__.TTF', 20)
t:SetPoint('BOTTOMLEFT', 0, 0)
t:SetSize(160, 40)
t:SetText('HELLO')
");
    let s = scene::build(&mut vm);
    let started = std::time::Instant::now();
    let img = r
        .render_blocking(&s, 1.0, None, &mut NoAssets)
        .expect("drawn");
    eprintln!(
        "{} on {}: {:?}",
        s.items.len(),
        r.adapter,
        started.elapsed()
    );
    assert_eq!((img.width, img.height), (s.width, s.height));
    let id = |name: &str| {
        let e = wowlua_engine::engine(&vm);
        (0..e.world.widgets.len())
            .find(|&i| e.world.widgets[i].name.as_deref() == Some(name))
            .map(wowlua_engine::widgets::WidgetId)
            .unwrap()
    };
    let b = scene::rect_of(&vm, id("Box")).unwrap();
    // Outside: the black behind the UI. Inside: red; the top left quarter
    // half blue over it.
    assert_eq!(px(&img, b.x - 3.0, b.y - 3.0), [0, 0, 0, 255]);
    let red = px(&img, b.x + b.w * 0.75, b.y + b.h * 0.25);
    assert!(red[0] > 250 && red[1] < 5 && red[2] < 5, "{red:?}");
    let mixed = px(&img, b.x + b.w * 0.2, b.y + b.h * 0.2);
    assert!(
        (120..136).contains(&mixed[0]) && (120..136).contains(&mixed[2]),
        "{mixed:?}"
    );
    // The missing texture: the magenta checker.
    let m = px(&img, b.x + b.w - 5.0, b.y + b.h - 5.0);
    assert!(m[0] == m[2] && m[1] == 0, "{m:?}");
    // The text: white glyphs over the red in its rectangle's bottom left.
    let mut white = 0;
    for y in (b.y + b.h * 0.6) as u32..(b.y + b.h) as u32 {
        for x in b.x as u32..(b.x + b.w * 0.6) as u32 {
            let p = px(&img, x as f32, y as f32);
            if p[1] > 200 && p[2] > 200 {
                white += 1;
            }
        }
    }
    assert!(white > 30, "glyph pixels: {white}");
    // A frame's own rectangle, and twice the size.
    let crop = scene::extent(&vm, id("Box")).unwrap();
    let part = r
        .render_blocking(&s, 2.0, Some(crop), &mut NoAssets)
        .unwrap();
    assert_eq!(part.width, (crop.w * 2.0).ceil() as u32);
    let png = wowlua_render::encode_png(&part).unwrap();
    let back = wowlua_render::decode_png(&png).unwrap();
    assert_eq!(back.pixels, part.pixels);
}

/// A model of one square (four vertices facing +X, unlit, a texture of
/// its own), chunked as the client's are.
fn square_model() -> (Vec<u8>, Vec<u8>) {
    let mut md = vec![0u8; 320];
    md[..4].copy_from_slice(b"MD20");
    md[4..8].copy_from_slice(&272u32.to_le_bytes());
    let put =
        |md: &mut Vec<u8>, at: usize, v: u32| md[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut md, 68, 1);
    // Vertices.
    let verts = md.len() as u32;
    for (y, z, u, v) in [
        (-1.0f32, -1.0f32, 0.0f32, 1.0f32),
        (1.0, -1.0, 1.0, 1.0),
        (-1.0, 1.0, 0.0, 0.0),
        (1.0, 1.0, 1.0, 0.0),
    ] {
        for f in [0.0, y, z] {
            md.extend_from_slice(&f32::to_le_bytes(f));
        }
        md.extend_from_slice(&[255, 0, 0, 0, 0, 0, 0, 0]);
        for f in [1.0f32, 0.0, 0.0, u, v, 0.0, 0.0] {
            md.extend_from_slice(&f.to_le_bytes());
        }
    }
    put(&mut md, 60, 4);
    put(&mut md, 64, verts);
    // One texture of the model's own, one unlit opaque material, one
    // texture combo.
    let tex = md.len() as u32;
    md.extend_from_slice(&[0; 16]);
    put(&mut md, 80, 1);
    put(&mut md, 84, tex);
    let mat = md.len() as u32;
    md.extend_from_slice(&[1, 0, 0, 0]);
    put(&mut md, 112, 1);
    put(&mut md, 116, mat);
    let combo = md.len() as u32;
    md.extend_from_slice(&[0, 0, 0, 0]);
    put(&mut md, 128, 1);
    put(&mut md, 132, combo);
    let chunk = |tag: &[u8], body: &[u8]| {
        let mut c = tag.to_vec();
        c.extend_from_slice(&(body.len() as u32).to_le_bytes());
        c.extend_from_slice(body);
        c
    };
    let mut file = chunk(b"MD21", &md);
    file.extend(chunk(b"SFID", &100u32.to_le_bytes()));
    file.extend(chunk(b"TXID", &200u32.to_le_bytes()));
    // The skin: the four vertices, two triangles, one submesh, one batch.
    let mut skin = vec![0u8; 48];
    skin[..4].copy_from_slice(b"SKIN");
    let arr = |s: &mut Vec<u8>, at: usize, n: u32, off: u32| {
        s[at..at + 4].copy_from_slice(&n.to_le_bytes());
        s[at + 4..at + 8].copy_from_slice(&off.to_le_bytes());
    };
    let lookup = skin.len() as u32;
    for i in 0u16..4 {
        skin.extend_from_slice(&i.to_le_bytes());
    }
    arr(&mut skin, 4, 4, lookup);
    let tris = skin.len() as u32;
    for i in [0u16, 1, 2, 2, 1, 3] {
        skin.extend_from_slice(&i.to_le_bytes());
    }
    arr(&mut skin, 12, 6, tris);
    let sub = skin.len() as u32;
    let mut s = vec![0u8; 48];
    s[6..8].copy_from_slice(&4u16.to_le_bytes());
    s[10..12].copy_from_slice(&6u16.to_le_bytes());
    skin.extend(s);
    arr(&mut skin, 28, 1, sub);
    let batch = skin.len() as u32;
    let mut b = vec![0u8; 24];
    b[8..10].copy_from_slice(&(-1i16).to_le_bytes());
    b[14..16].copy_from_slice(&1u16.to_le_bytes());
    skin.extend(b);
    arr(&mut skin, 36, 1, batch);
    (file, skin)
}

struct ModelAssets {
    model: Vec<u8>,
    skin: Vec<u8>,
}

impl wowlua_render::Assets for ModelAssets {
    fn image(&mut self, image: &scene::Image) -> Option<wowlua_render::Rgba> {
        (*image == scene::Image::FileId(200)).then(|| wowlua_render::Rgba {
            width: 2,
            height: 2,
            pixels: [255, 0, 0, 255].repeat(4),
        })
    }
    fn font(&mut self, _: &str) -> Option<Vec<u8>> {
        None
    }
    fn file(&mut self, id: u32) -> Option<Vec<u8>> {
        match id {
            1 => Some(self.model.clone()),
            100 => Some(self.skin.clone()),
            _ => None,
        }
    }
    fn model(&mut self, source: &scene::ModelSource) -> Option<wowlua_render::Display> {
        (*source == scene::ModelSource::Display(7)).then(|| wowlua_casc::display::Library::file(1))
    }
}

#[test]
fn draws_a_model_frames_model_in_its_place() {
    let mut r = match Renderer::new_blocking() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let mut vm = vm("local m = CreateFrame('PlayerModel', nil, UIParent)
m:SetSize(100, 100)
m:SetPoint('TOPLEFT', 50, -50)
m:SetDisplayInfo(7)
");
    let s = scene::build(&mut vm);
    let (model, skin) = square_model();
    let mut assets = ModelAssets { model, skin };
    let img = r
        .render_blocking(&s, 1.0, None, &mut assets)
        .expect("drawn");
    // The square fills the frame's middle; outside it, the screen.
    assert_eq!(px(&img, 100.0, 100.0), [255, 0, 0, 255]);
    assert_eq!(px(&img, 30.0, 100.0), [0, 0, 0, 255]);
    // Drawn again, the frame's texture is reused.
    let count = r.texture_count();
    r.render_blocking(&s, 1.0, None, &mut assets)
        .expect("drawn");
    assert_eq!(r.texture_count(), count);
}
