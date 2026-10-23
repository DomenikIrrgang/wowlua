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
