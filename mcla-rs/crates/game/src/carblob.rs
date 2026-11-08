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
//! of the model's box, the height 0.2326 r (R8). Traffic cars follow the decoded record (F-68081 -> F-18380,
//! see the traffic branch in `run`); the player car's own producer is not decoded.
//!
//! `MCLA_DEBUG_VIEW=270`: the pass off; 271: wheels only; 272: bodies only; 273: without the
//! cars' mask (the blobs darken the cars too); 275: no neon light quads.
//!
//! The neon (F-17121 -> F-62149 -> F-35120 `quadlight`): a car whose ExtNeonIntensity is over 0.001 makes four
//! light quads round its ground frame and its body quad turns into a fifth; each multiplies the frame by
//! 1 + 12 shape x lightColor (PS_CalcShadowsLight: 0.75 shape colour + gInvColorExpBias, the target's x16) - `neon_records`.

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
    cone: crate::render::ms::MsPipe,
    cone_inst: wgpu::Buffer,
    light: crate::render::ms::MsPipe,
    light_inst: wgpu::Buffer,
    cone_mesh: wgpu::Buffer,
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
    // (the head-lamp cones: the colour x the program's output, as the recorded blend 0x80008 - destination colour,
    // zero - only where the stencil is 0 and in front of the scene, back faces culled)
    let cone_inst = device.create_buffer(&wgpu::BufferDescriptor { label: Some("car cones"), size: (64 * CONE_SIZE) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let cone_mesh = wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor { label: Some("car cone mesh"), contents: bytemuck::cast_slice(&cone_mesh()), usage: wgpu::BufferUsages::VERTEX });
    // (the light quads, `quadlight_draw`: the cone's states with the light program - the five recorded
    // PS_CalcShadowsLight draws (work/lighting-trails/neon/cap/n23, n12) have RB_DEPTHCONTROL 0x708263,
    // RB_BLENDCONTROL 0x80008 (source x destination colour), PA_SU_SC_MODE_CNTL 0x18002, RB_STENCILREFMASK 0xff00 -
    // the occlusion quads' but for the blend - on the `ao_shadow` model, the same 16-sided cylinder)
    let light_inst = device.create_buffer(&wgpu::BufferDescriptor { label: Some("car neon lights"), size: (MAX_LIGHT * LIGHT_SIZE) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let light = {
        let (layout, shader) = (pl.clone(), shader.clone());
        crate::render::ms::MsPipe::new(device, move |device, v| {
            let attrs: Vec<wgpu::VertexAttribute> = (0..9).map(|i| wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16 * i as u64, shader_location: i + 1 }).collect();
            let pos = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }];
            let mut ds = v.ds(false, wgpu::CompareFunction::GreaterEqual, false, wgpu::DepthBiasState::default());
            let face = wgpu::StencilFaceState { compare: wgpu::CompareFunction::Equal, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op: wgpu::StencilOperation::Keep };
            ds.stencil = wgpu::StencilState { front: face, back: face, read_mask: 0xff, write_mask: 0 };
            let mult = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Dst, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("car neon light"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_light"),
                    buffers: &[
                        wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &pos },
                        wgpu::VertexBufferLayout { array_stride: LIGHT_SIZE as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &attrs },
                    ],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_light"), targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: mult, alpha: mult }), write_mask: wgpu::ColorWrites::COLOR })], compilation_options: Default::default() }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw, cull_mode: Some(wgpu::Face::Back), ..Default::default() },
                depth_stencil: Some(ds),
                multisample: v.ms(),
                multiview: None,
                cache: None,
            })
        })
    };
    let cone = {
        let (layout, shader) = (pl.clone(), shader.clone());
        crate::render::ms::MsPipe::new(device, move |device, v| {
            let attrs: Vec<wgpu::VertexAttribute> = (0..7).map(|i| wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16 * i as u64, shader_location: i + 1 }).collect();
            let pos = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }];
            let mut ds = v.ds(false, wgpu::CompareFunction::GreaterEqual, false, wgpu::DepthBiasState::default());
            let face = wgpu::StencilFaceState { compare: wgpu::CompareFunction::Equal, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op: wgpu::StencilOperation::Keep };
            ds.stencil = wgpu::StencilState { front: face, back: face, read_mask: 0xff, write_mask: 0 };
            let mult = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Dst, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("car cone"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_cone"),
                    buffers: &[
                        wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &pos },
                        wgpu::VertexBufferLayout { array_stride: CONE_SIZE as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &attrs },
                    ],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_cone"), targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: mult, alpha: mult }), write_mask: wgpu::ColorWrites::COLOR })], compilation_options: Default::default() }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw, cull_mode: Some(wgpu::Face::Back), ..Default::default() },
                depth_stencil: Some(ds),
                multisample: v.ms(),
                multiview: None,
                cache: None,
            })
        })
    };
    *STATE.lock().unwrap() = Some(State { blob, one, mask, apply, one_d, cone, cone_inst, light, light_inst, cone_mesh, layout, inst });
}

/// The `cone` model as read from the running original: 16 sides, radius 1 round z, z +-0.5, 180 indices.
fn cone_mesh() -> Vec<[f32; 3]> {
    let at = |k: i32, z: f32| {
        let a = k as f32 * std::f32::consts::TAU / 16.0;
        [a.cos(), a.sin(), z]
    };
    let mut v = Vec::with_capacity(180);
    for k in 0..16 {
        v.extend([at(k, -0.5), at(k + 1, -0.5), at(k + 1, 0.5), at(k, -0.5), at(k + 1, 0.5), at(k, 0.5)]);
    }
    for k in 1..15 {
        v.extend([at(0, 0.5), at(k, 0.5), at(k + 1, 0.5), at(0, -0.5), at(k + 1, -0.5), at(k, -0.5)]);
    }
    v
}

const CONE_SIZE: usize = 112;
/// a light quad's instance: the volume's four rows, c0..c3, the colour
const LIGHT_SIZE: usize = 144;
const MAX_LIGHT: usize = 64;

/// The light list's draw (F-35120, `quadlight`) of one record: the `ao_shadow` cylinder by the record's matrix, rows x
/// (r x 1.05) ([0x8203BE78]), y (r x 0.13125) ([0x8203EA38]), z (r x 2.0 + |c0 - c3|), the origin +128; r = sqrt(+28);
/// blockerPosQuad0..3 = c0..c3, lightColor = +144.
fn light_instance(r: &QuadRec) -> [[f32; 4]; 9] {
    let rad = r.c[0][3].sqrt();
    let d = v4(|i| r.c[0][i] - r.c[3][i]);
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let row = |k: usize, s: f32| [r.m[k][0] * s, r.m[k][1] * s, r.m[k][2] * s, 0.0];
    [row(0, rad * 1.05), row(1, rad * 0.131_25), row(2, rad * 2.0 + len), r.m[3], r.c[0], r.c[1], r.c[2], r.c[3], r.colour]
}

/// F-63160's box (the body record's radius r = 0.64 X, F-17121): the car's base resource ((car+20)+80 -> +32, F-63160 /
/// F-58...; the vehicle's .r83), its object at +16, the box max +16 / min +32; X = max.x - min.x (vsubfp). Both .r83 of a
/// car hold the same box. Read from the files: R8 X 2.9444 -> r2 3.5511, ZX-14 1.3808 -> 0.7809, the recorded body
/// quads' r2 (work/lighting-trails/neon/cap/n23, work/police17/cap/bike_move). None when the car has no .r83.
fn drawable_box(root: &std::path::Path, model: &str) -> Option<[f32; 4]> {
    static CACHE: Mutex<Option<std::collections::HashMap<String, Option<[f32; 4]>>>> = Mutex::new(None);
    let mut g = CACHE.lock().unwrap();
    let map = g.get_or_insert_with(Default::default);
    if let Some(v) = map.get(model) {
        return *v;
    }
    let read = || -> Option<[f32; 4]> {
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("resources/vehicle").join(model)).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "r83")).collect();
        files.sort();
        let res = crate::assets::load_resource_file(files.first()?).ok()?;
        let rd = mcla_formats::rage::Rd::new(&res);
        let o = rd.ptr(16)?;
        let v = |a: usize| [rd.f32(a), rd.f32(a + 4), rd.f32(a + 8), rd.f32(a + 12)];
        let (mx, mn) = (v(o + 0x10), v(o + 0x20));
        let d = v4(|i| mx[i] - mn[i]);
        (d[0].is_finite() && d[0] > 0.0).then_some(d)
    };
    let v = read();
    map.insert(model.to_string(), v);
    v
}

/// The neon cars' records (F-17121) with the port's inputs: (the car's body matrix, its body record, its four neon records).
/// The sizes W H L are the composite bound's box (F-98453, as the wheel fit leaves it - `CarParams::composite_box`); the
/// player's car makes no ground probe (f28 = 1). (approx.) the ground frame F-88547 (a probe 1 m above to 3 m below the
/// car: its hit normal and point) is the body matrix moved to the wheels' foot. The radius box F-63160 is
/// `drawable_box` (the car's .r83; without one, the port's old blob radius 0.99 x the model's width / 0.64).
/// f28 = sat(1 - T): a car other than the one the view follows ([0x8288DCF8], the player's) whose head state (+168) is
/// over 0.001 probes (F-20855: include 0x800 = the city bound, states 4 = fixed, one result) from its origin + its
/// up row x 2.0 ([0x827E6358]) 5.0 m ([0x82006BF8]) straight down; T = the hit's fraction (+64 of the result, 0.0
/// before the call). On a miss F-17121 takes the join with T as the result left it - (approx.) 0.0, i.e. f28 = 1.
/// The head state is the port's eased head lamp level `AmbLamps::s[0]`; the probe is the city's `probe_front`.
fn neon_lights(vehicles: &crate::vehicles::Vehicles, cam: Vec3, night: bool, collider: Option<&crate::physics::collision::WorldCollider>) -> Vec<(Mat4, QuadRec, [QuadRec; 4])> {
    let mut out = Vec::new();
    let blobs = vehicles.blob_cars();
    for (ci, m, n, player) in vehicles.neon_cars(vehicles.interior_glass) {
        // F-44722 calls F-17121 only for a car whose draw flag (+116) its visibility test set this frame: the
        // light volumes off screen draw nothing, the 450 m reach is the one that counts (approx.: the port's reach,
        // not the flag's own routine)
        if m.w_axis.truncate().distance(cam) > 450.0 {
            continue;
        }
        let c = &vehicles.cars[ci];
        // (a bike is not in `blob_cars`: its wheels' foot from the model's box)
        let (ground_y, width) = match blobs.iter().find(|b| b.m == m) {
            Some(b) => (b.ground_y, b.max.x - b.min.x),
            None => (c.model.min.y, c.model.max.x - c.model.min.x),
        };
        let (lo, hi) = c.params().composite_box();
        let (x, y, z) = (m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate());
        let foot = m.w_axis.truncate() + y * ground_y;
        let input = NeonIn {
            g: [x.extend(0.0).to_array(), y.extend(0.0).to_array(), z.extend(0.0).to_array(), foot.extend(1.0).to_array()],
            size: (hi - lo).extend(0.0).to_array(),
            box_: drawable_box(&vehicles.data_root, &c.model.name).unwrap_or([0.99 * width / 0.64, 0.0, 0.0, 0.0]),
            colour: n.color,
            intensity: n.intensity,
            pulse: n.pulse,
            scale: 1.0,
            up_y: m.y_axis.y,
            t: match collider {
                Some(w) if !player && c.amb_lamps.s[0] > 0.001 => {
                    let start = m.w_axis.truncate() + y * 2.0;
                    w.probe_front(start, Vec3::NEG_Y, 5.0).map_or(0.0, |h| h.t / 5.0)
                }
                _ => 0.0,
            },
            night,
            b6477: false,
        };
        if let (body, Some(neon)) = neon_records(&input) {
            if std::env::var("MCLA_GLOW_DUMP").is_ok() {
                eprintln!("neon car {ci} t {:.4} size {:?} body c0 {:?} c3 {:?} colour {:?}", input.t, input.size, body.c[0], body.c[3], body.colour);
            }
            out.push((m, body, neon));
        }
    }
    out
}

fn push(out: &mut Vec<f32>, rows: [[f32; 4]; 6]) {
    for r in rows {
        out.extend_from_slice(&r);
    }
}

/// The pass: after the scene (its depth is read), before the glow passes and the tone map.
/// `night`: the light quads' D (0.2 at night outside the garage, else 0.125).
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder, vehicles: &crate::vehicles::Vehicles, cam: Vec3, night: bool, collider: Option<&crate::physics::collision::WorldCollider>) {
    let guard = STATE.lock().unwrap();
    let Some(s) = guard.as_ref() else { return };
    let dbg = view();
    // (a car whose neon is on: its body record is the fifth light quad, not an occlusion quad - F-62149 sorts by +4)
    let lights = if dbg != 275 { neon_lights(vehicles, cam, night, collider) } else { Vec::new() };
    if !lights.is_empty() {
        let inst: Vec<[[f32; 4]; 9]> = lights.iter().flat_map(|(_, body, neon)| std::iter::once(body).chain(neon.iter()).map(light_instance)).take(MAX_LIGHT).collect();
        gpu.queue.write_buffer(&s.light_inst, 0, bytemuck::cast_slice(&inst));
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("car neon lights"), layout: &s.layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&gpu.depth) }] });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("car neon lights"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }) }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &gpu.globals_bg, &[]);
        pass.set_bind_group(1, &bind, &[]);
        pass.set_pipeline(&s.light);
        pass.set_stencil_reference(0);
        pass.set_vertex_buffer(0, s.cone_mesh.slice(..));
        pass.set_vertex_buffer(1, s.light_inst.slice(..(inst.len() * LIGHT_SIZE) as u64));
        pass.draw(0..180, 0..inst.len() as u32);
    }
    if !vehicles.cones.is_empty() && dbg != 274 {
        let n = vehicles.cones.len().min(64);
        gpu.queue.write_buffer(&s.cone_inst, 0, bytemuck::cast_slice(&vehicles.cones[..n]));
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("car cones"), layout: &s.layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&gpu.depth) }] });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("car cones"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }) }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &gpu.globals_bg, &[]);
        pass.set_bind_group(1, &bind, &[]);
        pass.set_pipeline(&s.cone);
        pass.set_stencil_reference(0);
        pass.set_vertex_buffer(0, s.cone_mesh.slice(..));
        pass.set_vertex_buffer(1, s.cone_inst.slice(..(n * CONE_SIZE) as u64));
        pass.draw(0..180, 0..n as u32);
    }
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
        if dbg != 271 && n < MAX_INST && !lights.iter().any(|l| l.0 == c.m) {
            let (width, length) = (c.max.x - c.min.x, c.max.z - c.min.z);
            let r = if c.ambient { length / 3.0 } else { 0.99 * width };
            // (the R8's measured span is 2.2318 - half the length of its lowest level of detail,
            // 4.4636 - against 2.149 from the model's box: the end spheres stood 4 cm short and the
            // blob's outer edge was a fifth too weak behind the car. Other cars: the box, (approx.))
            let span = if !c.ambient && (width - 1.9033).abs() < 0.003 && (length - 4.298).abs() < 0.02 { 2.2318 } else { length * 0.5 };
            if std::env::var("MCLA_GLOW_DUMP").is_ok() { eprintln!("blob body width {width:.4} length {length:.4} r {r:.4} span {span:.4}"); }
            let up = if c.ambient { 0.2 * r } else { 0.2326 * r };
            let foot = pos + y * c.ground_y;
            let mut first = foot + y * up - z * (span * 0.5);
            // (a traffic car's record, F-68081 -> F-18380, decoded 2026-10-29: L and H the length and
            // height (the bound box's max +16 minus min +32: the type's bound, `traffic::bound_box`, passed in as min / max); under 8.5 m r = L / 3 and the end spheres at L / 4 from the
            // car's ORIGIN along its z, else r = L / 4 and the ends at 0.33 L; both raised H / 4 along its y; the two
            // inner spheres at 1/3 and 2/3 between them (F-18380); the cylinder centred on the origin (F-35120 takes
            // the record's matrix), its length 2 r + the ends' distance. The truck's recorded blob: r 2.395, length
            // 8.383, centre = its origin. MCLA_BLOB_AMB_OLD=1: the fit as until then.)
            let (r, span, foot) = if c.ambient && std::env::var_os("MCLA_BLOB_AMB_OLD").is_none() {
                let h = c.max.y - c.min.y;
                let (r, k) = if length < 8.5 { (length / 3.0, 0.25) } else { (length / 4.0, 0.33) };
                first = pos + y * (h * 0.25) - z * (length * k);
                if std::env::var("MCLA_GLOW_DUMP").is_ok() { eprintln!("blob amb length {length:.4} height {h:.4} r {r:.4} span {:.4}", 2.0 * k * length); }
                (r, 2.0 * k * length, pos)
            } else {
                (r, span, foot)
            };
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

/// One record of the blob manager as F-18380 fills it (F-35120 draws it): the four sphere middles c0..c3 (w = r
/// squared), the volume's matrix (rows x, y, z, origin) and the colour; `light` = +4 (the light list, `quadlight`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadRec {
    pub c: [[f32; 4]; 4],
    pub m: [[f32; 4]; 4],
    pub colour: [f32; 4],
    pub light: bool,
}

/// What F-17121 reads of a car for its body record and its four neon records.
#[derive(Clone, Copy, Debug)]
pub struct NeonIn {
    /// the ground frame car+4512 (F-88547): rows x, y, z, origin
    pub g: [[f32; 4]; 4],
    /// the chassis bound's box, max (+16) minus min (+32) (F-98453 on (car+20)+8): W, H, L
    pub size: [f32; 4],
    /// F-63160's box, max minus min (x: the body record's radius, w: the off record's colour w)
    pub box_: [f32; 4],
    /// ExtNeonColor (the record's floats as stored), ExtNeonIntensity, the pulse car+404, car+268
    pub colour: [f32; 4],
    pub intensity: f32,
    pub pulse: f32,
    pub scale: f32,
    /// the body matrix's up row y ((car+20)+144, +20)
    pub up_y: f32,
    /// the ground probe's hit fraction (0.0 when no probe was made or it missed: always for the player's car)
    pub t: f32,
    /// D = 0.2 (not the garage, and the hour above +14568 or not at or above +14564) else 0.125
    pub night: bool,
    /// car+6477
    pub b6477: bool,
}

fn v4(f: impl Fn(usize) -> f32) -> [f32; 4] {
    [f(0), f(1), f(2), f(3)]
}
fn vmul(a: [f32; 4], k: f32) -> [f32; 4] {
    v4(|i| a[i] * k)
}
fn vmad(a: [f32; 4], k: f32, b: [f32; 4]) -> [f32; 4] {
    v4(|i| a[i] * k + b[i])
}

/// F-18380: c0 = A, c1 = A + (B - A) x 0.33333331, c2 = A + (B - A) x 0.66666663, c3 = B (xyz; w = r2), M copied.
fn quad_rec(a: [f32; 4], b: [f32; 4], m: [[f32; 4]; 4], r2: f32, colour: [f32; 4], light: bool) -> QuadRec {
    let d = v4(|i| b[i] - a[i]);
    let c1 = vmad(d, 0.333_333_31, a);
    let c2 = vmad(d, 0.666_666_63, a);
    let w = |v: [f32; 4]| [v[0], v[1], v[2], r2];
    QuadRec { c: [w(a), w(c1), w(c2), w(b)], m, colour, light }
}

/// F-17121's body record (car+560) and, when the neon is on, its four neon records (car+568), in the original's
/// f32 order (the vector products unfused, as the generated code's vmaddfp). Test `neon_records_match_the_original`.
pub fn neon_records(n: &NeonIn) -> (QuadRec, Option<[QuadRec; 4]>) {
    let [gx, gy, gz, g0] = n.g;
    let (w, h, l) = (n.size[0], n.size[1], n.size[2]);
    // f28 = sat(1 - t) by two fsel
    let f13 = 1.0 - n.t;
    let f12 = if f13 >= 0.0 { f13 } else { 0.0 };
    let f28 = if f12 - 1.0 >= 0.0 { 1.0 } else { f12 };
    // the body record: A, B = g0 +- gz L / 4 + gy H / 4; the frame's rows x 1.0, its origin g0
    let a = vmad(vmul(gy, h), 0.25, vmad(vmul(gz, l), 0.25, g0));
    let gzl = vmul(vmul(gz, l), 0.25);
    let b = vmad(vmul(gy, h), 0.25, v4(|i| g0[i] - gzl[i]));
    let m = [vmul(gx, 1.0), vmul(gy, 1.0), vmul(gz, 1.0), g0];
    // the strength k and the colour
    let f29 = (((n.up_y + 1.0) * 0.5) * n.scale) * f28;
    let f0 = f29 * if n.night { 0.2 } else { 0.125 };
    let k = (n.intensity * n.pulse) * f0;
    let col = vmul(n.colour, k);
    let x = n.box_[0] * 0.64;
    let r2 = (x * x) * f28;
    let on = n.intensity > 0.001 && !n.b6477;
    let body = if on { quad_rec(a, b, m, r2, col, true) } else { quad_rec(a, b, m, r2, [1.0, 1.0, 1.0, n.box_[3]], false) };
    if !on {
        return (body, None);
    }
    let rec = |p: [f32; 4], d: [f32; 4], ka: f32, kb: f32, mm: [[f32; 4]; 4]| {
        let mm = [mm[0], mm[1], mm[2], p];
        quad_rec(vmad(d, ka, p), vmad(d, kb, p), mm, 0.5, col, true)
    };
    // [0] / [1]: the sides, P = g0 + gx W x +-0.45, A / B = P + (gz L x 0.2) x +-0.5
    let r0 = rec(vmad(vmul(gx, w), 0.45, g0), vmul(vmul(gz, l), 0.2), 0.5, -0.5, m);
    let r1 = rec(vmad(vmul(gx, w), -0.45, g0), vmul(vmul(gz, l), 0.2), 0.5, -0.5, m);
    // the matrix turned by pi / 2 (F-25963: row0' = row0 cos - row2 sin, row2' = row0 sin + row2 cos)
    let ang = std::f32::consts::FRAC_PI_2 as f64;
    let (c, s) = (ang.cos() as f32, ang.sin() as f32);
    let t0 = v4(|i| m[0][i] * c - m[2][i] * s);
    let t2 = v4(|i| m[0][i] * s + m[2][i] * c);
    let mt = [t0, m[1], t2, g0];
    // [2] / [3]: the ends, P = g0 + gz L x -+0.5, A / B = P + (gx W x 0.2) x +-0.75
    let e2 = rec(vmad(vmul(gz, l), -0.5, g0), vmul(vmul(gx, w), 0.2), 0.75, -0.75, mt);
    let e3 = rec(vmad(vmul(gz, l), 0.5, g0), vmul(vmul(gx, w), 0.2), 0.75, -0.75, mt);
    (body, Some([r0, r1, e2, e3]))
}

#[cfg(test)]
mod neon_tests {
    use super::*;

    /// F-63160's box from the .r83 against the body quads' r2 recorded in the original (R8 3.5511, ZX-14 0.781).
    #[test]
    fn drawable_box_gives_the_recorded_radius() {
        let Ok(root) = std::env::var("MCLA_DATA_DIR") else { return };
        let root = std::path::PathBuf::from(root);
        for (car, r2) in [("vp_aud_r8_08", 3.5511f32), ("vp_kaw_ninja_zx14_06", 0.781)] {
            let x = drawable_box(&root, car).expect(car)[0];
            let got = (0.64 * x) * (0.64 * x);
            assert!((got - r2).abs() < 1e-3, "{car}: {got} vs {r2}");
        }
    }

    /// tools/neon_records_reference.py: F-17121 run offline, 1,000 random cars (frames, boxes, colours,
    /// intensities round the 0.001 gate, the hour round 6 and 18, the garage byte, the probe and its hit fraction).
    #[test]
    fn neon_records_match_the_original() {
        let txt = include_str!("testdata/neon_records_reference.txt");
        let f = |h: &str| f32::from_bits(u32::from_str_radix(h, 16).unwrap());
        let mut lines = txt.lines().peekable();
        let (mut cases, mut recs) = (0, 0);
        while let Some(l) = lines.next() {
            let Some(rest) = l.strip_prefix("CASE ") else { continue };
            let w: Vec<&str> = rest.split_whitespace().skip(1).collect();
            let u = |i: usize| u32::from_str_radix(w[i], 16).unwrap();
            let v: Vec<f32> = w[12..].iter().map(|h| f(h)).collect();
            let mut got = Vec::new();
            while let Some(n) = lines.peek() {
                if !n.starts_with("ADD ") {
                    break;
                }
                got.push(lines.next().unwrap().to_string());
            }
            cases += 1;
            if u(0) != 0 {
                assert!(got.is_empty());
                continue;
            }
            let probe = u(6) == 0 && f(w[1]) > 0.001 && u(9) != 0;
            let up = if u(5) != 0 { v[16 + 5] } else { v[5] };
            let hour = f(w[8]);
            let row = |o: usize| [v[o], v[o + 1], v[o + 2], v[o + 3]];
            let sub = |a: [f32; 4], b: [f32; 4]| v4(|i| a[i] - b[i]);
            let input = NeonIn {
                g: [row(116), row(120), row(124), row(128)],
                size: sub(row(104), row(108)),
                box_: sub(row(96), row(100)),
                colour: row(112),
                intensity: f(w[11]),
                pulse: f(w[3]),
                scale: f(w[2]),
                up_y: up,
                t: if probe { f(w[10]) } else { 0.0 },
                night: u(7) == 0 && (hour > 18.0 || !(hour >= 6.0)),
                b6477: u(4) != 0,
            };
            let (body, neon) = neon_records(&input);
            let mut want = vec![(512, body)];
            if let Some(n) = neon {
                want.extend([1024, 1184, 1344, 1504].into_iter().zip(n));
            }
            assert_eq!(got.len(), want.len(), "case {cases}");
            for (g, (off, r)) in got.iter().zip(&want) {
                let p: Vec<&str> = g.split_whitespace().collect();
                assert_eq!(p[1].parse::<i32>().unwrap(), *off, "case {cases}");
                let words: Vec<u32> = p[2..].iter().map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
                assert_eq!(words[1] >> 24 != 0, r.light, "case {cases} rec {off} flag");
                let mine: Vec<u32> = r.c.iter().chain(r.m.iter()).flatten().chain(r.colour.iter()).map(|x| x.to_bits()).collect();
                for (i, (a, b)) in words[4..].iter().zip(&mine).enumerate() {
                    let nan = f32::from_bits(*a).is_nan() && f32::from_bits(*b).is_nan();
                    assert!(a == b || nan, "case {cases} rec {off} word {} want {a:08x} got {b:08x}", i + 4);
                }
                recs += 1;
            }
        }
        assert!(cases > 900 && recs > 2000, "{cases} {recs}");
    }
}
