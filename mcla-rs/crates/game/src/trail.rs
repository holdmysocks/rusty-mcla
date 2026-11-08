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

/// F-40536's test of the camera (place, z axis) against the one saved last frame: a cut when the z axes'
/// dot (z z' fused into y y', then x x') is under 0.2 or the place moved more than 4 m (the distance's squares
/// x, then z, then y, fused); and the move's share sat(d x 1). Checked against the original's code run offline.
pub fn camera_cut(saved: (Vec3, Vec3), cam: (Vec3, Vec3)) -> (bool, f32) {
    use crate::physics::exact::fma;
    let (s, c) = (saved.1, cam.1);
    let dot = fma(c.x, s.x, fma(c.y, s.y, c.z * s.z));
    let d = saved.0 - cam.0;
    let dist = fma(d.y, d.y, fma(d.z, d.z, d.x * d.x)).sqrt();
    let cut = dot < 0.2 || dist > 4.0;
    let m = dist * 1.0;
    let m = if m >= 0.0 { m } else { 0.0 };
    (cut, if m - 1.0 >= 0.0 { 1.0 } else { m })
}

/// F-25303's place in view space: x r0 + (y r1 + (z r2 + r3)) by the vector unit (each product rounded),
/// the rows of the original's matrix being `view`'s columns.
pub fn view_point(view: &Mat4, p: Vec3) -> Vec3 {
    let (r0, r1, r2, r3) = (view.x_axis.truncate(), view.y_axis.truncate(), view.z_axis.truncate(), view.w_axis.truncate());
    r0 * p.x + (r1 * p.y + (r2 * p.z + r3))
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
        r.h[0] = view_point(view, place);
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
        let (cut, moved) = camera_cut(saved, cam);
        if cut {
            for r in self.recs.iter_mut() {
                r.count = 0;
            }
        }
        self.saved = Some(cam);
        self.moved = moved;
        self.cut = cut;
        if cut {
            return;
        }
        self.tick_records(paused);
    }

    /// F-89089 after the cut: paused, every record emptied; then each record not used this frame loses a
    /// point and is marked drawn (+316; the original sets it after the record's strip, or at once with two
    /// points or fewer - the ribbon of three or more always has two points or more, so every record).
    pub fn tick_records(&mut self, paused: bool) {
        if paused {
            for r in self.recs.iter_mut() {
                r.count = 0;
            }
        }
        let frame = self.frame;
        for r in self.recs.iter_mut() {
            if r.stamp != frame && r.count != 0 {
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
            let pair = pairs(&r, &rb);
            for j in 1..m {
                let (a0, b0) = pair[j - 1];
                let (a1, b1) = pair[j];
                v.extend_from_slice(&[a0, b0, a1, b0, b1, a1]);
            }
        }
        v
    }
}

/// F-89089's strip of one record: two vertices a ribbon point, A = centre - across and B = centre + across
/// (the vector unit), the start cap added to the first pair and the end cap to the last; u 0 (A) / 1 (B),
/// v 0 / 0.5 / 1 (first / middle / last); alpha = the low byte of trunc((alpha x factor) x 255) (factor 1 in the
/// middle, 0 at the ends; no clamp: 300 gives 44, a negative its two's complement byte, NaN and below -2^31 0,
/// 2^31 up 255); the colour's three bytes as they are (+48's low three, trunc(float(b) x (1/255) x 255) = b);
/// CentrePosition = the newest point. Checked against the original's code run offline.
pub fn pairs(r: &Record, rb: &Ribbon) -> Vec<(Vertex, Vertex)> {
    let m = rb.side.len();
    let c = r.h[0];
    let col = r.rgb.map(|b| b as f32 / 255.0);
    let mut pair = Vec::with_capacity(m);
    for j in 0..m {
        let (v, f) = if j == 0 { (0.0, 0.0) } else if j == m - 1 { (1.0, 0.0) } else { (0.5, 1.0) };
        let (mut a, mut b) = (vsub(rb.centre[j], rb.side[j]), vadd(rb.centre[j], rb.side[j]));
        if j == 0 {
            a = vadd(a, rb.start);
            b = vadd(b, rb.start);
        }
        if j == m - 1 {
            a = vadd(a, rb.end);
            b = vadd(b, rb.end);
        }
        let alpha = ((rb.alpha[j] * f) * 255.0) as i32 as u8;
        let vx = |p: Vec3, u: f32| Vertex { pos: [p.x, p.y, p.z, 1.0], uv: [u, v, 0.0, 0.0], col: [col[0], col[1], col[2], alpha as f32 / 255.0], centre: [c.x, c.y, c.z, 1.0] };
        pair.push((vx(a, 0.0), vx(b, 1.0)));
    }
    pair
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
    /// F-40536 and F-25303 against the original's code run offline (`tools/trail_small_reference.py`).
    #[test]
    fn camera_cut_and_push_as_the_original() {
        let f = |h: &str| f32::from_bits(u32::from_str_radix(h, 16).unwrap());
        let (mut cuts, mut pushes) = (0, 0);
        for l in include_str!("render/testdata/trail_small_reference.txt").lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            let v = |i: usize| glam::Vec3::new(f(w[i]), f(w[i + 1]), f(w[i + 2]));
            match w[0] {
                "CUT" => {
                    let (cut, moved) = super::camera_cut((v(4), v(1)), (v(10), v(7)));
                    assert_eq!((cut as u32, moved.to_bits()), (w[13].parse().unwrap(), f(w[14]).to_bits()), "{l}");
                    cuts += 1;
                }
                "PUSH" => {
                    let m: Vec<f32> = (1..17).map(|i| f(w[i])).collect();
                    let view = glam::Mat4::from_cols_slice(&m);
                    let h = super::view_point(&view, v(17));
                    assert_eq!(h.to_array().map(f32::to_bits), v(22).to_array().map(f32::to_bits), "{l}");
                    assert_eq!((f(w[20]) * f(w[21])).to_bits(), f(w[25]).to_bits(), "{l}");
                    pushes += 1;
                }
                _ => panic!("{l}"),
            }
        }
        assert_eq!((cuts, pushes), (4000, 4000));
    }

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
    fn the_free_record_pick_matches_the_original() {
        // tools/trail_free_reference.py: F-58134 run on 2,000 sets of the 96 records
        let text = include_str!("testdata/trail_free_reference.txt");
        let half = |l: &str| -> (Vec<u32>, Vec<u32>) {
            let (a, b) = l.split_once(" | ").unwrap();
            (a.split_whitespace().skip(2).map(|x| x.parse().unwrap()).collect(), b.split_whitespace().map(|x| x.parse().unwrap()).collect())
        };
        let lines: Vec<&str> = text.lines().collect();
        let (mut n, mut fails) = (0, 0);
        for pair in lines.chunks(2) {
            let (counts, drawn) = half(pair[0]);
            let ret: i32 = pair[1].split_whitespace().nth(1).unwrap().parse().unwrap();
            let (oc, od) = half(pair[1]);
            let mut t = Trails::default();
            for i in 0..RECORDS {
                t.recs[i].count = counts[i];
                t.recs[i].drawn = drawn[i] != 0;
            }
            let got = t.free();
            let ok = got == ret
                && (0..RECORDS).all(|i| t.recs[i].count == oc[i] && t.recs[i].drawn == (od[i] != 0));
            if !ok {
                fails += 1;
            }
            n += 1;
        }
        assert!(n == 2000 && fails == 0, "{fails} of {n} differ");
    }

    #[test]
    fn the_draw_matches_the_original() {
        // tools/trail_draw_reference.py: F-89089 run whole, the ribbon F-80988 a stub handing back chosen outputs
        let text = include_str!("testdata/trail_draw_reference.txt");
        let hx = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let mut cases: Vec<Vec<&str>> = Vec::new();
        for l in text.lines() {
            if l.starts_with("CASE") {
                cases.push(Vec::new());
            }
            cases.last_mut().unwrap().push(l);
        }
        let (mut frames, mut strips, mut verts, mut fails) = (0, 0, 0, 0);
        for c in &cases {
            let w: Vec<&str> = c[0].split_whitespace().collect();
            let (frame, cut, paused, passes): (u32, bool, bool, usize) = (w[3].parse().unwrap(), w[5] == "1", w[7] == "1", w[9].parse().unwrap());
            let mut t = Trails { frame, ..Default::default() };
            let mut out = Vec::new();
            for l in c {
                let w: Vec<&str> = l.split_whitespace().collect();
                if w[0] == "IN" {
                    let i: usize = w[1].parse().unwrap();
                    let col: u32 = w[5].parse().unwrap();
                    t.recs[i] = Record { count: w[2].parse().unwrap(), stamp: w[3].parse().unwrap(), drawn: w[4] == "1", rgb: [(col >> 16) as u8, (col >> 8) as u8, col as u8], ..Default::default() };
                } else if w[0] == "OUT" {
                    out.push((w[2].parse::<u32>().unwrap(), w[3] == "1"));
                }
            }
            let before: Vec<(u32, bool)> = t.recs.iter().map(|r| (r.count, r.drawn)).collect();
            let mut bad: Vec<String> = Vec::new();
            if cut {
                // (the cut empties the records itself, F-40536 - here a stub: nothing else is touched or drawn)
                if c.iter().any(|l| l.starts_with("RIB") || l.starts_with("V ")) || out != before {
                    bad.push("drew or changed a record on a cut".into());
                }
            } else {
                t.tick_records(paused);
                let after: Vec<(u32, bool)> = t.recs.iter().map(|r| (r.count, r.drawn)).collect();
                if after != out {
                    bad.push("counts / drawn".into());
                }
                let want: Vec<usize> = (0..RECORDS).filter(|&i| t.recs[i].count > 2).collect();
                let mut k = 0;
                let mut seen = Vec::new();
                while k < c.len() {
                    let w: Vec<&str> = c[k].split_whitespace().collect();
                    k += 1;
                    if w[0] != "RIB" {
                        continue;
                    }
                    let i: usize = w[1].parse().unwrap();
                    let m: usize = w[2].parse().unwrap();
                    seen.push(i);
                    let f = |n: usize| f32::from_bits(hx(w[3 + n]));
                    let v3 = |n: usize| Vec3::new(f(n), f(n + 1), f(n + 2));
                    let mut rb = Ribbon { start: v3(0), end: v3(3), ..Default::default() };
                    for j in 0..m {
                        rb.centre.push(v3(6 + 7 * j));
                        rb.side.push(v3(9 + 7 * j));
                        rb.alpha.push(f(12 + 7 * j));
                    }
                    let cp: Vec<u32> = c[k].split_whitespace().skip(1).map(hx).collect();
                    k += 1;
                    let mut r = t.recs[i];
                    r.h[0] = Vec3::new(f32::from_bits(cp[0]), f32::from_bits(cp[1]), f32::from_bits(cp[2]));
                    let pv = pairs(&r, &rb);
                    let words = |v: &Vertex| {
                        let b = |x: f32| (x * 255.0).round() as u32;
                        let col = b(v.col[3]) << 24 | b(v.col[0]) << 16 | b(v.col[1]) << 8 | b(v.col[2]);
                        [v.pos[0].to_bits(), v.pos[1].to_bits(), v.pos[2].to_bits(), 0, 0, 0, v.uv[0].to_bits(), v.uv[1].to_bits(), col]
                    };
                    let mine: Vec<[u32; 9]> = pv.iter().flat_map(|(a, b)| [words(a), words(b)]).collect();
                    for _ in 0..passes {
                        if c[k] != format!("B 4 {}", 2 * m) {
                            bad.push(format!("record {i}: {}", c[k]));
                        }
                        k += 1;
                        for (n, v) in mine.iter().enumerate() {
                            let o: Vec<u32> = c[k + n].split_whitespace().skip(1).enumerate().map(|(q, x)| if q == 8 { x.parse().unwrap() } else { hx(x) }).collect();
                            if o[..] != v[..] {
                                bad.push(format!("record {i} vertex {n}: {:08x?} vs {:08x?}", v, o));
                            }
                        }
                        k += mine.len();
                        if c[k] != "E" {
                            bad.push(format!("record {i}: no end"));
                        }
                        k += 1;
                    }
                    strips += 1;
                    verts += mine.len();
                }
                if seen != want {
                    bad.push(format!("ribbons for {seen:?} vs {want:?}"));
                }
            }
            if !bad.is_empty() {
                if fails < 4 {
                    eprintln!("{}: {}", c[0], bad[..bad.len().min(4)].join("; "));
                }
                fails += 1;
            }
            frames += 1;
        }
        assert!(frames == 30 && strips > 1000 && fails == 0, "{fails} of {frames} frames differ ({strips} strips, {verts} vertices)");
    }

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
