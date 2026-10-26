//! The occlusion blobs under the cars (LIGHTING_WORK.md "The car's shadow model, the blobs under
//! it, its tangent frame", car round 13; render/car_blob.wgsl has the pixel rules).
//!
//! The original (cars/0xf9cd0545, technique `quadao_draw`), after the cars' glass:
//! * per wheel of the PLAYER's car `VS_DrawSingle + PS_CalcShadowsSingle`: a unit sphere scaled
//!   by the wheel's matrix (2 w, 2 R, 2 R) x 2.8 R - half axes 5.6 R w along the axle, 5.6 R R
//!   across - `blockerPosSingle` = the wheel's middle and R, the axle's half length 0.18 x 5.6 R w;
//! * per car (traffic too) `VS_DrawQuad + PS_CalcShadowsQuad`: a cylinder along the car, length
//!   span + 2 r, half axes 1.05 r sideways and 1.05 r / 8 up, its middle at the wheels' foot;
//!   `blockerPosQuad0..3` = four sphere middles span / 3 apart, w = r squared; strength
//!   sat((60 - distance to the eye) / 30).
//! Measured: the R8 r 1.8844 (0.99 x its width 1.9033), span 2.2318 (half the length of its
//! lowest level of detail), middles 0.4383 above the cylinder's; traffic cars r 1.5204 / 1.4401,
//! span exactly 1.5 r, middles 0.3211 / 0.2651 up. (approx.) here: the span is half the length
//! of the model's box, traffic cars' r a third of it, the height 0.2326 r (R8) / 0.2 r (traffic).
//!
//! `MCLA_DEBUG_VIEW=270`: the pass off; 271: wheels only; 272: bodies only; 273: without the
//! cars' mask (the blobs darken the cars too).

use crate::render::Gpu;
use glam::{Mat4, Vec3};
use std::sync::Mutex;

/// What the pass needs of one car (`Vehicles::blob_cars`).
pub struct CarBlob {
    /// the body's matrix
    pub m: Mat4,
    /// the model's box, in the car's space
    pub min: Vec3,
    pub max: Vec3,
    /// the wheels' foot in the car's space
    pub ground_y: f32,
    /// a traffic car (`va_...`)
    pub ambient: bool,
    /// the player's wheels: middle (world), axle (world, unit), radius, width
    pub wheels: Vec<(Vec3, Vec3, f32, f32)>,
}

struct State {
    blob: crate::render::ms::MsPipe,
    one: crate::render::ms::MsPipe,
    mask: crate::render::ms::MsPipe,
    apply: crate::render::ms::MsPipe,
    one_d: crate::render::ms::MsPipe,
    layout: wgpu::BindGroupLayout,
    inst: wgpu::Buffer,
}

const MAX_INST: usize = 512;
const INST_SIZE: usize = 96;

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn view() -> u32 {
    static V: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_DEBUG_VIEW").ok().and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// Once, with the city's data.
pub fn load(gpu: &mut Gpu) {
    if view() == 270 {
        return;
    }
    let device = &gpu.device;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("car blob layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        }],
    });
    let inst = device.create_buffer(&wgpu::BufferDescriptor { label: Some("car blob instances"), size: (MAX_INST * INST_SIZE) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("car blob"), source: wgpu::ShaderSource::Wgsl(include_str!("render/car_blob.wgsl").into()) });
    let globals_layout = gpu.globals_layout.clone();
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("car blob pl"), bind_group_layouts: &[&globals_layout, &layout], push_constant_ranges: &[] });
    // (the cars are drawn by `Vehicles::draw`, which binds the city pipelines' groups)
    // (the layouts themselves: asking the opaque pipeline for them built it here, ~4 s with the driver's cache cold)
    let city = [gpu.globals_layout.clone(), gpu.tex_layout.clone(), gpu.mat_layout.clone(), gpu.obj_layout.clone()];
    let pl_city = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("car blob mask pl"), bind_group_layouts: &[&city[0], &city[1], &city[2], &city[3]], push_constant_ranges: &[] });
    let keep = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let attrs: Vec<wgpu::VertexAttribute> = (0..6).map(|i| wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16 * i as u64, shader_location: i }).collect();
    let depth_ro = |bias: i32| (false, if bias != 0 { wgpu::CompareFunction::GreaterEqual } else { wgpu::CompareFunction::Always }, wgpu::DepthBiasState { constant: bias, slope_scale: if bias != 0 { 2.0 } else { 0.0 }, clamp: 0.0 });
    // (per sample count of the scene - render/ms.rs)
    type Bufs = Vec<(u64, wgpu::VertexStepMode, Vec<wgpu::VertexAttribute>)>;
    let make = |label: &'static str, layout: &wgpu::PipelineLayout, vs: &'static str, fs: &'static str, buffers: Bufs, blend: wgpu::BlendState, mask: wgpu::ColorWrites, ds: Option<(bool, wgpu::CompareFunction, wgpu::DepthBiasState)>| {
        let (layout, shader) = (layout.clone(), shader.clone());
        crate::render::ms::MsPipe::new(device, move |device, v| {
            let buffers: Vec<wgpu::VertexBufferLayout> = buffers.iter().map(|(s, m, a)| wgpu::VertexBufferLayout { array_stride: *s, step_mode: *m, attributes: a }).collect();
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), buffers: &buffers, compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(blend), write_mask: mask })], compilation_options: Default::default() }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: ds.map(|(w, c, b)| v.ds(w, c, false, b)),
                multisample: v.ms(),
                multiview: None,
                cache: None,
            })
        })
    };
    let inst_layout: Bufs = vec![(INST_SIZE as u64, wgpu::VertexStepMode::Instance, attrs.clone())];
    let vtx: Bufs = vec![(crate::render::Vertex::LAYOUT.array_stride, crate::render::Vertex::LAYOUT.step_mode, crate::render::Vertex::LAYOUT.attributes.to_vec())];
    // the target's alpha x (1 - the blob's alpha); the colour stays
    let mul_alpha = wgpu::BlendState { color: keep, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add } };
    // the colour x the target's alpha
    let mul_color = wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::DstAlpha, operation: wgpu::BlendOperation::Add }, alpha: keep };
    let blob = make("car blob", &pl, "vs_blob", "fs_blob", inst_layout, mul_alpha, wgpu::ColorWrites::ALPHA, None);
    let one = make("car blob one", &pl, "vs_full", "fs_one", vec![], wgpu::BlendState::REPLACE, wgpu::ColorWrites::ALPHA, None);
    let mask = make("car blob mask", &pl_city, "vs_mask", "fs_one", vtx, wgpu::BlendState::REPLACE, wgpu::ColorWrites::ALPHA, Some(depth_ro(16)));
    let apply = make("car blob apply", &pl_city, "vs_full", "fs_apply", vec![], mul_color, wgpu::ColorWrites::COLOR, Some(depth_ro(0)));
    let one_d = make("car blob one (depth)", &pl_city, "vs_full", "fs_one", vec![], wgpu::BlendState::REPLACE, wgpu::ColorWrites::ALPHA, Some(depth_ro(0)));
    *STATE.lock().unwrap() = Some(State { blob, one, mask, apply, one_d, layout, inst });
}

fn push(out: &mut Vec<f32>, rows: [[f32; 4]; 6]) {
    for r in rows {
        out.extend_from_slice(&r);
    }
}

/// The pass: after the scene (its depth is read), before the glow passes and the tone map.
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder, vehicles: &crate::vehicles::Vehicles, cam: Vec3) {
    let guard = STATE.lock().unwrap();
    let Some(s) = guard.as_ref() else { return };
    let dbg = view();
    let mut data: Vec<f32> = Vec::new();
    let mut n = 0usize;
    for c in vehicles.blob_cars() {
        let pos = c.m.w_axis.truncate();
        let strength = ((60.0 - pos.distance(cam)) / 30.0).clamp(0.0, 1.0);
        if strength <= 0.0 {
            continue;
        }
        let (x, y, z) = (c.m.x_axis.truncate(), c.m.y_axis.truncate(), c.m.z_axis.truncate());
        if dbg != 272 {
            for (mid, axle, r, w) in &c.wheels {
                if n >= MAX_INST {
                    break;
                }
                // two directions across the axle
                let u = (y - *axle * y.dot(*axle)).normalize_or(Vec3::Y);
                let v = axle.cross(u);
                let (along, across) = (5.6 * r * w, 5.6 * r * r);
                push(&mut data, [mid.extend(0.0).to_array(), (*axle * along).extend(1.0).to_array(), (u * across).extend(0.0).to_array(), (v * across).extend(0.0).to_array(), mid.extend(*r).to_array(), axle.extend(0.18 * along).to_array()]);
                n += 1;
            }
        }
        if dbg != 271 && n < MAX_INST {
            let (width, length) = (c.max.x - c.min.x, c.max.z - c.min.z);
            let r = if c.ambient { length / 3.0 } else { 0.99 * width };
            // (the R8's measured span is 2.2318 - half the length of its lowest level of detail,
            // 4.4636 - against 2.149 from the model's box: the end spheres stood 4 cm short and the
            // blob's outer edge was a fifth too weak behind the car. Other cars: the box, (approx.))
            let span = if !c.ambient && (width - 1.9033).abs() < 0.003 && (length - 4.298).abs() < 0.02 { 2.2318 } else { length * 0.5 };
            if std::env::var("MCLA_GLOW_DUMP").is_ok() { eprintln!("blob body width {width:.4} length {length:.4} r {r:.4} span {span:.4}"); }
            let up = if c.ambient { 0.2 * r } else { 0.2326 * r };
            let foot = pos + y * c.ground_y;
            let first = foot + y * up - z * (span * 0.5);
            push(&mut data, [foot.extend(1.0).to_array(), (x * (1.05 * r)).extend(strength).to_array(), (y * (1.05 * r / 8.0)).extend(0.0).to_array(), (z * (span * 0.5 + r)).extend(0.0).to_array(), first.extend(r * r).to_array(), (z * (span / 3.0)).extend(0.0).to_array()]);
            n += 1;
        }
    }
    if n == 0 {
        return;
    }
    gpu.queue.write_buffer(&s.inst, 0, bytemuck::cast_slice(&data));
    let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("car blob"), layout: &s.layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&gpu.depth) }] });
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("car blobs"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &gpu.globals_bg, &[]);
        pass.set_bind_group(1, &bind, &[]);
        pass.set_pipeline(&s.one);
        pass.draw(0..3, 0..1);
        pass.set_pipeline(&s.blob);
        pass.set_vertex_buffer(0, s.inst.slice(..(n * INST_SIZE) as u64));
        pass.draw(0..6, 0..n as u32);
    }
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("car blobs: cars, apply"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &gpu.globals_bg, &[]);
        if dbg != 273 {
            pass.set_pipeline(&s.mask);
            let mut draws = 0usize;
            vehicles.draw(&mut pass, gpu, false, &mut draws, Some((cam, 110.0)), None);
            vehicles.draw_blended_plain(&mut pass, gpu, Some((cam, 110.0)));
        }
        // (the full-screen draws use the city layout, whose groups 1 - 3 the car draws above bind;
        // with every car out of the passes - LOD -1 - none did, so a car batch's are bound)
        let Some((c, b)) = vehicles.cars.iter().find_map(|c| c.model.batches.first().map(|b| (c, b))) else { return };
        let Some(mat) = gpu.mat_bgs.get(&(b.mode as u32)) else { return };
        pass.set_bind_group(1, b.bind.as_ref(), &[]);
        pass.set_bind_group(2, mat, &[]);
        pass.set_bind_group(3, &c.binding.bg, &[]);
        pass.set_pipeline(&s.apply);
        pass.draw(0..3, 0..1);
        pass.set_pipeline(&s.one_d);
        pass.draw(0..3, 0..1);
    }
}
