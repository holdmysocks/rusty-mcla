//! The lamps' glow pass that the original draws after its tone map (LIGHTING_WORK.md "The
//! lamps' glow pass after the tone map"): a "seed" quad and three "star" quads per light, in
//! display terms, added to the frame before the display ramp (render/halo.wgsl; the tone map
//! in post.wgsl adds the halo picture to its encoded value).
//!
//! The rule is the game's `AddGlow` (sub_822FC250) with the numbers of
//! tune/lighting/GlowTuning.xml; every array of that file has four entries, one a weather
//! (nice, cloudy, stormy, foggy - read from the running game while the weather was switched).

use crate::render::Gpu;
use glam::Vec3;
use std::sync::Mutex;

/// most glows a frame (the game's list holds 96)
pub const MAX: usize = 96;

/// tune/lighting/GlowTuning.xml, the entries the pass uses
#[derive(Clone, Debug)]
pub struct Tune {
    /// sm_fPropGlowIntensitys
    pub prop_intensity: [f32; 4],
    /// sm_PropGlowSeedSizes
    pub prop_size: [[f32; 2]; 4],
    /// sm_fSignalGlowSizes / sm_fSignalGlowIntensitys
    pub signal_size: [f32; 4],
    pub signal_intensity: [f32; 4],
    /// sm_fCarGlowIntensitys / sm_CarLightGlowSizess
    pub car_intensity: [f32; 4],
    pub car_size: [[f32; 2]; 4],
    /// sm_fStreakIntensitys / sm_fBloomIntensitys (the star's and the seed's factor)
    pub streak: [f32; 4],
    pub bloom: [f32; 4],
    /// StarSize
    pub star: [[f32; 2]; 4],
    /// MaxGlowDist, MaxGlowBias, MaxCarGlowBias, MinGlowBias
    pub max_dist: f32,
    pub max_bias: f32,
    pub max_car_bias: f32,
    pub min_bias: f32,
}

impl Default for Tune {
    fn default() -> Self {
        Tune {
            prop_intensity: [4.35, 4.3, 4.3, 2.0],
            prop_size: [[27.0, 13.8], [27.0, 14.0], [25.4, 9.8], [23.1875, 14.9]],
            signal_size: [4.52, 5.25, 8.7675, 10.0],
            signal_intensity: [4.52, 5.0, 5.0, 3.0],
            car_intensity: [5.35, 5.3, 5.3, 5.3],
            car_size: [[6.0, 2.6]; 4],
            streak: [0.15; 4],
            bloom: [1.0, 1.0, 1.0, 2.0],
            star: [[6.6, 3.85], [6.6, 3.85], [6.6, 3.85], [6.6, 3.2]],
            max_dist: 0.6,
            max_bias: 0.05,
            max_car_bias: 0.02,
            min_bias: 0.009,
        }
    }
}

/// The numbers between `<name ...>` and `</name>` (the first such element).
fn floats(text: &str, name: &str) -> Vec<f32> {
    let Some(i) = text.find(&format!("<{name}")) else { return Vec::new() };
    let rest = &text[i..];
    let (Some(a), Some(b)) = (rest.find('>'), rest.find(&format!("</{name}>"))) else {
        // `<name value="..."/>`
        let end = rest.find("/>").unwrap_or(0);
        return rest[..end].split('"').nth(1).and_then(|v| v.trim().parse().ok()).into_iter().collect();
    };
    if b < a {
        return Vec::new();
    }
    // (nested `<Item ...>` tags: keep what parses as a number)
    rest[a + 1..b].split(|c: char| c.is_whitespace() || c == '<' || c == '>').filter_map(|v| v.parse().ok()).collect()
}

impl Tune {
    pub fn parse(text: &str) -> Tune {
        let mut t = Tune::default();
        let four = |name: &str, d: [f32; 4]| {
            let v = floats(text, name);
            if v.len() >= 4 { [v[0], v[1], v[2], v[3]] } else { d }
        };
        let pairs = |name: &str, d: [[f32; 2]; 4]| {
            let v = floats(text, name);
            if v.len() >= 8 { [[v[0], v[1]], [v[2], v[3]], [v[4], v[5]], [v[6], v[7]]] } else { d }
        };
        let one = |name: &str, d: f32| floats(text, name).first().copied().unwrap_or(d);
        t.prop_intensity = four("sm_fPropGlowIntensitys", t.prop_intensity);
        t.prop_size = pairs("sm_PropGlowSeedSizes", t.prop_size);
        t.signal_size = four("sm_fSignalGlowSizes", t.signal_size);
        t.signal_intensity = four("sm_fSignalGlowIntensitys", t.signal_intensity);
        t.car_intensity = four("sm_fCarGlowIntensitys", t.car_intensity);
        t.car_size = pairs("sm_CarLightGlowSizess", t.car_size);
        t.streak = four("sm_fStreakIntensitys", t.streak);
        t.bloom = four("sm_fBloomIntensitys", t.bloom);
        t.star = pairs("StarSize", t.star);
        t.max_dist = one("MaxGlowDist", t.max_dist);
        t.max_bias = one("MaxGlowBias", t.max_bias);
        t.max_car_bias = one("MaxCarGlowBias", t.max_car_bias);
        t.min_bias = one("MinGlowBias", t.min_bias);
        t
    }
}

/// Whether a prop's lamp is lit at this hour (sub_82729438): after 17.9 - o and before 6.1 + o
/// o'clock, o = one of sixteen steps of a minute a lamp (the original takes bits 4..7 of the
/// prop's address x 1 / 960 h; here a hash of the lamp's place).
pub fn lamp_lit(pos: Vec3, hour: f32) -> bool {
    let o = ((pos.x.to_bits() ^ pos.z.to_bits().rotate_left(13)).wrapping_mul(0x9E37_79B1) >> 28) as f32 / 60.0;
    hour > 17.9 - o || hour < 6.1 + o
}

/// Which props are in the frame's list of lamp and signal props (the prop manager's third
/// per-view list, filled by sub_82734160 through the callback sub_82729E28 and read by
/// sub_82729438 -> AddGlow): the game walks its 40 m prop grid in a square spiral from the MAIN
/// camera's cell and takes a prop when, with R = its drawable's sphere radius (1.0 for a lamp
/// or signal prop under 0.15) and dd = max(distance from the camera to its box centre - R, 0):
///   dd <= 200 and dd < 80 R / tan(fov / 2) and (its box's sphere touches the view's frustum
///   or dd < 10 R / tan(fov / 2))
///   or: dd < 250 and dd < R / (0.0125 tan 67.5 deg) = 33.14 R and the occlusion query passes.
/// No other test (checked on the running game: 741 props round the gate, 103 in the list, all
/// explained - work/lighting/lm/rule.py).
pub struct ListView {
    cam: Vec3,
    tan: f32,
    planes: [glam::Vec4; 5],
    cell: (i32, i32),
    spiral: Vec<u16>,
}

/// the prop grid: cell 40 m, the first cell's centre (read from the running game: grid + 48)
const GRID: (f32, f32, f32) = (40.0, -3470.89, -2330.4475);
const SPIRAL_R: i32 = 9;

impl ListView {
    pub fn new(view_proj: glam::Mat4, cam: Vec3, forward: Vec3, fov_y: f32) -> ListView {
        let r = |i: usize| view_proj.row(i);
        let mut planes = [r(3) + r(0), r(3) - r(0), r(3) + r(1), r(3) - r(1), glam::Vec4::ZERO];
        for p in planes.iter_mut().take(4) {
            *p /= p.truncate().length().max(1e-9);
        }
        // (the near plane, 0.2 m in front)
        planes[4] = forward.extend(-forward.dot(cam) - 0.2);
        // (the spiral of sub_82734DB8: z up, x up, z down, x down, a ring wider each round)
        let n = (2 * SPIRAL_R + 1) as usize;
        let mut spiral = vec![u16::MAX; n * n];
        let (mut x, mut z, mut along_x, mut plus) = (0i32, 0i32, false, true);
        let (mut xmax, mut xmin, mut zmax, mut zmin) = (1, -1, 1, -1);
        for k in 0..(n * n) as u16 {
            if x.abs() > SPIRAL_R || z.abs() > SPIRAL_R {
                break;
            }
            spiral[((z + SPIRAL_R) as usize) * n + (x + SPIRAL_R) as usize] = k;
            match (along_x, plus) {
                (false, true) => { z += 1; if z == zmax { along_x = true; } }
                (false, false) => { z -= 1; if z == zmin { along_x = true; zmax += 1; zmin -= 1; } }
                (true, true) => { x += 1; if x == xmax { along_x = false; plus = false; } }
                (true, false) => { x -= 1; if x == xmin { along_x = false; plus = true; xmax += 1; xmin -= 1; } }
            }
        }
        let cell = (((cam.x - GRID.1) / GRID.0).round() as i32, ((cam.z - GRID.2) / GRID.0).round() as i32);
        ListView { cam, tan: (fov_y * 0.5).tan(), planes, cell, spiral }
    }

    /// `centre`, `w`: the prop's box centre and half diagonal; `r`: its drawable's radius;
    /// `hidden`: the occlusion test, asked only for the second branch.
    pub fn holds(&self, centre: Vec3, w: f32, r: f32, hidden: impl FnOnce() -> bool) -> bool {
        let r = if r < 0.15 { 1.0 } else { r };
        let dd = (centre.distance(self.cam) - r).max(0.0);
        if dd <= 200.0 && 0.0125 * self.tan * dd < r {
            if 0.1 * self.tan * dd < r || self.planes.iter().all(|p| p.dot(centre.extend(1.0)) >= -w) {
                return true;
            }
        }
        dd < 250.0 && 0.030_177_67 * dd < r && !hidden()
    }

    /// The place of a prop's cell in the walk (the list's order; AddGlow keeps the first 96).
    pub fn order(&self, origin: Vec3) -> u32 {
        let (x, z) = (((origin.x - GRID.1) / GRID.0).round() as i32 - self.cell.0, ((origin.z - GRID.2) / GRID.0).round() as i32 - self.cell.1);
        if x.abs() > SPIRAL_R || z.abs() > SPIRAL_R {
            return u32::MAX;
        }
        self.spiral[((z + SPIRAL_R) * (2 * SPIRAL_R + 1) + x + SPIRAL_R) as usize] as u32
    }
}

/// One light of the pass as the shader takes it (halo.wgsl `vs_halo`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Glow {
    /// the place (pulled towards the camera), a = the seed's strength
    pub pos_a: [f32; 4],
    /// the colour, a = the star's strength
    pub col_s: [f32; 4],
    /// the seed's half size (m), the star's long and short half diagonal (m)
    pub size: [f32; 4],
}

/// The vertex colour's alpha byte as the game writes it: min(factor x intensity, 10) x 25.5,
/// cut to a whole number; nothing under 0.001.
fn alpha(v: f32) -> f32 {
    if v < 0.001 { 0.0 } else { (v.min(10.0) * 25.5).floor().min(255.0) / 255.0 }
}

impl Tune {
    /// What `AddGlow` makes of a light: `size` and `intensity` as its caller hands them over,
    /// `car` = its flag for the lights of cars and signals (the size grows with distance so
    /// they stay visible, and the pull towards the camera goes by other numbers).
    fn glow(&self, w: usize, pos: Vec3, color: [f32; 3], mut size: [f32; 2], intensity: f32, car: bool, cam: Vec3, tan_half_fov: f32) -> Option<Glow> {
        let d = pos.distance(cam);
        let (seed, star) = (alpha(self.bloom[w] * intensity), alpha(self.streak[w] * intensity));
        if seed <= 0.0 && star <= 0.0 {
            return None;
        }
        let (mut hi, lo, reach) = if car { (self.max_car_bias * 0.5, 0.065, 10.0) } else { (self.max_bias, self.min_bias, self.max_dist) };
        if car {
            let k = d * tan_half_fov * (0.01 + 0.29 * (d / 400.0).clamp(0.0, 1.0));
            size = [size[0] + k, size[1] + k];
        } else if pos.distance_squared(Vec3::new(865.0, 23.0, -806.0)) < 169.0 || pos.distance_squared(Vec3::new(-3097.0, -15.4, 184.0)) < 169.0 {
            // (two spots where the game keeps the pull short)
            hi = 0.01;
        }
        let f = d.min(reach) / reach.max(1e-6);
        let bias = (f * f * (hi - lo) + lo) * (5.0 / 3.0);
        let p = pos + (cam - pos) * bias;
        Some(Glow {
            pos_a: [p.x, p.y, p.z, seed],
            col_s: [color[0], color[1], color[2], star],
            size: [size[0] * 0.1, size[1] * 0.1, size[0] * self.star[w][0] * 0.45 * 0.05, size[1] * self.star[w][1] * 0.45 * 0.05],
        })
    }

    /// A lit prop lamp (street light, gate lamp ...).
    pub fn prop(&self, w: usize, pos: Vec3, color: Vec3, cam: Vec3) -> Option<Glow> {
        self.glow(w, pos, [color.x, color.y, color.z], self.prop_size[w], self.prop_intensity[w], false, cam, 0.0)
    }

    /// A car's lit lamp (sub_8237AC90): `facing` the way the lamp looks, `k` its level x the
    /// kind's factor; the strength goes by how squarely it faces the camera.
    pub fn car(&self, w: usize, pos: Vec3, facing: Vec3, color: [f32; 3], k: f32, cam: Vec3, tan_half_fov: f32) -> Option<Glow> {
        let i = k * (cam - pos).normalize_or_zero().dot(facing).clamp(0.0, 1.0);
        if i <= 0.001 {
            return None;
        }
        self.glow(w, pos, color, self.car_size[w], self.car_intensity[w] * i, true, cam, tan_half_fov)
    }

    /// A police car's light bar lamp (sub_8235FB78): its own AddGlow call with the props' pull towards the
    /// camera (not the cars'), size 0.3 x 0.3, strength 20 (0x82002C68) x the car's siren state x how squarely
    /// the lamp faces the camera.
    pub fn police(&self, w: usize, pos: Vec3, facing: Vec3, color: [f32; 3], state: f32, cam: Vec3) -> Option<Glow> {
        let i = 20.0 * state * (cam - pos).normalize_or_zero().dot(facing).clamp(0.0, 1.0);
        self.glow(w, pos, color, [0.3, 0.3], i, false, cam, 0.0)
    }

    /// A signal head's lit lamp: `pos` its locator, `facing` the head's z axis (sub_82739948:
    /// the glow sits 0.3 m behind the locator and its strength goes by how squarely the lamp
    /// faces the camera).
    pub fn signal(&self, w: usize, pos: Vec3, facing: Vec3, color: [f32; 3], cam: Vec3, tan_half_fov: f32) -> Option<Glow> {
        let p = pos - facing * 0.3;
        let i = self.signal_intensity[w] * (cam - p).normalize_or_zero().dot(facing).clamp(0.0, 1.0);
        if i <= 0.001 {
            return None;
        }
        self.glow(w, p, color, [self.signal_size[w]; 2], i, true, cam, tan_half_fov)
    }
}

struct Halo {
    tune: Tune,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    star: wgpu::TextureView,
    buf: wgpu::Buffer,
    count: u32,
}

static STATE: Mutex<Option<Halo>> = Mutex::new(None);

/// MCLA_DEBUG_VIEW=115: the glows of before (the signal sprites in the scene pass, no halo pass).
pub fn old_look() -> bool {
    static OLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OLD.get_or_init(|| std::env::var("MCLA_DEBUG_VIEW").map_or(false, |v| v.trim() == "115"))
}

/// The console's gamma decoding of a stored byte (the star picture's fetch constant asks for it).
fn pwl(e: u8) -> f32 {
    let e = e as f32;
    let (k, off) = if e >= 192.0 { (8.0, -1024.0) } else if e >= 96.0 { (4.0, -256.0) } else if e >= 64.0 { (2.0, -64.0) } else { (1.0, 0.0) };
    let l = e * k + off;
    (l + (l * k / 1024.0).trunc()) / 1023.0
}

/// textures/effects/star_glow.dds decoded, with its mip chain (a 2 x 2 mean a level).
fn star_texture(gpu: &Gpu, data_dir: &str) -> wgpu::TextureView {
    let p = std::path::Path::new(data_dir).join("textures/effects/star_glow.dds");
    let (mut w, mut h, mut px): (u32, u32, Vec<f32>) = match std::fs::read(&p).ok().and_then(|d| mcla_formats::dds::parse_dds("star_glow.dds", &d).ok()) {
        Some(t) if t.width > 0 && t.height > 0 && !t.levels.is_empty() => (t.width, t.height, t.to_rgba8().iter().enumerate().map(|(i, &b)| if i % 4 == 3 { b as f32 / 255.0 } else { pwl(b) }).collect()),
        _ => {
            log::warn!("star_glow.dds missing: the lamps' glows have no star");
            (1, 1, vec![0.0, 0.0, 0.0, 1.0])
        }
    };
    let mut levels: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    loop {
        levels.push((w, h, px.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()));
        if w <= 1 || h <= 1 || w % 2 != 0 || h % 2 != 0 {
            break;
        }
        let (nw, nh) = (w / 2, h / 2);
        let mut next = vec![0.0f32; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let at = |xx: u32, yy: u32| px[((yy * w + xx) * 4 + c) as usize];
                    next[((y * nw + x) * 4 + c) as usize] = 0.25 * (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1));
                }
            }
        }
        (w, h, px) = (nw, nh, next);
    }
    let (w0, h0) = (levels[0].0, levels[0].1);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("star glow"),
        size: wgpu::Extent3d { width: w0, height: h0, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (i, (lw, lh, data)) in levels.iter().enumerate() {
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lw * 4), rows_per_image: Some(*lh) },
            wgpu::Extent3d { width: *lw, height: *lh, depth_or_array_layers: 1 },
        );
    }
    texture.create_view(&Default::default())
}

/// Once, with the city's data.
pub fn load(gpu: &mut Gpu, data_dir: &str) {
    let tune = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/lighting/GlowTuning.xml")).map(|t| Tune::parse(&t)).unwrap_or_default();
    let star = star_texture(gpu, data_dir);
    let device = &gpu.device;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("halo layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("halo sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    // (the frame's globals: the particles' pipeline has their layout)
    let globals_layout = gpu.pipeline_particle.get_bind_group_layout(0);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("halo"), source: wgpu::ShaderSource::Wgsl(include_str!("render/halo.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("halo pl"), bind_group_layouts: &[&globals_layout, &layout], push_constant_ranges: &[] });
    const ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("halo"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_halo"),
            buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Glow>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &ATTRS }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_halo"),
            // (blend one / one, as the original's 0x01010101)
            targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("halo glows"), size: (MAX * std::mem::size_of::<Glow>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    *STATE.lock().unwrap() = Some(Halo { tune, pipeline, layout, sampler, star, buf, count: 0 });
}

/// The tuning (None until `load`).
pub fn tune() -> Option<Tune> {
    STATE.lock().unwrap().as_ref().map(|h| h.tune.clone())
}

/// This frame's lights, nearest first (at most `MAX` are kept, as the game's list).
pub fn set(gpu: &Gpu, glows: &[Glow]) {
    if let Some(h) = STATE.lock().unwrap().as_mut() {
        let n = glows.len().min(MAX);
        if n > 0 {
            gpu.queue.write_buffer(&h.buf, 0, bytemuck::cast_slice(&glows[..n]));
        }
        h.count = n as u32;
    }
}

/// The pass: after the scene (its depth is read), before the tone map (which adds the
/// picture). Always clears the halo picture.
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder) {
    let guard = STATE.lock().unwrap();
    let bind = guard.as_ref().filter(|h| h.count > 0).map(|h| {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("halo"),
            layout: &h.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&h.star) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&h.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&gpu.depth) },
            ],
        })
    });
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("halo"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.post.halo_view(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    if let (Some(h), Some(bind)) = (guard.as_ref(), bind.as_ref()) {
        pass.set_pipeline(&h.pipeline);
        pass.set_bind_group(0, &gpu.globals_bg, &[]);
        pass.set_bind_group(1, bind, &[]);
        pass.set_vertex_buffer(0, h.buf.slice(..));
        pass.draw(0..24, 0..h.count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_lamp_as_recorded() {
        // capture hl_L_23.5: the lamp at 817.46 26.20 -805.12 seen from 813.65 19.57 -814.71 has its
        // quads at 817.14 25.65 -805.92, alpha bytes 110 (seed) and 16 (star), seed 27 x 13.8,
        // star half diagonals 80.19 and 23.91 (x 0.05 in the vertex program)
        let t = Tune::default();
        let g = t.prop(0, Vec3::new(817.46, 26.20, -805.12), Vec3::new(1.0, 0.937, 0.5), Vec3::new(813.65, 19.57, -814.71)).unwrap();
        assert!((g.pos_a[0] - 817.14).abs() < 0.02 && (g.pos_a[1] - 25.65).abs() < 0.02 && (g.pos_a[2] + 805.92).abs() < 0.02, "{:?}", g.pos_a);
        assert_eq!((g.pos_a[3] * 255.0).round() as i32, 110);
        assert_eq!((g.col_s[3] * 255.0).round() as i32, 16);
        assert!((g.size[0] - 2.7).abs() < 1e-4 && (g.size[1] - 1.38).abs() < 1e-4);
        assert!((g.size[2] / 0.05 - 80.19).abs() < 0.01 && (g.size[3] / 0.05 - 23.91).abs() < 0.01, "{:?}", g.size);
        // foggy: seed 102, star 7, size 23.19 x 14.9
        let g = t.prop(3, Vec3::new(817.46, 26.20, -805.12), Vec3::ONE, Vec3::new(813.65, 19.57, -814.71)).unwrap();
        assert_eq!(((g.pos_a[3] * 255.0).round() as i32, (g.col_s[3] * 255.0).round() as i32), (102, 7));
    }

    #[test]
    fn a_signal_grows_with_distance() {
        // the same capture: red lamps 100.5 m off are 6.37 across, 129.7 m off 7.51 (4.52 at the lamp)
        let t = Tune::default();
        let tan = (25.0f32.to_radians() * 0.5).tan();
        for (d, s) in [(100.5f32, 6.37f32), (129.66, 7.51)] {
            let g = t.signal(0, Vec3::new(0.0, 0.0, -d + 0.3), Vec3::Z, [1.0, 0.0, 0.0], Vec3::ZERO, tan).unwrap();
            assert!((g.size[0] * 10.0 - s).abs() < 0.03, "{d}: {}", g.size[0] * 10.0);
            assert_eq!((g.pos_a[3] * 255.0).round() as i32, 115);
        }
        // seen from behind: none
        assert!(t.signal(0, Vec3::new(0.0, 0.0, 50.0), Vec3::Z, [1.0, 0.0, 0.0], Vec3::ZERO, tan).is_none());
    }

    #[test]
    fn the_tuning_file_is_read() {
        let x = "<GlowTuning><sm_fPropGlowIntensitys content=\"float_array\">\n1.5\n2\n3\n4\n</sm_fPropGlowIntensitys><sm_fBloomIntensitys><Item content=\"float_array\">\n1\n1\n1\n3\n</Item></sm_fBloomIntensitys><StarSize content=\"vector2_array\">\n6 3\n6 3\n6 3\n6 2\n</StarSize><MaxGlowBias value=\"0.07\"/></GlowTuning>";
        let t = Tune::parse(x);
        assert_eq!(t.prop_intensity, [1.5, 2.0, 3.0, 4.0]);
        assert_eq!(t.bloom[3], 3.0);
        assert_eq!(t.star[3], [6.0, 2.0]);
        assert_eq!(t.max_bias, 0.07);
        assert_eq!(t.min_bias, 0.009);
    }
}
