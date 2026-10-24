//! The minimap's 220 x 220 target "__mini_map" drawn on the GPU, by the CPU target's rules (hudmap.rs `Target`,
//! `render_minimap`), and handed to the HUD movie in the egui texture the CPU's picture went into.
//!
//! The rules, as `Target` draws them: 4 samples a pixel (D3D's standard 4x positions, wgpu's 4x pattern) in float;
//! every sample shaded on its own (`@interpolate(perspective, sample)`: the texture read at the sample, perspective
//! correct); the texture trilinear from its box-filtered mips at one level of detail a triangle (`Img::sample`, done
//! in the shader with `textureLoad`); the ripple's alpha in place of the texture; colour blended by source alpha,
//! alpha by max; then the mask per sample, alpha - (1 - mask red) and not below 0 (mode 0); then the rim dots;
//! the samples averaged, rounded to 8 bits and premultiplied plainly (`Target::resolve`, `flash_ui::plain_image`).
//! The mask cannot blend in a float target (nothing clamps the subtraction), so the rim dots' alpha goes to a second
//! attachment and the last pass (one fragment a pixel, reading the four samples) applies the mask and takes the max.
//! 16-bit float samples instead of the CPU's 32-bit: within a level of the CPU's picture (docs/PERFORMANCE.md,
//! 2026-10-23). `MCLA_MINIMAP_CPU=1`: the CPU's target (the overhead map stays on it).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::hudmap::{Assets, Frame, Img, Queue, Ripple, Tune, ROAD_COLOURS};

pub fn enabled() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_MINIMAP_CPU").map_or(true, |v| v.trim() != "1"))
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct V {
    clip: [f32; 4],
    uv: [f32; 2],
    lod: f32,
    /// 0 flat, 1 textured, 2 the ripple
    mode: u32,
    colour: [f32; 4],
    /// the ripple: t x speed, frequency, exponent, amplitude
    rp: [f32; 4],
}

struct Cmd {
    range: Range<u32>,
    tex: Option<Arc<Img>>,
    rim: bool,
}

/// The overhead map's roads: every road of the city, drawn first; its camera stands still once the movie's easing
/// settles, so the triangles are kept on the GPU for the view they were made for (`key`) and made again only when it
/// moves (the CPU target's `roads` cache).
struct Roads {
    key: [u32; 16],
    /// None: the GPU already holds them
    verts: Option<Vec<V>>,
}

pub struct Job {
    size: [u32; 2],
    roads: Option<Roads>,
    verts: Vec<V>,
    cmds: Vec<Cmd>,
    clear: [f32; 3],
    mask: Option<Arc<Img>>,
    /// the egui texture the picture goes into
    tex: egui::TextureId,
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());

struct Builder {
    w: f32,
    h: f32,
    verts: Vec<V>,
    cmds: Vec<Cmd>,
}

impl Builder {
    /// One triangle as `Target::tri` takes it: `v` = (pixel x, y, 1/w).
    fn tri(&mut self, v: [(f32, f32, f32); 3], uv: [Vec2; 3], tex: Option<&Arc<Img>>, colour: [f32; 4], ripple: Option<&Ripple>, rim: bool) {
        let (nw, nh) = (self.w, self.h);
        let (x0, y0, x1, y1) = (v[0].0.min(v[1].0).min(v[2].0), v[0].1.min(v[1].1).min(v[2].1), v[0].0.max(v[1].0).max(v[2].0), v[0].1.max(v[1].1).max(v[2].1));
        if x1 < 0.0 || y1 < 0.0 || x0 >= nw || y0 >= nh {
            return;
        }
        let area = (v[1].0 - v[0].0) * (v[2].1 - v[0].1) - (v[2].0 - v[0].0) * (v[1].1 - v[0].1);
        if area.abs() < 1e-9 {
            return;
        }
        let tex = if ripple.is_some() { None } else { tex };
        let lod = tex.map_or(0.0, |t| {
            let ua = ((uv[1].x - uv[0].x) * (uv[2].y - uv[0].y) - (uv[2].x - uv[0].x) * (uv[1].y - uv[0].y)).abs() * (t.w * t.h) as f32;
            if ua > 0.0 {
                0.5 * (ua / area.abs()).log2()
            } else {
                0.0
            }
        });
        let mode = if ripple.is_some() { 2 } else if tex.is_some() { 1 } else { 0 };
        let rp = ripple.map_or([0.0; 4], |r| [r.t * r.speed, r.freq, r.falloff, r.amplitude]);
        let (hw, hh) = (nw * 0.5, nh * 0.5);
        let s = self.verts.len() as u32;
        for k in 0..3 {
            let w = 1.0 / v[k].2;
            self.verts.push(V { clip: [(v[k].0 / hw - 1.0) * w, (1.0 - v[k].1 / hh) * w, 0.5 * w, w], uv: uv[k].to_array(), lod, mode, colour, rp });
        }
        let ptr = |t: Option<&Arc<Img>>| t.map(|t| Arc::as_ptr(t) as usize);
        match self.cmds.last_mut() {
            Some(c) if c.range.end == s && c.rim == rim && ptr(c.tex.as_ref()) == ptr(tex) => c.range.end = s + 3,
            _ => self.cmds.push(Cmd { range: s..s + 3, tex: tex.cloned(), rim }),
        }
    }

    /// `Target::mesh`
    fn mesh(&mut self, mesh: &crate::hudmap::IconMesh, world: Mat4, view_proj: Mat4, colour: [f32; 4], ripple: Option<&Ripple>) {
        let (hw, hh) = (self.w * 0.5, self.h * 0.5);
        let wvp = view_proj * world;
        for part in &mesh.parts {
            let pts: Vec<Option<(f32, f32, f32)>> = part
                .pos
                .iter()
                .map(|p| {
                    let c = wvp * p.extend(1.0);
                    (c.w > 1e-4).then(|| (hw + hw * c.x / c.w, hh - hh * c.y / c.w, 1.0 / c.w))
                })
                .collect();
            for t in part.idx.chunks_exact(3) {
                let (Some(a), Some(b), Some(c)) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]) else { continue };
                let uv = |i: u32| part.uv.get(i as usize).copied().unwrap_or(Vec2::ZERO);
                self.tri([a, b, c], [uv(t[0]), uv(t[1]), uv(t[2])], part.tex.as_ref(), colour, ripple, false);
            }
        }
    }

    /// `Target::ground`
    fn ground(&mut self, tris: impl Iterator<Item = [Vec3; 3]>, view_proj: Mat4, colour: [f32; 4]) {
        let (hw, hh) = (self.w * 0.5, self.h * 0.5);
        for t in tris {
            let mut v = [(0.0, 0.0, 0.0); 3];
            let mut ok = true;
            for k in 0..3 {
                let c: Vec4 = view_proj * t[k].extend(1.0);
                if c.w <= 1e-4 {
                    ok = false;
                    break;
                }
                v[k] = (hw + hw * c.x / c.w, hh - hh * c.y / c.w, 1.0 / c.w);
            }
            if ok {
                self.tri(v, [Vec2::ZERO; 3], None, colour, None, false);
            }
        }
    }
}

/// `render_minimap` as a job for the GPU (the minimap, mode 0, 220 x 220; the overhead map, mode 2, 749 x 430), into the
/// egui texture `tex`.
#[allow(clippy::too_many_arguments)]
pub fn queue(w: usize, h: usize, assets: &mut Assets, tune: &Tune, roads: Option<&crate::mapmesh::RoadMap>, f: &Frame, q: &Queue, clear_rgb: [f32; 3], tex: egui::TextureId) {
    // (room for as many vertices as the last picture had: the list is not grown step by step every frame)
    static LAST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut b = Builder { w: w as f32, h: h as f32, verts: Vec::with_capacity(LAST.load(std::sync::atomic::Ordering::Relaxed)), cmds: Vec::new() };
    let mut road_job = None;
    // (no road is drawn in a frame without an icon - sub_822371E0)
    if !q.draws.is_empty() || !q.rim.is_empty() {
        if let (Some(roads), 2) = (roads, f.mode) {
            // (every triangle of the city: the view takes it all in)
            let key = f.view_proj.to_cols_array().map(f32::to_bits);
            let held = STATE.lock().ok().map_or(false, |st| st.as_ref().map_or(false, |st| st.roads.as_ref().map_or(false, |r| r.0 == key)))
                || JOBS.lock().ok().map_or(false, |j| j.iter().any(|j| j.roads.as_ref().map_or(false, |r| r.key == key && r.verts.is_some())));
            let verts = (!held).then(|| {
                let mut rb = Builder { w: w as f32, h: h as f32, verts: Vec::new(), cmds: Vec::new() };
                for (layer, (ckey, d)) in roads.layers.iter().zip(ROAD_COLOURS) {
                    let colour = tune.c(ckey, d);
                    if colour[3] <= 0.0 {
                        continue;
                    }
                    rb.ground(
                        layer.tris.iter().enumerate().map(|(i, t)| {
                            let y = layer.ys.get(i).copied().unwrap_or([0.0; 3]);
                            [Vec3::new(t[0].x, y[0], t[0].y), Vec3::new(t[1].x, y[1], t[1].y), Vec3::new(t[2].x, y[2], t[2].y)]
                        }),
                        f.view_proj,
                        colour,
                    );
                }
                rb.verts
            });
            road_job = Some(Roads { key, verts });
        } else if let Some(roads) = roads {
            let reach = f.cam_pos.y * 0.33 + 160.0;
            let centre = Vec2::new(f.cam_pos.x, f.cam_pos.z);
            let mut ids = Vec::new();
            for (layer, (key, d)) in roads.layers.iter().zip(ROAD_COLOURS) {
                let colour = tune.c(key, d);
                if colour[3] <= 0.0 {
                    continue;
                }
                layer.near(centre, reach, &mut ids);
                b.ground(
                    ids.iter().map(|&i| {
                        let t = layer.tris[i as usize];
                        let y = layer.ys.get(i as usize).copied().unwrap_or([0.0; 3]);
                        [Vec3::new(t[0].x, y[0], t[0].y), Vec3::new(t[1].x, y[1], t[1].y), Vec3::new(t[2].x, y[2], t[2].y)]
                    }),
                    f.view_proj,
                    colour,
                );
            }
        }
        for d in &q.draws {
            if let Some(m) = assets.mesh(d.mesh) {
                let ripple = (d.mesh == "target_ripple").then(|| Ripple::new(tune, f.tg, (d.m.w_axis.truncate() - f.cam_pos).length()));
                b.mesh(&m, d.m, f.view_proj, d.colour, ripple.as_ref());
            }
        }
    }
    // (`Target::rim`: after the mask - its alpha goes to the second attachment)
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    for sel in [false, true] {
        for r in q.rim.iter().rev().filter(|r| r.selected == sel) {
            let name = match r.kind {
                2 => "icon_flag",
                10 => "icon_flagBase",
                _ => "checkpoint_sphere",
            };
            let Some(mesh) = assets.mesh(name) else { continue };
            let (sn, cs) = r.angle.sin_cos();
            let (ox, oz) = (90.0 * sn, 90.0 * cs);
            let a = if r.selected { ((8.0 * f.tg).sin() + 1.0) * 0.5 } else { 1.0 };
            let colour = [r.colour[0], r.colour[1], r.colour[2], a];
            for part in &mesh.parts {
                let pts: Vec<(f32, f32, f32)> = part.pos.iter().map(|p| (cx + ox + 6.0 * p.x, cy - oz + 6.0 * p.z, 1.0)).collect();
                for t in part.idx.chunks_exact(3) {
                    let uv = |i: u32| part.uv.get(i as usize).copied().unwrap_or(Vec2::ZERO);
                    b.tri([pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]], [uv(t[0]), uv(t[1]), uv(t[2])], part.tex.as_ref(), colour, None, true);
                }
            }
        }
    }
    let mask = if f.mode == 0 { assets.mask0_arc() } else { None };
    if let Ok(mut j) = JOBS.lock() {
        // (one picture a texture and frame: a second call replaces the first - keeping roads it brought)
        if let Some(i) = j.iter().position(|x| x.tex == tex) {
            let old = j.remove(i);
            if let (Some(r), Some(o)) = (road_job.as_mut(), old.roads) {
                if r.verts.is_none() && o.key == r.key {
                    r.verts = o.verts;
                }
            }
        }
        LAST.store(b.verts.len(), std::sync::atomic::Ordering::Relaxed);
        j.push(Job { size: [w as u32, h as u32], roads: road_job, verts: b.verts, cmds: b.cmds, clear: clear_rgb, mask, tex });
    }
}

const ACC: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const RIM: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
const OUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Targets {
    size: [u32; 2],
    acc: wgpu::TextureView,
    rim: wgpu::TextureView,
    out: wgpu::Texture,
    out_view: wgpu::TextureView,
}

struct State {
    icons: wgpu::RenderPipeline,
    rims: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    t_layout: wgpu::BindGroupLayout,
    r_layout: wgpu::BindGroupLayout,
    white: wgpu::BindGroup,
    textures: HashMap<usize, (Arc<Img>, wgpu::BindGroup)>,
    /// by size (the minimap's and the overhead map's)
    targets: HashMap<[u32; 2], Targets>,
    /// the mask pictures (by their `Arc`, kept; 0 = none, white) at level 0
    masks: HashMap<usize, (Option<Arc<Img>>, wgpu::TextureView)>,
    /// the overhead map's roads for the view `key` (`Roads`)
    roads: Option<([u32; 16], Option<wgpu::Buffer>, u32)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// A picture's mip chain as a float texture (read with `textureLoad`).
fn texture(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, img: &Img) -> wgpu::BindGroup {
    let lv = img.levels();
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("minimap picture"),
        size: wgpu::Extent3d { width: lv[0].0 as u32, height: lv[0].1 as u32, depth_or_array_layers: 1 },
        mip_level_count: lv.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (l, (w, h, px)) in lv.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: l as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(px),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(*w as u32 * 16), rows_per_image: Some(*h as u32) },
            wgpu::Extent3d { width: *w as u32, height: *h as u32, depth_or_array_layers: 1 },
        );
    }
    let view = tex.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("minimap picture"), layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }] })
}

impl State {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> State {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("minimap"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let tex_entry = |binding, multisampled| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled }, count: None };
        let t_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("minimap picture"), entries: &[tex_entry(0, false)] });
        let r_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("minimap resolve"), entries: &[tex_entry(0, true), tex_entry(1, true), tex_entry(2, false)] });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("minimap"), bind_group_layouts: &[&t_layout], push_constant_ranges: &[] });
        let over = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        let max = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Max };
        let make = |rim: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("minimap"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<V>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2, 2 => Float32, 3 => Uint32, 4 => Float32x4, 5 => Float32x4] }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::ColorTargetState { format: ACC, blend: Some(wgpu::BlendState { color: over, alpha: max }), write_mask: if rim { wgpu::ColorWrites::COLOR } else { wgpu::ColorWrites::ALL } }),
                        Some(wgpu::ColorTargetState { format: RIM, blend: Some(wgpu::BlendState { color: max, alpha: max }), write_mask: if rim { wgpu::ColorWrites::RED } else { wgpu::ColorWrites::empty() } }),
                    ],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState { count: 4, mask: !0, alpha_to_coverage_enabled: false },
                multiview: None,
                cache: None,
            })
        };
        let icons = make(false);
        let rims = make(true);
        let rl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("minimap resolve"), bind_group_layouts: &[&r_layout], push_constant_ranges: &[] });
        let resolve = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("minimap resolve"),
            layout: Some(&rl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_resolve"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: OUT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let white = texture(device, queue, &t_layout, &Img::from_rgba8(1, 1, &[255; 4]));
        State { icons, rims, resolve, t_layout, r_layout, white, textures: HashMap::new(), targets: HashMap::new(), masks: HashMap::new(), roads: None }
    }

    fn bind(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, img: &Arc<Img>) {
        let k = Arc::as_ptr(img) as usize;
        if !self.textures.contains_key(&k) {
            let bg = texture(device, queue, &self.t_layout, img);
            self.textures.insert(k, (img.clone(), bg));
        }
    }
}

/// Make the pipelines now (at start-up, as `flashpic::init`).
pub fn init(device: &wgpu::Device, queue: &wgpu::Queue) {
    if let Ok(mut st) = STATE.lock() {
        st.get_or_insert_with(|| State::new(device, queue));
    }
}

/// Draw this frame's map pictures (those queued) and copy each into its egui texture in each of the UI's renderers.
pub fn render(device: &wgpu::Device, queue: &wgpu::Queue, renderers: [&egui_wgpu::Renderer; 3]) {
    let jobs = std::mem::take(&mut *JOBS.lock().unwrap());
    if jobs.is_empty() {
        return;
    }
    let Ok(mut st) = STATE.lock() else { return };
    let st = st.get_or_insert_with(|| State::new(device, queue));
    for job in jobs {
        render_job(device, queue, renderers, st, job);
    }
}

fn render_job(device: &wgpu::Device, queue: &wgpu::Queue, renderers: [&egui_wgpu::Renderer; 3], st: &mut State, job: Job) {
    use wgpu::util::DeviceExt;
    for c in &job.cmds {
        if let Some(t) = &c.tex {
            st.bind(device, queue, t);
        }
    }
    // (the mask picture's level 0, as `Target::mask` reads it; none: white, nothing taken away)
    let mk = job.mask.as_ref().map_or(0, |m| Arc::as_ptr(m) as usize);
    if !st.masks.contains_key(&mk) {
        let white = Img::from_rgba8(1, 1, &[255; 4]);
        let lv = job.mask.as_deref().unwrap_or(&white).levels();
        let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("minimap mask"), size: wgpu::Extent3d { width: lv[0].0 as u32, height: lv[0].1 as u32, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba32Float, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&lv[0].2),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lv[0].0 as u32 * 16), rows_per_image: Some(lv[0].1 as u32) },
            wgpu::Extent3d { width: lv[0].0 as u32, height: lv[0].1 as u32, depth_or_array_layers: 1 },
        );
        st.masks.insert(mk, (job.mask.clone(), tex.create_view(&Default::default())));
    }
    if let Some(r) = &job.roads {
        if let Some(v) = &r.verts {
            let b = (!v.is_empty()).then(|| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("overhead map roads"), contents: bytemuck::cast_slice(v), usage: wgpu::BufferUsages::VERTEX }));
            st.roads = Some((r.key, b, v.len() as u32));
        }
    }
    if !st.targets.contains_key(&job.size) {
        let ext = wgpu::Extent3d { width: job.size[0], height: job.size[1], depth_or_array_layers: 1 };
        let make = |label, format, samples, usage| device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: ext, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
        let both = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let out = make("__mini_map", OUT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        st.targets.insert(job.size, Targets {
            size: job.size,
            acc: make("__mini_map samples", ACC, 4, both).create_view(&Default::default()),
            rim: make("__mini_map rim", RIM, 4, both).create_view(&Default::default()),
            out_view: out.create_view(&Default::default()),
            out,
        });
    }
    let t = &st.targets[&job.size];
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("minimap") });
    {
        let vb = (!job.verts.is_empty()).then(|| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("minimap"), contents: bytemuck::cast_slice(&job.verts), usage: wgpu::BufferUsages::VERTEX }));
        let c = job.clear;
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("__mini_map"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment { view: &t.acc, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 0.0 }), store: wgpu::StoreOp::Store } }),
                Some(wgpu::RenderPassColorAttachment { view: &t.rim, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } }),
            ],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        if let (Some(r), Some((key, Some(b), n))) = (&job.roads, &st.roads) {
            if r.key == *key {
                pass.set_pipeline(&st.icons);
                pass.set_bind_group(0, &st.white, &[]);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..*n, 0..1);
            }
        }
        if let Some(vb) = vb.as_ref() {
            pass.set_vertex_buffer(0, vb.slice(..));
            for c in &job.cmds {
                pass.set_pipeline(if c.rim { &st.rims } else { &st.icons });
                pass.set_bind_group(0, c.tex.as_ref().map_or(&st.white, |t| &st.textures[&(Arc::as_ptr(t) as usize)].1), &[]);
                pass.draw(c.range.clone(), 0..1);
            }
        }
    }
    {
        let mask_view = &st.masks[&mk].1;
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("minimap resolve"),
            layout: &st.r_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&t.acc) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&t.rim) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(mask_view) },
            ],
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("__mini_map resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.out_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&st.resolve);
        pass.set_bind_group(0, &bg, &[]);
        pass.draw(0..3, 0..1);
    }
    let ext = wgpu::Extent3d { width: job.size[0], height: job.size[1], depth_or_array_layers: 1 };
    for r in renderers {
        if let Some(dst) = r.texture(&job.tex).and_then(|x| x.texture.as_ref()) {
            if dst.width() == job.size[0] && dst.height() == job.size[1] {
                enc.copy_texture_to_texture(t.out.as_image_copy(), dst.as_image_copy(), ext);
            }
        }
    }
    queue.submit([enc.finish()]);
    if let Ok(dir) = std::env::var("MCLA_SELF_DUMP") {
        crate::flashpic::dump(device, queue, &t.out, job.size, &format!("{dir}/{}.png", if job.size[0] == job.size[1] { "minimap" } else { "overhead" }));
    }
}

const SHADER: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(perspective, sample) uv: vec2<f32>,
    @location(1) @interpolate(flat) lod: f32,
    @location(2) @interpolate(flat) mode: u32,
    @location(3) @interpolate(flat) colour: vec4<f32>,
    @location(4) @interpolate(flat) rp: vec4<f32>,
};
@group(0) @binding(0) var pic: texture_2d<f32>;

@vertex fn vs_main(@location(0) clip: vec4<f32>, @location(1) uv: vec2<f32>, @location(2) lod: f32, @location(3) mode: u32, @location(4) colour: vec4<f32>, @location(5) rp: vec4<f32>) -> VOut {
    var o: VOut;
    o.pos = clip;
    o.uv = uv;
    o.lod = lod;
    o.mode = mode;
    o.colour = colour;
    o.rp = rp;
    return o;
}

// (Img::bilinear: texel centres at (i + 0.5) / size, clamped at the edges)
fn bilinear(l: i32, uv: vec2<f32>) -> vec4<f32> {
    let d = vec2<i32>(textureDimensions(pic, l));
    let x = clamp(uv * vec2<f32>(d) - 0.5, vec2<f32>(0.0), vec2<f32>(d - 1));
    let i0 = vec2<i32>(floor(x));
    let i1 = min(i0 + 1, d - 1);
    let f = x - vec2<f32>(i0);
    let a = textureLoad(pic, i0, l);
    let b = textureLoad(pic, vec2<i32>(i1.x, i0.y), l);
    let c = textureLoad(pic, vec2<i32>(i0.x, i1.y), l);
    let e = textureLoad(pic, i1, l);
    let top = a + (b - a) * f.x;
    let bot = c + (e - c) * f.x;
    return top + (bot - top) * f.y;
}

// (Img::sample)
fn trilinear(uv: vec2<f32>, lod_in: f32) -> vec4<f32> {
    let n = i32(textureNumLevels(pic));
    let lod = clamp(lod_in, 0.0, f32(n - 1));
    let l0 = i32(floor(lod));
    let f = lod - f32(l0);
    let a = bilinear(l0, uv);
    if f < 1e-3 || l0 + 1 >= n {
        return a;
    }
    let b = bilinear(l0 + 1, uv);
    return a + (b - a) * f;
}

// (Ripple::alpha; rp = t x speed, frequency, exponent, amplitude)
fn ripple(uv: vec2<f32>, rp: vec4<f32>) -> f32 {
    let p = uv * 2.0 - 1.0;
    let r = sqrt(p.x * p.x + p.y * p.y);
    let ph = (rp.x - r * rp.y) * 0.159155 + 0.5;
    let w = sin((ph - floor(ph)) * 6.2831855 - 3.1415927);
    let fall = pow(max(1.0 - r, 0.0), rp.z);
    return clamp(w * rp.w, 0.0, 1.0) * fall;
}

struct FOut { @location(0) c: vec4<f32>, @location(1) rim: vec4<f32> };

@fragment fn fs_main(i: VOut) -> FOut {
    var t = vec4<f32>(1.0);
    if i.mode == 1u {
        t = trilinear(i.uv, i.lod);
    } else if i.mode == 2u {
        t = vec4<f32>(1.0, 1.0, 1.0, ripple(i.uv, i.rp));
    }
    let src = t * i.colour;
    var o: FOut;
    o.c = src;
    o.rim = vec4<f32>(src.a, 0.0, 0.0, 0.0);
    return o;
}

@vertex fn vs_full(@builtin(vertex_index) k: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((k << 1u) & 2u), f32(k & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@group(0) @binding(0) var acc: texture_multisampled_2d<f32>;
@group(0) @binding(1) var rims: texture_multisampled_2d<f32>;
@group(0) @binding(2) var mask: texture_2d<f32>;

// (the mask's red at a sample, Img::sample at level 0, uv clamped to 0.05..0.95 - Target::mask)
fn mask_at(p: vec2<f32>, size: vec2<f32>) -> f32 {
    let d = vec2<i32>(textureDimensions(mask));
    let uv = clamp(p / size, vec2<f32>(0.05), vec2<f32>(0.95));
    let x = clamp(uv * vec2<f32>(d) - 0.5, vec2<f32>(0.0), vec2<f32>(d - 1));
    let i0 = vec2<i32>(floor(x));
    let i1 = min(i0 + 1, d - 1);
    let f = x - vec2<f32>(i0);
    let a = textureLoad(mask, i0, 0).r;
    let b = textureLoad(mask, vec2<i32>(i1.x, i0.y), 0).r;
    let c = textureLoad(mask, vec2<i32>(i0.x, i1.y), 0).r;
    let e = textureLoad(mask, i1, 0).r;
    let top = a + (b - a) * f.x;
    let bot = c + (e - c) * f.x;
    return top + (bot - top) * f.y;
}

@fragment fn fs_resolve(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(floor(pos.xy));
    let size = vec2<f32>(textureDimensions(acc));
    let sp = array<vec2<f32>, 4>(vec2<f32>(0.375, 0.125), vec2<f32>(0.875, 0.375), vec2<f32>(0.125, 0.625), vec2<f32>(0.625, 0.875));
    var sum = vec4<f32>(0.0);
    for (var s = 0; s < 4; s++) {
        let c = textureLoad(acc, px, s);
        let m = mask_at(vec2<f32>(px) + sp[s], size);
        let a = max(max(c.a - (1.0 - m), 0.0), textureLoad(rims, px, s).r);
        sum += vec4<f32>(c.rgb, a);
    }
    // (Target::resolve: the average, clamped, to 8 bits; then plain_color: premultiplied, (c a + 127) / 255)
    let v = floor(clamp(sum / 4.0, vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0 + 0.5);
    let p = floor((v.rgb * v.a + 127.0) / 255.0);
    return vec4<f32>(p, v.a) / 255.0;
}
"#;
