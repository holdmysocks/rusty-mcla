//! The effect list's MODEL particles (`ptxModel`: the litter on the wind - wind_debris_hw / _la /
//! _beach, receipts, flyers, paper, leaves - and the collision effects col_cans, col_bananas,
//! col_newspaper ...) drawn as the original draws them: each particle is its drawable's own
//! triangles, scaled, turned by three Euler angles and spun about a random axis.
//!
//! Rules read from the recomp (`work/map2/model/model_particle_rule.md`): birth `sub_824B5BA8`
//! (`Orient::birth`), step `sub_824B7930` (`Orient::step`), draw `sub_824BA460` -> `sub_824B6E40`
//! (`Orient::world`, `push`), the shader `mc4_rmptfx_model` (VSRrmptfxModel / PSRrmptfxModel,
//! `render/modelfx.wgsl`). Where they move, their colour and alpha are the particle player's
//! (`particles.rs`, the same as the sprites'); this file adds the orientation and the draw.

use crate::render::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Quat, Vec3};
use hashbrown::HashMap;
use mcla_formats::rage::ptfx::{Model, ModelDrawable, ParticleRule};
use mcla_formats::rage::texture::Texture;
use mcla_formats::rage::{Dictionary, Rd};
use std::sync::{Arc, Mutex};

/// model rule flag bit 8 (`rlwinm r10,r11,24,31,31` on rule+0x11c in the birth): one random
/// number for the three axes of the scale
const UNIFORM_SCALE: u32 = 1 << 8;
/// model rule flag bit 12: the matrix is also turned by the effect's (not set in any model rule)
const EFFECT_MATRIX: u32 = 1 << 12;
/// degrees to radians, the executable's 0x82006BFC
const DEG: f32 = 0.017453292;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A model particle's orientation and scale state (particle data D+0..8, the update state
/// S+0..12 and S+88, the size table bytes D+137..139).
#[derive(Clone, Copy, Debug, Default)]
pub struct Orient {
    /// the Euler angles at birth (radians, about x, y, z)
    euler: Vec3,
    /// the spin axis (normalised; zero when the random vector was)
    axis: Vec3,
    /// the angle about it (radians) and its speed (radians a second)
    angle: f32,
    speed: f32,
    /// the random numbers of the scale's x, y, z (one for all three with flag bit 8)
    size_r: [f32; 3],
    /// the effect instance's zoom (EI+500: its ZoomKF / 100) and the emit rule's SizeKFOT at the
    /// birth x 0.01 - the draw's scale factors
    zoom: f32,
    sot: Vec3,
}

impl Orient {
    /// The birth (`sub_824B5BA8`): the size random numbers, then the Euler angles (three fresh
    /// numbers, x first), the axis (three more), the spin speed (one) - in that order. `te` is the
    /// emitter's time at the birth (EM+128), `zoom` the effect's ZoomKF / 100, `sot` the emit
    /// rule's SizeKFOT (percent) at `te`.
    pub fn birth(pr: &ParticleRule, m: &Model, te: f32, zoom: f32, sot: [f32; 4], rnd: &mut dyn FnMut() -> f32) -> Orient {
        let r0 = rnd();
        let size_r = if pr.flags & UNIFORM_SCALE != 0 { [r0; 3] } else { [r0, rnd(), rnd()] };
        // InitialThetaMin / Max (degrees) keyed on the emitter's time
        let (a, b) = (m.init_theta_min.eval(te), m.init_theta_max.eval(te));
        let ex = lerp(a[0], b[0], rnd());
        let ey = lerp(a[1], b[1], rnd());
        let ez = lerp(a[2], b[2], rnd());
        // the spin axis: 'InitialRotationMin / Max' per component, normalised
        let (a, b) = (m.init_rotation_min.eval(te), m.init_rotation_max.eval(te));
        let ax = lerp(a[0], b[0], rnd());
        let ay = lerp(a[1], b[1], rnd());
        let az = lerp(a[2], b[2], rnd());
        // InitRotationSpeed (turns a second) -> radians a second; the angle starts at the
        // speed's value (both stores, S+88 and S+12)
        let k = m.init_rotation_speed.eval(te);
        let w = lerp(k[0], k[1], rnd()) * std::f32::consts::TAU;
        Orient {
            euler: Vec3::new(ex, ey, ez) * DEG,
            axis: Vec3::new(ax, ay, az).normalize_or_zero(),
            angle: w,
            speed: w,
            size_r,
            zoom,
            sot: Vec3::new(sot[0], sot[1], sot[2]) * 0.01,
        }
    }

    /// The step's spin (`sub_824B7930` 0x824B7FC8..0x824B8058): the speed damped by
    /// DampeningKF's w (no clamp), the angle on by it and by RotationSpeedKF (turns a second,
    /// over the life `t`). `r_rot`, `r_damp`: the particle's random numbers of the two keys;
    /// `dt` the particle's step.
    pub fn step(&mut self, pr: &ParticleRule, m: &Model, t: f32, r_rot: f32, r_damp: f32, dt: f32) {
        let rs = m.rotation_speed.eval(t);
        let rs = lerp(rs[0], rs[1], r_rot);
        let dw = lerp(pr.damp_min.eval(t)[3], pr.damp_max.eval(t)[3], r_damp);
        self.speed *= 1.0 - dw * dt;
        self.angle += self.speed * dt + rs * dt * std::f32::consts::TAU;
    }

    /// The draw's matrix (`sub_824B6E40`): world = pos + [E3 x] R(axis, angle) x Rz x Ry x Rx x
    /// diag(s) x v. s = SizeMin..Max at the life `t` by the size numbers x the zoom x SizeKFOT;
    /// the Euler angles + ThetaMinKF (degrees, keyed on the emitter's time `tem`).
    pub fn world(&self, flags: u32, m: &Model, t: f32, tem: f32, pos: Vec3, effect: Quat) -> Mat4 {
        let (s0, s1) = (m.size_min.eval(t), m.size_max.eval(t));
        let s = Vec3::new(lerp(s0[0], s1[0], self.size_r[0]), lerp(s0[1], s1[1], self.size_r[1]), lerp(s0[2], s1[2], self.size_r[2])) * self.zoom * self.sot;
        let th = m.theta_min.eval(tem);
        let e = self.euler + Vec3::new(th[0], th[1], th[2]) * DEG;
        // (sub_82350940: angle 0 -> no turn; a pure x / y / z axis turns about that axis - the
        // same as the general axis-angle form)
        let spin = if self.angle == 0.0 || self.axis == Vec3::ZERO { Mat3::IDENTITY } else { Mat3::from_axis_angle(self.axis, self.angle) };
        let mut r = spin * Mat3::from_rotation_z(e.z) * Mat3::from_rotation_y(e.y) * Mat3::from_rotation_x(e.x) * Mat3::from_diagonal(s);
        if flags & EFFECT_MATRIX != 0 {
            r = Mat3::from_quat(effect) * r;
        }
        Mat4::from_cols(r.x_axis.extend(0.0), r.y_axis.extend(0.0), r.z_axis.extend(0.0), pos.extend(1.0))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    color: [u8; 4],
}

/// One particle's draw: gWorld (columns), gForcedColor, the light picker's place (the effect's
/// cull-sphere centre: the four lights and LightingMode 2's key-light point are taken there) with
/// the rule's LightingMode, and the night fill light added to gLightAmbient there.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Inst {
    world: [[f32; 4]; 4],
    color: [f32; 4],
    at: [f32; 4],
    fill: [f32; 4],
}

const INST_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<Inst>() as u64,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &[
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 4 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 5 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 6 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 48, shader_location: 7 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 64, shader_location: 8 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 80, shader_location: 9 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 96, shader_location: 10 },
    ],
};

struct Mesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
    bind: Arc<wgpu::BindGroup>,
}

struct State {
    pipeline: wgpu::RenderPipeline,
    /// by (particle rule, drawable)
    meshes: HashMap<(u16, u16), Mesh>,
    /// this frame's particles in the order they are drawn
    list: Vec<((u16, u16), Inst)>,
    buf: Option<wgpu::Buffer>,
    cap: usize,
    /// frames drawn (the log's clock)
    log_frame: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn make_pipeline(gpu: &Gpu) -> wgpu::RenderPipeline {
    let device = &gpu.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("model particles"), source: wgpu::ShaderSource::Wgsl(include_str!("render/modelfx.wgsl").into()) });
    // (the globals and the effect sheets' texture group, as the sprites have them)
    let g0 = gpu.pipeline_particle.get_bind_group_layout(0);
    let g1 = gpu.pipeline_particle.get_bind_group_layout(1);
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("model particles pl"), bind_group_layouts: &[&g0, &g1], push_constant_ranges: &[] });
    let vtx = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 32, shader_location: 3 },
        ],
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("model particles"),
        layout: Some(&pl),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_model"), buffers: &[vtx, INST_LAYOUT], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_model"),
            // (RenderState of all 10 model rules: BlendSet 0 - alpha blended -, AlphaBlend 1)
            targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        // (two-sided: PSRrmptfxModel turns the normal by the face register)
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, front_face: wgpu::FrontFace::Cw, ..Default::default() },
        // (DepthTest 1, DepthWrite 1; the scene's reversed depth)
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::GreaterEqual, stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}

/// At the effect list's load: the triangles of drawable `card` of particle rule `rule` and its
/// diffuse texture from the effect list's dictionary (`rd`, `dict`). False when it has none.
/// The texture is decoded with gamma: the recorded model draws' DiffuseSampler fetch constant
/// asks for it on red, green and blue (word 0 0x810000fe: signs 3, 3, 3, 0 - the city's textures'
/// way), where the sprites' sheets ask for none.
pub fn add_mesh(gpu: &mut Gpu, rule: usize, card: usize, d: &ModelDrawable, rd: &Rd, dict: &Dictionary) -> bool {
    use wgpu::util::DeviceExt;
    let mm = &d.mesh;
    if mm.indices.is_empty() {
        return false;
    }
    let bare = d.texture.trim_end_matches(".dds");
    let Some(h) = [d.texture.clone(), bare.to_string(), format!("{bare}.dds")].iter().map(|n| mcla_formats::hash::joaat(n)).find(|h| dict.get(*h).is_some()) else { return false };
    let Some(t) = dict.get(h).and_then(|o| Texture::parse(rd, o).ok()) else { return false };
    let key = h ^ 0x7d00_0000;
    let tex = gpu.upload_texture(key, &t);
    let bind = gpu.texture_bind_group(&tex, &tex, key, key);
    let gpu: &Gpu = gpu;
    let mut st = STATE.lock().unwrap();
    let st = st.get_or_insert_with(|| State { pipeline: make_pipeline(gpu), meshes: HashMap::new(), list: Vec::new(), buf: None, cap: 0, log_frame: 0 });
    let verts: Vec<Vertex> = (0..mm.positions.len()).map(|i| Vertex { pos: mm.positions[i], normal: mm.normals[i], uv: mm.uv[i], color: mm.colors[i] }).collect();
    let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("model particle vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
    let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("model particle ib"), contents: bytemuck::cast_slice(&mm.indices), usage: wgpu::BufferUsages::INDEX });
    st.meshes.insert((rule as u16, card as u16), Mesh { vb, ib, count: mm.indices.len() as u32, bind });
    log::debug!("modelfx: rule {rule} drawable {card} {}: {} triangles", d.name, mm.indices.len() / 3);
    true
}

/// Before the particles' instances are made (`Particles::prepare`): this frame's list is emptied.
pub fn begin() {
    if let Some(st) = STATE.lock().unwrap().as_mut() {
        st.list.clear();
    }
}

/// One model particle this frame: its matrix, gForcedColor (the colour and alpha of
/// `sub_824B5728`), the light picker's place `at` and the rule's LightingMode.
pub fn push(rule: u16, card: u16, world: Mat4, color: [f32; 4], at: Vec3, lighting_mode: u32) {
    let fill = crate::fillmap::body_now(at.x, at.z);
    if let Some(st) = STATE.lock().unwrap().as_mut() {
        if st.meshes.contains_key(&(rule, card)) {
            st.list.push(((rule, card), Inst { world: world.to_cols_array_2d(), color, at: [at.x, at.y, at.z, lighting_mode as f32], fill: [fill[0], fill[1], fill[2], 0.0] }));
        }
    }
}

/// Draw this frame's model particles over the scene (before the sprites' pass: they write the
/// depth), in the order the particle player gave them.
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder) {
    let mut guard = STATE.lock().unwrap();
    let Some(st) = guard.as_mut() else { return };
    if st.list.is_empty() {
        return;
    }
    if st.buf.is_none() || st.cap < st.list.len() {
        st.cap = (st.list.len() * 2).max(64);
        st.buf = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("model particle instances"), size: (st.cap * std::mem::size_of::<Inst>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
    }
    let insts: Vec<Inst> = st.list.iter().map(|x| x.1).collect();
    let Some(buf) = st.buf.as_ref() else { return };
    gpu.queue.write_buffer(buf, 0, bytemuck::cast_slice(&insts));
    st.log_frame += 1;
    if st.log_frame % 120 == 1 && std::env::var("MCLA_MODELFX_LOG").is_ok() {
        if let Some((k, i)) = st.list.first() {
            log::info!("modelfx: {} models; first rule {} drawable {} world {:?} colour {:?} at {:?} fill {:?}", st.list.len(), k.0, k.1, i.world, i.color, i.at, i.fill);
        }
    }
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("model particles"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.post.scene_view(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &gpu.depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_pipeline(&st.pipeline);
    pass.set_bind_group(0, &gpu.globals_bg, &[]);
    pass.set_vertex_buffer(1, buf.slice(..));
    // (runs of the same drawable in one call; the order is kept)
    let mut i = 0;
    while i < st.list.len() {
        let key = st.list[i].0;
        let mut j = i + 1;
        while j < st.list.len() && st.list[j].0 == key {
            j += 1;
        }
        if let Some(m) = st.meshes.get(&key) {
            pass.set_bind_group(1, m.bind.as_ref(), &[]);
            pass.set_vertex_buffer(0, m.vb.slice(..));
            pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..m.count, 0, i as u32..j as u32);
        }
        i = j;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The draw's matrix against the rule's row-vector product, M = diag(s) Rx Ry Rz Raxis
    /// (`sub_82235E30` rows (1,0,0)(0,c,s)(0,-s,c), `sub_82204CB8` (c,0,-s)(0,1,0)(s,0,c),
    /// `sub_82235EE0` (c,s,0)(-s,c,0)(0,0,1), `sub_82227F50` the axis-angle form, m01 =
    /// xy(1-c) + s z), v_world = v M + pos.
    #[test]
    fn model_matrix_matches_row_form() {
        type M3 = [[f32; 3]; 3];
        fn mul(a: M3, b: M3) -> M3 {
            let mut r = [[0.0; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
                }
            }
            r
        }
        let (ex, ey, ez, ang) = (0.7f32, -1.3f32, 2.1f32, 0.9f32);
        let ax = Vec3::new(0.3, -0.5, 0.8).normalize();
        let s = Vec3::new(1.9, 1.7, 2.2);
        let rx = |a: f32| [[1.0, 0.0, 0.0], [0.0, a.cos(), a.sin()], [0.0, -a.sin(), a.cos()]];
        let ry = |a: f32| [[a.cos(), 0.0, -a.sin()], [0.0, 1.0, 0.0], [a.sin(), 0.0, a.cos()]];
        let rz = |a: f32| [[a.cos(), a.sin(), 0.0], [-a.sin(), a.cos(), 0.0], [0.0, 0.0, 1.0]];
        let (c, sn, t) = (ang.cos(), ang.sin(), 1.0 - ang.cos());
        let (x, y, z) = (ax.x, ax.y, ax.z);
        let raxis = [[t * x * x + c, t * x * y + sn * z, t * x * z - sn * y], [t * x * y - sn * z, t * y * y + c, t * y * z + sn * x], [t * x * z + sn * y, t * y * z - sn * x, t * z * z + c]];
        let d = [[s.x, 0.0, 0.0], [0.0, s.y, 0.0], [0.0, 0.0, s.z]];
        let m = mul(mul(mul(mul(d, rx(ex)), ry(ey)), rz(ez)), raxis);
        let o = Orient { euler: Vec3::new(ex, ey, ez), axis: ax, angle: ang, speed: 0.0, size_r: [0.0; 3], zoom: 1.0, sot: Vec3::ONE };
        let mut km = Model {
            drawables: Vec::new(),
            size_min: Default::default(),
            size_max: Default::default(),
            init_rotation_min: Default::default(),
            init_rotation_max: Default::default(),
            init_rotation_speed: Default::default(),
            rotation_speed: Default::default(),
            init_theta_min: Default::default(),
            init_theta_max: Default::default(),
            theta_min: Default::default(),
        };
        let key = |v: [f32; 4]| mcla_formats::rage::ptfx::Keyframe { keys: vec![mcla_formats::rage::ptfx::Key { t: 0.0, v, d: [0.0; 4] }] };
        km.size_min = key([s.x, s.y, s.z, 1.0]);
        km.size_max = key([s.x, s.y, s.z, 1.0]);
        let pos = Vec3::new(10.0, -3.0, 5.5);
        let w = o.world(0x2788, &km, 0.5, 0.5, pos, Quat::from_rotation_y(1.0));
        let v = Vec3::new(0.11, -0.07, 0.19);
        let row = Vec3::new((0..3).map(|k| [v.x, v.y, v.z][k] * m[k][0]).sum(), (0..3).map(|k| [v.x, v.y, v.z][k] * m[k][1]).sum(), (0..3).map(|k| [v.x, v.y, v.z][k] * m[k][2]).sum());
        assert!((w.transform_point3(v) - (row + pos)).length() < 1e-5, "{:?} vs {:?}", w.transform_point3(v), row + pos);
    }
}
