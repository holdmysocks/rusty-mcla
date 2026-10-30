//! Step 19g: the power-up effects drawn on the cars, as the original draws them (the types'
//! draw, vfunc +40, called by the holder's draw sub_8232AE70 from the vehicle's alpha pass
//! sub_82360518):
//! - Ice / MegaIce (sub_8237F3B0): `vehicle/shared_utility/fx_iceblock` (pCube1.mesh,
//!   util_iceblock.dds), powerupicecube.fxc "icetechnique";
//! - Shield / MegaShield / HitByAgro (sub_8237FC28): `fx_shield` (shield_geo.mesh, swirls.dds),
//!   "shieldtechnique";
//! - Stealth / MegaStealth (the type's +48 / +52, sub_8237D568 / sub_8237D610, around the car's
//!   own draw in sub_8235FB78): the car's meshes with "stealthtechnique".
//! The rules are in render/pu_fx.wgsl; SAVE_WORK.md "Step 19g".

use crate::powerups::{PowerUps, TypeTune};
use crate::render::Gpu;
use glam::{Mat4, Quat, Vec3};
use std::sync::Mutex;

/// One effect on one car for the program: the world matrix and four rows (pu_fx.wgsl `Inst`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Debug)]
pub struct Inst {
    pub m: [[f32; 4]; 4],
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
    pub d: [f32; 4],
}

const MAX: usize = 16;

struct Mesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
}

struct State {
    ice_in: crate::render::ms::MsPipe,
    ice_out: crate::render::ms::MsPipe,
    shield_in: crate::render::ms::MsPipe,
    shield_out: crate::render::ms::MsPipe,
    stealth_solid: crate::render::ms::MsPipe,
    stealth_glass: crate::render::ms::MsPipe,
    ice: Option<Mesh>,
    shield: Option<Mesh>,
    ice_bg: wgpu::BindGroup,
    shield_bg: wgpu::BindGroup,
    ice_ub: wgpu::Buffer,
    shield_ub: wgpu::Buffer,
    looks: [(u8, Look); 5],
    bolt: crate::render::ms::MsPipe,
    bolt_bg: wgpu::BindGroup,
    bolt_vb: wgpu::Buffer,
    laser: crate::render::ms::MsPipe,
    laser_bg: wgpu::BindGroup,
    laser_vb: wgpu::Buffer,
    /// m_powerupColor of every type's look (powerup_<name>.mcpowerup, +112) - FX_Color
    pu_colors: [[f32; 4]; 24],
}

/// most laser vertices a frame (8 cars x 2 fans x 7 layers x 6)
const LASER_MAX: usize = 8 * 2 * 7 * 6;

/// most lightning vertices a frame (ten bolts x two arcs x 10 segments x 6, for 8 cars)
const BOLT_MAX: usize = 8 * 10 * 2 * 12 * 6;

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// A text mesh (`Version 11`, the one material's TRIANGLES): positions, normals, the first UV
/// set as (x y z nx ny nz u v), and the indices.
pub fn parse_mesh(text: &str) -> Option<(Vec<[f32; 8]>, Vec<u32>)> {
    let idx_at = text.find("Idx ")?;
    let rest = &text[idx_at + 4..];
    let open = rest.find('{')?;
    let close = rest.find('}')?;
    let idx: Vec<u32> = rest[open + 1..close].split_whitespace().filter_map(|w| w.parse().ok()).collect();
    let v_at = text.find("Verts ")?;
    let rest = &text[v_at + 6..];
    let n: usize = rest.split_whitespace().next()?.parse().ok()?;
    let open = rest.find('{')?;
    let mut verts = Vec::with_capacity(n);
    for line in rest[open + 1..].lines() {
        let parts: Vec<&str> = line.split('/').map(|s| s.trim()).collect();
        if parts.len() < 7 {
            if line.trim() == "}" {
                break;
            }
            continue;
        }
        let f = |s: &str| -> Vec<f32> { s.split_whitespace().filter_map(|w| w.parse().ok()).collect() };
        // (position / weights / normal / three colours / UV sets: count, u, v ...)
        let (p, nn, uv) = (f(parts[0]), f(parts[2]), f(parts[6]));
        if p.len() < 3 || nn.len() < 3 || uv.len() < 3 {
            continue;
        }
        verts.push([p[0], p[1], p[2], nn[0], nn[1], nn[2], uv[1], uv[2]]);
        if verts.len() == n {
            break;
        }
    }
    (verts.len() == n && !idx.is_empty() && idx.iter().all(|&i| (i as usize) < n)).then_some((verts, idx))
}

// ---------------------------------------------------------------------------------------
// The rules

/// A key curve as the types keep it (`rmPtfxKeyframe` at the type's +60: keys of 48 bytes, +0
/// the time, +16 the value, +32 the slope to the next key - sub_824959F0), looked up as the draws
/// do: the first key from the second on whose time is not below `t`, and the key before it plus
/// its slope x (t - its time); past the last key, the last but one's line.
pub fn curve(keys: &[(f32, [f32; 2])], t: f32) -> [f32; 2] {
    let n = keys.len();
    if n == 0 {
        return [0.0; 2];
    }
    if n == 1 {
        return keys[0].1;
    }
    let mut i = 1;
    while i < n && keys[i].0 < t {
        i += 1;
    }
    let k = (i.min(n) - 1).min(n - 2);
    let (t0, v0) = keys[k];
    let (t1, v1) = keys[k + 1];
    let d = t1 - t0;
    let s = |a: f32, b: f32| if d > 0.0 { (b - a) / d } else { 0.0 };
    [v0[0] + s(v0[0], v1[0]) * (t - t0), v0[1] + s(v0[1], v1[1]) * (t - t0)]
}

/// The Ice's curve (sub_8237CCB8 0x8237CDE8..0x8237CE48): (0, 1), (0.5, 1), (1, 0) - the
/// block's HEIGHT over the effect (t = 1 - time left / ActiveTime): whole for the first half,
/// then down to nothing (the "melt" of sub_8237CFD8 at half time).
pub const ICE_KEYS: [(f32, [f32; 2]); 3] = [(0.0, [1.0, 0.0]), (0.5, [1.0, 0.0]), (1.0, [0.0, 0.0])];

/// The Shield's curve (sub_8237DD60 0x8237DFB4..0x8237E088): FX_Timer1 = (the swirl layer's
/// weight, the colour layer's weight) over the flash clock (+376): a flash when a shot is taken.
pub const SHIELD_KEYS: [(f32, [f32; 2]); 5] = [(0.0, [1.0, 1.0]), (0.076, [4.93, 7.852]), (0.2, [2.696, 0.539]), (0.27, [1.141, 1.71]), (1.0, [1.0, 1.0])];

/// The tune of an on-car effect (the manager's file `tune/powerup/0xe49d3ba2`, the Ice and
/// Shield parsers sub_8237CE60 / sub_8237E0C0, over the constructors' values).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// FX_TexColor (Ice +128, Shield +432)
    pub tex_color: [f32; 4],
    /// FX_FuzzColor (Shield +416)
    pub fuzz_color: [f32; 4],
    /// FX_FresnelMin / Max / Exp (Ice +108 / +112 / +116 from the file; Shield +400 / +404 /
    /// +408 = 0.125 / 1 / 4.5, not in the file) and Shield's FX_EnvMapAlpha (+412 = 0.17)
    pub fresnel: [f32; 3],
    pub env_alpha: f32,
    /// the size scale and the offset: car, motorcycle (sub_82332098), Golf '83 ("vp_vw_golf_83")
    pub size: [Vec3; 3],
    pub offset: [Vec3; 3],
}

/// Ice (1), MegaIce (12), Shield (5), MegaShield (16), HitByAgro (23): their looks.
pub fn load_looks(data_dir: &str) -> [(u8, Look); 5] {
    let path = std::path::Path::new(data_dir).join("tune/powerup/0xe49d3ba2");
    let v = std::fs::read_to_string(&path).map(|t| crate::powerups::parse_tune(&t)).unwrap_or_default();
    let get = |name: &str, k: &str| v.get(&format!("{name}.{k}")).cloned();
    let v3 = |name: &str, k: &str, d: Vec3| get(name, k).filter(|x| x.len() >= 3).map_or(d, |x| Vec3::new(x[0], x[1], x[2]));
    let v4 = |name: &str, k: &str, d: [f32; 4]| get(name, k).filter(|x| x.len() >= 4).map_or(d, |x| [x[0], x[1], x[2], x[3]]);
    let f1 = |name: &str, k: &str, d: f32| get(name, k).and_then(|x| x.first().copied()).unwrap_or(d);
    let ice = |name: &str| Look {
        // (the constructor sub_8237CCB8: 0.26 / 2.02 / 4.5, (0.08, 0.128, 0.146, 1.78), size
        // (0.971, 0.676, 2.184) for all three, offset (0, -0.014, 0), motorcycle (0.971, 1.665,
        // 2.184) / (0, -0.054, 0))
        tex_color: v4(name, "m_TexColor", [0.08, 0.128, 0.146, 1.78]),
        fuzz_color: [0.0; 4],
        fresnel: [f1(name, "m_FresnelMin", 0.26), f1(name, "m_FresnelMax", 2.02), f1(name, "m_FresnelExp", 4.5)],
        env_alpha: 0.0,
        size: [
            v3(name, "m_IceBlockSizeScale", Vec3::new(0.971, 0.676, 2.184)),
            v3(name, "m_IceBlockSizeScale_Moto", Vec3::new(0.971, 1.665, 2.184)),
            v3(name, "m_IceBlockSizeScaleHackGolf83", Vec3::new(0.971, 0.676, 2.184)),
        ],
        offset: [
            v3(name, "m_IceBlockPosOffset", Vec3::new(0.0, -0.014, 0.0)),
            v3(name, "m_IceBlockPosOffset_Moto", Vec3::new(0.0, -0.054, 0.0)),
            v3(name, "m_IceBlockPosOffsetHackGolf83", Vec3::new(0.0, -0.014, 0.0)),
        ],
    };
    let shield = |name: &str| Look {
        // (the constructor sub_8237DD60: FuzzColor (0.1, 0.2, 1, 0.349), TexColor (0.6, 0.4, 1,
        // 0.43), size (1.23, 0.906, 2.373) / offset (0, -0.29, 0.02); motorcycle (1.303, 1.798,
        // 2.388) / (0, -0.213, -0.022); fresnel 0.125 / 1 / 4.5 and EnvMapAlpha 0.17 at +400..+412)
        tex_color: v4(name, "m_TexColor", [0.6, 0.4, 1.0, 0.43]),
        fuzz_color: v4(name, "m_FuzzColor", [0.1, 0.2, 1.0, 0.349]),
        fresnel: [0.125, 1.0, 4.5],
        env_alpha: 0.17,
        size: [
            v3(name, "m_ShieldSizeScale", Vec3::new(1.23, 0.906, 2.373)),
            v3(name, "m_ShieldSizeScale_Moto", Vec3::new(1.303, 1.798, 2.388)),
            v3(name, "m_ShieldSizeScaleHackGolf83", Vec3::new(1.23, 0.906, 2.373)),
        ],
        offset: [
            v3(name, "m_ShieldPosOffset", Vec3::new(0.0, -0.29, 0.02)),
            v3(name, "m_ShieldPosOffset_Moto", Vec3::new(0.0, -0.213, -0.022)),
            v3(name, "m_ShieldPosOffsetHackGolf83", Vec3::new(0.0, -0.29, 0.02)),
        ],
    };
    [(1, ice("Ice")), (12, ice("MegaIce")), (5, shield("Shield")), (16, shield("MegaShield")), (23, shield("HitByAgro"))]
}

/// Which of the three sizes: 0 a car, 1 a motorcycle (sub_82332098), 2 the Golf '83 (its name).
pub fn variant(bike: bool, name: &str) -> usize {
    if bike {
        1
    } else if name == "vp_vw_golf_83" {
        2
    } else {
        0
    }
}

/// The ice block's world matrix (sub_8237F3B0): the car's rows scaled by |size.x bz|,
/// |size.y curve(t) bz|, |size.z bz| - bz the car's bound's box maximum z (the composite bound
/// sub_8255B9A8 +16, element z), curve(t) the Ice curve's x - and moved by the offset along the
/// car's (unscaled) axes. The mesh stands on its base (y 0 .. 1.04, x and z +-0.58).
pub fn ice_matrix(rot: Quat, pos: Vec3, bz: f32, size: Vec3, offset: Vec3, t: f32) -> Mat4 {
    let h = curve(&ICE_KEYS, t)[0];
    let s = Vec3::new((size.x * bz).abs(), (size.y * h * bz).abs(), (size.z * bz).abs());
    Mat4::from_translation(pos + rot * offset) * Mat4::from_quat(rot) * Mat4::from_scale(s)
}

/// The shield's world matrix (sub_8237FC28): the car's rows scaled by |size bz| (no curve), the
/// mesh first turned about its own up axis by the game's running clock in seconds (0x827D7518,
/// sub_82202E38 / sub_82204CB8: one radian a second), and moved by the offset along the SCALED
/// axes. The mesh is a sphere of radius 0.531 standing on its base.
pub fn shield_matrix(rot: Quat, pos: Vec3, bz: f32, size: Vec3, offset: Vec3, clock: f32) -> Mat4 {
    let s = (size * bz).abs();
    Mat4::from_translation(pos + rot * (s * offset)) * Mat4::from_quat(rot) * Mat4::from_scale(s) * Mat4::from_rotation_y(clock)
}

// ---------------------------------------------------------------------------------------
// GPU

fn find(data_dir: &str, dir: &str, file: &str) -> Option<Vec<u8>> {
    let base = std::path::Path::new(data_dir).join("vehicle");
    for top in ["shared_utility", "0xd8bee6cc"] {
        for f in [file.to_string(), format!("0x{:08x}", mcla_formats::hash::joaat(file))] {
            let p = base.join(top).join(dir).join(&f);
            if let Ok(d) = std::fs::read(&p) {
                return Some(d);
            }
        }
    }
    None
}

/// Once, with the city's data.
pub fn load(gpu: &mut Gpu, data_dir: &str) {
    let mesh = |gpu: &Gpu, dir: &str, file: &str| -> Option<Mesh> {
        let text = String::from_utf8_lossy(&find(data_dir, dir, file)?).to_string();
        let (v, i) = parse_mesh(&text)?;
        use wgpu::util::DeviceExt;
        let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(file), contents: bytemuck::cast_slice(&v), usage: wgpu::BufferUsages::VERTEX });
        let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(file), contents: bytemuck::cast_slice(&i), usage: wgpu::BufferUsages::INDEX });
        Some(Mesh { vb, ib, count: i.len() as u32 })
    };
    let ice = mesh(gpu, "fx_iceblock", "pCube1.mesh");
    let shield = mesh(gpu, "fx_shield", "shield_geo.mesh");
    if ice.is_none() || shield.is_none() {
        log::warn!("pu_fx: power-up meshes missing (ice {}, shield {})", ice.is_some(), shield.is_some());
    }
    let mut tex = |dir: &str, file: &str| -> wgpu::TextureView {
        let t = find(data_dir, dir, file).and_then(|d| mcla_formats::dds::parse_dds(file, &d).ok());
        match t {
            Some(t) => gpu.upload_texture(mcla_formats::hash::joaat(&format!("pu_fx/{file}")), &t).texture.create_view(&Default::default()),
            None => {
                log::warn!("pu_fx: texture {file} missing");
                crate::render::create_rgba(&gpu.device, &gpu.queue, 1, 1, &[255, 255, 255, 255], file).view
            }
        }
    };
    let ice_tex = tex("fx_iceblock", "util_iceblock.dds");
    let shield_tex = tex("fx_shield", "swirls.dds");
    // (lightning.dds, `tune/lighting/textures` - sub_822ED628)
    let bolt_tex = {
        let p = std::path::Path::new(data_dir).join("tune/lighting/textures/lightning.dds");
        match std::fs::read(&p).ok().and_then(|d| mcla_formats::dds::parse_dds("lightning.dds", &d).ok()) {
            Some(t) => gpu.upload_texture(mcla_formats::hash::joaat("pu_fx/lightning.dds"), &t).texture.create_view(&Default::default()),
            None => {
                log::warn!("pu_fx: lightning.dds missing");
                crate::render::create_rgba(&gpu.device, &gpu.queue, 1, 1, &[255, 255, 255, 255], "lightning").view
            }
        }
    };
    // (the laser's DiffuseSampler: "__lasersmoke" = laser_smoke.dds of the cars' global textures)
    let laser_tex = {
        let base = std::path::Path::new(data_dir).join("textures/global/cars/globaltex");
        let d = std::fs::read(base.join("laser_smoke.dds")).or_else(|_| std::fs::read(base.join(format!("0x{:08x}", mcla_formats::hash::joaat("laser_smoke.dds")))));
        match d.ok().and_then(|d| mcla_formats::dds::parse_dds("laser_smoke.dds", &d).ok()) {
            Some(t) => gpu.upload_texture(mcla_formats::hash::joaat("pu_fx/laser_smoke.dds"), &t).texture.create_view(&Default::default()),
            None => {
                log::warn!("pu_fx: laser_smoke.dds missing");
                crate::render::create_rgba(&gpu.device, &gpu.queue, 1, 1, &[128, 128, 128, 255], "laser").view
            }
        }
    };
    let device = &gpu.device;
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("pu_fx"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("pu_fx layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ],
    });
    let ub = |label: &str| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (MAX * std::mem::size_of::<Inst>()) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let (ice_ub, shield_ub) = (ub("pu_fx ice"), ub("pu_fx shield"));
    let bg = |view: &wgpu::TextureView, buf: &wgpu::Buffer| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pu_fx"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }, wgpu::BindGroupEntry { binding: 2, resource: buf.as_entire_binding() }],
        })
    };
    let ice_bg = bg(&ice_tex, &ice_ub);
    let bolt_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("pu_fx lightning"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bolt_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pu_fx lightning"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&bolt_tex) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&bolt_sampler) }, wgpu::BindGroupEntry { binding: 2, resource: ice_ub.as_entire_binding() }],
    });
    let laser_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pu_fx laser"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&laser_tex) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }, wgpu::BindGroupEntry { binding: 2, resource: ice_ub.as_entire_binding() }],
    });
    let laser_vb = device.create_buffer(&wgpu::BufferDescriptor { label: Some("pu_fx laser"), size: (LASER_MAX * 36) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let bolt_vb = device.create_buffer(&wgpu::BufferDescriptor { label: Some("pu_fx lightning"), size: (BOLT_MAX * 36) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let shield_bg = bg(&shield_tex, &shield_ub);
    let globals_layout = gpu.pipeline_particle.get_bind_group_layout(0);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("pu_fx"), source: wgpu::ShaderSource::Wgsl(include_str!("render/pu_fx.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("pu_fx pl"), bind_group_layouts: &[&globals_layout, &layout], push_constant_ranges: &[] });
    let pl_car = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("pu_fx stealth pl"), bind_group_layouts: &[&globals_layout, &layout, &gpu.obj_layout], push_constant_ranges: &[] });
    let attrs = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
    ];
    let vl = wgpu::VertexBufferLayout { array_stride: 32, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs };
    // (alpha blend on - state 8 of sub_8217F768 - and the alpha test off (state 7); the Shield and
    // the Stealth set blend mode 7 (source alpha, one); the Ice draw sets none and finds the
    // vehicle pass's: taken as mode 0, source alpha / inverse source alpha)
    let over = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let make = |label: &'static str, pl: &wgpu::PipelineLayout, vs: &'static str, fs: &'static str, blend: wgpu::BlendComponent, zwrite: bool, buffers: &[wgpu::VertexBufferLayout]| {
        // (per sample count of the scene - render/ms.rs)
        let (pl, shader) = (pl.clone(), shader.clone());
        let bufs: Vec<(u64, wgpu::VertexStepMode, Vec<wgpu::VertexAttribute>)> = buffers.iter().map(|b| (b.array_stride, b.step_mode, b.attributes.to_vec())).collect();
        crate::render::ms::MsPipe::new(device, move |device, v| {
        let buffers: Vec<wgpu::VertexBufferLayout> = bufs.iter().map(|(s, m, a)| wgpu::VertexBufferLayout { array_stride: *s, step_mode: *m, attributes: a }).collect();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), buffers: &buffers, compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fs),
                targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: blend, alpha: blend }), write_mask: wgpu::ColorWrites::COLOR })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            // (the techniques' passes: ZWriteEnable 0; reversed depth, tested)
            depth_stencil: Some(v.ds(zwrite, wgpu::CompareFunction::GreaterEqual, false, Default::default())),
            multisample: v.ms(),
            multiview: None,
            cache: None,
        })
        })
    };
    let fxv = [vl.clone()];
    let carv = [crate::render::Vertex::LAYOUT];
    let ice_in = make("pu_fx ice in", &pl, "vs_fx", "fs_ice_in", over, false, &fxv);
    let ice_out = make("pu_fx ice out", &pl, "vs_fx", "fs_ice_out", over, false, &fxv);
    let shield_in = make("pu_fx shield in", &pl, "vs_fx", "fs_shield_in", add, false, &fxv);
    let shield_out = make("pu_fx shield out", &pl, "vs_fx", "fs_shield_out", add, false, &fxv);
    // (sub_8235FB78 0x823602BC..0x82360488: ZWriteEnable and ZEnable on for the body's draws,
    // then ZWriteEnable off for the rest)
    let stealth_solid = make("pu_fx stealth", &pl_car, "vs_stealth", "fs_stealth", add, true, &carv);
    let stealth_glass = make("pu_fx stealth glass", &pl_car, "vs_stealth", "fs_stealth", add, false, &carv);
    // (sub_822EDD80: blend mode 7, alpha blend on, ZWriteEnable 0, ZEnable 1)
    let battrs = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 12, shader_location: 1 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 28, shader_location: 2 },
    ];
    let bolt = make("pu_fx lightning", &pl, "vs_bolt", "fs_bolt", add, false, &[wgpu::VertexBufferLayout { array_stride: 36, step_mode: wgpu::VertexStepMode::Vertex, attributes: &battrs }]);
    // (the laser: SrcBlend ONE, DestBlend ONE, ZWriteEnable 0, CullMode none)
    let one = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let lattrs = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 24, shader_location: 2 },
    ];
    let laser = make("pu_fx laser", &pl, "vs_laser", "fs_laser", one, false, &[wgpu::VertexBufferLayout { array_stride: 36, step_mode: wgpu::VertexStepMode::Vertex, attributes: &lattrs }]);
    let pu_colors: [[f32; 4]; 24] = {
        let l = crate::powerups::load_looks(data_dir);
        std::array::from_fn(|i| l[i].powerup_color)
    };
    *STATE.lock().unwrap() = Some(State { ice_in, ice_out, shield_in, shield_out, stealth_solid, stealth_glass, ice, shield, ice_bg, shield_bg, ice_ub, shield_ub, looks: load_looks(data_dir), bolt, bolt_bg, bolt_vb, laser, laser_bg, laser_vb, pu_colors });
}

/// The car's bound's box maximum z (the composite bound's +0x10 row, element z): the hull's
/// (the body part's) - the wheels' parts lie inside it. Without a hull: the model's box.
pub fn bound_max_z(c: &crate::vehicles::CarInstance) -> f32 {
    c.model.params.hull.as_ref().map_or(c.model.max.z, |h| h.max.z)
}

/// What is drawn this frame: the ice blocks' and the shields' instances.
pub fn instances(pu: &PowerUps, vehicles: &crate::vehicles::Vehicles, looks: &[(u8, Look); 5]) -> (Vec<Inst>, Vec<Inst>) {
    let (mut ice, mut shield) = (Vec::new(), Vec::new());
    let alpha = vehicles.render_alpha;
    for h in &pu.holders {
        let Some(e) = h.effect.as_ref() else { continue };
        let Some(c) = vehicles.cars.get(h.car) else { continue };
        let Some((_, look)) = looks.iter().find(|(k, _)| *k == e.kind) else { continue };
        let t: &TypeTune = &pu.types[e.kind as usize];
        let pose = c.render_pose(alpha);
        let var = variant(c.model.bike, &c.model.name);
        let bz = bound_max_z(c);
        if e.kind == 1 || e.kind == 12 {
            let tt = 1.0 - e.left / t.active_time;
            let m = ice_matrix(pose.rot, pose.pos, bz, look.size[var], look.offset[var], tt);
            ice.push(Inst { m: m.to_cols_array_2d(), a: look.tex_color, b: [look.fresnel[0], look.fresnel[1], look.fresnel[2], 0.0], c: [0.0; 4], d: [0.0; 4] });
        } else {
            let fade = crate::powerups::fade_fx(t, e.left);
            let m = shield_matrix(pose.rot, pose.pos, bz, look.size[var], look.offset[var], pu.time);
            let t1 = curve(&SHIELD_KEYS, h.fx.flash);
            let f = look.fuzz_color;
            let tc = look.tex_color;
            shield.push(Inst {
                m: m.to_cols_array_2d(),
                a: [f[0], f[1], f[2], f[3] * fade],
                b: [tc[0], tc[1], tc[2], tc[3] * fade],
                c: [t1[0], t1[1], h.fx.t2, h.fx.t3],
                d: [look.fresnel[0], look.fresnel[1], look.fresnel[2], look.env_alpha * fade],
            });
        }
    }
    ice.truncate(MAX);
    shield.truncate(MAX);
    (ice, shield)
}

/// The pass: over the finished scene (its depth tested), before the tone map. The stealth cars
/// first (the vehicle pass), then the ice blocks and shields (the holder's draw, after the car).
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder, vehicles: &crate::vehicles::Vehicles, pu: Option<&PowerUps>, cam: Vec3) {
    let guard = STATE.lock().unwrap();
    let Some(s) = guard.as_ref() else { return };
    let (ice, shield) = pu.map_or((Vec::new(), Vec::new()), |pu| instances(pu, vehicles, &s.looks));
    let (ice, shield) = (&ice[..], &shield[..]);
    let bolts = pu.map_or(Vec::new(), |pu| bolt_vertices(pu, cam));
    let nb = bolts.len().min(BOLT_MAX);
    let lasers = pu.map_or(Vec::new(), |pu| laser_vertices(pu, vehicles, &s.pu_colors));
    let nl = lasers.len().min(LASER_MAX);
    if nl > 0 {
        gpu.queue.write_buffer(&s.laser_vb, 0, bytemuck::cast_slice(&lasers[..nl]));
    }
    // (MCLA_PU_FX_LOG=1: what is drawn, once a second)
    if std::env::var("MCLA_PU_FX_LOG").is_ok() {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 30 == 0 && (nb > 0 || !ice.is_empty() || !shield.is_empty()) {
            log::info!("pu_fx: {} ice, {} shields, {} lightning vertices, first {:?}, cam {cam}", ice.len(), shield.len(), nb, bolts.first());
        }
    }
    if nb > 0 {
        gpu.queue.write_buffer(&s.bolt_vb, 0, bytemuck::cast_slice(&bolts[..nb]));
    }
    let stealth = vehicles.cars.iter().any(|c| c.pu_stealth);
    if ice.is_empty() && shield.is_empty() && !stealth && nb == 0 && nl == 0 {
        return;
    }
    if !ice.is_empty() {
        gpu.queue.write_buffer(&s.ice_ub, 0, bytemuck::cast_slice(ice));
    }
    if !shield.is_empty() {
        gpu.queue.write_buffer(&s.shield_ub, 0, bytemuck::cast_slice(shield));
    }
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("power-up effects"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_bind_group(0, &gpu.globals_bg, &[]);
    if stealth {
        pass.set_bind_group(1, &s.ice_bg, &[]);
        for glass in [false, true] {
            pass.set_pipeline(if glass { &s.stealth_glass } else { &s.stealth_solid });
            for c in vehicles.cars.iter().filter(|c| c.pu_stealth) {
                pass.set_bind_group(2, &c.binding.bg, &[]);
                for (bi, b) in c.model.batches.iter().enumerate() {
                    if b.mode.blended() != glass {
                        continue;
                    }
                    pass.set_bind_group(2, c.batch_object(bi), &[]);
                    pass.set_vertex_buffer(0, b.vb.slice(..));
                    pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..b.count, 0, 0..1);
                }
                if !glass {
                    if let Some(w) = c.wheels.as_ref().or(c.model.wheels.as_ref()).or(vehicles.wheels.as_ref()) {
                        for (wi, wb) in c.wheel_bindings.iter().enumerate() {
                            if c.model.bike && wi % 2 == 1 {
                                continue;
                            }
                            pass.set_bind_group(2, &wb.bg, &[]);
                            for b in &w.batches {
                                pass.set_vertex_buffer(0, b.vb.slice(..));
                                pass.set_index_buffer(b.ib.slice(..), wgpu::IndexFormat::Uint32);
                                pass.draw_indexed(0..b.count, 0, 0..1);
                            }
                        }
                    }
                }
            }
        }
    }
    for (mesh, bg, n, pin, pout) in [(&s.ice, &s.ice_bg, ice.len(), &s.ice_in, &s.ice_out), (&s.shield, &s.shield_bg, shield.len(), &s.shield_in, &s.shield_out)] {
        let Some(m) = mesh.as_ref() else { continue };
        if n == 0 {
            continue;
        }
        pass.set_bind_group(1, bg, &[]);
        pass.set_vertex_buffer(0, m.vb.slice(..));
        pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
        for p in [pin, pout] {
            pass.set_pipeline(p);
            pass.draw_indexed(0..m.count, 0, 0..n as u32);
        }
    }
    if nl > 0 {
        pass.set_pipeline(&s.laser);
        pass.set_bind_group(1, &s.laser_bg, &[]);
        pass.set_vertex_buffer(0, s.laser_vb.slice(..(nl * 36) as u64));
        pass.draw(0..nl as u32, 0..1);
    }
    if nb > 0 {
        pass.set_pipeline(&s.bolt);
        pass.set_bind_group(1, &s.bolt_bg, &[]);
        pass.set_vertex_buffer(0, s.bolt_vb.slice(..(nb * 36) as u64));
        pass.draw(0..nb as u32, 0..1);
    }
}

/// The lightning's triangles this frame (the strips as lists): (x, y, z, r, g, b, a, u, v); the
/// alpha x time left / ActiveTime (sub_8237F800 / sub_8237FA80).
pub fn bolt_vertices(pu: &PowerUps, cam: Vec3) -> Vec<[f32; 9]> {
    let mut out = Vec::new();
    for h in &pu.holders {
        let (Some(l), Some(e)) = (h.bolts.as_ref(), h.effect.as_ref()) else { continue };
        let fade = e.left / pu.types[e.kind as usize].active_time;
        for strip in l.strips(cam, fade) {
            for w in strip.windows(2) {
                let (a, b) = (w[0], w[1]);
                let v = |p: Vec3, c: [f32; 4], u: f32, vv: f32| [p.x, p.y, p.z, c[0], c[1], c[2], c[3], u, vv];
                let (a0, a1, b0, b1) = (v(a.0, a.2, a.3, 0.0), v(a.1, a.2, a.3, 1.0), v(b.0, b.2, b.3, 0.0), v(b.1, b.2, b.3, 1.0));
                out.extend_from_slice(&[a0, a1, b0, b0, a1, b1]);
            }
        }
    }
    out
}

/// The shot's sparks (sub_82748340, from the landing sub_8237C5C0): the rmptfx effect
/// "powerup_sparks" at the TARGET, (0, 0.5, 0) along its axes, moving with it (+416 = its
/// velocity), its colour tint m_beamColor (+192) of the shot type's look. The original starts
/// the instance once and lets it run; the port's player keeps an effect while it is asked for:
/// asked for during the record's 2 s.
pub fn sparks(pu: &PowerUps, vehicles: &crate::vehicles::Vehicles, fx: &mut crate::particles::Particles, cam: Vec3) {
    let looks = &pu.looks;
    for h in &pu.hits {
        let Some(c) = vehicles.cars.get(h.to) else { continue };
        let p = c.state.pos + c.state.rot * Vec3::new(0.0, 0.5, 0.0);
        let tint = looks.get(h.kind as usize).map_or([1.0; 3], |l| [l.beam_color[0], l.beam_color[1], l.beam_color[2]]);
        fx.play_tinted(h.key, mcla_formats::hash::joaat("powerup_sparks"), p, tint, [0.0; 3], 1.0, p.distance(cam), &[]);
    }
}

/// The hit light's strength (F-62430: all round, as the siren's).
pub const HIT_LIGHT: f32 = 800.0;

/// The hit light (F-24672): the target record nearest the camera within 1000 m, for its 2 s even when dark. x = 6 t - 1 from
/// the record's start (mgr +40 = 32; (t 192 - 32) / 32). Place: the point (0, 0.5, 1.5) of the shooter and the target, blended
/// by sat(x) (F-81450), 2 m up. Colour: 0.5 (a A + b B) sat(1 - 2 |(x + 0.5) / 2 - 0.5|), a (0.372, 0.702, 0.831), b (0.537,
/// 0.569, 0.98), A = 0.8 g(3 x) + 0.2, B = 0.9 g(9 x) + 0.1.
pub fn hit_light(pu: &PowerUps, vehicles: &crate::vehicles::Vehicles, cam: Vec3) -> Option<(Vec3, [f32; 3])> {
    let mut best: Option<(f32, Vec3, f32)> = None;
    for h in &pu.hits {
        let (Some(a), Some(b)) = (vehicles.cars.get(h.from), vehicles.cars.get(h.to)) else { continue };
        let x = (pu.time - h.start) * 6.0 - 1.0;
        let local = Vec3::new(0.0, 0.5, 1.5);
        let p = (a.state.pos + a.state.rot * local).lerp(b.state.pos + b.state.rot * local, x.clamp(0.0, 1.0));
        let d = p.distance(cam);
        if d < best.map_or(1000.0, |q| q.0) {
            best = Some((d, p, x));
        }
    }
    let (_, p, x) = best?;
    Some((p + Vec3::Y * 2.0, hit_light_colour(x)))
}

fn hit_light_colour(x: f32) -> [f32; 3] {
    let env = (1.0 - 2.0 * ((x + 0.5) * 0.5 - 0.5).abs()).clamp(0.0, 1.0);
    let (ka, kb) = (0.8 * noise1(3.0 * x) + 0.2, 0.9 * noise1(9.0 * x) + 0.1);
    let mix = |a: f32, b: f32| 0.5 * (a * ka + b * kb) * env;
    [mix(0.372, 0.537), mix(0.702, 0.569), mix(0.831, 0.98)]
}

#[cfg(test)]
mod hit_light_tests {
    use super::*;

    /// Dark at x = -1 and x = 1.5, the envelope's ends; lit between.
    #[test]
    fn the_envelope() {
        assert_eq!(hit_light_colour(-1.0), [0.0; 3]);
        assert_eq!(hit_light_colour(1.5), [0.0; 3]);
        assert!(hit_light_colour(0.3).iter().any(|c| *c != 0.0));
    }
}

// ---------------------------------------------------------------------------------------
// The shot's laser fan (step 19g)

/// The laser's layers (the shoot-effect manager's constructor sub_82746B78: 16 records of 28
/// bytes at +1412, all (10, 0, 0, 0, 0, 16, 0.5) - brightness 0, not seen - and the first seven
/// set): width, sideways offset, sway, sway rate, brightness, flicker rate, flicker depth.
pub const LASER_LAYERS: [[f32; 7]; 7] = [
    [10.0, 0.0, 0.0, 0.0, 0.224, 16.0, 0.5],
    [0.15, 10.0, 9.0, 5.824, 0.025, 4.075, 1.0],
    [5.375, 0.0, 1.35, 5.625, 0.025, 29.5, 1.0],
    [2.35, 5.25, 2.55, 2.35, 0.025, 57.152, 1.0],
    [0.075, 10.0, 5.5, 3.6, 0.075, 16.0, 0.5],
    [0.1, 3.28, 3.4, 12.48, 0.125, 16.0, 1.0],
    [1.925, 4.1, 3.075, 1.3, 0.025, 48.0, 1.0],
];

/// The game's 1D gradient noise (sub_8261E060): the permutation at 0x828252B8, the x of the 16
/// gradients at 0x82825AC0, fade 6t^5 - 15t^4 + 10t^3.
pub fn noise1(x: f32) -> f32 {
    const P: [u8; 256] = [151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175, 74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180];
    const GX: [f32; 16] = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, -1.0, 0.0, 0.0];
    let fl = x.floor();
    let f = x - fl;
    let i = (fl as i32 & 255) as usize;
    let a = GX[(P[i] & 15) as usize] * f;
    let b = GX[(P[(i + 1) & 255] & 15) as usize] * (f - 1.0);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    a + (b - a) * u
}

/// One layer's ScaleParams at `t` seconds since the slot started (sub_82747760, x the fade on
/// x and z - sub_82747E20): (w fade, 0, ((2 g(sway rate t) - 1) sway + offset) fade,
/// (1 - g(flicker rate t) depth) brightness).
pub fn laser_params(r: &[f32; 7], t: f32, fade: f32) -> [f32; 4] {
    let z = (2.0 * noise1(r[3] * t) - 1.0) * r[2] + r[1];
    let w = (1.0 - noise1(r[5] * t) * r[6]) * r[4];
    [r[0] * fade, 0.0, z * fade, w]
}

/// The fade of a slot (sub_82747E20): in over +1396 (0.66 s) as (1 - cos(pi x)) / 2, out over
/// +1400 (0.25 s) after its end as (1 + cos(pi x)) / 2.
pub fn laser_fade(t: f32, dur: f32) -> f32 {
    let pi = std::f32::consts::PI;
    if t < dur {
        let x = (t / 0.66).clamp(0.0, 1.0);
        1.0 - ((x * pi).cos() + 1.0) * 0.5
    } else {
        let x = ((t - dur) / crate::powerups::LASER_OUT).clamp(0.0, 1.0);
        ((x * pi).cos() + 1.0) * 0.5
    }
}

/// The laser fans' triangles: (world xyz, local xyz, k rgb) - see pu_fx.wgsl fs_laser. The frame:
/// the shooter's matrix moved by (0, 0.5, 0) along its axes (a motorcycle (0, 0.5, -0.5), turned
/// round (0, 0.5, 0.5)); backwards: its x and z rows turned round.
pub fn laser_vertices(pu: &PowerUps, vehicles: &crate::vehicles::Vehicles, colors: &[[f32; 4]; 24]) -> Vec<[f32; 9]> {
    let mut out = Vec::new();
    // (VS_Laser's alphaWide / alphaNarrow: the effect file's 0.5 / 16)
    let (wide, narrow) = (0.5f32, 16.0f32);
    for h in &pu.holders {
        let Some(c) = vehicles.cars.get(h.car) else { continue };
        let pose = c.render_pose(vehicles.render_alpha);
        for l in &h.lasers {
            let t = pu.time - l.start;
            let dur = l.end - l.start;
            if t > dur + crate::powerups::LASER_OUT || t < 0.0 {
                continue;
            }
            let fade = laser_fade(t, dur);
            let off = if c.model.bike { Vec3::new(0.0, 0.5, if l.back { 0.5 } else { -0.5 }) } else { Vec3::new(0.0, 0.5, 0.0) };
            let o = pose.pos + pose.rot * off;
            let sx = if l.back { -1.0 } else { 1.0 };
            let (ax, az) = (pose.rot * Vec3::X * sx, pose.rot * Vec3::Z * sx);
            let col = colors.get(l.kind as usize).copied().unwrap_or([1.0; 4]);
            for r in LASER_LAYERS.iter() {
                let sp = laser_params(r, t, fade);
                let e = (wide - narrow) * (sp[0] * 0.1).max(0.0).powf(0.2) + narrow;
                let k = [4.0 * col[0] * sp[3] * e, 4.0 * col[1] * sp[3] * e, 4.0 * col[2] * sp[3] * e];
                // (the strip: (u, v) = (-1, 0), (-1, 1), (1, 0), (1, 1); z = 0 / -30)
                let lp = |u: f32, v: f32| Vec3::new(v * (u * sp[0] + sp[2]), 0.0, -30.0 * v);
                let vx = |p: Vec3| {
                    let w = o + ax * p.x + az * p.z;
                    [w.x, w.y, w.z, p.x, p.y, p.z, k[0], k[1], k[2]]
                };
                let (a, b, cc, d) = (vx(lp(-1.0, 0.0)), vx(lp(-1.0, 1.0)), vx(lp(1.0, 0.0)), vx(lp(1.0, 1.0)));
                out.extend_from_slice(&[a, b, cc, cc, b, d]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_as_the_draws_read_them() {
        // the ice: whole to half time, then down to nothing
        assert_eq!(curve(&ICE_KEYS, 0.0)[0], 1.0);
        assert_eq!(curve(&ICE_KEYS, 0.5)[0], 1.0);
        assert!((curve(&ICE_KEYS, 0.75)[0] - 0.5).abs() < 1e-6);
        assert!(curve(&ICE_KEYS, 1.0)[0].abs() < 1e-6);
        // the shield's flash: the keys themselves, and 1 at rest
        let k = curve(&SHIELD_KEYS, 0.076);
        assert!((k[0] - 4.93).abs() < 1e-5 && (k[1] - 7.852).abs() < 1e-5);
        let r = curve(&SHIELD_KEYS, 1.0);
        assert!((r[0] - 1.0).abs() < 1e-5 && (r[1] - 1.0).abs() < 1e-5);
        let mid = curve(&SHIELD_KEYS, 0.1);
        assert!((mid[0] - (4.93 + (2.696 - 4.93) / 0.124 * 0.024)).abs() < 1e-4);
    }

    #[test]
    fn effect_fade_flickers_out() {
        let t = crate::powerups::TypeTune::constructed(5);
        let mut t = t;
        t.active_time = 5.0;
        assert_eq!(crate::powerups::fade_fx(&t, 5.0), 1.0);
        assert_eq!(crate::powerups::fade_fx(&t, 1.6), 1.0);
        // at 1.2 s left: r = 0.8, (1 + 4 cos(20 x 0.04)) x 0.8
        let want = ((1.0 + 4.0 * (20.0f32 * 0.04).cos()) * 0.8).clamp(0.0, 1.0);
        assert!((crate::powerups::fade_fx(&t, 1.2) - want).abs() < 1e-6);
        assert_eq!(crate::powerups::fade_fx(&t, 0.0), 0.0);
        // somewhere in the last 30 % it goes to 0 (the flicker)
        assert!((1..150).any(|i| crate::powerups::fade_fx(&t, i as f32 * 0.01) == 0.0));
    }

    #[test]
    fn ice_and_shield_matrices() {
        // a car facing -z at the origin, bound max z 2.2: the block 0.99425 x 2.2 wide x 1.15 m
        // of mesh, its base 0.014 m below the car's origin
        let m = ice_matrix(Quat::IDENTITY, Vec3::ZERO, 2.2, Vec3::new(0.99425, 0.7425, 2.184), Vec3::new(0.0, -0.014, 0.0), 0.0);
        let top = m.transform_point3(Vec3::new(0.0, 1.0, 0.0));
        assert!((top.y - (0.7425 * 2.2 - 0.014)).abs() < 1e-5);
        // melted to half at 3/4 of the time
        let m = ice_matrix(Quat::IDENTITY, Vec3::ZERO, 2.2, Vec3::new(0.99425, 0.7425, 2.184), Vec3::ZERO, 0.75);
        assert!((m.transform_point3(Vec3::Y).y - 0.5 * 0.7425 * 2.2).abs() < 1e-5);
        // the shield's offset is along the scaled axes
        let m = shield_matrix(Quat::IDENTITY, Vec3::ZERO, 2.0, Vec3::new(1.31, 1.111, 2.373), Vec3::new(0.0, -0.2875, 0.0), 0.0);
        assert!((m.transform_point3(Vec3::ZERO).y - (-0.2875 * 1.111 * 2.0)).abs() < 1e-5);
    }

    #[test]
    fn laser_noise_and_fades() {
        // the noise is 0 on whole numbers and stays within +-0.5 (gradients +-1, 0)
        for i in -5..300 {
            assert!(noise1(i as f32).abs() < 1e-6);
            let v = noise1(i as f32 * 0.37 + 0.13);
            assert!(v.abs() <= 0.5 + 1e-5, "{v}");
        }
        // permutation 151 & 15 = 7 -> gradient -1; at 0.5: -0.5 x fade(0.5) + (g(160&15=0)=1 x -0.5) ...
        let want = { let a = -0.5f32; let b = 1.0 * (0.5 - 1.0); a + (b - a) * 0.5 };
        assert!((noise1(0.5) - want).abs() < 1e-6);
        assert_eq!(laser_fade(0.0, 5.0), 0.0);
        assert!((laser_fade(0.66, 5.0) - 1.0).abs() < 1e-6);
        assert!((laser_fade(0.33, 5.0) - 0.5).abs() < 1e-5);
        assert!((laser_fade(5.0, 5.0) - 1.0).abs() < 1e-6);
        assert!(laser_fade(5.25, 5.0).abs() < 1e-6);
        // layer 0 at t = 0: width 10 x fade, no sideways offset, brightness (1 - 0) x 0.224
        let p = laser_params(&LASER_LAYERS[0], 0.0, 1.0);
        assert_eq!(p, [10.0, 0.0, 0.0, 0.224]);
    }

    #[test]
    #[ignore]
    fn meshes_from_the_disc() {
        let data = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        for (dir, file, nv) in [("fx_iceblock", "pCube1.mesh", 387usize), ("fx_shield", "shield_geo.mesh", 0)] {
            let d = find(&data, dir, file).expect(file);
            let (v, i) = parse_mesh(&String::from_utf8_lossy(&d)).expect(file);
            if nv > 0 {
                assert_eq!(v.len(), nv);
            }
            let (mut lo, mut hi) = (Vec3::splat(1e9), Vec3::splat(-1e9));
            for p in &v {
                lo = lo.min(Vec3::new(p[0], p[1], p[2]));
                hi = hi.max(Vec3::new(p[0], p[1], p[2]));
            }
            // winding against the normals
            let mut out = 0;
            for t in i.chunks(3) {
                let p = |k: u32| Vec3::new(v[k as usize][0], v[k as usize][1], v[k as usize][2]);
                let n = Vec3::new(v[t[0] as usize][3], v[t[0] as usize][4], v[t[0] as usize][5]);
                if (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).dot(n) > 0.0 {
                    out += 1;
                }
            }
            println!("{file}: {} verts, {} tris, box {lo} .. {hi}, ccw-outward {out}", v.len(), i.len() / 3);
        }
    }
}
