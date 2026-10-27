//! Plant impostors as the original makes them (work/map/render_rule.md section 3, notes in
//! work/map/pass8_notes.md).
//!
//! Every plant type that can be a card owns a 128 x 128 colour picture and a 128 x 128 normal
//! picture. While its cards are in view the pictures are drawn again every other frame - the
//! types take turns by their `ScheduledFrame` mask of `tune/MultiPropTuning.xml` - by an
//! orthographic camera that looks along the main camera's view direction (pitch included, world
//! up). By day the picture is the lit model, at night its colour times the baked occlusion, the
//! light then being worked out per pixel of the card with the normal picture. The card is one
//! quad per plant along the plant's own axis, widened across the camera's direction and tilted
//! to stand square to it: built in `city.wgsl` `vs_inst` from the instance (kind 4, uv1.y 104).
//!
//! The pictures here are separate textures, as in the original (no atlas). `MCLA_NO_IMPOSTORS=1`:
//! none of this - the cards baked at load (`bake_card`) are drawn as before.

use super::{PropBatch, PropModel};
use crate::render::{Globals, Gpu, GpuTexture, MatMode, Vertex};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Side of a picture in texels.
const TILE: u32 = 128;
/// The generation surface is multisampled and resolved, as the original's (4x).
const SAMPLES: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// The shadow impostor (render_rule.md 3d): an 8-bit silhouette of the plant seen along the
/// shadow pass's light direction, `TILE` texels a side, or `MASK_BIG` when the type's bounding
/// sphere is over 2 m.
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const MASK_BIG: u32 = 256;

fn mask_target(gpu: &Gpu, label: &str, format: wgpu::TextureFormat, samples: u32, size: u32) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: if samples > 1 { wgpu::TextureUsages::RENDER_ATTACHMENT } else { wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING },
        view_formats: &[],
    })
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TileUniform {
    view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    sun_col: [f32; 4],
    ambient: [f32; 4],
}

/// What `Props::load` hands over for a plant type with a card.
pub(super) struct TypeInfo {
    /// the plant's model
    pub model: usize,
    /// its baked card's model (what the runs name as their card)
    pub baked: usize,
    pub name: String,
    pub min: Vec3,
    pub max: Vec3,
    pub sphere: f32,
    pub radius: f32,
}

struct Type {
    model: usize,
    /// the model of its impostor card (one quad, `MatMode::Foliage`, the two pictures bound)
    card: usize,
    /// the model's box: width max(x, z), height, middle height
    width: f32,
    height: f32,
    centre_y: f32,
    /// `ScheduledFrame`: bit mask of the frames (of four) it is redrawn in
    sched: u32,
    color: Arc<GpuTexture>,
    normal: Arc<GpuTexture>,
    buf: wgpu::Buffer,
    bg: wgpu::BindGroup,
    /// the shadow impostor: the mask, the model of its card in the shadow pass (one quad, the
    /// mask bound as its texture, uv1.y = 105), the mask camera's uniform; `mask_valid` once drawn
    mask: Arc<GpuTexture>,
    mask_card: usize,
    mask_buf: wgpu::Buffer,
    mask_bg: wgpu::BindGroup,
    mask_valid: bool,
    /// the frame its cards were last drawn in
    stamp: AtomicU64,
    valid: bool,
    night: bool,
}

pub(super) struct Impostors {
    types: Vec<Type>,
    /// baked card model -> index into `types` (u32::MAX: none)
    by_baked: Vec<u32>,
    /// the leaves' program and the trunk's
    pipeline: wgpu::RenderPipeline,
    pipeline_trunk: wgpu::RenderPipeline,
    ms_color: wgpu::TextureView,
    ms_normal: wgpu::TextureView,
    ms_depth: wgpu::TextureView,
    /// the masks' programs (leaves / trunk) and their multisampled surfaces: [128, 256] colour, depth
    pipeline_mask: wgpu::RenderPipeline,
    pipeline_mask_trunk: wgpu::RenderPipeline,
    ms_mask: [wgpu::TextureView; 2],
    ms_mask_depth: [wgpu::TextureView; 2],
    /// group 2: the materials' sampler
    samp_bg: wgpu::BindGroup,
    /// the type whose mask is drawn next (one a frame, in turn), the hour of the last call, masks drawn so far
    mask_next: usize,
    mask_hour: f32,
    mask_total: u64,
    /// false with MCLA_NO_SHADOW_IMPOSTORS: the shadow pass keeps the baked cards
    mask_on: bool,
    frame: u64,
    /// pictures drawn so far, and in the last frame (for the log)
    total: u64,
    log_every: u64,
}

/// `ScheduledFrame` per type name (lower case) from `tune/MultiPropTuning.xml`.
fn schedule(data_dir: &str) -> Vec<(String, u32)> {
    let text = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/MultiPropTuning.xml")).unwrap_or_default();
    let mut out = Vec::new();
    for item in text.split("<Item").skip(1) {
        let name = item.split("<Name").nth(1).and_then(|t| t.split_once('>')).and_then(|t| t.1.split('<').next()).map(|n| n.trim().to_ascii_lowercase());
        let value = item.split("<ScheduledFrame").nth(1).and_then(|t| t.split('"').nth(1)).and_then(|v| v.trim().parse::<u32>().ok());
        if let (Some(n), Some(v)) = (name, value) {
            out.push((n, v));
        }
    }
    out
}

impl Impostors {
    /// The pictures, the generation pipeline and one card model per type (pushed onto `models`).
    /// None with `MCLA_NO_IMPOSTORS=1` or without types.
    pub(super) fn new(gpu: &mut Gpu, data_dir: &str, models: &mut Vec<PropModel>, infos: &[TypeInfo]) -> Option<Impostors> {
        if std::env::var("MCLA_NO_IMPOSTORS").map_or(false, |v| v != "0") || infos.is_empty() {
            return None;
        }
        let sched = schedule(data_dir);
        let tile_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("impostor tile layout"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
        });
        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("impostor tile"), source: wgpu::ShaderSource::Wgsl(include_str!("impostor.wgsl").into()) });
        // (the materials' sampler in a group of its own, made once: it left the texture group - render/mod.rs `sampler`)
        let samp_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("impostor sampler layout"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }],
        });
        let samp_bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("impostor sampler"), layout: &samp_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(&gpu.sampler) }] });
        let pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("impostor tile pl"), bind_group_layouts: &[&tile_layout, &gpu.tex_layout, &samp_layout], push_constant_ranges: &[] });
        let make = |entry: &str| gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("impostor tile"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_tile"), buffers: &[Vertex::LAYOUT], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry),
                targets: &[
                    Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                    Some(wgpu::ColorTargetState { format: NORMAL_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::Less, stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
            cache: None,
        });
        let (pipeline, pipeline_trunk) = (make("fs_leaf"), make("fs_trunk"));
        let make_mask = |entry: &str| gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("impostor shadow mask"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_tile"), buffers: &[Vertex::LAYOUT], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(entry), targets: &[Some(wgpu::ColorTargetState { format: MASK_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::Less, stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
            cache: None,
        });
        let (pipeline_mask, pipeline_mask_trunk) = (make_mask("fs_mask_leaf"), make_mask("fs_mask_trunk"));
        let ms_mask = [TILE, MASK_BIG].map(|s| mask_target(gpu, "impostor ms mask", MASK_FORMAT, SAMPLES, s).create_view(&Default::default()));
        let ms_mask_depth = [TILE, MASK_BIG].map(|s| mask_target(gpu, "impostor ms mask depth", wgpu::TextureFormat::Depth32Float, SAMPLES, s).create_view(&Default::default()));
        let target = |gpu: &Gpu, label: &str, format: wgpu::TextureFormat, samples: u32| -> wgpu::Texture {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: TILE, height: TILE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: if samples > 1 { wgpu::TextureUsages::RENDER_ATTACHMENT } else { wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING },
                view_formats: &[],
            })
        };
        let ms_color = target(gpu, "impostor ms colour", COLOR_FORMAT, SAMPLES).create_view(&Default::default());
        let ms_normal = target(gpu, "impostor ms normal", NORMAL_FORMAT, SAMPLES).create_view(&Default::default());
        let ms_depth = target(gpu, "impostor ms depth", wgpu::TextureFormat::Depth32Float, SAMPLES).create_view(&Default::default());
        let mut types = Vec::new();
        let mut by_baked = vec![u32::MAX; models.len()];
        for info in infos {
            let size = info.max - info.min;
            if !size.is_finite() || size.min_element() <= 0.01 {
                continue;
            }
            let (width, height) = (size.x.max(size.z), size.y);
            let centre_y = info.min.y + 0.5 * height;
            let picture = |gpu: &Gpu, label: &str, format: wgpu::TextureFormat| -> Arc<GpuTexture> {
                let texture = target(gpu, label, format, 1);
                let view = texture.create_view(&Default::default());
                Arc::new(GpuTexture { texture, view, width: TILE, height: TILE })
            };
            let color = picture(gpu, "impostor colour", COLOR_FORMAT);
            let normal = picture(gpu, "impostor normal", NORMAL_FORMAT);
            let buf = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("impostor tile"), contents: bytemuck::bytes_of(&TileUniform::zeroed()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
            let bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("impostor tile bg"), layout: &tile_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }] });
            // The card: one quad. A corner's `pos` is (side sign, top sign, 0), its `normal` the
            // type's (half width, CenterY, HeightY), uv1 = (sphere radius, 104): city.wgsl
            // `vs_inst` builds the corner from the instance with them (VSPropInstanceFoliage).
            // (the picture's edge texels are kept out of the sampler's wrap by half a texel)
            let (e0, e1) = (0.5 / TILE as f32, 1.0 - 0.5 / TILE as f32);
            let consts = [0.5 * width, centre_y, 0.5 * height];
            let verts: Vec<Vertex> = [(-1.0, 1.0, e0, e0), (1.0, 1.0, e1, e0), (1.0, -1.0, e1, e1), (-1.0, -1.0, e0, e1)]
                .iter()
                .map(|&(s, t, u, v)| Vertex { pos: [s, t, 0.0], normal: consts, uv0: [u, v], uv1: [info.sphere, 104.0], color: [255, 255, 255, 0], color1: [0, 0, 0, 255], uv2: [0.0, 0.0] })
                .collect();
            let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
            let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("impostor card vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
            let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("impostor card ib"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
            let (k0, k1) = (mcla_formats::hash::joaat(&format!("impostor:{}", info.name)), mcla_formats::hash::joaat(&format!("impostor normal:{}", info.name)));
            let bind = gpu.texture_bind_group(&color, &normal, k0, k1);
            models.push(PropModel { batches: vec![PropBatch { vb, ib, count: 6, bind, mode: MatMode::Foliage }], radius: info.radius, sphere: info.sphere, kind: 3 });
            let card_ix = models.len() - 1;
            // The shadow impostor: the mask and the quad the shadow pass draws for the type
            // (render/shadow.wgsl `vs_shadow_inst` builds it from the light direction, uv1.y = 105).
            let msize = if info.sphere > 2.0 { MASK_BIG } else { TILE };
            let mask = {
                let texture = mask_target(gpu, "impostor shadow mask", MASK_FORMAT, 1, msize);
                let view = texture.create_view(&Default::default());
                Arc::new(GpuTexture { texture, view, width: msize, height: msize })
            };
            let mask_buf = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("impostor mask tile"), contents: bytemuck::bytes_of(&TileUniform::zeroed()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
            let mask_bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("impostor mask tile bg"), layout: &tile_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: mask_buf.as_entire_binding() }] });
            let (m0, m1) = (0.5 / msize as f32, 1.0 - 0.5 / msize as f32);
            let mverts: Vec<Vertex> = [(-1.0, 1.0, m0, m0), (1.0, 1.0, m1, m0), (1.0, -1.0, m1, m1), (-1.0, -1.0, m0, m1)]
                .iter()
                .map(|&(s, t, u, v)| Vertex { pos: [s, t, 0.0], normal: consts, uv0: [u, v], uv1: [info.sphere, 105.0], color: [255, 255, 255, 0], color1: [0, 0, 0, 255], uv2: [0.0, 0.0] })
                .collect();
            let mvb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("impostor shadow card vb"), contents: bytemuck::cast_slice(&mverts), usage: wgpu::BufferUsages::VERTEX });
            let mib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("impostor shadow card ib"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
            let km = mcla_formats::hash::joaat(&format!("impostor shadow:{}", info.name));
            let mbind = gpu.texture_bind_group(&mask, &mask, km, km);
            models.push(PropModel { batches: vec![PropBatch { vb: mvb, ib: mib, count: 6, bind: mbind, mode: MatMode::Foliage }], radius: info.radius, sphere: info.sphere, kind: 3 });
            let mask_card = models.len() - 1;
            let lname = info.name.to_ascii_lowercase();
            let sched = sched.iter().find(|s| s.0 == lname).map_or(3, |s| s.1);
            if info.baked < by_baked.len() {
                by_baked[info.baked] = types.len() as u32;
            }
            log::debug!("impostor type {}: width {:.3} height {:.3} centre y {:.3} ScheduledFrame {}", info.name, width, height, centre_y, sched);
            types.push(Type { model: info.model, card: card_ix, width, height, centre_y, sched, color, normal, buf, bg, mask, mask_card, mask_buf, mask_bg, mask_valid: false, stamp: AtomicU64::new(0), valid: false, night: false });
        }
        if types.is_empty() {
            return None;
        }
        log::info!("impostors: {} plant types with a {} x {} picture ({} named in MultiPropTuning.xml), redrawn every other frame while their cards are in view", types.len(), TILE, TILE, sched.len());
        let mask_on = !std::env::var("MCLA_NO_SHADOW_IMPOSTORS").map_or(false, |v| v != "0");
        log::info!("impostors: shadow masks {} ({} of {} x {}, the rest {} x {}), one redrawn a frame", if mask_on { "on" } else { "off (MCLA_NO_SHADOW_IMPOSTORS)" }, types.iter().filter(|t| t.mask.width == MASK_BIG).count(), MASK_BIG, MASK_BIG, TILE, TILE);
        Some(Impostors { types, by_baked, pipeline, pipeline_trunk, ms_color, ms_normal, ms_depth, pipeline_mask, pipeline_mask_trunk, ms_mask, ms_mask_depth, samp_bg, mask_next: 0, mask_hour: f32::NAN, mask_total: 0, mask_on, frame: 2, total: 0, log_every: std::env::var("MCLA_IMPOSTOR_LOG").ok().and_then(|v| v.parse().ok()).unwrap_or(0) })
    }

    /// The model to draw in the scene pass where a run names the baked card `baked`: the type's
    /// impostor card (and the type is noted as drawn this frame - F-48007's stamp).
    pub(super) fn card(&self, baked: usize) -> usize {
        match self.by_baked.get(baked) {
            Some(&t) if t != u32::MAX => {
                let ty = &self.types[t as usize];
                ty.stamp.store(self.frame, Ordering::Relaxed);
                ty.card
            }
            _ => baked,
        }
    }

    /// The model to draw in the shadow pass where a run names the baked card `baked`: the type's
    /// shadow card once its mask has been drawn (F-48104 with the shadow flag).
    pub(super) fn shadow_card(&self, baked: usize) -> usize {
        match self.by_baked.get(baked) {
            Some(&t) if t != u32::MAX && self.mask_on && self.types[t as usize].mask_valid => self.types[t as usize].mask_card,
            _ => baked,
        }
    }

    /// Before the shadow pass (F-80686 / F-61167): the silhouette masks along the
    /// pass's light direction - one type a frame in turn, all of them when the time of day
    /// moved by 1 / 120 h or more since the last call (and at the first call). The tile camera
    /// of 3b with the axis -L, the whole target, no wind; the model as it is (the original
    /// takes LOD 1 where there is one).
    pub(super) fn update_shadow(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, light_vp: Mat4, hour: f32, models: &[PropModel]) {
        let n = self.types.len();
        if !self.mask_on || n == 0 {
            return;
        }
        // (the direction clip depth grows along is the light's way L; the axis is -L. The same
        // three numbers, with the same sign, are what shadow.wgsl builds the quad from.)
        let row = Vec3::new(light_vp.x_axis.z, light_vp.y_axis.z, light_vp.z_axis.z);
        if !row.is_finite() || row.length_squared() < 1e-12 {
            return;
        }
        let c = -row.normalize();
        let moved = (hour - self.mask_hour).abs();
        let all = !(moved.min(24.0 - moved) < 1.0 / 120.0);
        self.mask_hour = hour;
        let (first, count) = if all { (0, n) } else { (self.mask_next % n, 1) };
        if !all {
            self.mask_next = (self.mask_next + 1) % n;
        }
        for k in 0..count {
            let ty = &mut self.types[(first + k) % n];
            let Some(model) = models.get(ty.model) else { continue };
            let u = TileUniform { view_proj: Self::tile_camera(c, ty.width, ty.height, ty.centre_y).to_cols_array_2d(), sun_dir: [0.0; 4], sun_col: [0.0; 4], ambient: [0.0; 4] };
            gpu.queue.write_buffer(&ty.mask_buf, 0, bytemuck::bytes_of(&u));
            let big = (ty.mask.width == MASK_BIG) as usize;
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("impostor shadow mask"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.ms_mask[big], resolve_target: Some(&ty.mask.view), ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.ms_mask_depth[big], depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &ty.mask_bg, &[]);
            pass.set_bind_group(2, &self.samp_bg, &[]);
            for b in &model.batches {
                pass.set_pipeline(if b.mode as u32 == MatMode::Foliage as u32 { &self.pipeline_mask } else { &self.pipeline_mask_trunk });
                pass.set_bind_group(1, b.bind.as_ref(), &[]);
                pass.set_vertex_buffer(0, b.vb.slice(..));
                pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..b.count, 0, 0..1);
            }
            drop(pass);
            ty.mask_valid = true;
            self.mask_total += 1;
        }
        if self.log_every > 0 && self.frame % self.log_every == 0 {
            log::info!("impostors: shadow masks drawn {} ({} so far), axis {:.3} {:.3} {:.3}, hour {:.4}", count, self.mask_total, c.x, c.y, c.z, hour);
        }
    }

    /// The tile camera of F-73098 for the main camera's back vector `c`: world -> clip of
    /// an orthographic view along -c, world up, the plant filling the width. The side and up
    /// vectors and the projected height are `tile_vectors`' (the original's arithmetic).
    fn tile_camera(c: Vec3, width: f32, height: f32, centre_y: f32) -> Mat4 {
        let (side, up, hproj) = tile_vectors([c.x, c.y, c.z, 0.0], width, height);
        let (side, up) = (Vec3::new(side[0], side[1], side[2]), Vec3::new(up[0], up[1], up[2]));
        let hproj = hproj.max(1e-3);
        let centre = Vec3::new(0.0, centre_y, 0.0);
        let (kx, ky) = (2.0 / width, 2.0 / hproj);
        // clip.x = 2/W dot(p, side); clip.y = 2/Hproj dot(p - centre, U);
        // clip.z = (dot(centre - p, c) + 150) / 200  (eye 50 m back, near -100, far +100)
        Mat4::from_cols(
            Vec4::new(side.x * kx, up.x * ky, -c.x / 200.0, 0.0),
            Vec4::new(side.y * kx, up.y * ky, -c.y / 200.0, 0.0),
            Vec4::new(side.z * kx, up.z * ky, -c.z / 200.0, 0.0),
            Vec4::new(0.0, -centre.dot(up) * ky, (centre.dot(c) + 150.0) / 200.0, 1.0),
        )
    }

    /// Once a frame, before the scene pass (F-65228): draw the pictures of the types that
    /// are due - those whose cards were drawn in the last frame and whose `ScheduledFrame` has
    /// this frame's bits (0 and 2 on even frames, 1 and 3 on odd ones), or whose picture is of
    /// the other half of the day, or was never drawn.
    pub(super) fn update(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, g: &Globals, models: &[PropModel]) {
        let frame = self.frame;
        self.frame += 1;
        // the main camera's back vector: minus the direction clip w grows along
        let fwd = Vec3::new(g.view_proj[0][3], g.view_proj[1][3], g.view_proj[2][3]);
        if !fwd.is_finite() || fwd.length_squared() < 1e-12 {
            return;
        }
        let c = -fwd.normalize();
        let night = g.tone[3] > 0.5;
        let mask = (1u32 << (frame & 1)) * 5;
        let mut drawn = 0u32;
        for ty in self.types.iter_mut() {
            if ty.stamp.load(Ordering::Relaxed) + 1 < frame {
                continue;
            }
            if ty.valid && ty.night == night && ty.sched & mask == 0 {
                continue;
            }
            let Some(model) = models.get(ty.model) else { continue };
            let u = TileUniform {
                view_proj: Self::tile_camera(c, ty.width, ty.height, ty.centre_y).to_cols_array_2d(),
                // (w: 1 = night; + 2 under MCLA_DEBUG_VIEW=112: the picture as it was drawn before
                // 2026-10-14 - sRGB-decoded texture, no 8-bit steps; see impostor.wgsl)
                sun_dir: [g.sun_dir[0], g.sun_dir[1], g.sun_dir[2], (if night { 1.0 } else { 0.0 }) + (if (g.params[3] - 112.0).abs() < 0.5 { 2.0 } else { 0.0 })],
                sun_col: [g.sun_color[0] * g.sun_dir[3], g.sun_color[1] * g.sun_dir[3], g.sun_color[2] * g.sun_dir[3], g.grade[3]],
                ambient: g.ambient_sky,
            };
            gpu.queue.write_buffer(&ty.buf, 0, bytemuck::bytes_of(&u));
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("impostor tile"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment { view: &self.ms_color, resolve_target: Some(&ty.color.view), ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard } }),
                    Some(wgpu::RenderPassColorAttachment { view: &self.ms_normal, resolve_target: Some(&ty.normal.view), ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.5, g: 1.0, b: 0.5, a: 1.0 }), store: wgpu::StoreOp::Discard } }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.ms_depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &ty.bg, &[]);
            pass.set_bind_group(2, &self.samp_bg, &[]);
            for b in &model.batches {
                pass.set_pipeline(if b.mode as u32 == MatMode::Foliage as u32 { &self.pipeline } else { &self.pipeline_trunk });
                pass.set_bind_group(1, b.bind.as_ref(), &[]);
                pass.set_vertex_buffer(0, b.vb.slice(..));
                pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..b.count, 0, 0..1);
            }
            drop(pass);
            ty.valid = true;
            ty.night = night;
            drawn += 1;
        }
        self.total += drawn as u64;
        if self.log_every > 0 && frame % self.log_every == 0 {
            let live = self.types.iter().filter(|t| t.stamp.load(Ordering::Relaxed) + 1 >= frame).count();
            log::info!("impostors: frame {} drew {} pictures ({} types in view, {} pictures so far), back vector {:.3} {:.3} {:.3}", frame, drawn, live, self.total, c.x, c.y, c.z);
        }
    }
}

/// One lane operation of the original's vector code: the cross product it builds from two shuffles, two multiplies and a
/// subtraction (each rounded to single, no fused multiply-add), the sign bit flipped last. `a` and `b` as given are `a x b`
/// negated, i.e. `b x a`.
fn cross_flipped(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (a9, a7) = ([a[1], a[2], a[0], a[3]], [a[2], a[0], a[1], a[3]]);
    let (b9, b7) = ([b[1], b[2], b[0], b[3]], [b[2], b[0], b[1], b[3]]);
    std::array::from_fn(|i| f32::from_bits((a9[i] * b7[i] - a7[i] * b9[i]).to_bits() ^ 0x8000_0000))
}

/// The tile camera's side vector, up vector and projected height as F-73098 computes them for the back vector `c` (w lane
/// included) and a plant of width `w` (the larger of the box's x and z extent) and height `h`: the side direction
/// `(c.z, 0, -c.x)` made unit with the hardware dot product (z^2 + (y^2 + x^2)), `1 / sqrt` and one Newton step, all single
/// precision (0 where the length is 0), the up vector `c x side`, and `(1 - up.y^2) w + h up.y` as a fused multiply-add
/// of the doubles rounded to single. The vectors written into the camera object are these times half the width.
pub fn tile_vectors(c: [f32; 4], w: f32, h: f32) -> ([f32; 4], [f32; 4], f32) {
    let mc = c.map(|v| 0.0 - v);
    let raw = cross_flipped([0.0, 1.0, 0.0, 0.0], mc);
    let l2 = raw[2] * raw[2] + (raw[1] * raw[1] + raw[0] * raw[0]);
    let r0 = 1.0 / l2.sqrt();
    let r = if r0 == f32::INFINITY { 0.0 } else { r0 };
    let (r2, half_r) = (r * r, 0.5 * r);
    let step = f32::from_bits((l2 * r2 - 1.0).to_bits() ^ 0x8000_0000);
    let k = step * half_r + r;
    let side = raw.map(|v| v * k);
    let up = cross_flipped(mc, side);
    let one_minus = -((up[1] as f64) * (up[1] as f64) - 1.0) as f32;
    let hproj = ((one_minus as f64).mul_add(w as f64, (h * up[1]) as f64)) as f32;
    (side, up, hproj)
}

/// `tile_vectors` for the plant's box (`mn`, `mx`, the corners the function reads): the width is the larger of the x and z
/// extents (the x one when it is not smaller), the height the y extent. Returns the camera object's two vectors (side and up
/// times half the width, the up vector's y lane times 0), and the six numbers of the orthographic projection call (left,
/// right, bottom, top, near, far).
pub fn tile_frame(c: [f32; 4], mn: [f32; 4], mx: [f32; 4]) -> ([f32; 4], [f32; 3], [f32; 6]) {
    let ext = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
    let w = if ext[0] - ext[2] >= 0.0 { ext[0] } else { ext[2] };
    let h = mx[1] - mn[1];
    let w2 = w * 0.5;
    let (side, up, hproj) = tile_vectors(c, w, h);
    (side.map(|v| v * w2), [up[0] * w2, up[1] * 0.0, up[2] * w2], [w * -0.5, w2, hproj * -0.5, hproj * 0.5, -100.0, 100.0])
}

#[cfg(test)]
mod tile_camera_tests {
    use super::*;

    /// F-73098 run offline from the original's code (`tools/tile_camera_reference.py`): the recorded call (title screen) and
    /// back vectors of an open-city recording with random boxes, random unit vectors and the edge cases (straight up / down,
    /// zero, tiny horizontal part, not unit). The back vector, box corners and result words are hex float bits.
    #[test]
    fn tile_frame_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let d = |s: &str| f64::from_bits(u64::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for line in include_str!("testdata/tile_camera_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let t: Vec<&str> = i.split(' ').collect();
            let v4 = |k: usize| [f(t[k]), f(t[k + 1]), f(t[k + 2]), f(t[k + 3])];
            let r5: i32 = t[12].parse().unwrap();
            let (c, mn, mx) = (v4(0), v4(4), v4(8));
            let (vecs, stubs) = o.split_once(" | ").unwrap();
            let w: Vec<&str> = vecs.split(' ').collect();
            let (side, up, ortho) = tile_frame(c, mn, mx);
            let bits = |x: f32| format!("{:08x}", x.to_bits());
            let got: Vec<String> = side.iter().chain(up.iter()).map(|x| bits(*x)).collect();
            assert_eq!(got, w.iter().map(|x| x.to_string()).collect::<Vec<_>>(), "{line}");
            for st in stubs.split(" ; ") {
                let p: Vec<&str> = st.split(' ').collect();
                let want: Vec<f64> = p[1..].iter().map(|x| d(x)).collect();
                let got: Vec<f64> = match p[0] {
                    "ortho" => ortho.iter().map(|x| *x as f64).collect(),
                    "ortho2" if r5 < 0 => vec![0.0, 0.0, 1.0, 1.0, 0.0, 1.0],
                    "ortho2" => vec![(r5 as f32 * 1.0) as f64, 0.0, 1.0, 1.0, 0.0, 1.0],
                    _ => continue,
                };
                let b = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
                assert_eq!(b(&got), b(&want), "{} {line}", p[0]);
            }
            n += 1;
        }
        assert!(n > 100);
    }
}
