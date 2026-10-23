//! The slipstream trail (`vehicle/shared_utility/sst_trail`), drawn behind a car another car is drafting, as
//! the original draws it (decode: work/slipstream/trail/trail_rule.md; docs/VEHICLE_SIM.md slipstream section).
//!
//! - The points (mcCarModel +7200 block, update sub_82379D48 from the car's frame update sub_823567F8): 32 float4
//!   (+0, the program's skinningData) and 32 float2 drifts (+512). Each frame every point moves one place back
//!   (k <- k - 1, k = 31..1), rising 0.25 m/s (0x82008DD0) x dt and its w (the texture's v offset) growing by its
//!   drift's x x dt; point 0 = the car's place + 0.75 (0x8201CCA4) x its up row + 2.0 (0x82040E94) x its back
//!   (+z) row, w 0, with a new drift ((r - 0.5) x 0.1 (0x82004288), two 23-bit draws of the game's MWC 0x82860880,
//!   the first into y). The car's model makes them all 0 and the drifts (r - 0.5) x 5.0 (sub_82379A58). The "moved
//!   more than 0.05 m" test compares with +800, which only the model's set-up writes (-100000, -1e6, -1e6), so the
//!   points move every frame. The original's frame is 1/30 s (TIME +8 outside the tick loop); the port moves them
//!   every 1/30 s of the car's ticks with dt = 1/30 and puts point 0 on the drawn pose each frame.
//! - The draw (sub_82379B30, from the vehicle's alpha pass sub_82360518 after the car's own draws, unless the
//!   "nossttrail" switch or the renderer's garage flag): while the faded value (+780 = `sst.trail`) > 0.001
//!   (0x820022B4), the trail model with the identity matrix (the points are in the world), skinningData = the 32
//!   points, effectFade = the faded value, stencil test NOT EQUAL against the local player's car's stencil id + 1
//!   (states 20 / 24 / 23 / 25, kept: the trail is not drawn over that car) when it has one.
//! - SSTTrail.fx (cars/0xc53f6c0e, technique "draw"): SrcBlend SRCALPHA, DestBlend ONE, no Z write, alpha test
//!   alpha > 0, no culling; DiffuseSampler `__ssttrail` = sst_trail_tex.dds (cars' globaltex). VS_Trail and
//!   PS_Trail are in render/sst_trail.wgsl.

use crate::render::Gpu;
use glam::{Quat, Vec3};
use std::sync::Mutex;

/// The block's points.
pub const POINTS: usize = 32;
/// The original's frame (the update runs once a frame).
pub const FRAME: f32 = 1.0 / 30.0;
/// The faded value the draw needs (0x820022B4).
pub const DRAW_MIN: f32 = 0.001;
/// Most trails drawn in a frame (the original keeps at most the player's own and the one it drafts).
const MAX: usize = 8;

/// The game's shared random numbers (MWC 0x82860880, a = 0x5CDCFAA7) as the trail draws them. *(approx.)* the
/// original's generator is shared with everything else and seeded at run time: the port's draws are its own.
static RNG: Mutex<(u32, u32)> = Mutex::new((0x2545_F491, 1));

fn rand23() -> f32 {
    let mut s = RNG.lock().unwrap_or_else(|e| e.into_inner());
    let x = s.0 as u64 * 0x5CDC_FAA7 + s.1 as u64;
    *s = (x as u32, (x >> 32) as u32);
    (s.0 & 0x7F_FFFF) as f32 * (1.0 / 8_388_608.0)
}

/// One car's trail block (mcCarModel +7200).
#[derive(Clone, Debug)]
pub struct Trail {
    /// +0: the points (x, y, z, texture v offset), newest first
    pub pts: [[f32; 4]; POINTS],
    /// +512: each point's drift (x drives its w; y is carried along, never read)
    pub drift: [[f32; 2]; POINTS],
    /// the port's: car time since the last move (the original moves them once a frame)
    pub acc: f32,
}

impl Default for Trail {
    /// sub_82379A58 (from the model's set-up sub_8235AF90): points 0, drifts (r - 0.5) x 5.0 (0x82006BF8).
    fn default() -> Self {
        let mut drift = [[0.0f32; 2]; POINTS];
        for d in drift.iter_mut() {
            let (a, b) = (rand23(), rand23());
            *d = [(b - 0.5) * 5.0, (a - 0.5) * 5.0];
        }
        Trail { pts: [[0.0; 4]; POINTS], drift, acc: 0.0 }
    }
}

/// Point 0 for a car at `pos` turned by `rot`: + 0.75 x its up row + 2.0 x its +z (back) row.
pub fn head(pos: Vec3, rot: Quat) -> Vec3 {
    pos + rot * Vec3::Y * 0.75 + rot * Vec3::Z * 2.0
}

impl Trail {
    /// One frame of sub_82379D48 (`dt` = the frame's step): the points move one place back, then the new head.
    pub fn shift(&mut self, head: Vec3, dt: f32) {
        for k in (1..POINTS).rev() {
            let d = self.drift[k - 1];
            let mut p = self.pts[k - 1];
            p[1] += dt * 0.25;
            p[3] += d[0] * dt;
            self.pts[k] = p;
            self.drift[k] = d;
        }
        self.pts[0] = [head.x, head.y, head.z, 0.0];
        let (a, b) = (rand23(), rand23());
        self.drift[0] = [(b - 0.5) * 0.1, (a - 0.5) * 0.1];
    }
}

/// After each tick: every car with a slipstream turbo (the block works only with chassis +668) moves its points
/// once per 1/30 s of its time.
pub fn update(cars: &mut [crate::vehicles::CarInstance], dt: f32) {
    for c in cars.iter_mut() {
        if !crate::vehicles::Vehicles::has_sst(c) {
            continue;
        }
        let h = head(c.state.pos, c.state.rot);
        let t = &mut c.sst_trail;
        t.acc += dt;
        while t.acc >= FRAME * 0.999 {
            t.acc -= FRAME;
            t.shift(h, FRAME);
        }
    }
}

/// VS_Trail for one vertex (the mesh's x, y, its u = ring index, v): the world position, the texture
/// coordinates and the fade along the trail (render/sst_trail.wgsl does the same). Ring i sits on point i; its
/// sideways axis is horizontal, square to point i - point j (j = i - 1, for ring 0 point 1 with the sign turned).
pub fn vertex(pts: &[[f32; 4]; POINTS], x: f32, y: f32, u: f32, v: f32) -> (Vec3, [f32; 2], f32) {
    let i = (u.trunc() as i32).clamp(0, POINTS as i32 - 1) as usize;
    let (j, s) = if i == 0 { (1, -1.0) } else { (i - 1, 1.0) };
    let (p, q) = (pts[i], pts[j]);
    let d = Vec3::new(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
    let h2 = d.x * d.x + d.z * d.z;
    // (the program divides by the horizontal length; with none - two points on one vertical - it has none
    // either: the port keeps the ring on its point)
    let side = if h2 > 0.0 { glam::Vec2::new(d.z, -d.x) * (s / h2.sqrt()) } else { glam::Vec2::ZERO };
    let w = Vec3::new(p[0] + side.x * x, p[1] + y, p[2] + side.y * x);
    let f = if i == 0 { 0.0 } else { 1.0 - u * 0.03125 };
    (w, [u * 0.03125, v + p[3]], f)
}

/// PS_Trail's alpha (before the target's exponent bias, which takes gInvColorExpBias back; gLightAmbient.w 1).
pub fn alpha(tex_a: f32, fade: f32, along: f32) -> f32 {
    tex_a * 0.08 * fade * along
}

// ---------------------------------------------------------------------------------------
// GPU

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Inst {
    p: [[f32; 4]; POINTS],
    fade: [f32; 4],
}

struct State {
    pipe: crate::render::ms::MsPipe,
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
    ub: wgpu::Buffer,
    bg: wgpu::BindGroup,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Once, with the city's data: the model (sst_trail.mesh), the texture, the program.
pub fn load(gpu: &mut Gpu, data_dir: &str) {
    let root = std::path::Path::new(data_dir);
    let mesh = ["shared_utility", "0xd8bee6cc"].iter().find_map(|top| {
        let dir = root.join("vehicle").join(top).join("sst_trail");
        std::fs::read(dir.join("sst_trail.mesh")).or_else(|_| std::fs::read(dir.join(format!("0x{:08x}", mcla_formats::hash::joaat("sst_trail.mesh"))))).ok()
    });
    let Some((verts, idx)) = mesh.and_then(|m| crate::pu_fx::parse_mesh(&String::from_utf8_lossy(&m))) else {
        log::warn!("sst_trail: sst_trail.mesh missing");
        return;
    };
    let device = &gpu.device;
    use wgpu::util::DeviceExt;
    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("sst_trail vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("sst_trail ib"), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
    let base = root.join("textures/global/cars/globaltex");
    let tex = std::fs::read(base.join("sst_trail_tex.dds"))
        .or_else(|_| std::fs::read(base.join(format!("0x{:08x}", mcla_formats::hash::joaat("sst_trail_tex.dds")))))
        .ok()
        .and_then(|d| mcla_formats::dds::parse_dds("sst_trail_tex.dds", &d).ok());
    let view = match tex {
        Some(t) => gpu.upload_texture(mcla_formats::hash::joaat("sst_trail/sst_trail_tex.dds"), &t).texture.create_view(&Default::default()),
        None => {
            log::warn!("sst_trail: sst_trail_tex.dds missing");
            crate::render::create_rgba(&gpu.device, &gpu.queue, 1, 1, &[0, 0, 0, 0], "sst_trail").view
        }
    };
    let device = &gpu.device;
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("sst_trail"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sst_trail layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ],
    });
    let ub = device.create_buffer(&wgpu::BufferDescriptor { label: Some("sst_trail points"), size: (MAX * std::mem::size_of::<Inst>()) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sst_trail"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }, wgpu::BindGroupEntry { binding: 2, resource: ub.as_entire_binding() }],
    });
    let globals_layout = gpu.pipeline_particle.get_bind_group_layout(0);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sst_trail"), source: wgpu::ShaderSource::Wgsl(include_str!("render/sst_trail.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sst_trail pl"), bind_group_layouts: &[&globals_layout, &layout], push_constant_ranges: &[] });
    let pipe = crate::render::ms::MsPipe::new(device, move |device, v| {
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 1 },
        ];
        // (SrcBlend SRCALPHA, DestBlend ONE; the alpha test's alpha > 0 changes nothing under it)
        let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        // (stencil NOT EQUAL the player car's id, kept; the reference is set per frame - 255 when the player's car
        // has none, which no pixel holds)
        let face = wgpu::StencilFaceState { compare: wgpu::CompareFunction::NotEqual, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op: wgpu::StencilOperation::Keep };
        let stencil = if v.scene { wgpu::StencilState { front: face, back: face, read_mask: 0xff, write_mask: 0 } } else { Default::default() };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sst_trail"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_trail"), buffers: &[wgpu::VertexBufferLayout { array_stride: 32, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs }], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_trail"),
                targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::COLOR })],
                compilation_options: Default::default(),
            }),
            // (CullMode 0: both sides)
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            // (ZWriteEnable 0; reversed depth, tested)
            depth_stencil: Some(wgpu::DepthStencilState { format: v.depth_format(), depth_write_enabled: false, depth_compare: wgpu::CompareFunction::GreaterEqual, stencil, bias: Default::default() }),
            multisample: v.ms(),
            multiview: None,
            cache: None,
        })
    });
    *STATE.lock().unwrap() = Some(State { pipe, vb, ib, count: idx.len() as u32, ub, bg });
}

/// The trails drawn this frame: each car with a turbo whose faded value is over 0.001 and that is drawn, its
/// points with point 0 on the drawn pose.
fn instances(vehicles: &crate::vehicles::Vehicles) -> Vec<Inst> {
    if std::env::var("MCLA_NOSSTTRAIL").is_ok_and(|v| v.trim() == "1") {
        return Vec::new();
    }
    // (MCLA_SST_TRAIL_FORCE=1, to look at it: the player's car's trail drawn at full strength)
    let force = std::env::var("MCLA_SST_TRAIL_FORCE").is_ok_and(|v| v.trim() == "1");
    let mut out = Vec::new();
    for (ci, c) in vehicles.cars.iter().enumerate() {
        let Some(vs) = c.state.veh.as_ref() else { continue };
        let fade = if force && c.is_player { 1.0 } else { vs.sst.trail };
        if !(fade > DRAW_MIN) || !crate::vehicles::Vehicles::has_sst(c) || vehicles.lod_pick.get(ci).is_some_and(|&l| l < 0) {
            continue;
        }
        let pose = c.render_pose(vehicles.render_alpha);
        let mut p = c.sst_trail.pts;
        let h = head(pose.pos, pose.rot);
        p[0] = [h.x, h.y, h.z, 0.0];
        out.push(Inst { p, fade: [fade, 0.0, 0.0, 0.0] });
        if out.len() == MAX {
            break;
        }
    }
    out
}

/// The pass: over the scene after its draws (the cars' alpha pass draws it, after the car), before the particles.
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder, vehicles: &crate::vehicles::Vehicles) {
    let guard = STATE.lock().unwrap();
    let Some(s) = guard.as_ref() else { return };
    let list = instances(vehicles);
    // (MCLA_SST_TRAIL_LOG=1: each drawn frame's count of the scene frames and the trails' fade and head)
    if std::env::var("MCLA_SST_TRAIL_LOG").is_ok_and(|v| v.trim() == "1") {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if !list.is_empty() {
            log::info!("sst_trail: frame {n}: {} trails, first fade {:.3} head {:?} tail {:?}", list.len(), list[0].fade[0], list[0].p[0], list[0].p[POINTS - 1]);
        }
    }
    if list.is_empty() {
        return;
    }
    gpu.queue.write_buffer(&s.ub, 0, bytemuck::cast_slice(&list));
    // (the local player's car's stencil id: the original's 0x827E7628 = that car's +6028 + 1, the id its body
    // draws with - the port's car keeps one id for all its draws)
    let reference = vehicles.cars.iter().position(|c| c.is_player).map(crate::render::mb::stencil_of).filter(|&r| r != 0).unwrap_or(255);
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("sst trail"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: None, stencil_ops: None }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_bind_group(0, &gpu.globals_bg, &[]);
    pass.set_pipeline(&s.pipe);
    pass.set_bind_group(1, &s.bg, &[]);
    pass.set_stencil_reference(reference);
    pass.set_vertex_buffer(0, s.vb.slice(..));
    pass.set_index_buffer(s.ib.slice(..), wgpu::IndexFormat::Uint32);
    pass.draw_indexed(0..s.count, 0, 0..list.len() as u32);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VS_Trail's outputs against the program's own microcode run word by word (work/slipstream/trail/vsrun.py,
    /// a0-relative constants): the points 10 + 0.7 i ..., four of the mesh's vertices.
    #[test]
    fn sst_trail_vertex_matches_microcode() {
        let pts = microcode_points();
        let cases: [((f32, f32, f32, f32), [f32; 3], [f32; 2], f32); 4] = [
            ((-0.1953125, -0.24511719, 31.0, 0.0), [31.646268, 1.064883, 35.525533], [0.96875, 0.02405], 0.03125),
            ((0.70703125, 0.17675781, 0.0, 0.59375), [10.447766, 1.176758, -5.244476], [0.0, 0.620127], 0.0),
            ((-1.0, 0.0, 1.0, 0.25), [9.761867, 1.01, -3.159489], [0.03125, 0.244949], 0.96875),
            ((0.383, 0.231, 16.0, 0.53125), [21.656605, 1.391, 15.664825], [0.5, 0.511587], 0.5),
        ];
        for ((x, y, u, v), w, uv, f) in cases {
            let (pw, puv, pf) = vertex(&pts, x, y, u, v);
            assert!((pw - Vec3::from(w)).abs().max_element() < 2e-4, "{pw} vs {w:?}");
            assert!((puv[0] - uv[0]).abs() < 1e-5 && (puv[1] - uv[1]).abs() < 1e-5, "{puv:?} vs {uv:?}");
            assert!((pf - f).abs() < 1e-6);
        }
    }

    fn microcode_points() -> [[f32; 4]; POINTS] {
        MICRO_PTS
    }

    /// the 32 points vsrun.py makes (random.seed(1): x 10 + 0.7 i +- 0.2, y 1 + 0.01 i, z -5 + 1.3 i +- 0.2, w +- 0.05)
    const MICRO_PTS: [[f32; 4]; POINTS] = [
        [9.8537457, 1.0, -4.8610265, 0.0263775],
        [10.6020276, 1.01, -3.701826, -0.0050509],
        [11.4606372, 1.02, -2.2845107, -0.040614],
        [11.911339, 1.03, -0.965694, -0.0067233],
        [12.904912, 1.04, 0.0008424, -0.0054613],
        [13.588616, 1.05, 1.3915049, 0.0445271],
        [14.360571, 1.06, 2.612236, -0.0474554],
        [14.916565, 1.07, 4.2756597, -0.0118796],
        [15.4866398, 1.08, 5.3688466, -0.0470959],
        [16.1886767, 1.09, 6.675155, -0.0004188],
        [16.8932338, 1.1, 7.8923466, -0.0281219],
        [17.6838414, 1.11, 9.2159126, -0.047851],
        [18.5350312, 1.12, 10.6225817, 0.0142294],
        [18.9743625, 1.13, 12.0970174, 0.0359947],
        [19.648356, 1.14, 13.1330781, 0.0221484],
        [20.5844767, 1.15, 14.6745762, -0.0077893],
        [21.3320143, 1.16, 15.8681222, -0.0196631],
        [21.9350322, 1.17, 17.2529916, 0.0346197],
        [22.6021135, 1.18, 18.4356009, -0.0465474],
        [23.197096, 1.19, 19.8189617, -0.0085686],
        [23.869203, 1.2, 21.0195195, 0.0203041],
        [24.7697943, 1.21, 22.2498812, -0.0061038],
        [25.4033706, 1.22, 23.711377, 0.0020938],
        [26.057302, 1.23, 24.8958774, -0.0470425],
        [26.6173949, 1.24, 26.2813528, 0.0483188],
        [27.5372735, 1.25, 27.4574399, -0.0329651],
        [28.2008954, 1.26, 28.9928307, 0.0270523],
        [28.915847, 1.27, 30.2441159, -0.0267824],
        [29.6055087, 1.28, 31.580987, 0.0077795],
        [30.2836527, 1.29, 32.6077118, 0.0047996],
        [31.1828465, 1.3, 33.8022837, 0.0283655],
        [31.8281944, 1.31, 35.4544718, 0.0240503],
    ];

    #[test]
    fn sst_trail_shift_moves_points_back() {
        let mut t = Trail { pts: [[0.0; 4]; POINTS], drift: [[0.0; 2]; POINTS], acc: 0.0 };
        t.drift[0] = [0.03, -0.02];
        t.pts[0] = [1.0, 2.0, 3.0, 0.0];
        t.shift(Vec3::new(5.0, 6.0, 7.0), FRAME);
        // point 1 = old point 0, 0.25 m/s up, w + drift.x dt; drift carried
        assert_eq!(t.pts[0], [5.0, 6.0, 7.0, 0.0]);
        assert!((t.pts[1][1] - (2.0 + 0.25 * FRAME)).abs() < 1e-6 && (t.pts[1][3] - 0.03 * FRAME).abs() < 1e-7);
        assert_eq!(t.drift[1], [0.03, -0.02]);
        assert!(t.drift[0][0].abs() <= 0.05 && t.drift[0][1].abs() <= 0.05);
        // head: 0.75 up, 2.0 back (+z)
        assert_eq!(head(Vec3::ZERO, Quat::IDENTITY), Vec3::new(0.0, 0.75, 2.0));
        assert_eq!(alpha(1.0, 1.0, 1.0), 0.08);
    }
}
