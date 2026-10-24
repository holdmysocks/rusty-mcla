//! A Flash movie's stage drawn on the GPU into a picture of its own, by the CPU rasteriser's rules
//! (`mcla_flash::raster::Canvas`): the `3dicons` movie's WorldIconRenderTarget (hookicons.rs).
//!
//! The rules, as `Canvas::with_samples(.., SAMPLES_4X)` draws them: 4 samples a pixel at D3D's standard 4x positions
//! (wgpu's 4x pattern is the same); the colour once per pixel at its centre, the vertex colour times the texel, the
//! texel from the straight-alpha picture (bilinear for kinds 0x40 / 0x41, else the nearest; wrapped for 0x40 / 0x42,
//! else clamped - done in the shader with `textureLoad`, as `TexImage::sample`); blended "over" (colour by source
//! alpha, alpha a + d (1 - a)) over transparent black; masks per sample (a stencil: each mask raises the samples it
//! covers inside the masks before it, its pop lowers them); the samples averaged. egui's renderer filters its
//! textures premultiplied and in linear light, which brightened the icon's glyph edges by up to 100 levels against
//! the CPU's picture (docs/PERFORMANCE.md, 2026-10-23); this pipeline gives the CPU's picture.
//!
//! The game thread queues a stage (`queue`), the render loop draws the queued ones before the UI pass (`render`),
//! the UI's callbacks bind the result (`view`). `MCLA_SELF_DUMP=dir` writes each picture drawn as `dir/<key>.png`.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use mcla_flash::draw::{DrawList, Item, TexRef};
use mcla_flash::raster::TexImage;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct V {
    /// clip space
    pos: [f32; 2],
    /// 0..255
    color: [f32; 4],
    uv: [f32; 2],
    /// 1 textured, 2 wrapped, 4 bilinear
    mode: u32,
}

enum Cmd {
    Draw { range: Range<u32>, tex: Option<Arc<TexImage>>, level: u32 },
    /// a mask's triangles: the samples at `level` they cover go up one
    Raise { range: Range<u32>, level: u32 },
    /// a popped mask's triangles again: its samples at `level` go down one
    Lower { range: Range<u32>, level: u32 },
}

struct Job {
    key: String,
    size: [u32; 2],
    verts: Vec<V>,
    cmds: Vec<Cmd>,
}

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());

/// Queue `list` drawn into the `size` picture `key` (stage (x, y) at `(x, y) scale + offset` pixels, as
/// `Canvas::draw`), its pictures from `tex`.
pub fn queue(key: &str, size: [u32; 2], list: &DrawList, scale: f32, offset: [f32; 2], tex: &dyn Fn(TexRef) -> Option<Arc<TexImage>>) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let mut verts: Vec<V> = Vec::new();
    let mut cmds = Vec::new();
    let mut level = 0u32;
    let mut masks: Vec<Range<u32>> = Vec::new();
    let mut defining: Option<u32> = None;
    for item in &list.items {
        match item {
            Item::MaskBegin => defining = Some(verts.len() as u32),
            Item::MaskEnd => {
                if let Some(s) = defining.take() {
                    let range = s..verts.len() as u32;
                    cmds.push(Cmd::Raise { range: range.clone(), level });
                    level += 1;
                    masks.push(range);
                }
            }
            Item::MaskPop => {
                if let Some(range) = masks.pop() {
                    cmds.push(Cmd::Lower { range, level });
                    level -= 1;
                }
            }
            Item::Triangles { tex: t, kind, verts: vs } => {
                let image = if defining.is_some() || *t == TexRef::None { None } else { tex(*t) };
                let mode = if image.is_some() { 1 | if *kind == 0x40 || *kind == 0x42 { 2 } else { 0 } | if *kind == 0x40 || *kind == 0x41 { 4 } else { 0 } } else { 0 };
                let s = verts.len() as u32;
                for v in &vs[..vs.len() / 3 * 3] {
                    let (x, y) = (v.pos[0] * scale + offset[0], v.pos[1] * scale + offset[1]);
                    verts.push(V { pos: [x / w * 2.0 - 1.0, 1.0 - y / h * 2.0], color: [v.color[0] as f32, v.color[1] as f32, v.color[2] as f32, v.color[3] as f32], uv: v.uv, mode });
                }
                if defining.is_none() && verts.len() as u32 > s {
                    cmds.push(Cmd::Draw { range: s..verts.len() as u32, tex: image, level });
                }
            }
        }
    }
    if let Ok(mut j) = JOBS.lock() {
        j.retain(|j| j.key != key);
        j.push(Job { key: key.to_string(), size, verts, cmds });
    }
}

struct Target {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    stencil: wgpu::TextureView,
    resolve: wgpu::Texture,
    view: wgpu::TextureView,
}

struct State {
    draw: wgpu::RenderPipeline,
    raise: wgpu::RenderPipeline,
    lower: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    white: wgpu::BindGroup,
    /// the movies' pictures, by their `Arc` (kept, so the address stays theirs)
    textures: HashMap<usize, (Arc<TexImage>, wgpu::BindGroup)>,
    targets: HashMap<String, Target>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SAMPLES: u32 = 4;

/// `key`'s picture, once drawn.
pub fn view(key: &str) -> Option<wgpu::TextureView> {
    STATE.lock().ok()?.as_ref()?.targets.get(key).map(|t| t.view.clone())
}

fn texture(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, w: u32, h: u32, rgba: &[u8]) -> wgpu::BindGroup {
    let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
    let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("flashpic texture"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: FORMAT, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &rgba[..(w * h * 4) as usize],
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        size,
    );
    let view = tex.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("flashpic texture"), layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }] })
}

impl State {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> State {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("flashpic"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flashpic"),
            entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None }],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("flashpic"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let make = |colour: bool, pass_op: wgpu::StencilOperation| {
            let face = wgpu::StencilFaceState { compare: wgpu::CompareFunction::Equal, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op };
            let blend = wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("flashpic"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<V>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4, 2 => Float32x2, 3 => Uint32] }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: Some(blend), write_mask: if colour { wgpu::ColorWrites::ALL } else { wgpu::ColorWrites::empty() } })],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Stencil8,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::Always,
                    stencil: wgpu::StencilState { front: face, back: face, read_mask: 0xff, write_mask: if colour { 0 } else { 0xff } },
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
                multiview: None,
                cache: None,
            })
        };
        let draw = make(true, wgpu::StencilOperation::Keep);
        let raise = make(false, wgpu::StencilOperation::IncrementClamp);
        let lower = make(false, wgpu::StencilOperation::DecrementClamp);
        let white = texture(device, queue, &layout, 1, 1, &[255; 4]);
        State { draw, raise, lower, layout, white, textures: HashMap::new(), targets: HashMap::new() }
    }

    fn target(&mut self, device: &wgpu::Device, key: &str, size: [u32; 2]) -> &Target {
        if self.targets.get(key).map_or(true, |t| t.size != size) {
            let ext = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
            let make = |label, format, samples, usage| device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: ext, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
            let msaa = make("flashpic", FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
            let stencil = make("flashpic masks", wgpu::TextureFormat::Stencil8, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
            let resolve = make("flashpic resolved", FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
            let view = resolve.create_view(&Default::default());
            self.targets.insert(key.to_string(), Target { size, msaa: msaa.create_view(&Default::default()), stencil: stencil.create_view(&Default::default()), resolve, view });
        }
        &self.targets[key]
    }
}

/// Make the pipelines now (at start-up: made on first use they stopped the frame that first showed an icon).
pub fn init(device: &wgpu::Device, queue: &wgpu::Queue) {
    if let Ok(mut st) = STATE.lock() {
        st.get_or_insert_with(|| State::new(device, queue));
    }
}

/// Draw this frame's queued pictures (before the UI pass that shows them).
pub fn render(device: &wgpu::Device, queue: &wgpu::Queue) {
    let jobs = std::mem::take(&mut *JOBS.lock().unwrap());
    if jobs.is_empty() {
        return;
    }
    let Ok(mut st) = STATE.lock() else { return };
    let st = st.get_or_insert_with(|| State::new(device, queue));
    use wgpu::util::DeviceExt;
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("flashpic") });
    for job in &jobs {
        for c in &job.cmds {
            if let Cmd::Draw { tex: Some(t), .. } = c {
                let k = Arc::as_ptr(t) as usize;
                if !st.textures.contains_key(&k) {
                    let bg = texture(device, queue, &st.layout, t.w, t.h, &t.rgba);
                    st.textures.insert(k, (t.clone(), bg));
                }
            }
        }
        st.target(device, &job.key, job.size);
        let t = &st.targets[&job.key];
        let vb = (!job.verts.is_empty()).then(|| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("flashpic"), contents: bytemuck::cast_slice(&job.verts), usage: wgpu::BufferUsages::VERTEX }));
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flashpic"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.msaa, resolve_target: Some(&t.view), ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.stencil, depth_ops: None, stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Discard }) }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let Some(vb) = vb.as_ref() else { continue };
        pass.set_vertex_buffer(0, vb.slice(..));
        for c in &job.cmds {
            let (pipe, range, level, bg) = match c {
                Cmd::Draw { range, tex, level } => (&st.draw, range, level, tex.as_ref().map_or(&st.white, |t| &st.textures[&(Arc::as_ptr(t) as usize)].1)),
                Cmd::Raise { range, level } => (&st.raise, range, level, &st.white),
                Cmd::Lower { range, level } => (&st.lower, range, level, &st.white),
            };
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            pass.set_stencil_reference(*level);
            pass.draw(range.clone(), 0..1);
        }
    }
    queue.submit([enc.finish()]);
    if let Ok(dir) = std::env::var("MCLA_SELF_DUMP") {
        for job in &jobs {
            dump(device, queue, &st.targets[&job.key].resolve, job.size, &format!("{dir}/{}.png", job.key.replace('#', "_")));
        }
    }
}

/// The picture read back and written as a PNG (premultiplied RGBA, as drawn; an Rgba8Unorm texture).
pub fn dump(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture, size: [u32; 2], path: &str) {
    let row = (size[0] * 4).div_ceil(256) * 256;
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("flashpic dump"), size: (row * size[1]) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size[1]) } },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::Maintain::Wait);
    let data = buf.slice(..).get_mapped_range();
    let px: Vec<u8> = data.chunks(row as usize).flat_map(|r| r[..(size[0] * 4) as usize].iter().copied()).collect();
    if let Ok(f) = std::fs::File::create(path) {
        let mut e = png::Encoder::new(std::io::BufWriter::new(f), size[0], size[1]);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let _ = e.write_header().and_then(|mut w| w.write_image_data(&px));
    }
}

const SHADER: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) mode: u32,
};
@group(0) @binding(0) var tex: texture_2d<f32>;

@vertex fn vs_main(@location(0) pos: vec2<f32>, @location(1) color: vec4<f32>, @location(2) uv: vec2<f32>, @location(3) mode: u32) -> VOut {
    var o: VOut;
    o.pos = vec4<f32>(pos, 0.0, 1.0);
    o.color = color;
    o.uv = uv;
    o.mode = mode;
    return o;
}

// (TexImage::texel: wrapped or clamped, 0..255)
fn texel(p: vec2<i32>, wrap: bool) -> vec4<f32> {
    let d = vec2<i32>(textureDimensions(tex));
    var q: vec2<i32>;
    if wrap {
        q = ((p % d) + d) % d;
    } else {
        q = clamp(p, vec2<i32>(0), d - 1);
    }
    return round(textureLoad(tex, q, 0) * 255.0);
}

// (TexImage::sample)
fn sample(uv: vec2<f32>, mode: u32) -> vec4<f32> {
    let wrap = (mode & 2u) != 0u;
    let d = vec2<f32>(textureDimensions(tex));
    let x = uv * d - 0.5;
    if (mode & 4u) == 0u {
        return texel(vec2<i32>(floor(x + 0.5)), wrap);
    }
    let x0 = floor(x);
    let f = x - x0;
    let i = vec2<i32>(x0);
    let a = texel(i, wrap);
    let b = texel(i + vec2<i32>(1, 0), wrap);
    let c = texel(i + vec2<i32>(0, 1), wrap);
    let e = texel(i + vec2<i32>(1, 1), wrap);
    return (a * (1.0 - f.x) + b * f.x) * (1.0 - f.y) + (c * (1.0 - f.x) + e * f.x) * f.y;
}

@fragment fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    var c = i.color;
    if (i.mode & 1u) != 0u {
        c = c * sample(i.uv, i.mode) / 255.0;
    }
    return vec4<f32>(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(255.0)) / 255.0, clamp(c.a / 255.0, 0.0, 1.0));
}
"#;
