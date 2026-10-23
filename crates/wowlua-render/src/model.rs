//! Models (spec debugger §18.11, decision T318): a model frame's or scene's
//! models drawn with depth and light into a texture of the frame's size,
//! which the UI pass then draws in the frame's place.
//!
//! Model files are read once and kept with the sequences read so far;
//! each frame poses them on the CPU (bones, then skinning) and draws one
//! vertex buffer.

use std::collections::HashMap;

use wgpu::util::DeviceExt;
use wowlua_casc::display::{Composite, Display, Texture};
use wowlua_casc::m2::{self, Mat4, Model, Skin, Vec3};
use wowlua_engine::scene::{self, Image, ModelCamera};

use crate::{Assets, Renderer};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
    flags: u32,
}

const UNLIT: u32 = 1;
const ALPHA_KEY: u32 = 2;
const OPAQUE: u32 = 4;

/// A model file read, with its first skin profile.
pub(crate) struct Loaded {
    model: Model,
    skin: Skin,
}

/// The body's pose and attachment points, for the parts on it.
pub(crate) struct Body {
    points: HashMap<u32, Mat4>,
    pose: Vec<Mat4>,
}

/// What the model pass keeps between frames.
#[derive(Default)]
pub(crate) struct Models {
    /// Model files by ID (none: not readable).
    files: HashMap<u32, Option<Loaded>>,
    /// Displays by their source.
    displays: HashMap<String, Option<Display>>,
    pipelines: HashMap<(u16, bool), wgpu::RenderPipeline>,
    layout: Option<(
        wgpu::BindGroupLayout,
        wgpu::BindGroupLayout,
        wgpu::PipelineLayout,
    )>,
    shader: Option<wgpu::ShaderModule>,
    sampler: Option<wgpu::Sampler>,
    groups: HashMap<usize, wgpu::BindGroup>,
    /// Composited textures by their layers.
    composites: HashMap<String, usize>,
}

/// A batch to draw: its vertices' range and how.
struct Draw {
    blend: u16,
    depth_write: bool,
    tex: usize,
    start: u32,
    end: u32,
}

/// The view and projection of a camera, and whether anything was framed.
fn look_at(eye: Vec3, forward: Vec3, up: Vec3) -> Mat4 {
    let f = normalize(forward);
    let r = normalize(cross(f, up));
    let u = cross(r, f);
    [
        r[0],
        u[0],
        -f[0],
        0.0,
        r[1],
        u[1],
        -f[1],
        0.0,
        r[2],
        u[2],
        -f[2],
        0.0,
        -dot(r, eye),
        -dot(u, eye),
        dot(f, eye),
        1.0,
    ]
}

fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y / 2.0).tan();
    let mut m = [0.0; 16];
    m[0] = f / aspect;
    m[5] = f;
    m[10] = far / (near - far);
    m[11] = -1.0;
    m[14] = near * far / (near - far);
    m
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: Vec3) -> Vec3 {
    let l = dot(a, a).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn lerp3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// A rotation about Z (yaw), Y (pitch), X (roll).
fn orientation(yaw: f32, pitch: f32, roll: f32) -> Mat4 {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sr, cr) = roll.sin_cos();
    let rz: Mat4 = [
        cy, sy, 0.0, 0.0, -sy, cy, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let ry: Mat4 = [
        cp, 0.0, -sp, 0.0, 0.0, 1.0, 0.0, 0.0, sp, 0.0, cp, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let rx: Mat4 = [
        1.0, 0.0, 0.0, 0.0, 0.0, cr, sr, 0.0, 0.0, -sr, cr, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    m2::mul(&m2::mul(&rz, &ry), &rx)
}

/// The vertical field of view of a diagonal one at an aspect ratio.
fn vertical_fov(diagonal: f32, aspect: f32) -> f32 {
    2.0 * ((diagonal / 2.0).tan() / (1.0 + aspect * aspect).sqrt()).atan()
}

/// A display's key in the cache.
fn source_key(s: &scene::ModelSource) -> String {
    format!("{s:?}")
}

impl Renderer {
    fn model_layouts(&mut self) {
        if self.models.layout.is_some() {
            return;
        }
        let d = &self.device;
        let globals = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tex = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("model"),
            bind_group_layouts: &[Some(&globals), Some(&tex)],
            immediate_size: 0,
        });
        self.models.shader = Some(d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("model"),
            source: wgpu::ShaderSource::Wgsl(include_str!("model.wgsl").into()),
        }));
        self.models.sampler = Some(d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("model"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }));
        self.models.layout = Some((globals, tex, layout));
    }

    /// The pipeline of an M2 blend mode, writing depth or not.
    fn model_pipeline(&mut self, blend: u16, depth_write: bool) -> &wgpu::RenderPipeline {
        let (layout, shader) = (
            &self.models.layout.as_ref().unwrap().2,
            self.models.shader.as_ref().unwrap(),
        );
        let device = &self.device;
        self.models
            .pipelines
            .entry((blend, depth_write))
            .or_insert_with(|| {
                use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
                let c = |src, dst| C {
                    src_factor: src,
                    dst_factor: dst,
                    operation: O::Add,
                };
                let keep = c(F::Zero, F::One);
                let state = match blend {
                    0 | 1 => None,
                    2 => Some(wgpu::BlendState {
                        color: c(F::SrcAlpha, F::OneMinusSrcAlpha),
                        alpha: c(F::One, F::OneMinusSrcAlpha),
                    }),
                    3 => Some(wgpu::BlendState {
                        color: c(F::One, F::One),
                        alpha: keep,
                    }),
                    4 => Some(wgpu::BlendState {
                        color: c(F::SrcAlpha, F::One),
                        alpha: keep,
                    }),
                    5 => Some(wgpu::BlendState {
                        color: c(F::Dst, F::Zero),
                        alpha: keep,
                    }),
                    6 => Some(wgpu::BlendState {
                        color: c(F::Dst, F::Src),
                        alpha: keep,
                    }),
                    _ => Some(wgpu::BlendState {
                        color: c(F::One, F::OneMinusSrcAlpha),
                        alpha: c(F::One, F::OneMinusSrcAlpha),
                    }),
                };
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("model"),
                    layout: Some(layout),
                    vertex: wgpu::VertexState {
                        module: shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<MVertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Uint32],
                        })],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(depth_write),
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: wgpu::StencilState::default(),
                        bias: wgpu::DepthBiasState::default(),
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: shader,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: crate::FORMAT,
                            blend: state,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
    }

    fn model_group(&mut self, tex: usize) {
        if self.models.groups.contains_key(&tex) {
            return;
        }
        let g = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.models.layout.as_ref().unwrap().1,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.textures[tex].view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(self.models.sampler.as_ref().unwrap()),
                },
            ],
        });
        self.models.groups.insert(tex, g);
    }

    /// A model file, read once (with its skeleton and first skin).
    fn loaded(&mut self, file: u32, assets: &mut dyn Assets) -> bool {
        if let Some(l) = self.models.files.get(&file) {
            return l.is_some();
        }
        let loaded = (|| {
            let bytes = assets.file(file)?;
            let mut model = Model::parse(&bytes)?;
            if let Some(skel) = model.skeleton_file
                && let Some(b) = assets.file(skel)
            {
                if let Some(parent) = Model::skeleton_parent(&b)
                    && let Some(pb) = assets.file(parent)
                {
                    model.load_skeleton(&pb);
                }
                model.load_skeleton(&b);
            }
            let skin = Skin::parse(&assets.file(*model.skin_files.first()?)?)?;
            Some(Loaded { model, skin })
        })();
        let ok = loaded.is_some();
        self.models.files.insert(file, loaded);
        ok
    }

    /// Reads a sequence's `.anim` file in, where it is elsewhere.
    fn ensure_anim(&mut self, file: u32, sequence: usize, assets: &mut dyn Assets) {
        let Some(Some(l)) = self.models.files.get(&file) else {
            return;
        };
        if let Some(anim) = l.model.needs_anim(sequence) {
            let bytes = assets.file(anim);
            if let Some(Some(l)) = self.models.files.get_mut(&file) {
                match bytes {
                    Some(b) => l.model.load_anim(sequence, &b),
                    // Unreadable: marked read, so not asked again.
                    None => l.model.load_anim(sequence, &[]),
                }
            }
        }
    }

    /// A composited texture (a character's skin), made once.
    fn composite(&mut self, c: &Composite, assets: &mut dyn Assets) -> usize {
        let key = format!("{c:?}");
        if let Some(&t) = self.models.composites.get(&key) {
            return t;
        }
        let img = crate::composite::composite(c, &mut |f| assets.image(&Image::FileId(f)));
        let t = self.upload(&img);
        self.models.composites.insert(key, t);
        t
    }

    /// Draws a model item into a texture of its size at `scale`: the
    /// texture's index, or none when nothing could be drawn.
    pub(crate) fn draw_model(
        &mut self,
        item: &scene::Model,
        scale: f32,
        assets: &mut dyn Assets,
    ) -> Option<usize> {
        let max = self.device.limits().max_texture_dimension_2d;
        let width = ((item.rect.w * scale).round() as u32).clamp(1, max);
        let height = ((item.rect.h * scale).round() as u32).clamp(1, max);
        let aspect = width as f32 / height as f32;
        self.model_layouts();
        let mut verts: Vec<MVertex> = Vec::new();
        let mut draws: Vec<Draw> = Vec::new();
        // The posed points of every actor, for a frame's camera.
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        let mut portrait: Option<(Vec3, Vec3, f32)> = None;
        let mut own_camera: Option<(Vec3, Vec3, f32)> = None;
        let ms = (item.time * 1000.0) as u32;
        for actor in &item.actors {
            let key = source_key(&actor.source);
            if !self.models.displays.contains_key(&key) {
                let d = assets.model(&actor.source);
                self.models.displays.insert(key.clone(), d);
            }
            let Some(display) = self.models.displays.get(&key).cloned().flatten() else {
                continue;
            };
            let scale = actor.scale * display.scale;
            let mut world = m2::translation(actor.position);
            world = m2::mul(&world, &orientation(actor.yaw, actor.pitch, actor.roll));
            world = m2::mul(&world, &m2::scaling([scale; 3]));
            // The body's attachment points, for parts there.
            let mut body_points: Option<Body> = None;
            for part in &display.parts {
                if !self.loaded(part.file, assets) {
                    continue;
                }
                let seq = {
                    let Some(Some(l)) = self.models.files.get(&part.file) else {
                        continue;
                    };
                    l.model
                        .sequence(actor.animation as u16)
                        .or_else(|| l.model.sequence(0))
                };
                if let Some(s) = seq {
                    self.ensure_anim(part.file, s, assets);
                }
                // Taken out while drawn (its textures load meanwhile).
                let Some(Some(l)) = self.models.files.remove(&part.file) else {
                    continue;
                };
                self.draw_part(
                    &l,
                    part,
                    actor,
                    &display,
                    &item.camera,
                    world,
                    seq,
                    ms,
                    &mut body_points,
                    &mut portrait,
                    &mut own_camera,
                    &mut verts,
                    &mut draws,
                    &mut lo,
                    &mut hi,
                    assets,
                );
                self.models.files.insert(part.file, Some(l));
            }
        }
        self.finish_model(
            item, width, height, aspect, verts, draws, lo, hi, portrait, own_camera,
        )
    }

    /// One part's batches, posed, into the vertex list.
    #[allow(clippy::too_many_arguments)]
    fn draw_part(
        &mut self,
        l: &Loaded,
        part: &wowlua_casc::display::Part,
        actor: &scene::Actor,
        display: &Display,
        camera: &ModelCamera,
        world: Mat4,
        seq: Option<usize>,
        ms: u32,
        body_points: &mut Option<Body>,
        portrait: &mut Option<(Vec3, Vec3, f32)>,
        own_camera: &mut Option<(Vec3, Vec3, f32)>,
        verts: &mut Vec<MVertex>,
        draws: &mut Vec<Draw>,
        lo: &mut [f32; 3],
        hi: &mut [f32; 3],
        assets: &mut dyn Assets,
    ) {
        let (m, skin) = (&l.model, &l.skin);
        let time = seq.map_or(0, |s| {
            let d = m.sequences[s].duration;
            if d > 0 { ms % d } else { 0 }
        });
        // A part made for the body's skeleton takes its pose.
        let pose = match body_points.as_ref() {
            Some(b) if part.skinned && b.pose.len() >= m.bones.len() => b.pose.clone(),
            _ => m.pose(seq, time),
        };
        let mut part_world = world;
        if let Some(att) = part.attachment {
            let Some(a) = body_points.as_ref().and_then(|p| p.points.get(&att)) else {
                return;
            };
            part_world = m2::mul(&world, a);
        }
        let skinned = m.skinned(&pose);
        if part.attachment.is_none() && body_points.is_none() {
            *body_points = Some(Body {
                points: m
                    .attachments
                    .iter()
                    .filter_map(|a| Some((a.id, m.attachment(a.id, &pose)?)))
                    .collect(),
                pose: pose.clone(),
            });
            if let Some(c) = m.camera(0) {
                *portrait = Some((
                    m2::transform(&world, c.position),
                    m2::transform(&world, c.target),
                    c.fov,
                ));
            }
            if let ModelCamera::Frame {
                camera: Some(i), ..
            } = camera
                && let Some(c) = m.cameras.get((*i).max(0) as usize)
            {
                *own_camera = Some((
                    m2::transform(&world, c.position),
                    m2::transform(&world, c.target),
                    c.fov,
                ));
            }
        }
        // Opaque batches first, then the rest by priority.
        let mut order: Vec<usize> = (0..skin.batches.len()).collect();
        order.sort_by_key(|&b| {
            let batch = &skin.batches[b];
            let blend = m
                .materials
                .get(batch.material as usize)
                .map_or(0, |x| x.blend);
            (blend > 1, batch.priority, b)
        });
        for b in order {
            let batch = skin.batches[b];
            let Some(sub) = skin.submeshes.get(batch.submesh as usize) else {
                continue;
            };
            if !part.shows(sub.id) {
                continue;
            }
            let mat = m
                .materials
                .get(batch.material as usize)
                .copied()
                .unwrap_or(m2::Material { flags: 0, blend: 0 });
            let tint = m.tint(&batch, seq, time);
            let alpha = tint.alpha * actor.alpha * display.alpha;
            if alpha <= 0.002 {
                continue;
            }
            let tex_index = m
                .texture_combos
                .get(batch.texture_combo as usize)
                .copied()
                .unwrap_or(0) as usize;
            let tex = match m.textures.get(tex_index) {
                Some(t) if t.kind == 0 => self.texture(&Image::FileId(t.file), assets),
                Some(t) => match part.textures.get(&t.kind) {
                    Some(Texture::File(f)) => self.texture(&Image::FileId(*f), assets),
                    Some(Texture::Composite(c)) => self.composite(c, assets),
                    None => self.white,
                },
                None => self.white,
            };
            self.model_group(tex);
            let uv_m = m.uv_transform(&batch, seq, time);
            let mut flags = 0;
            if mat.flags & 1 != 0 {
                flags |= UNLIT;
            }
            match mat.blend {
                0 => flags |= OPAQUE,
                1 => flags |= ALPHA_KEY,
                _ => {}
            }
            let color = [tint.color[0], tint.color[1], tint.color[2], alpha];
            let start = verts.len() as u32;
            let first = sub.index_start as usize;
            let end = (first + sub.index_count as usize).min(skin.indices.len());
            for &vi in &skin.indices[first.min(end)..end] {
                let Some((p, n)) = skinned.get(vi as usize) else {
                    continue;
                };
                let v = &m.vertices[vi as usize];
                let wp = m2::transform(&part_world, *p);
                let wn = normalize(m2::transform_dir(&part_world, *n));
                for k in 0..3 {
                    lo[k] = lo[k].min(wp[k]);
                    hi[k] = hi[k].max(wp[k]);
                }
                let uv = match &uv_m {
                    Some(t) => {
                        let q = m2::transform(t, [v.uv[0], v.uv[1], 0.0]);
                        [q[0], q[1]]
                    }
                    None => v.uv,
                };
                verts.push(MVertex {
                    pos: wp,
                    normal: wn,
                    uv,
                    color,
                    flags,
                });
            }
            let end = verts.len() as u32;
            if end > start {
                draws.push(Draw {
                    blend: mat.blend,
                    depth_write: mat.blend <= 1 && mat.flags & 0x10 == 0,
                    tex,
                    start,
                    end,
                });
            }
        }
    }

    /// The camera, and the vertices drawn into the model's texture.
    #[allow(clippy::too_many_arguments)]
    fn finish_model(
        &mut self,
        item: &scene::Model,
        width: u32,
        height: u32,
        aspect: f32,
        verts: Vec<MVertex>,
        draws: Vec<Draw>,
        lo: [f32; 3],
        hi: [f32; 3],
        portrait: Option<(Vec3, Vec3, f32)>,
        own_camera: Option<(Vec3, Vec3, f32)>,
    ) -> Option<usize> {
        if verts.is_empty() {
            return None;
        }
        // The camera.
        let (view, proj) = match &item.camera {
            ModelCamera::Scene {
                position,
                forward,
                up,
                fov,
                near,
                far,
                ..
            } => (
                look_at(*position, *forward, *up),
                perspective(fov.max(0.01), aspect, near.max(0.001), far.max(near + 0.01)),
            ),
            ModelCamera::Frame {
                portrait_zoom,
                distance_scale,
                ..
            } => {
                let (eye, target, fov_y) = match own_camera {
                    Some((eye, target, fov)) => (eye, target, vertical_fov(fov, aspect)),
                    None => {
                        // The whole model in view from in front (+X).
                        let center = lerp3(lo, hi, 0.5);
                        let fov_y: f32 = 0.6;
                        let half_h = (hi[2] - lo[2]) / 2.0;
                        let half_w = (hi[1] - lo[1]) / 2.0;
                        let t = (fov_y / 2.0).tan();
                        let dist =
                            (half_h / t).max(half_w / (t * aspect)) * 1.05 + (hi[0] - lo[0]) / 2.0;
                        let full = (
                            [center[0] + dist * distance_scale, center[1], center[2]],
                            center,
                            fov_y,
                        );
                        match portrait {
                            Some((pe, pt, pf)) if *portrait_zoom > 0.0 => {
                                let z = portrait_zoom.clamp(0.0, 1.0);
                                (
                                    lerp3(full.0, pe, z),
                                    lerp3(full.1, pt, z),
                                    full.2 + (vertical_fov(pf, aspect) - full.2) * z,
                                )
                            }
                            _ => full,
                        }
                    }
                };
                let forward = sub(target, eye);
                let dist = dot(forward, forward).sqrt();
                let radius = dot(sub(hi, lo), sub(hi, lo)).sqrt().max(0.01);
                (
                    look_at(eye, forward, [0.0, 0.0, 1.0]),
                    perspective(
                        fov_y,
                        aspect,
                        (dist - radius).max(dist * 0.01).max(0.01),
                        dist + radius * 2.0,
                    ),
                )
            }
        };
        let view_proj = m2::mul(&proj, &view);
        let l = &item.light;
        let (ambient, diffuse) = if l.enabled {
            (l.ambient, l.diffuse)
        } else {
            ([1.0; 3], [0.0; 3])
        };
        let mut globals = Vec::with_capacity(28);
        globals.extend_from_slice(&view_proj);
        globals.extend_from_slice(&[l.direction[0], l.direction[1], l.direction[2], 0.0]);
        globals.extend_from_slice(&[ambient[0], ambient[1], ambient[2], 1.0]);
        globals.extend_from_slice(&[diffuse[0], diffuse[1], diffuse[2], 1.0]);
        let ubuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("model globals"),
                contents: bytemuck::cast_slice(&globals),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let ugroup = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.models.layout.as_ref().unwrap().0,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            }],
        });
        let vbuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("model vertices"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let color = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("model"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("model depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        for d in &draws {
            self.model_pipeline(d.blend.min(7), d.depth_write);
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("model"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("model"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_bind_group(0, &ugroup, &[]);
            for d in &draws {
                pass.set_pipeline(&self.models.pipelines[&(d.blend.min(7), d.depth_write)]);
                pass.set_bind_group(1, &self.models.groups[&d.tex], &[]);
                pass.draw(d.start..d.end, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        // The frame's model textures take the same slots each frame.
        let tex = crate::Tex {
            texture: color,
            view: color_view,
            width,
            height,
        };
        let slot = match self.model_slots.get(self.model_slot_next).copied() {
            Some(s) => {
                self.textures[s] = tex;
                self.bind_groups.retain(|k, _| k.0 != s && k.1 != s);
                s
            }
            None => {
                self.textures.push(tex);
                self.model_slots.push(self.textures.len() - 1);
                self.textures.len() - 1
            }
        };
        self.model_slot_next += 1;
        Some(slot)
    }
}
