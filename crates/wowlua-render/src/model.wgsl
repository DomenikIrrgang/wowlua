// A model's batches: a texture times the batch's color, lit by the frame's
// light, into a transparent target (colors premultiplied by alpha).

struct Globals {
    view_proj: mat4x4<f32>,
    // The direction the light shines in.
    light_dir: vec4<f32>,
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct In {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    // 1: unlit, 2: alpha key, 4: opaque.
    @location(4) flags: u32,
};

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) flags: u32,
};

@vertex
fn vs(v: In) -> Out {
    var o: Out;
    o.pos = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.normal = v.normal;
    o.uv = v.uv;
    o.color = v.color;
    o.flags = v.flags;
    return o;
}

@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    var c = textureSample(tex, samp, i.uv) * i.color;
    if (i.flags & 2u) != 0u {
        if c.a < 0.502 {
            discard;
        }
    }
    if (i.flags & 1u) == 0u {
        let n = normalize(i.normal);
        let d = max(dot(n, -normalize(g.light_dir.xyz)), 0.0);
        let light = min(g.ambient.rgb + g.diffuse.rgb * d, vec3<f32>(1.0));
        c = vec4<f32>(c.rgb * light, c.a);
    }
    if (i.flags & 6u) != 0u {
        c.a = 1.0;
    }
    return c;
}
