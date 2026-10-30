//! The 3D world behind the UI (decision T522): the terrain of the tiles
//! within the view distance (`farclip`), as the camera sees it, lit by a
//! sun and faded into the fog; the UI is drawn over it.
//!
//! A tile is read once (`Assets::terrain`) and kept on the GPU: its
//! chunks' grids (each square four triangles around its middle, holes
//! left out) in one vertex and index buffer, each chunk with its textures
//! and a blend texture of its layers (red, green and blue the second to
//! fourth layer's).

use std::collections::HashMap;

use wgpu::util::DeviceExt;
use wowlua_casc::adt::{self, CHUNK, TILE, Tile, UNIT};
use wowlua_engine::scene::WorldView;

use crate::{Assets, Renderer, Rgba};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view: [f32; 16],
    proj: [f32; 16],
    sun: [f32; 4],
    fog: [f32; 4],
    eye: [f32; 4],
}

/// The sky's and the fog's color (unverified: the zone's light, later).
const SKY: [f32; 3] = [0.62, 0.74, 0.86];
/// Toward the sun (north, west, up), and how much light the shade keeps.
const SUN: [f32; 4] = [0.35, 0.25, 0.9, 0.45];
/// Where the fog starts, of the view distance.
const FOG_START: f32 = 0.55;
/// The nearest the camera draws (yards).
const NEAR: f32 = 0.2;

struct GpuChunk {
    start: u32,
    count: u32,
    group: wgpu::BindGroup,
    center: [f64; 3],
}

struct GpuTile {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    chunks: Vec<GpuChunk>,
}

struct Setup {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    chunk_layout: wgpu::BindGroupLayout,
    tiled: wgpu::Sampler,
    clamped: wgpu::Sampler,
}

/// The world's GPU state: the pipeline, the tiles read so far and the
/// depth buffer.
#[derive(Default)]
pub(crate) struct World {
    setup: Option<Setup>,
    tiles: HashMap<(u32, i32, i32), Option<GpuTile>>,
    depth: Option<(wgpu::TextureView, u32, u32)>,
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// A tile's grids: each chunk's 145 points (north, west, up) and its
/// triangles, with each chunk's range of indices.
fn mesh(tile: &Tile, column: i32, row: i32) -> (Vec<TVertex>, Vec<u32>, Vec<(u32, u32)>) {
    let (north, west) = adt::corner(column, row);
    let mut verts = Vec::with_capacity(tile.chunks.len() * 145);
    let mut indices = Vec::with_capacity(tile.chunks.len() * 768);
    let mut ranges = Vec::with_capacity(tile.chunks.len());
    for c in &tile.chunks {
        let base = verts.len() as u32;
        let n0 = north - f64::from(c.y) * CHUNK;
        let w0 = west - f64::from(c.x) * CHUNK;
        for j in 0..17 {
            let (row, middle) = (j / 2, j % 2 == 1);
            let cols = if middle { 8 } else { 9 };
            let off = if middle { 0.5 } else { 0.0 };
            for col in 0..cols {
                let (r, k) = (row as f64 + off, col as f64 + off);
                verts.push(TVertex {
                    pos: [
                        (n0 - r * UNIT) as f32,
                        (w0 - k * UNIT) as f32,
                        c.height(row, col, middle),
                    ],
                    normal: [0.0; 3],
                    uv: [(k / 8.0) as f32, (r / 8.0) as f32],
                });
            }
        }
        let start = indices.len() as u32;
        for row in 0..8 {
            for col in 0..8 {
                if c.hole(row, col) {
                    continue;
                }
                let nw = base + (row * 17 + col) as u32;
                let (ne, sw) = (nw + 1, nw + 17);
                let (se, m) = (sw + 1, nw + 9);
                indices.extend([m, nw, ne, m, ne, se, m, se, sw, m, sw, nw]);
            }
        }
        // (each point's normal: its triangles', facing up)
        let mut normals = vec![[0f32; 3]; 145];
        for t in indices[start as usize..].chunks(3) {
            let p = |i: u32| verts[i as usize].pos;
            let mut n = cross(sub(p(t[1]), p(t[0])), sub(p(t[2]), p(t[0])));
            if n[2] < 0.0 {
                n = [-n[0], -n[1], -n[2]];
            }
            for &i in t {
                let a = &mut normals[(i - base) as usize];
                a[0] += n[0];
                a[1] += n[1];
                a[2] += n[2];
            }
        }
        for (v, n) in verts[base as usize..].iter_mut().zip(normals) {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            v.normal = if l > 0.0 {
                [n[0] / l, n[1] / l, n[2] / l]
            } else {
                [0.0, 0.0, 1.0]
            };
        }
        ranges.push((start, indices.len() as u32 - start));
    }
    (verts, indices, ranges)
}

/// A chunk's blend texture: the second to fourth layers' blends in red,
/// green and blue (64 × 64, from the north-west).
fn blend_of(c: &adt::Chunk) -> Rgba {
    let mut pixels = vec![0u8; 64 * 64 * 4];
    for (k, layer) in c.layers.iter().enumerate().skip(1).take(3) {
        if let Some(a) = &layer.alpha {
            for (i, v) in a.iter().enumerate().take(4096) {
                pixels[i * 4 + k - 1] = *v;
            }
        }
    }
    for i in 0..4096 {
        pixels[i * 4 + 3] = 255;
    }
    Rgba {
        width: 64,
        height: 64,
        pixels,
    }
}

impl Renderer {
    fn world_setup(&mut self) {
        if self.world.setup.is_some() {
            return;
        }
        let d = &self.device;
        let globals_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world globals"),
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
        let chunk_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world chunk"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                texture_entry(4),
                sampler_entry(5),
                sampler_entry(6),
            ],
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world"),
            bind_group_layouts: &[Some(&globals_layout), Some(&chunk_layout)],
            immediate_size: 0,
        });
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world"),
            source: wgpu::ShaderSource::Wgsl(include_str!("world.wgsl").into()),
        });
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<TVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: crate::FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let globals = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let sampler = |address| {
            d.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("world"),
                address_mode_u: address,
                address_mode_v: address,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        self.world.setup = Some(Setup {
            pipeline,
            globals,
            globals_group,
            chunk_layout,
            tiled: sampler(wgpu::AddressMode::Repeat),
            clamped: sampler(wgpu::AddressMode::ClampToEdge),
        });
    }

    /// A tile on the GPU, read and built on first use.
    fn world_tile(&mut self, map: u32, x: i32, y: i32, assets: &mut dyn Assets) {
        if self.world.tiles.contains_key(&(map, x, y)) {
            return;
        }
        let gpu = assets.terrain(map, x, y).map(|tile| {
            let (verts, indices, ranges) = mesh(&tile, x, y);
            let vertices = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("terrain"),
                    contents: bytemuck::cast_slice(&verts),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let index_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("terrain"),
                    contents: bytemuck::cast_slice(&indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
            let (north, west) = adt::corner(x, y);
            let mut chunks = Vec::with_capacity(tile.chunks.len());
            for (c, (start, count)) in tile.chunks.iter().zip(ranges) {
                let mut layers = [self.white; 4];
                for (slot, l) in layers.iter_mut().zip(&c.layers) {
                    if let Some(&file) = tile.textures.get(l.texture as usize) {
                        *slot = self.model_texture(file, assets);
                    }
                }
                let blend = self.upload(&blend_of(c));
                let setup = self.world.setup.as_ref().expect("the world's setup");
                let view = |i: usize| wgpu::BindingResource::TextureView(&self.textures[i].view);
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("terrain chunk"),
                    layout: &setup.chunk_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: view(layers[0]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: view(layers[1]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: view(layers[2]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: view(layers[3]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: view(blend),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::Sampler(&setup.tiled),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::Sampler(&setup.clamped),
                        },
                    ],
                });
                let middle = c.height(4, 4, false);
                chunks.push(GpuChunk {
                    start,
                    count,
                    group,
                    center: [
                        north - (f64::from(c.y) + 0.5) * CHUNK,
                        west - (f64::from(c.x) + 0.5) * CHUNK,
                        f64::from(middle),
                    ],
                });
            }
            GpuTile {
                vertices,
                indices: index_buffer,
                chunks,
            }
        });
        self.world.tiles.insert((map, x, y), gpu);
    }

    /// Draws the world into `view` (cleared to the sky first), with a
    /// depth buffer of its size: the tiles within the view distance, the
    /// chunks in front of the camera.
    pub(crate) fn draw_world(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        w: &WorldView,
        assets: &mut dyn Assets,
    ) {
        self.world_setup();
        if self
            .world
            .depth
            .as_ref()
            .is_none_or(|(_, dw, dh)| (*dw, *dh) != (width, height))
        {
            let depth = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("world depth"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let v = depth.create_view(&wgpu::TextureViewDescriptor::default());
            self.world.depth = Some((v, width, height));
        }
        // The tiles within the view distance of the eye.
        let (cx, cy) = adt::tile_of(w.eye[0], w.eye[1]);
        let reach = (w.far / TILE).ceil() as i32;
        let mut near = Vec::new();
        for y in cy - reach..=cy + reach {
            for x in cx - reach..=cx + reach {
                let (n, wst) = adt::corner(x, y);
                // (the tile's nearest point to the eye)
                let dn = (w.eye[0] - w.eye[0].clamp(n - TILE, n)).abs();
                let dw = (w.eye[1] - w.eye[1].clamp(wst - TILE, wst)).abs();
                if dn.hypot(dw) <= w.far {
                    near.push((x, y));
                }
            }
        }
        for &(x, y) in &near {
            self.world_tile(w.map, x, y, assets);
        }
        let aspect = width as f32 / height.max(1) as f32;
        let fov_y = 2.0 * ((w.fov as f32 / 2.0).tan() / aspect).atan();
        let f32s = |a: [f64; 3]| [a[0] as f32, a[1] as f32, a[2] as f32];
        let globals = Globals {
            view: crate::model::look_at(f32s(w.eye), f32s(w.look), f32s(w.up)),
            proj: crate::model::perspective(fov_y, aspect, NEAR, w.far as f32),
            sun: {
                let l = (SUN[0] * SUN[0] + SUN[1] * SUN[1] + SUN[2] * SUN[2]).sqrt();
                [SUN[0] / l, SUN[1] / l, SUN[2] / l, SUN[3]]
            },
            fog: [SKY[0], SKY[1], SKY[2], w.far as f32 * FOG_START],
            eye: [
                w.eye[0] as f32,
                w.eye[1] as f32,
                w.eye[2] as f32,
                w.far as f32,
            ],
        };
        let setup = self.world.setup.as_ref().expect("the world's setup");
        self.queue
            .write_buffer(&setup.globals, 0, bytemuck::bytes_of(&globals));
        let depth = &self.world.depth.as_ref().expect("the depth buffer").0;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("world"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(SKY[0]),
                        g: f64::from(SKY[1]),
                        b: f64::from(SKY[2]),
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
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
        pass.set_pipeline(&setup.pipeline);
        pass.set_bind_group(0, &setup.globals_group, &[]);
        for (x, y) in near {
            let Some(Some(tile)) = self.world.tiles.get(&(w.map, x, y)) else {
                continue;
            };
            pass.set_vertex_buffer(0, tile.vertices.slice(..));
            pass.set_index_buffer(tile.indices.slice(..), wgpu::IndexFormat::Uint32);
            for c in &tile.chunks {
                let rel = [
                    c.center[0] - w.eye[0],
                    c.center[1] - w.eye[1],
                    c.center[2] - w.eye[2],
                ];
                let ahead = rel[0] * w.look[0] + rel[1] * w.look[1] + rel[2] * w.look[2];
                let far = (rel[0] * rel[0] + rel[1] * rel[1]).sqrt();
                // (a chunk wholly behind the eye, or past the view distance)
                if c.count == 0 || ahead < -1.5 * CHUNK || far > w.far + CHUNK {
                    continue;
                }
                pass.set_bind_group(1, &c.group, &[]);
                pass.draw_indexed(c.start..c.start + c.count, 0, 0..1);
            }
        }
    }
}
