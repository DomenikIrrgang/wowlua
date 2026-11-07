// The UI's quads: a texture times the vertex color, its masks' alphas (up
// to three, as the client allows; the unused ones white), and the client's
// per-texture switches.

struct Screen {
    size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var mask0: texture_2d<f32>;
@group(1) @binding(2) var mask1: texture_2d<f32>;
@group(1) @binding(3) var mask2: texture_2d<f32>;
@group(1) @binding(4) var samp: sampler;

struct In {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mask_uv0: vec2<f32>,
    @location(4) mask_uv1: vec2<f32>,
    @location(5) mask_uv2: vec2<f32>,
    // 1: desaturated, 2: alpha key.
    @location(6) flags: u32,
};

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) mask_uv0: vec2<f32>,
    @location(3) mask_uv1: vec2<f32>,
    @location(4) mask_uv2: vec2<f32>,
    @location(5) @interpolate(flat) flags: u32,
};

@vertex
fn vs(v: In) -> Out {
    var o: Out;
    let ndc = vec2<f32>(v.pos.x / screen.size.x * 2.0 - 1.0, 1.0 - v.pos.y / screen.size.y * 2.0);
    o.pos = vec4<f32>(ndc, 0.0, 1.0);
    o.uv = v.uv;
    o.color = v.color;
    o.mask_uv0 = v.mask_uv0;
    o.mask_uv1 = v.mask_uv1;
    o.mask_uv2 = v.mask_uv2;
    o.flags = v.flags;
    return o;
}

@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    var c = textureSample(tex, samp, i.uv) * i.color;
    // The masks intersect: each pixel takes every mask's alpha.
    c.a = c.a * textureSample(mask0, samp, i.mask_uv0).a
        * textureSample(mask1, samp, i.mask_uv1).a
        * textureSample(mask2, samp, i.mask_uv2).a;
    if (i.flags & 1u) != 0u {
        let g = dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
        c = vec4<f32>(g, g, g, c.a);
    }
    if (i.flags & 2u) != 0u {
        if c.a < 0.5 {
            discard;
        }
        c.a = 1.0;
    }
    return c;
}
