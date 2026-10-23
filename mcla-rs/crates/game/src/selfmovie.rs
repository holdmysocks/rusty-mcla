//! "fullscreenself": the menus' 3D-scene movies drawn into a picture of their own and laid on their mesh as its
//! texture, as the original does it.
//!
//! The original (`work/ui-sharp/agent_msaa/notes.md` sections 2 and 6; UI.md "UI edge smoothing"): a movie with a
//! `dyn3Ds` variable (sub_82724E68) and a `player0mesh` (sub_827232E8 / sub_82722B00) renders itself into the
//! shared 960 x 640 8888 target "fullscreenself" (0x8290A9A8, 2x MSAA, sub_821F8960): cleared, the stage drawn
//! flat and squeezed onto the whole picture, resolved (sub_82720C58 -> sub_827237C0 -> sub_82722C00). The mesh
//! that has `useMovieAsTexture` or is the `player0mesh` then samples that picture (sub_82724220) in the UI pass:
//! linear / linear, level 0 only, wrap, no anisotropy. Self movies: pause, garage, navsys, navsys_cursor,
//! raceeditor, 3dicons, sidekick, 0xc10aaf41.
//!
//! Here: during the UI frame a movie queues its flat shapes once (`queue`); the render loop draws each queued
//! movie into its own 960 x 640 2x picture with a 2-sample egui renderer and resolves it (`render_jobs`), and
//! the mesh is drawn by a paint callback (`Paint`) with its clip-space vertices, perspective-correct.
//! *(approx.)* the picture is cleared to transparent black (the original clears to the movie's background
//! colour); the premultiplied picture drawn "over" gives the same composite as drawing the movie straight.
//! `MCLA_SELF_MOVIE=0`: the old way (the movie's triangles projected one by one).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The original's fullscreenself size.
pub const W: u32 = 960;
pub const H: u32 = 640;
/// Its sample count (sub_821F8960's block: 2). A device without 2-sample colour targets (WebGPU guarantees 1 and 4)
/// gets 4 instead *(approx.)*.
pub fn samples() -> u32 {
    if TWO.load(std::sync::atomic::Ordering::Relaxed) { 2 } else { 4 }
}

static TWO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set once the device is made (render/mod.rs).
pub fn set_two_samples(ok: bool) {
    TWO.store(ok, std::sync::atomic::Ordering::Relaxed);
}

pub fn enabled() -> bool {
    std::env::var("MCLA_SELF_MOVIE").map_or(true, |v| v.trim() != "0")
}

struct Job {
    key: String,
    shapes: Vec<egui::epaint::ClippedShape>,
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());

/// Whether `key`'s picture is already queued this frame (one render per movie and frame, for every mesh).
pub fn queued(key: &str) -> bool {
    JOBS.lock().map_or(false, |j| j.iter().any(|j| j.key == key))
}

pub fn queue(key: &str, shapes: Vec<egui::epaint::ClippedShape>) {
    if let Ok(mut j) = JOBS.lock() {
        if !j.iter().any(|j| j.key == key) {
            j.push(Job { key: key.to_string(), shapes });
        }
    }
}

struct Pic {
    msaa: wgpu::TextureView,
    resolve: wgpu::TextureView,
    bg: wgpu::BindGroup,
}

/// The pictures and their binding, shared by the UI pass's renderers.
pub struct Shared {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pics: HashMap<String, Pic>,
}

impl Shared {
    pub fn new(device: &wgpu::Device) -> Arc<Mutex<Shared>> {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("self movie"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        // (the fetch constant of draws 4476 / 4477 in gps_whole: linear / linear, mip 0 only, wrap)
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("self movie"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Arc::new(Mutex::new(Shared { layout, sampler, pics: HashMap::new() }))
    }

    fn pic(&mut self, device: &wgpu::Device, key: &str, format: wgpu::TextureFormat) -> &Pic {
        if !self.pics.contains_key(key) {
            let size = wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 };
            let msaa = device.create_texture(&wgpu::TextureDescriptor { label: Some("fullscreenself"), size, mip_level_count: 1, sample_count: samples(), dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] });
            let resolve = device.create_texture(&wgpu::TextureDescriptor { label: Some("fullscreenself resolved"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
            let rv = resolve.create_view(&Default::default());
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("self movie"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&rv) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }],
            });
            self.pics.insert(key.to_string(), Pic { msaa: msaa.create_view(&Default::default()), resolve: rv, bg });
        }
        &self.pics[key]
    }
}

/// Draw this frame's queued movies into their pictures (`renderer`: 2 samples, the UI pass's colour format,
/// no depth; its textures kept as the screen renderer's). One submission per movie: the renderer's buffers
/// are rewritten for each.
pub fn render_jobs(ctx: &egui::Context, device: &wgpu::Device, queue: &wgpu::Queue, renderer: &mut egui_wgpu::Renderer, shared: &Arc<Mutex<Shared>>, format: wgpu::TextureFormat) {
    let jobs = std::mem::take(&mut *JOBS.lock().unwrap());
    let Ok(mut sh) = shared.lock() else { return };
    let desc = egui_wgpu::ScreenDescriptor { size_in_pixels: [W, H], pixels_per_point: 1.0 };
    for job in jobs {
        let prims = ctx.tessellate(job.shapes, 1.0);
        let pic = sh.pic(device, &job.key, format);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("fullscreenself") });
        let extra = renderer.update_buffers(device, queue, &mut enc, &prims, &desc);
        {
            let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fullscreenself"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &pic.msaa, resolve_target: Some(&pic.resolve), ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut pass = pass.forget_lifetime();
            renderer.render(&mut pass, &prims, &desc);
        }
        queue.submit(extra.into_iter().chain([enc.finish()]));
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vtx {
    /// clip space
    pub pos: [f32; 4],
    pub uv: [f32; 2],
    /// the mesh's own alpha
    pub alpha: f32,
}

/// The mesh pipeline of one UI renderer (its sample count and depth attachment) and this frame's vertex buffers.
pub struct SelfRes {
    pipeline: wgpu::RenderPipeline,
    shared: Arc<Mutex<Shared>>,
    bufs: HashMap<u64, (wgpu::Buffer, u32)>,
}

impl SelfRes {
    pub fn new(device: &wgpu::Device, shared: Arc<Mutex<Shared>>, format: wgpu::TextureFormat, samples: u32, depth: bool) -> SelfRes {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("self movie"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("self movie"), bind_group_layouts: &[&shared.lock().unwrap().layout], push_constant_ranges: &[] });
        // (the picture holds premultiplied colour: the movie drawn "over" transparent black)
        let colour = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        let alpha = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::OneMinusDstAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("self movie"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vtx>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2, 2 => Float32] }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState { color: colour, alpha }), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: depth.then(|| wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: false, depth_compare: wgpu::CompareFunction::Always, stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
            cache: None,
        });
        SelfRes { pipeline, shared, bufs: HashMap::new() }
    }

    /// Drop last frame's vertex buffers (before the UI renderer prepares this frame's callbacks).
    pub fn new_frame(&mut self) {
        self.bufs.clear();
    }
}

const SHADER: &str = r#"
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) alpha: f32 };
@group(0) @binding(0) var pic: texture_2d<f32>;
@group(0) @binding(1) var pic_s: sampler;
@vertex fn vs_main(@location(0) pos: vec4<f32>, @location(1) uv: vec2<f32>, @location(2) alpha: f32) -> VOut {
    var o: VOut;
    o.pos = pos;
    o.uv = uv;
    o.alpha = alpha;
    return o;
}
@fragment fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    return textureSample(pic, pic_s, i.uv) * i.alpha;
}
"#;

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// One mesh carrying a movie's picture: triangles in clip space of the callback's rectangle (the screen).
pub struct Paint {
    key: String,
    verts: Vec<Vtx>,
    id: u64,
}

impl Paint {
    pub fn new(key: &str, verts: Vec<Vtx>) -> Paint {
        Paint { key: key.to_string(), verts, id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) }
    }
}

impl egui_wgpu::CallbackTrait for Paint {
    fn prepare(&self, device: &wgpu::Device, _queue: &wgpu::Queue, _sd: &egui_wgpu::ScreenDescriptor, _enc: &mut wgpu::CommandEncoder, res: &mut egui_wgpu::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        use wgpu::util::DeviceExt;
        if let Some(r) = res.get_mut::<SelfRes>() {
            if !self.verts.is_empty() {
                let b = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("self movie mesh"), contents: bytemuck::cast_slice(&self.verts), usage: wgpu::BufferUsages::VERTEX });
                r.bufs.insert(self.id, (b, self.verts.len() as u32));
            }
        }
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, res: &egui_wgpu::CallbackResources) {
        let Some(r) = res.get::<SelfRes>() else { return };
        let Some((b, n)) = r.bufs.get(&self.id) else { return };
        let Ok(sh) = r.shared.lock() else { return };
        let Some(pic) = sh.pics.get(&self.key) else { return };
        pass.set_pipeline(&r.pipeline);
        pass.set_bind_group(0, &pic.bg, &[]);
        pass.set_vertex_buffer(0, b.slice(..));
        pass.draw(0..*n, 0..1);
    }
}

/// Queue `movie`'s flat picture (once a frame) and paint a mesh with it: `clip` (of the rectangle `rect`) and `uv`
/// per vertex of a triangle list (uv over the stage, 0..1), `alpha` the mesh's own.
pub fn paint_mesh(ctx: &egui::Context, painter: &egui::Painter, rect: egui::Rect, movie: &mut crate::flash_ui::FlashMovie, list: &mcla_flash::draw::DrawList, tris: &[(glam::Vec4, [f32; 2])], alpha: f32) {
    let key = movie.name.clone();
    if !queued(&key) {
        let shapes = movie.self_shapes(ctx, list, [W as f32, H as f32]);
        queue(&key, shapes);
    }
    let verts = tris.iter().map(|(c, uv)| Vtx { pos: c.to_array(), uv: *uv, alpha }).collect();
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, Paint::new(&key, verts)));
}
