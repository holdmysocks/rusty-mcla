//! The sun's glow quad (LIGHTING.md "The low sun's veil"): one additive full-screen pass over the finished scene,
//! before the tone map's chain (render/sun_glow.wgsl has the rule and where it comes from), and the occlusion
//! probe whose count fades it (LIGHTING.md "The sun glow's fade"; F-63954).
//!
//! F-63954, once a frame before the glow's draw F-77533: the two queries in turn (+12 the one this frame): the
//! one drawn two frames ago is read, its count kept when not ready; visible = clamp(count / 1250, 0, 1)
//! ([`visible_share`]); then colour writes, depth writes and stencil off (the depth test and cull as the
//! frame has them: GEQUAL, the cars' 0x18002, read from the running original), the same query begun, the
//! 8-slice sphere of radius 20 at camera + sun direction x (20 x 2 + 700) drawn ([`ellipsoid`],
//! [`probe_centre`]), ended, the index flipped. The query's count is the hardware's ZPass sample count
//! (RB_ZPASS_SAMPLES, summed by the library F-13473, nothing divided) of the 1280 x 720 scene at the
//! original's 2 (4 in the menus) samples. wgpu's occlusion queries are binary on D3D12 / Metal / GL, so the
//! port marks the samples the sphere passes into a target of the scene's samples and counts them in a
//! compute pass, scaled to the original's resolution and samples (the vertical lens is the same at any
//! aspect: the sphere's area goes with the height squared).

use crate::render::Gpu;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// F-63954's visible share: float(count) / float(total), then fsel: below 0 (or NaN) 0, from 1 up 1.
pub fn visible_share(count: u32, total: i32) -> f32 {
    let v = count as f32 / total as f32;
    let v = if v >= 0.0 { v } else { 0.0 };
    if v - 1.0 >= 0.0 { 1.0 } else { v }
}

/// The glow object's count (+0) and query index (+12).
#[derive(Clone, Copy, Debug, Default)]
pub struct QueryStep {
    pub count: u32,
    pub idx: usize,
}

impl QueryStep {
    /// One call of F-63954: the query `idx` is read (`answer`: its count when ready), the visible share of the
    /// count (old or new) is the frame's, the same query is drawn again and the index flips. Returns
    /// (the query read and drawn, visible).
    pub fn step(&mut self, answer: Option<u32>, total: i32) -> (usize, f32) {
        if let Some(c) = answer {
            self.count = c;
        }
        let v = visible_share(self.count, total);
        let q = self.idx;
        self.idx = (self.idx == 0) as usize;
        (q, v)
    }
}

/// The probe's place: camera + direction x float(radius x 2 + distance) (fused), each lane (dir x k + cam) + 0.
pub fn probe_centre(cam: [f32; 3], dir: [f32; 3], radius: f32, dist: f32) -> [f32; 3] {
    let k = crate::physics::exact::fma(radius, 2.0, dist);
    std::array::from_fn(|c| (dir[c] * k + cam[c]) + 0.0)
}

/// The solid ellipsoid of F-68903 (radii r, `slices`): for i = n .. 1 (n = slices / 2) one strip of 2 (slices + 1)
/// vertices, for j = 0 .. slices (rx cos p sin a, ry cos a, sin p sin a rz) and (rx cos q sin b, ry cos b,
/// sin q sin b rz), a = float(i pi) / n, b = float((i - 1) pi) / n, p = float(j 2 pi) / slices, q the same with
/// j + 1; the guest's single-precision cos / sin, every product rounded.
pub fn ellipsoid(r: [f32; 3], slices: i32) -> Vec<Vec<[f32; 3]>> {
    use crate::physics::exact::{guest_cos, guest_sin};
    let (pi, two_pi) = (std::f32::consts::PI, 6.2831855f32);
    let cs = |x: f32| guest_cos(x as f64) as f32;
    let sn = |x: f32| guest_sin(x as f64) as f32;
    let n = slices >> 1;
    let (nf, sf) = (n as f32, slices as f32);
    let mut out = Vec::new();
    let mut i = n;
    while i > 0 {
        let a = (i as f32 * pi) / nf;
        let b = ((i - 1) as f32 * pi) / nf;
        let ya = r[1] * cs(a);
        let yb = r[1] * cs(b);
        let (sa, sb) = (sn(a), sn(b));
        let mut strip = Vec::with_capacity(2 * (slices as usize + 1));
        for j in 0..=slices {
            let p = (j as f32 * two_pi) / sf;
            let q = ((j + 1) as f32 * two_pi) / sf;
            let x1 = (r[0] * cs(p)) * sa;
            let x2 = (r[0] * cs(q)) * sb;
            let z1 = (sn(p) * sa) * r[2];
            let z2 = (sn(q) * sb) * r[2];
            strip.push([x1, ya, z1]);
            strip.push([x2, yb, z2]);
        }
        out.push(strip);
        i -= 1;
    }
    out
}

/// The probe's radius (+28) and slices (F-63954 passes 8).
const PROBE_SLICES: i32 = 8;
/// The original's scene height (its 1280 x 720 target).
const ORIGINAL_HEIGHT: f32 = 720.0;

/// A query's read-back slot: copied this frame, mapping (the callback's flag: 0 waiting, 1 done, 2 failed), or free.
enum Slot {
    Free,
    Copied,
    Mapping(Arc<AtomicU8>),
}

struct Probe {
    pipeline: crate::render::ms::MsPipe,
    layout: wgpu::BindGroupLayout,
    vertices: wgpu::Buffer,
    strips: Vec<std::ops::Range<u32>>,
    consts: wgpu::Buffer,
    count_layouts: [wgpu::BindGroupLayout; 2],
    count_pipes: [wgpu::ComputePipeline; 2],
    counter: wgpu::Buffer,
    readback: [wgpu::Buffer; 2],
    slots: [Slot; 2],
    /// the target the probe marks: (samples, width, height, view)
    mark: Option<(u32, u32, u32, wgpu::TextureView)>,
    step: QueryStep,
    /// the original's sample count this frame (2, 4 in the menus) - set by the frame
    original_samples: u32,
}

struct SunGlow {
    pipeline: crate::render::ms::MsPipe,
    layout: wgpu::BindGroupLayout,
    /// rgb = the sky's SunColor of the hour, w = the share of the sun the query sees
    consts: wgpu::Buffer,
    probe: Probe,
}

static STATE: Mutex<Option<SunGlow>> = Mutex::new(None);

/// The frame's scene sample count as the original has it (aamode's ORIGINAL rule: 2, 4 with the menu or garage flag).
pub fn set_original_samples(n: u32) {
    if let Some(s) = STATE.lock().unwrap().as_mut() {
        s.probe.original_samples = n;
    }
}

/// Once, with the city's data.
pub fn load(gpu: &mut Gpu) {
    let device = &gpu.device;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sun glow layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        }, wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let consts = device.create_buffer(&wgpu::BufferDescriptor { label: Some("sun glow consts"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    // (the frame's globals: the particles' pipeline has their layout)
    let globals_layout = gpu.pipeline_particle.get_bind_group_layout(0);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sun glow"), source: wgpu::ShaderSource::Wgsl(include_str!("render/sun_glow.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sun glow pl"), bind_group_layouts: &[&globals_layout, &layout], push_constant_ranges: &[] });
    // (blend one / one, as the original's 0x01010101; the target's alpha is left alone)
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let pipeline = {
        let (pl, shader) = (pl.clone(), shader.clone());
        crate::render::ms::MsPipe::new(device, move |device, v| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sun glow"),
        layout: Some(&pl),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_sun_glow"), buffers: &[], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_sun_glow"),
            targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::COLOR })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: v.ms(),
        multiview: None,
        cache: None,
    }))
    };
    let probe = load_probe(device, &globals_layout, &shader);
    *STATE.lock().unwrap() = Some(SunGlow { pipeline, layout, consts, probe });
}

fn load_probe(device: &wgpu::Device, globals_layout: &wgpu::BindGroupLayout, shader: &wgpu::ShaderModule) -> Probe {
    use wgpu::util::DeviceExt;
    let cs = crate::sky::cloud_speed();
    let strips = ellipsoid([cs.glow_query_size; 3], PROBE_SLICES);
    let mut verts: Vec<[f32; 4]> = Vec::new();
    let mut ranges = Vec::new();
    for s in &strips {
        let a = verts.len() as u32;
        verts.extend(s.iter().map(|p| [p[0], p[1], p[2], 1.0]));
        ranges.push(a..verts.len() as u32);
    }
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("sun probe sphere"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
    let consts = device.create_buffer(&wgpu::BufferDescriptor { label: Some("sun probe consts"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sun probe layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sun probe pl"), bind_group_layouts: &[globals_layout, &layout], push_constant_ranges: &[] });
    let shader = shader.clone();
    let pipeline = crate::render::ms::MsPipe::new(device, move |device, v| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sun probe"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_sun_probe"),
            buffers: &[wgpu::VertexBufferLayout { array_stride: 16, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x4] }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_sun_probe"),
            targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        // (the frame's cull at the probe, 0x18002 as the cars': clockwise front, Face::Front culled - render/mod.rs
        // pipeline_opaque_cull; the depth test GEQUAL, no depth or stencil writes)
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, front_face: wgpu::FrontFace::Cw, cull_mode: Some(wgpu::Face::Front), ..Default::default() },
        depth_stencil: Some(v.ds(false, wgpu::CompareFunction::GreaterEqual, false, Default::default())),
        multisample: v.ms(),
        multiview: None,
        cache: None,
    }));
    let count_src = include_str!("render/sun_probe_count.wgsl");
    let mk = |ms: bool| {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun probe count"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: ms }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let src = count_src.replace("MARK_TYPE", if ms { "texture_multisampled_2d<f32>" } else { "texture_2d<f32>" }).replace("SAMPLES", if ms { "textureNumSamples(marks)" } else { "1u" });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sun probe count"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sun probe count pl"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("sun probe count"), layout: Some(&pl), module: &module, entry_point: Some("cs_count"), compilation_options: Default::default(), cache: None });
        (layout, pipe)
    };
    let (l1, p1) = mk(false);
    let (l2, p2) = mk(true);
    let counter = device.create_buffer(&wgpu::BufferDescriptor { label: Some("sun probe counter"), size: 4, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let rb = |i| device.create_buffer(&wgpu::BufferDescriptor { label: Some(if i == 0 { "sun probe query 0" } else { "sun probe query 1" }), size: 4, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    Probe {
        pipeline,
        layout,
        vertices,
        strips: ranges,
        consts,
        count_layouts: [l1, l2],
        count_pipes: [p1, p2],
        counter,
        readback: [rb(0), rb(1)],
        slots: [Slot::Free, Slot::Free],
        mark: None,
        step: QueryStep::default(),
        original_samples: 2,
    }
}

/// The port's marked samples as the original's count: x (720 / scene height)^2 x original samples / port samples.
pub fn original_count(marked: u32, scene_h: u32, port_samples: u32, original_samples: u32) -> u32 {
    let k = (ORIGINAL_HEIGHT as f64 / scene_h.max(1) as f64).powi(2) * original_samples as f64 / port_samples.max(1) as f64;
    (marked as f64 * k).round().min(u32::MAX as f64) as u32
}

impl Probe {
    /// The query of this frame: read the slot drawn two frames ago (kept count when not ready), the visible share.
    fn read(&mut self, gpu: &Gpu, total: i32) -> (usize, f32) {
        // (last frame's copy has been submitted since: start its read-back)
        for s in 0..2 {
            if matches!(self.slots[s], Slot::Copied) {
                let flag = Arc::new(AtomicU8::new(0));
                let f = flag.clone();
                self.readback[s].slice(..).map_async(wgpu::MapMode::Read, move |r| f.store(if r.is_ok() { 1 } else { 2 }, Ordering::Release));
                self.slots[s] = Slot::Mapping(flag);
            }
        }
        let _ = gpu.device.poll(wgpu::Maintain::Poll);
        let i = self.step.idx;
        let mut answer = None;
        if let Slot::Mapping(flag) = &self.slots[i] {
            match flag.load(Ordering::Acquire) {
                1 => {
                    let marked = u32::from_le_bytes(self.readback[i].slice(..).get_mapped_range()[..4].try_into().unwrap());
                    self.readback[i].unmap();
                    let (h, n) = self.mark.as_ref().map(|m| (m.2, m.0)).unwrap_or((gpu.scene_h, gpu.scene_n));
                    answer = Some(original_count(marked, h, n, self.original_samples));
                    self.slots[i] = Slot::Free;
                }
                2 => {
                    self.readback[i].unmap();
                    self.slots[i] = Slot::Free;
                }
                _ => {}
            }
        }
        let r = self.step.step(answer, total);
        // (MCLA_SUN_PROBE_LOG: the count and the share each frame)
        if std::env::var_os("MCLA_SUN_PROBE_LOG").is_some() {
            log::info!("sun probe: answer {answer:?} count {} visible {:.4}", self.step.count, r.1);
        }
        r
    }

    /// The probe of this frame into query `q` (skipped while that slot's last read-back has not come back).
    fn draw(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, q: usize, centre: [f32; 3]) {
        if !matches!(self.slots[q], Slot::Free) {
            return;
        }
        let (n, w, h) = (gpu.scene_n, gpu.scene_w.max(1), gpu.scene_h.max(1));
        if self.mark.as_ref().map(|m| (m.0, m.1, m.2)) != Some((n, w, h)) {
            let t = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("sun probe marks"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: n,
                dimension: wgpu::TextureDimension::D2,
                format: crate::render::post::SCENE_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.mark = Some((n, w, h, t.create_view(&Default::default())));
        }
        let mark = &self.mark.as_ref().unwrap().3;
        gpu.queue.write_buffer(&self.consts, 0, bytemuck::cast_slice(&[centre[0], centre[1], centre[2], 1.0]));
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sun probe"), layout: &self.layout, entries: &[wgpu::BindGroupEntry { binding: 2, resource: self.consts.as_entire_binding() }] });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sun probe"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: mark, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                // (the scene's depth, read only: the probe writes neither depth nor stencil)
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: None, stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &gpu.globals_bg, &[]);
            pass.set_bind_group(1, &bind, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            for r in &self.strips {
                pass.draw(r.clone(), 0..1);
            }
        }
        enc.clear_buffer(&self.counter, 0, None);
        let ms = (n > 1) as usize;
        let cbind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun probe count"),
            layout: &self.count_layouts[ms],
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(mark) }, wgpu::BindGroupEntry { binding: 1, resource: self.counter.as_entire_binding() }],
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sun probe count"), timestamp_writes: None });
            pass.set_pipeline(&self.count_pipes[ms]);
            pass.set_bind_group(0, &cbind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        enc.copy_buffer_to_buffer(&self.counter, 0, &self.readback[q], 0, 4);
        self.slots[q] = Slot::Copied;
    }
}

/// The pass: after the scene (its depth is read), before the tone map's chain. `sun_color` is the sky file's
/// SunColorKF at the hour, as stored (no decoding); `sun_dir` the sky's sun direction (the glow's own).
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder, sun_color: [f32; 3], cam: [f32; 3], sun_dir: [f32; 3]) {
    let mut guard = STATE.lock().unwrap();
    let Some(s) = guard.as_mut() else { return };
    let cs = crate::sky::cloud_speed();
    // (F-63954: the query read, the share, then the probe drawn into the same query; the glow F-77533 after)
    let (q, visible) = s.probe.read(gpu, cs.glow_pixels);
    let centre = probe_centre(cam, sun_dir, cs.glow_query_size, cs.glow_distance);
    if std::env::var_os("MCLA_SUN_PROBE_LOG").is_some() {
        log::info!("sun probe: camera {cam:?} sun {sun_dir:?} centre {centre:?}");
    }
    s.probe.draw(gpu, enc, q, centre);
    gpu.queue.write_buffer(&s.consts, 0, bytemuck::cast_slice(&[sun_color[0], sun_color[1], sun_color[2], visible, cs.glow_distance, cs.glow_half_size, cs.glow_intensity, 0.0]));
    let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sun glow"),
        layout: &s.layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&gpu.depth) }, wgpu::BindGroupEntry { binding: 1, resource: s.consts.as_entire_binding() }],
    });
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("sun glow"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_pipeline(&s.pipeline);
    pass.set_bind_group(0, &gpu.globals_bg, &[]);
    pass.set_bind_group(1, &bind, &[]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> u32 {
        u32::from_str_radix(s, 16).unwrap()
    }

    /// F-63954 run offline on 600 cases (tools/sun_visible_reference.py): the count kept or taken, the visible
    /// share's bits, the query read and drawn, the index after, the probe's place - for every case whose queries exist.
    #[test]
    fn query_step_matches_the_original() {
        let text = include_str!("testdata/sun_visible_reference.txt");
        let (mut n, mut probes) = (0, 0);
        for case in text.split("CASE ").skip(1) {
            let lines: Vec<&str> = case.lines().collect();
            let h: Vec<&str> = lines[0].split_whitespace().collect();
            let field = |k: &str| h[h.iter().position(|x| *x == k).unwrap() + 1];
            let (count0, idx, total, ready, answer) = (field("count0").parse::<u32>().unwrap(), field("idx").parse::<usize>().unwrap(), field("total").parse::<i32>().unwrap(), field("ready") == "1", field("answer").parse::<u32>().unwrap());
            let reset: Vec<i32> = vec![h[h.iter().position(|x| *x == "reset").unwrap() + 1].parse().unwrap(), h[h.iter().position(|x| *x == "reset").unwrap() + 2].parse().unwrap()];
            let create = hex(field("create"));
            let out: Vec<&str> = lines.iter().find(|l| l.starts_with("OUT ")).unwrap().split_whitespace().collect();
            if reset[0] != reset[1] && create == 0 {
                // (re-creating the queries failed: +33 set, the intensity 0, nothing read - the port's queries cannot fail)
                assert_eq!(out[7], "1");
                continue;
            }
            let mut st = QueryStep { count: count0, idx };
            let (q, v) = st.step(ready.then_some(answer), total);
            assert_eq!(out[1].parse::<u32>().unwrap(), st.count, "case {}", h[0]);
            assert_eq!(hex(out[2]), v.to_bits(), "case {} visible", h[0]);
            assert_eq!(out[4].parse::<usize>().unwrap(), st.idx, "case {} index", h[0]);
            // (the query objects as they are after the call: re-made after a device reset)
            let qa = hex(out[5 + q]);
            let read = lines.iter().find(|l| l.starts_with("READ ")).unwrap();
            assert_eq!(hex(&read[5..]), qa);
            let begin = lines.iter().find(|l| l.starts_with("BEGIN ")).unwrap();
            assert_eq!(hex(&begin[6..]), qa);
            if let Some(sp) = lines.iter().find(|l| l.starts_with("SPHERE ")) {
                let w: Vec<&str> = sp.split_whitespace().collect();
                assert_eq!((hex(w[1]), w[2], w[3], w[4]), (20f32.to_bits(), "8", "0", "1"));
                probes += 1;
            }
            n += 1;
        }
        assert!(n > 500 && probes == n, "{n} cases, {probes} probes");
    }

    /// The probe's place against the run's sphere matrices (the translation row), from the case's camera and sun.
    #[test]
    fn probe_centre_matches_the_original() {
        // (the cases' camera and direction are in the generator: the reference prints the matrix the sphere gets)
        let text = include_str!("testdata/sun_visible_reference.txt");
        let mut n = 0;
        for case in text.split("CASE ").skip(1) {
            let Some(cam) = case.lines().find(|l| l.starts_with("CAMDIR ")) else { continue };
            let w: Vec<f32> = cam.split_whitespace().skip(1).map(|x| f32::from_bits(hex(x))).collect();
            let Some(sp) = case.lines().find(|l| l.starts_with("SPHERE ")) else { continue };
            let sp: Vec<&str> = sp.split_whitespace().collect();
            let c = probe_centre([w[3], w[4], w[5]], [w[0], w[1], w[2]], 20.0, 700.0);
            for k in 0..3 {
                assert_eq!(hex(sp[5 + 12 + k]), c[k].to_bits(), "{}", case.lines().next().unwrap());
            }
            n += 1;
        }
        assert!(n > 500, "{n}");
    }

    /// F-68903's solid ellipsoid run offline (tools/sun_probe_reference.py: 5 cases, 366 vertices): every word.
    #[test]
    fn ellipsoid_matches_the_original() {
        let text = include_str!("testdata/sun_probe_reference.txt");
        let mut verts = 0;
        for case in text.split("case ").skip(1) {
            let mut lines = case.lines();
            let h: Vec<&str> = lines.next().unwrap().split_whitespace().collect();
            let r = [h[0].parse::<f32>().unwrap(), h[1].parse::<f32>().unwrap(), h[2].parse::<f32>().unwrap()];
            let strips = ellipsoid(r, h[3].parse().unwrap());
            let mut want: Vec<Vec<[u32; 3]>> = Vec::new();
            for l in lines {
                if l.starts_with("B ") {
                    want.push(Vec::new());
                } else if let Some(v) = l.strip_prefix("V ") {
                    let w: Vec<u32> = v.split_whitespace().map(hex).collect();
                    want.last_mut().unwrap().push([w[0], w[1], w[2]]);
                }
            }
            assert_eq!(strips.len(), want.len());
            for (s, ws) in strips.iter().zip(&want) {
                assert_eq!(s.len(), ws.len());
                for (p, wp) in s.iter().zip(ws) {
                    assert_eq!([p[0].to_bits(), p[1].to_bits(), p[2].to_bits()], *wp, "case {h:?}");
                    verts += 1;
                }
            }
        }
        assert_eq!(verts, 366);
    }

    #[test]
    fn original_count_scales_to_720_lines_and_the_original_samples() {
        assert_eq!(original_count(1000, 720, 2, 2), 1000);
        assert_eq!(original_count(2250, 1080, 2, 2), 1000);
        assert_eq!(original_count(1000, 720, 4, 2), 500);
        assert_eq!(original_count(500, 720, 1, 2), 1000);
    }
}
