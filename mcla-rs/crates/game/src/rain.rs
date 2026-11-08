//! Rain as the original draws it (LIGHTING_WORK.md, "Rain as the original draws it"):
//!
//! - `mcRain`: 8192 particles simulated on the GPU by the effect `rain_system`
//!   (ParticleUpdatePS of `shaders/effects/fxl_final/0x9e307cae`), here on the CPU with the
//!   same rule and the same numbers (`tune/rmptfx/raintuning.xml`; acceleration and step are
//!   the executable's own: (0, -0.98, 0) and 0.05 an update). A drop is born in a box that
//!   rides ahead of the camera, falls with the wind, and on the ground plays the four frames
//!   of the splash picture before it is born again.
//! - the street lamps' cones of light in the rain (`rage_spotlightfog`, ps_mainFast): one cone
//!   a lamp, only while it rains and the lamps are on; density and strength are the storm's
//!   `m_LampGlows` (TODSunStormy).
//!
//! Not from the original: what stops a drop is the collision's highest face at its place, looked
//! up once when it is born (the original tests against a depth picture taken from above every
//! update).
//!
//! The four point lights (PointLights / PointLightColors): AddGlow's list of the four glows nearest the
//! camera (halo.rs `rain_take`, F-92524 / F-58928; in the recordings the player's tail lamps,
//! reach 9 cm).
//!
//! `MCLA_DEBUG_VIEW=97`: the older stand-in (game.rs) instead. `MCLA_RAIN_CONES=0`,
//! `MCLA_RAIN_DROPS=0`, `MCLA_RAIN_SPOT=0`: without the cones / the drops / the drops' spot.

use crate::props::PropLamp;
use crate::render::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use std::sync::Mutex;

/// What F-19533 sends the cone shader for one lamp (the shader variables in the order it sets them; LightInvMatrix,
/// the general 4x4 inverse F-25750 of `matrix`, is left out).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeSetup {
    /// LightMatrix rows: the two side axes x tan x length, the direction x length, the apex (w 1); identity when the
    /// length is not above 0
    pub matrix: [[f32; 4]; 4],
    /// LightPosition: the apex (the lamp pulled back along its direction by the pass's +52), w the edge's offset
    pub pos: [f32; 4],
    /// LightColor: the lamp's colour, w = 50 x the colour weight
    pub colour: [f32; 4],
    /// LightDirection: the lamp's direction, w the edge's factor
    pub dir: [f32; 4],
    /// LightVolume: (density, strength x fade, 0.2 / length, colour weight)
    pub volume: [f32; 4],
    /// LightFallOff
    pub falloff: f32,
    /// the cone's length (F-19533's min(sqrt(luma x intensity / edge), longest))
    pub len: f32,
    /// the half width per metre (F-46735 on the copy)
    pub tan: f32,
}

/// The faded strength the cone pass F-71942 hands F-19533 for a listed lamp `d` metres off with the entry's glow factor
/// (+112): ((1 - max((d - (300 - 50)) / 50, 0)) x factor) x min(max((d - 20) / 10, 0), 1) x strength, in that order
/// (a NaN on the way in stays in the far term, the near term takes 0). Lamps nearer than 20 m are skipped by the caller;
/// beyond 250 x 0.3 = 75 m the pass draws its second (far) cone mesh. `cone_fade_matches_the_original`.
pub fn cone_fade(d: f32, factor: f32, strength: f32) -> f32 {
    let (near, far) = ((d - 20.0) / 10.0, (d - (300.0f32 - 50.0)) / 50.0);
    let near = if near >= 0.0 { near } else { 0.0 };
    let far = if -far >= 0.0 { 0.0 } else { far };
    let near = if near - 1.0 >= 0.0 { 1.0 } else { near };
    (1.0 - far) * factor * near * strength
}

/// F-19533 for one lamp (pos, colour, dir as the light record's vectors; intensity, ConeAngle, Penumbra), the pass's
/// pull-back (+52, the original writes 0.2 there after each lamp), longest cone (+56) and edge brightness (+64), and
/// its three arguments (density, strength x fade, colour weight). The copy F-10025 / set F-25852: cone x 1.077,
/// penumbra + a global that is never written (0); type 1 while the cone is at most 359.99991. The edge F-51633 with
/// the library cos; the luma fused 0.3 / 0.6 / 0.1; the axes by the vector unit (cross with (1, 0, 0), exact 1 / sqrt,
/// the products not fused), the half width F-46735. Checked against F-19533 run offline on 1,500 lamps
/// (`tools/cone_pass_reference.py`, test `cone_setup_matches_the_original`).
pub fn cone_setup(pos: [f32; 4], colour: [f32; 4], dir: [f32; 4], intensity: f32, cone: f32, penumbra: f32, pass: [f32; 3], args: [f32; 3]) -> ConeSetup {
    use crate::physics::exact::{dot3, fma, guest_cos};
    let [pull, longest, edge] = pass;
    let apex: [f32; 4] = std::array::from_fn(|i| pos[i] - dir[i] * pull);
    let (c, p) = (cone * 1.077, penumbra + 0.0);
    let kind = if c > 359.999_908_4 { 0 } else { 1 };
    // (F-51633: the edge from the outer and inner angles, x 1/720 x 6.28)
    let (mut outer, mut inner_add) = (c * 0.001_388_888_9 * 6.28, p * 0.001_388_888_9 * 6.28);
    if !(p >= 0.0) {
        outer = inner_add + outer;
        inner_add = -inner_add;
    }
    let cos_in = guest_cos((inner_add + outer) as f64) as f32;
    let cos_out = guest_cos(outer as f64) as f32;
    let k = 1.0 / (cos_in - cos_out);
    let (dir_w, pos_w) = (-k, 1.0 - -(k * cos_out));
    let luma = fma(colour[2], 0.1, fma(colour[1], 0.6, colour[0] * 0.3));
    let root = (luma * intensity / edge).sqrt();
    let len = if longest - root >= 0.0 { root } else { longest };
    let tan = crate::props::light_cone_tan(kind, c, p);
    let mut matrix = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    if len > 0.0 {
        let d = Vec3::new(dir[0], dir[1], dir[2]);
        // (vnmsubfp: -(a b - c), the product rounded)
        let side = Vec3::new(-(0.0 * d.z - 0.0 * d.y), -(0.0 * d.x - 1.0 * d.z), -(1.0 * d.y - 0.0 * d.x));
        let a = side * (1.0 / dot3(side, side).sqrt());
        let b = Vec3::new(-(d.y * a.z - d.z * a.y), -(d.z * a.x - d.x * a.z), -(d.x * a.y - d.y * a.x));
        let tl = tan * len;
        let b = b * (1.0 / dot3(b, b).sqrt()) * tl;
        let a = a * tl;
        matrix = [[a.x, a.y, a.z, 0.0], [b.x, b.y, b.z, 0.0], [d.x * len, d.y * len, d.z * len, 0.0], [apex[0], apex[1], apex[2], 1.0]];
    }
    let [density, strength, colw] = args;
    ConeSetup {
        matrix,
        pos: [apex[0], apex[1], apex[2], pos_w],
        colour: [colour[0], colour[1], colour[2], colw * 50.0],
        dir: [dir[0], dir[1], dir[2], dir_w],
        volume: [density, strength, 0.2 / len, colw],
        falloff: 1.4,
        len,
        tan,
    }
}

/// particles of the original (a 128 x 64 target, 32768 vertices a draw)
const DROPS: usize = 8192;
/// most cones a frame: the original's list holds 256 (F-59499; it drew 79 at the reference spot,
/// lamps up to 235 m away)
const CONES: usize = 256;
/// how far a lamp's cone is drawn
const CONE_REACH: f32 = 300.0;
/// vertices of a cone (render/rain.wgsl CONE_SEGS x 3)
const CONE_VERTS: u32 = 72;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DropInst {
    pos_state: [f32; 4],
    vel: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ConeInst {
    apex_h: [f32; 4],
    dir_tan: [f32; 4],
    color_fade: [f32; 4],
    lamp: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RainU {
    global_light: [f32; 4],
    scale_offset: [f32; 4],
    tune: [f32; 4],
    spot_pos: [f32; 4],
    spot_dir: [f32; 4],
    spot_params: [f32; 4],
    spot_color: [f32; 4],
    fade_sphere: [f32; 4],
    lights: [[f32; 4]; 4],
    light_cols: [[f32; 4]; 4],
}

#[derive(Clone, Copy)]
struct Drop {
    pos: Vec3,
    /// 1.25 falling; 1 .. 0 on the ground (0.75 .. 0: the splash's frames); below 0 dead
    state: f32,
    vel: Vec3,
    /// the height it lands on (the first surface from above at its place)
    floor: f32,
}

/// What a frame hands over.
pub struct Frame<'a> {
    pub cam_pos: [f32; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    /// the weather's rain, 0 .. 1
    pub rain: f32,
    /// m_KeyColor + m_KeyAmbLight of the hour, and m_NFLFade
    pub light: Option<(Vec3, f32)>,
    /// the player's car (on the ground), if there is one
    pub car: Option<Vec3>,
    pub lamps: &'a [PropLamp],
    /// the weather as GlowTuning.xml counts it (0 nice, 1 cloudy, 2 stormy, 3 foggy) and the hour
    pub weather: usize,
    /// the weather being blended towards and how far (the light manager's weighted sum of the four
    /// weathers' m_LampGlows, which feeds the cone pass of F-59499)
    pub next: Option<(usize, f32)>,
    pub hour: f32,
    /// the city's collision, for what is over a drop's place
    pub collider: Option<&'a crate::physics::collision::WorldCollider>,
}

struct Rain {
    tune: std::collections::HashMap<String, f32>,
    /// m_LampGlows of TODSunNice / Cloudy / Stormy / Foggy: density, strength, the brightness a
    /// cone ends at, LightColor.w / 50
    lamp_glows: [[f32; 4]; 4],
    drops: Vec<Drop>,
    seed: u32,
    raining: bool,
    last: Option<std::time::Instant>,
    acc: f32,
    last_cam: Vec3,
    drop_buf: wgpu::Buffer,
    cone_buf: wgpu::Buffer,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    drop_pipeline: crate::render::ms::MsPipe,
    cone_pipeline: crate::render::ms::MsPipe,
    drop_count: u32,
    cone_count: u32,
    dumped: bool,
}

static STATE: Mutex<Option<Rain>> = Mutex::new(None);

/// MCLA_DEBUG_VIEW=97: the stand-in of before (lines and rings drawn over the frame).
pub fn old_look() -> bool {
    static OLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OLD.get_or_init(|| std::env::var("MCLA_DEBUG_VIEW").map_or(false, |v| v.trim() == "97"))
}

fn env_off(name: &str) -> bool {
    std::env::var(name).map_or(false, |v| v.trim() == "0")
}

/// The storm's `m_LampGlows` (one key: LightVolume.x and the most of LightVolume.y of the cones).
fn lamp_glows(data_dir: &str, name: &str) -> Option<[f32; 4]> {
    for e in std::fs::read_dir(std::path::Path::new(data_dir).join("tune/lighting")).ok()?.flatten() {
        let Ok(x) = std::fs::read_to_string(e.path()) else { continue };
        if !x.contains(&format!("<szName>{name}</szName>")) {
            continue;
        }
        let i = x.find("<m_LampGlows>")?;
        let rest = &x[i..];
        let a = rest.find("<KeyData")?;
        let a = a + rest[a..].find('>')? + 1;
        let b = a + rest[a..].find("</KeyData>")?;
        let v: Vec<f32> = rest[a..b].split_whitespace().filter_map(|t| t.parse().ok()).collect();
        // (time, four channels)
        if v.len() >= 5 {
            return Some([v[1], v[2], v[3], v[4]]);
        }
    }
    None
}

pub fn load(gpu: &mut Gpu, data_dir: &str) {
    let mut tune = std::collections::HashMap::new();
    if let Ok(t) = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/rmptfx/raintuning.xml")) {
        for l in t.lines() {
            let mut it = l.split_whitespace();
            if let (Some(k), Some(v)) = (it.next(), it.next().and_then(|v| v.parse::<f32>().ok())) {
                tune.insert(k.to_string(), v);
            }
        }
    }
    let tex = |gpu: &mut Gpu, name: &str| {
        let p = std::path::Path::new(data_dir).join("textures/effects").join(name);
        match std::fs::read(&p).ok().and_then(|d| mcla_formats::dds::parse_dds(name, &d).ok()) {
            // (the programs read them gamma decoded, like every picture of the frame)
            Some(t) => gpu.upload_texture(mcla_formats::hash::joaat(&format!("effects/{}", name)), &t),
            None => {
                log::warn!("rain texture {} missing", name);
                gpu.white.clone()
            }
        }
    };
    let (streak, splash) = (tex(gpu, "rain.dds"), tex(gpu, "rainanim.dds"));
    let device = &gpu.device;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("rain layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ],
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("rain uniform"), size: std::mem::size_of::<RainU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("rain sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rain"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&streak.view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&splash.view) },
            wgpu::BindGroupEntry { binding: 3, resource: uniform.as_entire_binding() },
        ],
    });
    // (the frame's globals: the particles' pipeline has their layout)
    let globals_layout = gpu.pipeline_particle.get_bind_group_layout(0);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("rain"), source: wgpu::ShaderSource::Wgsl(include_str!("render/rain.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("rain pl"), bind_group_layouts: &[&globals_layout, &layout], push_constant_ranges: &[] });
    const DROP_ATTRS: [wgpu::VertexAttribute; 2] = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 1 },
    ];
    const CONE_ATTRS: [wgpu::VertexAttribute; 4] = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 1 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 2 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 48, shader_location: 3 },
    ];
    // (the original: blend control 0x07060706 = source alpha / one minus source alpha, depth
    // tested and not written - register 0x2200 = 0x00700762)
    let make = |vs: &'static str, fs: &'static str, attrs: &'static [wgpu::VertexAttribute], stride: u64, topology: wgpu::PrimitiveTopology| {
        let (pl, shader) = (pl.clone(), shader.clone());
        crate::render::ms::MsPipe::new(device, move |device, v| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rain"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), buffers: &[wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Instance, attributes: attrs }], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fs),
                targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology, cull_mode: None, ..Default::default() },
            depth_stencil: Some(v.ds(false, wgpu::CompareFunction::GreaterEqual, false, Default::default())),
            multisample: v.ms(),
            multiview: None,
            cache: None,
        }))
    };
    let drop_pipeline = make("vs_drop", "fs_drop", &DROP_ATTRS, std::mem::size_of::<DropInst>() as u64, wgpu::PrimitiveTopology::TriangleStrip);
    let cone_pipeline = make("vs_cone", "fs_cone", &CONE_ATTRS, std::mem::size_of::<ConeInst>() as u64, wgpu::PrimitiveTopology::TriangleList);
    let buf = |label: &str, size: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let rain = Rain {
        lamp_glows: [
            lamp_glows(data_dir, "TODSunNice").unwrap_or([0.027265, 0.12641, 0.260257, 0.479203]),
            lamp_glows(data_dir, "TODSunCloudy").unwrap_or([0.027, 0.124, 0.313, 0.479]),
            lamp_glows(data_dir, "TODSunStormy").unwrap_or([0.114, 0.29, 0.1605, 0.15]),
            lamp_glows(data_dir, "TODSunFoggy").unwrap_or([0.114224, 0.293103, 0.3125, 0.152004]),
        ],
        tune,
        drops: vec![Drop { pos: Vec3::ZERO, state: -0.25, vel: Vec3::ZERO, floor: 0.0 }; DROPS],
        seed: 0x9E37_79B9,
        raining: false,
        last: None,
        acc: 0.0,
        last_cam: Vec3::ZERO,
        drop_buf: buf("rain drops", DROPS * std::mem::size_of::<DropInst>()),
        cone_buf: buf("rain cones", CONES * std::mem::size_of::<ConeInst>()),
        uniform,
        bind,
        drop_pipeline,
        cone_pipeline,
        drop_count: 0,
        cone_count: 0,
        dumped: false,
    };
    *STATE.lock().unwrap() = Some(rain);
}

impl Rain {
    fn t(&self, k: &str, d: f32) -> f32 {
        self.tune.get(k).copied().unwrap_or(d)
    }

    fn rnd(&mut self) -> f32 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) as f32 / 16_777_216.0
    }

    /// One update of every particle (ParticleUpdatePS). `back`: the camera's z axis (it looks
    /// the other way), `limit`: a new drop lives if its three numbers add up to more.
    fn tick(&mut self, cam: Vec3, cam_step: Vec3, back: Vec3, ground: f32, limit: f32, collider: Option<&crate::physics::collision::WorldCollider>) {
        // Acceleration_Delta: the executable's own (0, -0.98, 0) and 0.05 a frame
        let (acc, dt) = (Vec3::new(0.0, -0.98, 0.0), 0.05);
        let emit = Vec3::new(self.t("EmitterPosX", -17.0), self.t("EmitterPosY", 6.0), self.t("EmitterPosZ", -29.2));
        let range = Vec3::new(self.t("EmitterPosRangeX", 39.5), self.t("EmitterPosRangeY", -1.8), self.t("EmitterPosRangeZ", 37.8));
        let ahead = self.t("EmitterCamdistance", -16.0);
        let wind = Vec3::new(self.t("WindX", 0.0), self.t("WindY", -7.25), self.t("WindZ", 0.0));
        let gust = Vec3::new(self.t("WindRandomRangeX", 4.0), self.t("WindRandomRangeY", -11.2), self.t("WindRandomRangeZ", 0.0));
        let splash_up = self.t("SplashOffset", 0.068);
        for i in 0..self.drops.len() {
            let mut d = self.drops[i];
            if d.state < 0.0 {
                // born again: four numbers of the noise picture place it and set its speed
                // (the same number for x and the fall, for z and the drift - as the program has it)
                let n = [self.rnd(), self.rnd(), self.rnd(), self.rnd()];
                d.vel = wind + acc + Vec3::new(n[1] * gust.x, n[3] * gust.y, n[2] * gust.z);
                d.pos = cam + emit + Vec3::new(n[3] * range.x, n[2] * range.y, n[1] * range.z) + Vec3::new(back.x, 0.0, back.z) * ahead + cam_step;
                d.state = if n[0] + n[1] + n[2] > limit { 1.25 } else { 0.0 };
                // (the original tests every update against a depth picture of the place taken from
                // 58 m over the camera, 122 m deep: whatever is highest there stops the drop - a roof
                // over it at once. Here: the collision's first face from that height, once a drop.)
                d.floor = collider.and_then(|c| c.ground(Vec3::new(d.pos.x, cam.y, d.pos.z), 58.0, 64.0)).map_or(ground, |h| h.point.y);
            }
            if d.pos.y <= d.floor || d.state < 1.25 {
                // on the ground: it stays, one step of the splash an update
                d.vel = Vec3::ZERO;
                d.state -= 0.25;
                d.pos.y = d.floor + splash_up;
            } else {
                d.pos += d.vel * dt;
                d.vel = (d.vel + acc * dt) * 0.99995;
            }
            self.drops[i] = d;
        }
    }

    /// The lamps' cones (rage_spotlightfog) as the game lists and draws them (F-59499,
    /// F-71942, F-19533; LIGHTING_WORK.md "Lamps, round 2"): none at all unless the
    /// weather's m_LampGlows x times y reaches 0.01 (storm 0.033, fog 0.033, nice and cloudy
    /// 0.003 - recorded: 68 cones a frame on a stormy and on a foggy night, none on a nice or
    /// cloudy one or by day); a cone for every LIT spot lamp 20 .. 300 m from the camera,
    /// LightVolume.y = y x sat((d - 20) / 10) x (1 - max((d - 250) / 50, 0)); its length is where
    /// the lamp's brightness (0.3 r + 0.6 g + 0.1 b) x Intensity / d^2 falls to m_LampGlows.z, at
    /// most 16 m (storm: 16 m but two kinds of 10.72 and 12.87; fog 7.68 .. 14.44).
    fn cones(&mut self, gpu: &Gpu, f: &Frame) {
        let mut lg = self.lamp_glows[f.weather.min(3)];
        if let Some((nw, k)) = f.next {
            let b = self.lamp_glows[nw.min(3)];
            for i in 0..4 {
                lg[i] += (b[i] - lg[i]) * k;
            }
        }
        let [density, strength, edge, colw] = lg;
        if density * strength < 0.01 {
            return;
        }
        let cam = Vec3::new(f.cam_pos[0], f.cam_pos[1], f.cam_pos[2]);
        let old = std::env::var("MCLA_DEBUG_VIEW").map_or(false, |v| v.trim() == "116");
        let mut cones: Vec<(f32, ConeInst)> = Vec::new();
        for l in f.lamps {
            let d = l.pos.distance(cam);
            // (only spot lights are listed; a light file without a cone gives [0, 1])
            if d < 20.0 || d >= CONE_REACH || l.cone == [0.0, 1.0] || !crate::halo::lamp_lit(l.pos, f.hour) {
                continue;
            }
            // (the entry's glow factor +112: 1 for the listed lamps, F-59499)
            let fade = cone_fade(d, 1.0, strength);
            if fade <= 0.0 {
                continue;
            }
            let dir = -l.back.normalize_or_zero();
            // (F-19533 for the lamp - rain::cone_setup: the apex pulled back 0.2 m along the light (the pass's +52, which it
            // sets to 0.2 after each lamp), the length min(sqrt(luma x Intensity / m_LampGlows.z), 16), the half width
            // F-46735, the edge cos x LightDirection.w + LightPosition.w from the lamp's own ConeAngle and Penumbra;
            // a length not above 0 = the identity LightMatrix = nothing drawn. MCLA_DEBUG_VIEW=116: 16 m and 2.388 for
            // every lamp, as before.)
            // (the lamp as the light table gives it back - packed by F-49342, fetched by F-31785: half floats for the
            // direction, colour, cone and intensity, the penumbra a byte (-50 -> -50.5), the place shorts on the lamps' box)
            let (h, hc, hd, at) = (crate::props::table_half, l.color, dir, crate::props::table_place(l.pos));
            let s = cone_setup([at.x, at.y, at.z, 0.0], [h(hc.x), h(hc.y), h(hc.z), 0.0], [h(hd.x), h(hd.y), h(hd.z), 0.0], h(l.intensity), h(l.cone_deg[0]), crate::props::table_penumbra(l.cone_deg[1]), [0.2, 16.0, edge], [density, fade, colw]);
            let (len, tan) = if old { (16.0, 38.21 / 16.0) } else { (s.len, s.tan) };
            if !(len > 0.0) {
                continue;
            }
            cones.push((d, ConeInst { apex_h: [s.pos[0], s.pos[1], s.pos[2], len], dir_tan: [dir.x, dir.y, dir.z, tan], color_fade: [l.color.x, l.color.y, l.color.z, fade], lamp: [s.colour[3], s.dir[3], s.pos[3], density] }));
        }
        cones.sort_by(|a, b| b.0.total_cmp(&a.0));
        let skip = cones.len().saturating_sub(CONES);
        let insts: Vec<ConeInst> = cones[skip..].iter().map(|c| c.1).collect();
        if !self.dumped && std::env::var("MCLA_RAIN_DUMP").is_ok() {
            self.dumped = true;
            eprintln!("rain: {} cones (of {} lamps), weather {}, hour {}, camera {:?}", insts.len(), f.lamps.len(), f.weather, f.hour, cam);
            for c in cones.iter().rev().take(8) {
                eprintln!("  cone {:.1} m at {:?} len {:.2} dir {:?} colour {:?}", c.0, &c.1.apex_h[..3], c.1.apex_h[3], &c.1.dir_tan[..3], c.1.color_fade);
            }
        }
        self.cone_count = insts.len() as u32;
        if !insts.is_empty() {
            gpu.queue.write_buffer(&self.cone_buf, 0, bytemuck::cast_slice(&insts));
        }
    }

    fn prepare(&mut self, gpu: &Gpu, f: &Frame) {
        self.drop_count = 0;
        self.cone_count = 0;
        if !old_look() && !env_off("MCLA_RAIN_CONES") {
            self.cones(gpu, f);
        }
        // (mcRain: nothing at or below Rainthreshold)
        let thr = self.t("Rainthreshold", 0.00855);
        if old_look() || f.rain <= thr {
            self.raining = false;
            self.last = None;
            return;
        }
        let limit = (1.0 - (f.rain - thr) / (1.0 - thr)) * 2.5;
        let cam = Vec3::new(f.cam_pos[0], f.cam_pos[1], f.cam_pos[2]);
        // the camera's forward: a point on the middle ray
        let m = Mat4::from_cols_array_2d(&f.inv_view_proj);
        let q = m * Vec4::new(0.0, 0.0, 0.5, 1.0);
        let fwd = if q.w.abs() > 1e-9 { (q.truncate() / q.w - cam).normalize_or_zero() } else { Vec3::NEG_Z };
        let back = -fwd;
        let ground = f.car.map_or(cam.y - 2.0, |c| c.y);
        let now = std::time::Instant::now();
        if !self.raining {
            // a storm that is already there: let the particles spread over their fall
            for d in self.drops.iter_mut() {
                d.state = -0.25;
            }
            for k in 0..400 {
                // (not all born in the same update)
                if k < 24 {
                    let n = self.drops.len();
                    for d in self.drops[k * n / 24..].iter_mut() {
                        d.state = -0.25;
                    }
                }
                self.tick(cam, Vec3::ZERO, back, ground, limit, f.collider);
            }
            self.raining = true;
            self.acc = 0.0;
            self.last_cam = cam;
        } else if let Some(last) = self.last {
            // the original updates once a frame at 30 frames a second
            self.acc += (now - last).as_secs_f32().min(0.25) * 30.0;
            let mut n = 0;
            while self.acc >= 1.0 && n < 4 {
                let step = cam - self.last_cam;
                self.last_cam = cam;
                self.tick(cam, if step.length() < 20.0 { step } else { Vec3::ZERO }, back, ground, limit, f.collider);
                self.acc -= 1.0;
                n += 1;
            }
            self.acc = self.acc.min(1.0);
        }
        self.last = Some(now);
        let night = f.light.map_or(false, |l| l.1 > 0.0);
        let (key, _) = f.light.unwrap_or((Vec3::splat(0.5), 0.0));
        let ambient = key + Vec3::new(self.t("RainAmbientRed", 0.292), self.t("RainAmbientGreen", 0.291), self.t("RainAmbientBlue", 0.293));
        let spot_on = !env_off("MCLA_RAIN_SPOT");
        let car = f.car.unwrap_or(Vec3::ZERO);
        let (lights, light_cols) = crate::halo::rain_take();
        if std::env::var("MCLA_RAIN_DUMP").is_ok() {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 100 == 99 {
                for (l, c) in lights.iter().zip(light_cols.iter()) {
                    eprintln!("  rain light {:.3} {:.3} {:.3} w {:.5} colour {:.5} {:.5} {:.5}", l[0], l[1], l[2], l[3], c[0], c[1], c[2]);
                }
            }
        }
        let u = RainU {
            global_light: [ambient.x, ambient.y, ambient.z, 1.0],
            scale_offset: [
                if night { self.t("SplashOpacityNight", 0.1225) } else { self.t("SplashOpacity", 0.095) },
                self.t("Rainstrength", 0.0),
                self.t("SplashSizeScale", 0.135),
                if night { self.t("RainOpacityNight", 0.0175) } else { self.t("RainOpacity", 0.014) },
            ],
            tune: [self.t("SplashOffset", 0.068), thr, self.t("Streakthickness", 0.136), self.t("Streaklength", 0.105)],
            // (SpotLightPos, SpotLightDir and the sphere's centre are one and the same vector in
            // the recordings - the car's place; SpotLightParams (10, 3), the sphere 1.5 m)
            spot_pos: [car.x, car.y, car.z, 0.0],
            spot_dir: [car.x, car.y, car.z, 0.0],
            spot_params: [if f.car.is_some() && spot_on { 10.0 } else { 0.0 }, 3.0, 0.0, 0.0],
            spot_color: [self.t("RainSpotLightRed", 0.364), self.t("RainSpotLightGreen", 0.315), self.t("RainSpotLightBlue", 0.279), 0.0],
            fade_sphere: [car.x, car.y, car.z, if f.car.is_some() { 1.5 } else { 0.0 }],
            lights,
            light_cols,
        };
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        if !env_off("MCLA_RAIN_DROPS") {
            // (between two updates the falling ones are drawn on their way)
            let part = self.acc.clamp(0.0, 1.0) * 0.05;
            let insts: Vec<DropInst> = self
                .drops
                .iter()
                .filter(|d| d.state >= 0.0 && (d.state < 0.99 || d.state > 1.2))
                .map(|d| {
                    let p = if d.state > 1.2 { d.pos + d.vel * part } else { d.pos };
                    DropInst { pos_state: [p.x, p.y, p.z, d.state], vel: [d.vel.x, d.vel.y, d.vel.z, 0.0] }
                })
                .collect();
            self.drop_count = insts.len() as u32;
            gpu.queue.write_buffer(&self.drop_buf, 0, bytemuck::cast_slice(&insts));
        }
        let _ = (night, limit, ground);
    }
}

/// Once a frame, before the scene pass.
pub fn prepare(gpu: &Gpu, f: &Frame) {
    if let Some(r) = STATE.lock().unwrap().as_mut() {
        r.prepare(gpu, f);
    }
}

/// In the scene pass, after everything else of the world (the original: right after its fog).
pub fn draw(pass: &mut wgpu::RenderPass<'_>) {
    let guard = STATE.lock().unwrap();
    let Some(r) = guard.as_ref() else { return };
    if r.cone_count == 0 && r.drop_count == 0 {
        return;
    }
    pass.set_bind_group(1, &r.bind, &[]);
    if r.cone_count > 0 {
        pass.set_pipeline(&r.cone_pipeline);
        pass.set_vertex_buffer(0, r.cone_buf.slice(..));
        pass.draw(0..CONE_VERTS, 0..r.cone_count);
    }
    if r.drop_count > 0 {
        pass.set_pipeline(&r.drop_pipeline);
        pass.set_vertex_buffer(0, r.drop_buf.slice(..));
        pass.draw(0..4, 0..r.drop_count);
    }
}

#[cfg(test)]
mod cone_fade_tests {
    /// F-71942 run offline (`tools/cone_fade_reference.py`; qsort, F-19533 and the mesh draw F-20535 stubbed to print):
    /// 500 lists, 4,634 entries, 1,813 lamps set up; the gate (density x strength >= 0.01), the 20 m skip, the faded
    /// strength, density and edge words, the mesh (far beyond 250 x 0.3 m) and the count cleared after.
    #[test]
    fn cone_fade_matches_the_original() {
        let t = include_str!("testdata/cone_fade_reference.txt");
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut lines = t.lines();
        let (mut lists, mut lamps) = (0, 0);
        while let Some(l) = lines.next() {
            let c: Vec<&str> = l.split_whitespace().skip(1).collect();
            let (dens, stre, edge) = (f32::from_bits(h(c[0])), f32::from_bits(h(c[1])), f32::from_bits(h(c[2])));
            let ents: Vec<(f32, f32)> = (0..c[3].parse::<usize>().unwrap())
                .map(|_| {
                    let e: Vec<u32> = lines.next().unwrap().split_whitespace().skip(1).map(h).collect();
                    (f32::from_bits(e[0]), f32::from_bits(e[1]))
                })
                .collect();
            if !(dens * stre < 0.01) {
                for (i, &(d, fac)) in ents.iter().enumerate() {
                    if d < 20.0 {
                        continue;
                    }
                    let mesh = if d > 250.0f32 * 0.3 { 0xBBBB0000u32 } else { 0xAAAA0000 };
                    let want = format!("LAMP {:08x} {:08x} {:08x} {:08x} {i}", dens.to_bits(), super::cone_fade(d, fac, stre).to_bits(), edge.to_bits(), mesh);
                    assert_eq!(lines.next().unwrap(), want, "{l}");
                    assert_eq!(lines.next().unwrap(), format!("DRAW {mesh:08x} 0000cccc"));
                    lamps += 1;
                }
            }
            assert_eq!(lines.next(), Some("END 0"), "{l}");
            lists += 1;
        }
        assert_eq!((lists, lamps), (500, 1813));
    }
}

#[cfg(test)]
mod cone_setup_tests {
    /// F-19533 run offline whole (`tools/cone_pass_reference.py`; only the shader variable setter stubbed to print):
    /// 1,500 lamps (the light files' direction and angles, random ones, cones past the 360 type limit, zero intensity),
    /// every word of LightMatrix, LightPosition, LightColor, LightDirection, LightVolume and LightFallOff equal.
    /// LightInvMatrix (F-25750's inverse) is not compared: the port's shader does not take it.
    #[test]
    fn cone_setup_matches_the_original() {
        let h = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let text = include_str!("render/testdata/cone_pass_reference.txt");
        let mut n = 0;
        for case in text.split("CASE ").skip(1) {
            let mut lines = case.lines();
            let w: Vec<f32> = lines.next().unwrap().split_whitespace().map(h).collect();
            let v4 = |i: usize| [w[i], w[i + 1], w[i + 2], w[i + 3]];
            let got = super::cone_setup(v4(0), v4(4), v4(8), w[12], w[13], w[14], [w[15], w[16], w[17]], [w[18], w[19], w[20]]);
            let mut sets = std::collections::HashMap::new();
            for l in lines {
                let t: Vec<&str> = l.split_whitespace().collect();
                assert_eq!(t[0], "SET");
                sets.insert(t[1].parse::<u32>().unwrap(), t[2..].iter().map(|x| u32::from_str_radix(x, 16).unwrap()).collect::<Vec<u32>>());
            }
            let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<u32>>();
            let m: Vec<f32> = got.matrix.iter().flatten().copied().collect();
            for (k, want) in [(40, bits(&m)), (8, bits(&got.pos)), (12, bits(&got.colour)), (16, bits(&got.dir)), (20, bits(&got.volume)), (48, bits(&[got.falloff]))] {
                assert_eq!(sets[&k], want, "case {n} variable {k}: {case:.200}");
            }
            n += 1;
        }
        assert_eq!(n, 1500);
    }

    /// The street lamps' cone (ConeAngle 125, Penumbra -50, through the light table): the recorded edge
    /// cos x 2.8058 - 1.0837 (LIGHTING.md, the lamps' cones).
    #[test]
    fn the_street_lamps_recorded_edge() {
        let p = crate::props::table_penumbra(-50.0);
        assert!((p + 50.5).abs() < 1e-4, "{p}");
        let s = super::cone_setup([0.0; 4], [1.0, 0.75, 0.55, 0.0], [0.0, -1.0, 0.0, 0.0], 55.0, crate::props::table_half(125.0), p, [0.2, 16.0, 1.0], [1.0; 3]);
        assert!((s.dir[3] - 2.8058).abs() < 1e-4 && (s.pos[3] + 1.0837).abs() < 1e-4, "{:?} {:?}", s.dir, s.pos);
    }
}
