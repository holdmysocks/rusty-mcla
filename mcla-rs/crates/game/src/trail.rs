//! The lamps' light trails (docs/LIGHTING.md "Light trails"): a ribbon behind every lit prop and
//! car lamp that the original draws after the glows of its glow pass (PSTrailSeedRT), additive
//! into the halo picture. The rule is the original's, translated from its code:
//!
//! - `AddGlow` (F-92524) keeps one of 96 records a light (the caller keeps the slot): slot -1 or a
//!   record left undrawn last frame or held by another light takes the first free record
//!   (F-58134); a record not used last frame starts empty. Then the history push (F-25303): eight
//!   points in VIEW space, newest first, each with its frame and value = the caller's trail
//!   strength x the camera's "moved" factor; the width is the glow's size.x after its distance
//!   growth.
//! - The draw (F-89089) first runs the camera cut (F-40536): the camera turned (forward dot < 0.2)
//!   or moved over 4 m since the last frame - every record emptied and no trail this frame; the
//!   moved factor = min(1, the camera's move this frame). Paused: every record emptied. A record
//!   not used this frame loses a point; one with three or more points becomes a ribbon (F-80988).
//!
//! The original runs at 30 frames a second: here the records live on a 1/30 s tick, and between
//! ticks the newest point follows the lamp in the current view.

use crate::render::Gpu;
use glam::{Mat4, Vec2, Vec3, Vec4};
use std::collections::HashMap;
use std::sync::Mutex;

/// the original's frame
pub const FRAME: f32 = 1.0 / 30.0;
/// records (the glow list's 96)
pub const RECORDS: usize = 96;
/// points a record
pub const POINTS: usize = 8;

/// Denormals in and out of the vector unit are zero.
#[inline]
fn ftz(x: f32) -> f32 {
    if x != 0.0 && x.is_subnormal() { 0.0f32.copysign(x) } else { x }
}

fn vmap(a: Vec3, b: Vec3, f: impl Fn(f32, f32) -> f32) -> Vec3 {
    Vec3::new(ftz(f(ftz(a.x), ftz(b.x))), ftz(f(ftz(a.y), ftz(b.y))), ftz(f(ftz(a.z), ftz(b.z))))
}

fn vadd(a: Vec3, b: Vec3) -> Vec3 {
    vmap(a, b, |x, y| x + y)
}

fn vsub(a: Vec3, b: Vec3) -> Vec3 {
    vmap(a, b, |x, y| x - y)
}

fn vmul(a: Vec3, b: Vec3) -> Vec3 {
    vmap(a, b, |x, y| x * y)
}

/// vmsum3fp of a with itself.
fn vdot(a: Vec3) -> f32 {
    let p = vmul(a, a);
    ftz(ftz(0.0 + p.z) + ftz(p.y + p.x))
}

/// vrsqrtefp (1 / sqrt), an infinite estimate replaced by 0, one Newton step -(x y y - 1) (0.5 y) + y.
fn rsq(x: f32) -> f32 {
    let x = ftz(x);
    let y0 = ftz(1.0 / ftz(x.sqrt()));
    let y = if y0 == f32::INFINITY { 0.0 } else { y0 };
    let e = -ftz(ftz(x * ftz(y * y)) - 1.0);
    ftz(ftz(e * ftz(0.5 * y)) + y)
}

fn unit(v: Vec3) -> Vec3 {
    vmul(v, Vec3::splat(rsq(vdot(v))))
}

/// The product as the code has it: vnmsubfp -(a.yzx b.zxy - a.zxy b.yzx), i.e. b cross a (a zero comes out -0).
fn cross_ba(a: Vec3, b: Vec3) -> Vec3 {
    let zxy = |v: Vec3| Vec3::new(v.z, v.x, v.y);
    let yzx = |v: Vec3| Vec3::new(v.y, v.z, v.x);
    -vsub(vmul(yzx(a), zxy(b)), vmul(zxy(a), yzx(b)))
}

/// One light's record (the original's 336-byte record, 96 of them).
#[derive(Clone, Copy, Debug, Default)]
pub struct Record {
    /// +304 points held (at most 8)
    pub count: u32,
    /// +308 the frame of the last use
    pub stamp: u32,
    /// +312 the light (the caller's own pointer in the original)
    pub id: u64,
    /// +316 drawn since the last use
    pub drawn: bool,
    /// +48 the colour's bytes
    pub rgb: [u8; 3],
    /// +324 the width (the glow's size.x)
    pub width: f32,
    /// +112 the points in view space, newest first
    pub h: [Vec3; POINTS],
    /// +240 their frames
    pub stamps: [u32; POINTS],
    /// +272 their values
    pub vals: [f32; POINTS],
    /// (the port: the newest point's world place, for the frames between ticks)
    pub place: Vec3,
}

/// The ribbon of one record (F-80988 with f1 = 0.75): centre, half width across, alpha a point; the
/// two end caps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ribbon {
    pub centre: Vec<Vec3>,
    pub side: Vec<Vec3>,
    pub alpha: Vec<f32>,
    pub start: Vec3,
    pub end: Vec3,
}

/// F-80988: the points (the newest, then the middles of the history's neighbours), their places on the
/// screen by `screen` (the device's view-projection-viewport matrix, view = identity: pixels), a
/// point only where it is 2 pixels or more on from the last one; across the way = the way x the
/// point's direction, blended with twice the last across; the caps along (the way across) x the
/// direction at the first and the last point. `q0`: the first point's place on the screen - the
/// original reads a stack slot no one wrote (see docs/LIGHTING.md), here the point's projection.
pub fn ribbon(r: &Record, frame: u32, fade: f32, screen: impl Fn(Vec3) -> Vec2) -> Ribbon {
    let n = r.count as usize;
    let mut p = [Vec3::ZERO; POINTS];
    let mut q = [Vec2::ZERO; POINTS];
    p[0] = r.h[0];
    q[0] = screen(p[0]);
    for i in 1..n {
        p[i] = vmul(vadd(r.h[i], r.h[i - 1]), Vec3::splat(0.5));
        q[i] = screen(p[i]);
    }
    // (0.045 as the image holds it: 0x3D3851EB, a step under the nearest float)
    let w = Vec3::splat(ftz(r.width * f32::from_bits(0x3D38_51EB)));
    let len = |a: Vec2, b: Vec2| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        ((dx as f64 * dx as f64 + (dy * dy) as f64) as f32).sqrt()
    };
    let mut way = Vec3::new(0.0, 1.0, 0.0);
    for k in 0..n.saturating_sub(1) {
        if len(q[k], q[k + 1]) > 2.0 {
            way = vsub(p[k + 1], p[k]);
            break;
        }
    }
    let mut out = Ribbon::default();
    let mut across = Vec3::ZERO;
    let mut first = true;
    for i in 0..n.saturating_sub(1) {
        let mut dir = vsub(p[i + 1], p[i]);
        if len(q[i], q[i + 1]) < 2.0 {
            dir = way;
            if !first {
                continue;
            }
        }
        way = dir;
        out.centre.push(p[i]);
        let np = unit(p[i]);
        let mut s = unit(cross_ba(np, unit(dir)));
        if !first {
            s = unit(vadd(vadd(s, across), across));
        }
        across = s;
        let t = unit(cross_ba(np, s));
        if out.side.is_empty() {
            out.start = vmul(t, w);
            out.end = vmul(vsub(Vec3::ZERO, t), w);
        } else if i + 2 == n {
            out.end = vmul(vsub(Vec3::ZERO, t), w);
        }
        out.side.push(vmul(s, w));
        let age = frame.wrapping_sub(r.stamps[i]) as i32;
        out.alpha.push(((fade as f64).powf(age as f64) as f32) * r.vals[i]);
        first = false;
    }
    if out.side.len() <= 1 {
        // (one point: doubled; its second alpha is a stale slot in the original - both ends draw at 0)
        if let (Some(&s), Some(&c)) = (out.side.first(), out.centre.first()) {
            out.side.push(s);
            out.centre.push(c);
            out.alpha.push(0.0);
        }
    }
    out
}

/// The trails' state from frame to frame.
pub struct Trails {
    pub recs: [Record; RECORDS],
    /// the slot each caller keeps (-1: none yet)
    pub slots: HashMap<u64, i32>,
    /// the original's frame counter
    pub frame: u32,
    acc: f32,
    /// the camera's place and z axis at the last draw (the cut's saved matrix)
    saved: Option<(Vec3, Vec3)>,
    /// min(1, the camera's move a frame) (the push's factor)
    pub moved: f32,
    /// this tick's draw was cut
    cut: bool,
    /// this frame is a tick
    tick: bool,
}

impl Default for Trails {
    fn default() -> Self {
        Trails { recs: [Record::default(); RECORDS], slots: HashMap::new(), frame: 1, acc: 0.0, saved: None, moved: 0.0, cut: false, tick: false }
    }
}

impl Trails {
    /// A frame of `dt` seconds: whether the original's frame turns (the lights are added and the
    /// draw's update runs only then).
    pub fn begin(&mut self, dt: f32) -> bool {
        self.acc += dt.clamp(0.0, 0.25);
        self.tick = self.acc >= FRAME * 0.999;
        if self.tick {
            self.acc = (self.acc - FRAME).min(FRAME);
            self.frame = self.frame.wrapping_add(1);
        }
        self.tick
    }

    /// F-58134: the first record with no points, made empty; -1 if none.
    fn free(&mut self) -> i32 {
        for (i, r) in self.recs.iter_mut().enumerate() {
            if r.count == 0 {
                r.drawn = false;
                r.count = 0;
                return i as i32;
            }
        }
        -1
    }

    /// AddGlow's trail part (F-92524) for one light on a tick: `place` the glow's place (pulled towards the
    /// camera), `width` its size.x after the growth, `trail` the caller's trail strength, `view` the frame's
    /// world-to-view matrix.
    pub fn add(&mut self, id: u64, place: Vec3, rgb: [f32; 3], width: f32, trail: f32, view: &Mat4) {
        let frame = self.frame;
        let mut s = self.slots.get(&id).copied().unwrap_or(-1);
        let reuse = s >= 0 && (s as usize) < RECORDS && {
            let r = &self.recs[s as usize];
            !(r.stamp == frame.wrapping_sub(1) && !r.drawn) && r.id == id
        };
        if !reuse {
            s = self.free();
        }
        if !(0..RECORDS as i32).contains(&s) {
            self.slots.insert(id, -1);
            return;
        }
        self.slots.insert(id, s);
        let moved = self.moved;
        let r = &mut self.recs[s as usize];
        if r.stamp != frame.wrapping_sub(1) {
            r.drawn = false;
            r.count = 0;
        }
        r.stamp = frame;
        r.rgb = rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        r.id = id;
        // F-25303: the history a point on, the newest in view space
        for k in (1..POINTS).rev() {
            r.h[k] = r.h[k - 1];
            r.stamps[k] = r.stamps[k - 1];
            r.vals[k] = r.vals[k - 1];
        }
        r.h[0] = view.transform_point3(place);
        r.stamps[0] = frame;
        r.vals[0] = trail * moved;
        r.count = (r.count + 1).min(POINTS as u32);
        r.width = width;
        r.place = place;
    }

    /// The draw's update on a tick (F-89089 up to the ribbons): the camera cut (F-40536), the pause, a
    /// point less for a record not used this frame. `cam`: the camera's place and z axis.
    pub fn update(&mut self, cam: (Vec3, Vec3), paused: bool) {
        let saved = self.saved.unwrap_or(cam);
        let d = cam.0 - saved.0;
        let dist = ((d.x * d.x + d.y * d.y) + d.z * d.z).sqrt();
        let cut = saved.1.dot(cam.1) < 0.2 || dist > 4.0;
        if cut {
            for r in self.recs.iter_mut() {
                r.count = 0;
            }
        }
        self.saved = Some(cam);
        self.moved = dist.clamp(0.0, 1.0);
        self.cut = cut;
        if cut {
            return;
        }
        if paused {
            for r in self.recs.iter_mut() {
                r.count = 0;
            }
        }
        let frame = self.frame;
        for r in self.recs.iter_mut() {
            if r.stamp != frame && r.count > 0 {
                r.count -= 1;
            }
            r.drawn = true;
        }
    }

    /// The ribbons' vertices for this frame (two triangles a step of the original's strip). `view` the
    /// frame's world-to-view matrix; `screen` view space to the original's pixels.
    pub fn vertices(&self, view: &Mat4, screen: impl Fn(Vec3) -> Vec2 + Copy) -> Vec<Vertex> {
        let mut v = Vec::new();
        if self.cut {
            return v;
        }
        for r in self.recs.iter().filter(|r| r.count > 2) {
            let mut r = *r;
            if !self.tick && r.stamp == self.frame {
                // (between ticks the newest point is the lamp's place now)
                r.h[0] = view.transform_point3(r.place);
            }
            let rb = ribbon(&r, self.frame, 0.75, screen);
            let m = rb.side.len();
            if m == 0 {
                continue;
            }
            let c = r.h[0];
            let col = r.rgb.map(|b| b as f32 / 255.0);
            let mut pair = Vec::with_capacity(m);
            for j in 0..m {
                let (u, f) = if j == 0 { (0.0, 0.0) } else if j == m - 1 { (1.0, 0.0) } else { (0.5, 1.0) };
                let (mut a, mut b) = (rb.centre[j] - rb.side[j], rb.centre[j] + rb.side[j]);
                if j == 0 {
                    a += rb.start;
                    b += rb.start;
                }
                if j == m - 1 {
                    a += rb.end;
                    b += rb.end;
                }
                let alpha = ((rb.alpha[j] * f * 255.0) as i32).clamp(0, 255) as f32 / 255.0;
                let vx = |p: Vec3, x: f32| Vertex { pos: [p.x, p.y, p.z, 1.0], uv: [x, u, 0.0, 0.0], col: [col[0], col[1], col[2], alpha], centre: [c.x, c.y, c.z, 1.0] };
                pair.push((vx(a, 0.0), vx(b, 1.0)));
            }
            for j in 1..m {
                let (a0, b0) = pair[j - 1];
                let (a1, b1) = pair[j];
                v.extend_from_slice(&[a0, b0, a1, b0, b1, a1]);
            }
        }
        v
    }
}

/// One vertex of a ribbon (trail.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// view space
    pub pos: [f32; 4],
    pub uv: [f32; 4],
    pub col: [f32; 4],
    /// the newest point (the program's CentrePosition), view space
    pub centre: [f32; 4],
}

/// most vertices a frame: 96 records of 8 points, 6 a step
const MAX_VERTS: usize = RECORDS * (POINTS - 1) * 6;

struct Gpux {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    buf: wgpu::Buffer,
    count: u32,
}

static STATE: Mutex<Option<(Trails, Option<Gpux>)>> = Mutex::new(None);

/// The original's screen at 1280 x 720 (the 2-pixel test is in its pixels): view space through `proj`.
pub fn screen_of(proj: Mat4) -> impl Fn(Vec3) -> Vec2 + Copy {
    move |p: Vec3| {
        let c = proj * Vec4::new(p.x, p.y, p.z, 1.0);
        let w = if c.w.abs() < 1e-12 { 1e-12 } else { c.w };
        Vec2::new(c.x / w * 640.0 + 640.0, -c.y / w * 360.0 + 360.0)
    }
}

/// With the halo pass's pipelines.
pub fn load(gpu: &Gpu) {
    let device = &gpu.device;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("trail layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
        ],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("trail"), source: wgpu::ShaderSource::Wgsl(include_str!("render/trail.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("trail pl"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
    const ATTRS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4];
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("trail"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_trail"),
            buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &ATTRS }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_trail"),
            targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("trail proj"), size: 64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("trail verts"), size: (MAX_VERTS * std::mem::size_of::<Vertex>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut st = STATE.lock().unwrap();
    let trails = st.take().map(|s| s.0).unwrap_or_default();
    *st = Some((trails, Some(Gpux { pipeline, layout, uniform, buf, count: 0 })));
}

/// The frame's start: whether this frame is one of the original's (a tick); on a tick hand every light to
/// `add` before `finish`.
pub fn begin(dt: f32) -> bool {
    let mut st = STATE.lock().unwrap();
    st.get_or_insert_with(|| (Trails::default(), None)).0.begin(dt)
}

/// AddGlow's trail part for one light (on a tick).
pub fn add(id: u64, place: Vec3, rgb: [f32; 3], width: f32, trail: f32, view: &Mat4) {
    if let Some((t, _)) = STATE.lock().unwrap().as_mut() {
        t.add(id, place, rgb, width, trail, view);
    }
}

/// The draw's update (on a tick) and this frame's ribbons. `view` world to view, `proj` view to clip (the
/// port's), `cam` the camera's place and z axis; `on` false: nothing drawn (the garage, a car picture).
pub fn finish(gpu: &Gpu, view: Mat4, proj: Mat4, cam: (Vec3, Vec3), paused: bool, on: bool) {
    let mut st = STATE.lock().unwrap();
    let Some((t, g)) = st.as_mut() else { return };
    if t.tick {
        t.update(cam, paused);
    }
    let Some(g) = g.as_mut() else { return };
    // (MCLA_NO_TRAILS: none drawn - the records still run)
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let on = on && !*OFF.get_or_init(|| std::env::var("MCLA_NO_TRAILS").is_ok());
    let mut v = if on { t.vertices(&view, screen_of(proj)) } else { Vec::new() };
    v.truncate(MAX_VERTS);
    if !v.is_empty() {
        gpu.queue.write_buffer(&g.buf, 0, bytemuck::cast_slice(&v));
        gpu.queue.write_buffer(&g.uniform, 0, bytemuck::cast_slice(&proj.to_cols_array()));
    }
    g.count = v.len() as u32;
}

/// The ribbons into the halo pass (after the glows, as the original's draw order).
pub fn draw<'a>(gpu: &Gpu, pass: &mut wgpu::RenderPass<'a>) {
    let st = STATE.lock().unwrap();
    let Some((_, Some(g))) = st.as_ref() else { return };
    if g.count == 0 {
        return;
    }
    let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("trail"),
        layout: &g.layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: g.uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&gpu.depth) },
        ],
    });
    pass.set_pipeline(&g.pipeline);
    pass.set_bind_group(0, &bind, &[]);
    pass.set_vertex_buffer(0, g.buf.slice(..));
    pass.draw(0..g.count, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_light_keeps_its_record_and_the_history_runs() {
        let mut t = Trails::default();
        t.moved = 1.0;
        let view = Mat4::IDENTITY;
        for k in 0..10 {
            t.begin(FRAME);
            t.add(7, Vec3::new(k as f32, 0.0, -10.0), [1.0, 0.5, 0.0], 27.0, 0.5, &view);
            t.update((Vec3::ZERO, Vec3::Z), false);
        }
        let r = &t.recs[0];
        assert_eq!((r.count, r.id, r.rgb), (8, 7, [255, 128, 0]));
        assert_eq!(r.h[0].x, 9.0);
        assert_eq!(r.h[7].x, 2.0);
        // (a light not added loses a point a frame)
        t.begin(FRAME);
        t.update((Vec3::ZERO, Vec3::Z), false);
        assert_eq!(t.recs[0].count, 7);
        // (a camera turned away: all gone)
        t.begin(FRAME);
        t.update((Vec3::ZERO, Vec3::X), false);
        assert_eq!(t.recs[0].count, 0);
    }

    /// The original's F-80988 run offline on 4,000 records (tools/trail_ribbon_reference.py): every output
    /// float bit for bit, the projection callee a stub both sides compute alike.
    #[test]
    fn the_ribbon_matches_the_original() {
        let text = include_str!("testdata/trail_ribbon_ref.txt");
        let h = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let (mut n, mut fails) = (0, 0);
        for line in text.lines().filter(|l| !l.is_empty()) {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let i: Vec<&str> = inp.split_whitespace().collect();
            let o: Vec<&str> = out.split_whitespace().collect();
            // count frame width, m (16), 8 x (h xyz, stamp, val)
            let mut r = Record { count: i[0].parse().unwrap(), width: h(i[2]), ..Default::default() };
            let frame: u32 = i[1].parse().unwrap();
            let m: Vec<f32> = (0..16).map(|k| h(i[3 + k])).collect();
            for k in 0..POINTS {
                let b = 19 + k * 5;
                r.h[k] = Vec3::new(h(i[b]), h(i[b + 1]), h(i[b + 2]));
                r.stamps[k] = i[b + 3].parse().unwrap();
                r.vals[k] = h(i[b + 4]);
            }
            let screen = |p: Vec3| {
                // (the stub: x m0 + y m4 + z m8 + m12 a row, then / w)
                let row = |c: usize| ((p.x * m[c] + p.y * m[4 + c]) + p.z * m[8 + c]) + m[12 + c];
                let w = row(3);
                Vec2::new(row(0) / w, row(1) / w)
            };
            let rb = ribbon(&r, frame, 0.75, screen);
            // (k points out, of them `real` made by the loop - a doubled one has no alpha of its own)
            let (k, real): (usize, usize) = (o[0].parse().unwrap(), o[1].parse().unwrap());
            let mut bad: Vec<String> = Vec::new();
            if rb.side.len() != k {
                bad.push(format!("count {} vs {k}", rb.side.len()));
            } else {
                let mut at = 2;
                let mut v3 = |v: Vec3, what: &str, bad: &mut Vec<String>| {
                    for (c, x) in [v.x, v.y, v.z].into_iter().enumerate() {
                        if x.to_bits() != h(o[at + c]).to_bits() {
                            bad.push(format!("{what}[{c}] {x:e} vs {:e}", h(o[at + c])));
                        }
                    }
                    at += 3;
                };
                v3(rb.start, "start", &mut bad);
                v3(rb.end, "end", &mut bad);
                for j in 0..k {
                    v3(rb.centre[j], "centre", &mut bad);
                    v3(rb.side[j], "side", &mut bad);
                }
                for j in 0..real {
                    if rb.alpha[j].to_bits() != h(o[at + j]).to_bits() {
                        bad.push(format!("alpha{j} {:e} vs {:e}", rb.alpha[j], h(o[at + j])));
                    }
                }
            }
            if !bad.is_empty() {
                if fails < 6 {
                    eprintln!("case {n}: {}", bad.join("; "));
                }
                fails += 1;
            }
            n += 1;
        }
        assert!(n >= 4000 && fails == 0, "{fails} of {n} differ");
    }
}
