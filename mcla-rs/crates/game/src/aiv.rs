//! The rage `aiv` race-line follower of the computer racers, ported 1:1 from the executable
//! (decode: work/racerai/raceline.md; aiv code A-53277..A-78268, optimiser F-88594..F-78843).
//!
//! The object holds a corridor of nav-mesh portals (ring of 128), the brain's goals (ring of 8),
//! the race line's control points (ring of 64, Catmull-Rom knots), up to 32 obstacles and the
//! aivRaceLineTuning. Each frame [`Aiv::update`] takes the car state (F-12813, which also
//! builds one leg of the line, F-85983) and runs the optimiser (F-62607, the job
//! "UpdateRaceLine", here synchronously); [`Aiv::output`] (F-65301) gives the chassis pedals.
//!
//! Ring indices are absolute, as in the original: element `i` sits in slot
//! `(i - base + start + 1) mod N`. The line state that the optimiser saves and restores
//! (the 5040-byte block aiv+25440..+30480) is [`Line`].
//!
//! Runtime-initialised vector constants (zero in the image), decided from their initialisers
//! (static constructors F-89044..F-50173) and their use:
//! 0x82860930 = (0,0,0,1) (copy of 0x827D8440), 0x82860950 = xyz lane mask, 0x82860910 /
//! 0x82860A00 = x,z(,w) lane mask, 0x828609E0 = +inf splat (rsqrte guard). 0x828608E0 and
//! 0x828608C0 have no initialiser in the static-constructor block: read as the zero vector
//! (they are the fallback of the normalisations and the floor of the half-space clamps
//! vmaxfp(dot, c) in F-15125, where only 0 makes sense).
#![allow(dead_code)] // (wired into the racers by brain.rs)
use crate::physics::exact::{dot3, dot_xyz, fma, fms, fnms, guest_acos, guest_cos, guest_sin};
use glam::Vec3;
use mcla_formats::rage::aitune::AiTune;
use std::sync::Arc;

/// aivRaceLineTuning (registration F-88098, member table 0x82810A38); `Default` = the code
/// defaults of F-46101. No tune file sets them in the game.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AivTuning {
    pub max_steps_ahead: i32,
    /// (approx.: the default is `*(u32*)0x828D814C != 0`, a runtime word next to the "aivperf"
    /// registration, 0 in the image)
    pub restart_every_update: bool,
    pub max_steps_to_recurse: i32,
    pub recursive_falloff: f32,
    pub initial_segment_length: f32,
    pub samples_per_segment: i32,
    pub length_variance_weight: f32,
    pub off_road_weight: f32,
    pub max_curvature_penalty: f32,
    pub linear_adjustment: f32,
    /// degrees
    pub angular_adjustment: f32,
    pub angular_adjustment_threshold: f32,
    pub angular_adjustment_falloff: f32,
    pub adjustment_falloff: f32,
    pub adjustment_falloff_trigger: f32,
    pub adjustment_reset_threshold: f32,
    pub buffer_zone: f32,
    pub obstacle_lookahead_time: f32,
    pub max_speed_scalar: f32,
    pub max_total_accel_scalar: f32,
    pub max_total_decel_scalar: f32,
    pub max_brake_decel_scalar: f32,
    pub average_speed_weight: f32,
    pub max_obstacle_height_delta: f32,
    pub max_obstacle_flat_delta: f32,
    pub max_obstacle_prediction_dist: f32,
    pub collision_speed_penalty: f32,
}

impl Default for AivTuning {
    fn default() -> Self {
        AivTuning {
            max_steps_ahead: 20,
            restart_every_update: false,
            max_steps_to_recurse: 1,
            recursive_falloff: 0.5,
            initial_segment_length: 25.0,
            samples_per_segment: 3,
            length_variance_weight: 0.2,
            off_road_weight: 0.1,
            max_curvature_penalty: 5.0,
            linear_adjustment: 1.0,
            angular_adjustment: 10.0,
            angular_adjustment_threshold: 0.1,
            angular_adjustment_falloff: 0.75,
            adjustment_falloff: 0.1,
            adjustment_falloff_trigger: 1.0,
            adjustment_reset_threshold: 0.099,
            buffer_zone: 1.0,
            obstacle_lookahead_time: 6.0,
            max_speed_scalar: 1.0,
            max_total_accel_scalar: 1.0,
            max_total_decel_scalar: 0.75,
            max_brake_decel_scalar: 1.0,
            average_speed_weight: 0.5,
            max_obstacle_height_delta: 5.0,
            max_obstacle_flat_delta: 25.0,
            max_obstacle_prediction_dist: 100.0,
            collision_speed_penalty: 6.705,
        }
    }
}

/// One nav-mesh portal as the brain passes it to F-95594: the shared edge A-B of two
/// consecutive path polygons, the polygon normal, the polygon flag bit 0x2 ("off-road") and the
/// two extensions (0 for polygon edges; the path element's +28/+32 in F-78429).
#[derive(Debug, Clone, Copy)]
pub struct Portal {
    pub a: Vec3,
    pub b: Vec3,
    pub normal: Vec3,
    pub offroad: bool,
    pub ext: (f32, f32),
}

/// One obstacle as the brain passes it to F-70624 (the selection, F-53450, is the
/// brain's). `extent` is what the brain passes; F-70624 stores half of it (x along `dir`'s
/// side, z along `dir`). `flags`: [`OBST_FIRST_LEG`] (0x80: only tested against the car's leg),
/// [`OBST_TYPE0`] (0x40, not read by the follower).
#[derive(Debug, Clone, Copy)]
pub struct Obstacle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub dir: Vec3,
    pub extent: (f32, f32),
    pub flags: u8,
}
pub const OBST_FIRST_LEG: u8 = 0x80;
pub const OBST_TYPE0: u8 = 0x40;

/// The car state of F-12813 plus the values the brain sets once (A-59878).
#[derive(Debug, Clone, Copy)]
pub struct AivCar {
    pub pos: Vec3,
    /// matrix row 0 (stored at aiv+30816, read by the output mapping)
    pub right: Vec3,
    /// -row 2
    pub forward: Vec3,
    pub vel: Vec3,
    pub ang_vel: Vec3,
    /// nitro tune words +4, +8, +12 of car+664's tune ([1] = the accel term c1, [2] = duration)
    pub nitro_tune: [f32; 3],
    /// the brain's nitro value (brain+28), used as c1 max(t1, 0)^2 and counted down by the forward pass
    pub nitro_t: f32,
    /// slingshot tune words +0, +4, +8 of car+668's tune ([1] = c2, [2] = duration)
    pub sling_tune: [f32; 3],
    pub sling_t: f32,
    /// brain+292, -1 = none
    pub speed_cap: f32,
    /// 2 x the bound's half extents (F-98453(car+8)+16 x 2)
    pub size: Vec3,
    /// g = -9.8 x car vfunc 44 (negative)
    pub gravity: f32,
}

/// (the port's) a frame's aiv steps worked out ahead (`Aiv::ahead`): the state and inputs they were worked out from, and
/// the aiv they left.
pub struct AivAhead {
    epoch: u64,
    obstacles: Vec<Obstacle>,
    car: AivCar,
    activate: bool,
    dt: f32,
    out: Aiv,
}

fn obstacle_bits(o: &Obstacle) -> [u32; 12] {
    let b = |x: f32| x.to_bits();
    [b(o.pos.x), b(o.pos.y), b(o.pos.z), b(o.vel.x), b(o.vel.y), b(o.vel.z), b(o.dir.x), b(o.dir.y), b(o.dir.z), b(o.extent.0), b(o.extent.1), o.flags as u32]
}

fn aiv_car_bits(c: &AivCar) -> Vec<u32> {
    let mut w = Vec::with_capacity(32);
    for v in [c.pos, c.right, c.forward, c.vel, c.ang_vel, c.size] {
        w.extend([v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]);
    }
    w.extend(c.nitro_tune.iter().chain(&c.sling_tune).map(|x| x.to_bits()));
    w.extend([c.nitro_t, c.sling_t, c.speed_cap, c.gravity].map(f32::to_bits));
    w
}

/// The chassis pedals of F-28541 as written (before mcOpponentInput's smoothing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AivOut {
    pub throttle: f32,
    pub steer: f32,
    pub brake: f32,
    pub handbrake: f32,
    pub valid: bool,
}

// ---------------------------------------------------------------------------------------------
// rings and state

#[derive(Clone, Copy, Debug)]
struct Ring<T: Copy, const N: usize> {
    d: [T; N],
    start: i32,
    count: i32,
    base: i32,
}

impl<T: Copy, const N: usize> Ring<T, N> {
    fn new(z: T) -> Self {
        Ring { d: [z; N], start: 0, count: 0, base: 0 }
    }
    fn clear(&mut self) {
        self.start = 0;
        self.count = 0;
        self.base = 0;
    }
    fn end(&self) -> i32 {
        self.base + self.count
    }
    fn slot(&self, i: i32) -> usize {
        (i - self.base + self.start + 1).rem_euclid(N as i32) as usize
    }
    fn get(&self, i: i32) -> T {
        self.d[self.slot(i)]
    }
    fn set(&mut self, i: i32, v: T) {
        let s = self.slot(i);
        self.d[s] = v;
    }
    fn push(&mut self, v: T) {
        let s = (self.start + self.count + 1).rem_euclid(N as i32) as usize;
        self.d[s] = v;
        self.count += 1;
    }
    fn pop_front(&mut self) {
        self.start = (self.start + 1) % N as i32;
        self.base += 1;
        self.count -= 1;
    }
    fn pop_back(&mut self) {
        self.count -= 1;
    }
    fn last(&self) -> T {
        self.get(self.end() - 1)
    }
}

/// A portal as stored (176 bytes in the original): raw ends, shrunk ends, normal and the planes
/// of the cell to the next portal (F-68053), plus the flag byte (0x80 = off-road).
#[derive(Clone, Copy, Default, Debug)]
struct Cell {
    a: Vec3,
    b: Vec3,
    sa: Vec3,
    sb: Vec3,
    n: Vec3,
    fwd: Vec3,
    back: Vec3,
    side_a: Vec3,
    side_b: Vec3,
    sside_a: Vec3,
    sside_b: Vec3,
    offroad: bool,
}

#[derive(Clone, Copy, Default, Debug)]
struct Goal {
    p0: Vec3,
    /// p0.w: radius, or the lerp fraction of a gate
    w: f32,
    p1: Vec3,
    /// p1.w (+28)
    speed: f32,
    /// (portal end - 2) << 13 | brain flags & 0x1FFF
    word: i32,
}

impl Goal {
    fn gate(&self) -> bool {
        self.word & 1 != 0
    }
    fn portal(&self) -> i32 {
        self.word >> 13
    }
    fn target(&self) -> Vec3 {
        if self.gate() { (self.p1 - self.p0) * self.w + self.p0 } else { self.p0 }
    }
}

/// One sample of the sampled line (56 bytes).
#[derive(Clone, Copy, Default, Debug)]
struct Sample {
    /// +0 forward-pass speed
    v: f32,
    /// +4 goal speed in, allowed speed after the backward pass
    allowed: f32,
    /// +8
    v_curve: f32,
    /// +12 table speed x MaxSpeedScalar
    v_table: f32,
    /// +16 vertical curvature
    kv: f32,
    /// +20 signed plan curvature
    kappa: f32,
    /// +24 g T.y
    g_ty: f32,
    /// +28 g L.y
    g_ly: f32,
    /// +32 grip limit
    a_max: f32,
    /// +36
    ds: f32,
    /// +40
    ty: f32,
    /// +44
    xy: f32,
    /// +48 time weight
    w: f32,
    /// +52 +1 at the last sample of a collision-flagged point, else -1
    coll: f32,
}

const NS: usize = 66;

/// The block the optimiser saves and restores (aiv+25440..+30480).
#[derive(Clone, Copy, Debug)]
struct Line {
    pts: Ring<Vec3, 64>,
    /// point -> portal << 1 | collision bit
    pp: Ring<i32, 64>,
    s: [Sample; NS],
    n: i32,
    f: i32,
    j: f32,
    t: f32,
}

/// The job's scratch (F-61160): backup states, dirty bits, spline coefficients.
#[derive(Clone, Debug)]
struct Scratch {
    backup: [Line; 4],
    dirty: [u32; 2],
    coef: [[Vec3; 4]; 64],
    legs: i32,
}

/// The vehicle tables (aivVehicleTuning) with the derived scales of F-49493.
#[derive(Clone, Debug)]
struct Veh {
    t: Arc<AiTune>,
    spd_k: f32,
    knee: f32,
    low_k: f32,
    high_k: f32,
}

impl Veh {
    fn new(t: Arc<AiTune>) -> Veh {
        let knee = t.max_curvature * 0.1;
        Veh { spd_k: 127.0 / t.max_speed, knee, low_k: 64.0 / knee, high_k: 63.0 / (t.max_curvature - knee), t }
    }
    /// The class's read of a 128-entry speed table at x = speed index (unclamped fraction): the lower entry plus the step
    /// times the fraction as one fused multiply-add (F-67089, F-45930, F-71244 do it so).
    fn read(row: &[f32], x: f32) -> f32 {
        if row.len() < 2 {
            return 0.0;
        }
        let x = if x > 0.0 { x } else { 0.0 };
        let i = x.min(126.1).floor();
        let k = (i as usize).min(row.len() - 2);
        let (a, b) = (row[k], row[k + 1]);
        fma(b - a, x - i, a)
    }
    /// F-45930: max(0.1, MaxEngineAccelByGear [147 + 128 g + i]) = accel row g + 1.
    fn accel(&self, g: i32, v: f32) -> f32 {
        let r = (g.max(0) as usize + 1).min(self.t.accel_rows.len().saturating_sub(1));
        Self::read(self.t.accel_rows.get(r).map_or(&[][..], |r| &r[..]), self.spd_k * v).max(0.1)
    }
    /// F-71244: max(0.1, MaxEngineDecelByGear [1043 + 128 g + i]) = decel row g + 1.
    fn decel(&self, g: i32, v: f32) -> f32 {
        let d = &self.t.decel;
        if d.is_empty() {
            return 0.1;
        }
        // (approx.: AiTune reads 7 of the 8 rows; gear index 6 (row 7) falls back to row 6)
        let r = (g.max(0) as usize + 1).min(d.len() - 1);
        Self::read(&d[r], self.spd_k * v).max(0.1)
    }
    /// The benefits' read of the acceleration row 0 (F-82077 / F-96973 inline it): the index (word 11 x v, below or at 0
    /// as 0, NaN as 0) floored by the 2^23 rounding and capped at 126.1, the fraction from the index unclamped above, the lerp
    /// fused, then at least 0.1.
    fn accel0_inline(&self, v: f32) -> f32 {
        let x = self.spd_k * v;
        let x = fsel(-x, 0.0, x);
        let xc = fsel(x, x, 0.0);
        let xc = fsel(xc - 126.1, 126.1, xc);
        let h = fsel(xc, -8388608.0, 8388608.0);
        let r = (xc - h) + h;
        let fl = fsel(xc - r, r, r - 1.0);
        let row = &self.t.accel;
        let j = fl as usize;
        if row.len() < j + 2 {
            return 0.1;
        }
        let a = fma(row[j + 1] - row[j], x - fl, row[j]);
        fsel(0.1 - a, 0.1, a)
    }
    /// F-82077 (nitrous, `cubic` false) / F-96973 (slingshot, true): the fraction of a boost's time gain the grip-limited
    /// line keeps. Two forward runs over the samples f+1 .. n-1 from the forward speed (at least 0.1): the boosted run under
    /// the grip limit and the sample's allowed speed, the free boosted run; the boost c x b^2 (b^3) with b = 1 - t / dur
    /// (at least 0), the drag term ((d3 |k| + d2) v + d1, at least 0)^4; (T - ta) / (T - tb) clamped 0 .. 1, 0 unless
    /// T - tb > 1e-6. The arithmetic as the original's: fused multiply-adds, its selects and term order.
    #[allow(clippy::too_many_arguments)]
    fn boost_benefit(&self, s: &[Sample], f: i32, n: i32, v_fwd: f32, c: f32, dur: f32, inv_g: f32, total: f32, cubic: bool) -> f32 {
        let (d1, d2, d3) = (self.t.drag[0], self.t.drag[1], self.t.drag[2]);
        let mut va = fsel(0.1 - v_fwd, 0.1, v_fwd);
        let mut vb = va;
        let (mut ta, mut tb) = (0.0f32, 0.0f32);
        let inv_t = 1.0 / dur;
        let boost = |b: f32, e: f32| if cubic { fma(c * b * b, b, e) } else { fma(c * b, b, e) };
        let mut i = f + 1;
        while i < n && i >= 0 && (i as usize) < s.len() {
            let q = &s[i as usize];
            let ba = fnms(inv_t, ta, 1.0);
            let ba = fsel(-ba, 0.0, ba);
            let bb = fnms(inv_t, tb, 1.0);
            let bb = fsel(-bb, 0.0, bb);
            let k = q.kappa.abs();
            let ds2 = q.ds * 2.0;
            // the grip-limited run
            let alat = fma(q.kappa * va, va, q.g_ly);
            let nn = fnms(q.kv * inv_g * va, va, 1.0);
            let nn = fsel(nn, nn, 0.0);
            let nn = fsel(nn - 1.0, 1.0, nn);
            let dra = fma(fma(d3, k, d2), va, d1);
            let dra = fsel(-dra, 0.0, dra);
            let ga = nn * q.a_max;
            let l2 = fms(ga, ga, alat * alat);
            let lim = fsel(-l2, 0.0, l2).sqrt();
            let e = fnms(dra * dra * dra, dra, self.accel0_inline(va));
            let a = boost(ba, e) + q.g_ty;
            let a = fsel(a - lim, lim, a);
            let v2 = fma(a * q.ds, 2.0, va * va);
            let na = fsel(0.1 - v2, 0.1, v2).sqrt();
            let na = fsel(na - q.allowed, q.allowed, na);
            // the free run
            let drb = fma(fma(d3, k, d2), vb, d1);
            let drb = fsel(-drb, 0.0, drb);
            let ab = fnms(drb * drb * drb, drb, boost(bb, self.accel0_inline(vb))) + q.g_ty;
            let v2 = fma(ab * q.ds, 2.0, vb * vb);
            let nb = fsel(0.1 - v2, 0.1, v2).sqrt();
            ta += ds2 / (na + va);
            tb += ds2 / (nb + vb);
            va = na;
            vb = nb;
            i += 1;
        }
        let r = if total - tb > 1e-6 { (total - ta) / (total - tb) } else { 0.0 };
        let r = fsel(r, r, 0.0);
        fsel(r - 1.0, 1.0, r)
    }
    /// F-67089
    fn steer_scalar(&self, v: f32) -> f32 {
        Self::read(&self.t.steer_scalar, v.abs() * self.spd_k)
    }
    /// the curvature speed tables as F-27864 reads them (max 0)
    fn curve_speed(&self, k: f32) -> f32 {
        let (row, x) = if k < self.knee { (&self.t.speed_low_curv, self.low_k * k) } else { (&self.t.speed_high_curv, (k - self.knee) * self.high_k) };
        if row.len() < 2 {
            return 0.0;
        }
        let i = x.min(62.1).floor();
        let i = if i > 0.0 { i } else { 0.0 };
        let kk = (i as usize).min(row.len() - 2);
        let v = (row[kk + 1] - row[kk]) * (x - i) + row[kk];
        if v > 0.0 { v } else { 0.0 }
    }
}

// ---------------------------------------------------------------------------------------------
// geometry helpers

/// F-41334: normalise if |v|^2 > eps, else the fallback.
pub(crate) fn norm_or(v: Vec3, eps: f32, fb: Vec3) -> Vec3 {
    let l2 = fma(v.z, v.z, fma(v.x, v.x, v.y * v.y));
    if !(l2 > eps) {
        return fb;
    }
    // frsqrte as 1/sqrt rounded to single, two Newton steps in double, the scale rounded to single; the vector
    // multiply flushes denormals
    let x = l2 as f64;
    let y0 = (1.0 / x.sqrt()) as f32 as f64;
    let e0 = -((y0 * y0).mul_add(x, -1.0));
    let y1 = e0.mul_add(y0 * 0.5, y0);
    let e1 = -((y1 * y1).mul_add(x, -1.0));
    let k = e1.mul_add(y1 * 0.5, y1) as f32;
    let ftz = |a: f32| if a.abs() < f32::MIN_POSITIVE { 0.0 * a } else { a };
    Vec3::new(ftz(ftz(v.x) * k), ftz(ftz(v.y) * k), ftz(ftz(v.z) * k))
}

/// the scalar acos of the original's helpers: the guest runtime's acos of a value clamped by `fsel`, rounded to single
pub(crate) fn acos32(c: f32) -> f32 {
    let lo = if c - -1.0 >= 0.0 { c } else { -1.0 };
    let hi = if lo - 1.0 >= 0.0 { 1.0 } else { lo };
    guest_acos(hi as f64) as f32
}

/// rsqrte-guarded normalisation (vrsqrtefp + one Newton step; zero stays zero)
fn norm0(v: Vec3) -> Vec3 {
    let l2 = v.length_squared();
    if l2 > 0.0 { v * (1.0 / l2.sqrt()) } else { Vec3::ZERO }
}

/// F-52074: signed XZ angle between two unit vectors.
pub(crate) fn ang_xz_unit(a: Vec3, b: Vec3) -> f32 {
    let dot = fma(a.x, b.x, b.z * a.z);
    let s = fms(b.x, a.z, a.x * b.z);
    acos32(dot) * if s >= 0.0 { 1.0 } else { -1.0 }
}

/// F-32060: signed XZ angle from a to b (0 when either is degenerate).
pub fn ang_xz(a: Vec3, b: Vec3) -> f32 {
    let aa = fma(a.x, a.x, a.z * a.z);
    let bb = fma(b.x, b.x, b.z * b.z);
    let norm = (bb * aa).sqrt();
    if norm - 1e-12 >= 0.0 {
        let dot = fma(b.z, a.z, b.x * a.x);
        let s = fms(a.z, b.x, b.z * a.x);
        acos32(dot / norm) * if s >= 0.0 { 1.0 } else { -1.0 }
    } else {
        0.0
    }
}

/// F-88594: unsigned 3D angle.
pub fn ang3(a: Vec3, b: Vec3) -> f32 {
    let aa = fma(a.z, a.z, fma(a.x, a.x, a.y * a.y));
    let bb = fma(b.z, b.z, fma(b.x, b.x, b.y * b.y));
    let norm = (bb * aa).sqrt();
    if norm - 1e-12 >= 0.0 {
        let dot = fma(b.y, a.y, fma(b.x, a.x, b.z * a.z));
        acos32(dot / norm)
    } else {
        0.0
    }
}

/// F-57591: rotation about y.
pub(crate) fn rot_y(v: Vec3, a: f32) -> Vec3 {
    let s = guest_sin(a as f64) as f32;
    let c = guest_cos(a as f64) as f32;
    Vec3::new(fma(v.x, c, v.z * s), v.y, fms(v.z, c, v.x * s))
}

/// F-91226: segment P0-P1 against portal Q0-Q1 in XZ -> (s along the segment, t along the portal).
fn seg_portal(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3, one_sided: bool) -> Option<(f32, f32)> {
    let dqx = q1.x - q0.x;
    let dpz = p1.z - p0.z;
    let dpx = p1.x - p0.x;
    let dqz = q1.z - q0.z;
    let den = fms(dqz, dpx, dqx * dpz);
    if one_sided && den < 1e-5 {
        return None;
    }
    if den.abs() < 1e-5 {
        return None;
    }
    if dqx.abs() < 1e-5 {
        if dpx.abs() < 1e-5 {
            return None;
        }
        let s = (q0.x - p0.x) / dpx;
        let t = (fma(s, dpz, p0.z) - q0.z) / dqz;
        return Some((s, t));
    }
    if dqz.abs() < 1e-5 {
        if dpz.abs() < 1e-5 {
            return None;
        }
        let s = (q0.z - p0.z) / dpz;
        let t = (fma(s, dpx, p0.x) - q0.x) / dqx;
        return Some((s, t));
    }
    let s = fma(p0.z - q0.z, dqx, (q0.x - p0.x) * dqz) / den;
    let t = (fma(s, dpx, p0.x) - q0.x) / dqx;
    Some((s, t))
}

/// the plane-normal construction of F-68053: v made perpendicular to e (when |e|^2 > 1e-4), normalised (fallback 0).
/// As its vector code forms it: the dots by `vmsum3fp` (dot3), 1 / |e|^2 as the reciprocal estimate with one Newton step,
/// v - e s as `vnmsubfp` (-(e s - v)).
fn plane_n(v: Vec3, e: Vec3) -> Vec3 {
    let l2 = dot3(e, e);
    let v = if l2 > 1e-4 {
        let r0 = 1.0 / l2;
        let r = r0 * (-(r0 * l2 - 1.0)) + r0;
        let s = dot3(v, e) * r;
        Vec3::new(-(e.x * s - v.x), -(e.y * s - v.y), -(e.z * s - v.z))
    } else {
        v
    };
    norm_or(v, 1e-12, Vec3::ZERO)
}

/// F-17888
fn sin_ramp(x: f32) -> f32 {
    if x <= 0.0 {
        0.0
    } else if x >= 1.0 {
        1.0
    } else {
        guest_sin((x * 1.570_796_4) as f64) as f32
    }
}

/// `fsel`: a if c >= 0 (NaN counts as false), else b.
#[inline]
fn fsel(c: f32, a: f32, b: f32) -> f32 {
    if c >= 0.0 { a } else { b }
}

/// The original's square root of its race-line passes: x times the refined reciprocal root (the reciprocal root rounded to
/// single, then one Newton step in double), rounded to single - not a correctly rounded square root.
#[inline]
fn sqrt_nr(x: f32) -> f32 {
    let xd = x as f64;
    let y = (1.0 / xd.sqrt()) as f32 as f64;
    let e = -((y * y).mul_add(xd, -1.0));
    (e.mul_add(y * 0.5, y) * xd) as f32
}

/// The vector unit's refined reciprocal root of a squared length: 1/sqrt, zero where that is infinite, then one Newton step
/// (all single-precision, products rounded before the sums).
#[inline]
pub(crate) fn vrsqrt1(l2: f32) -> f32 {
    let y0 = 1.0 / l2.sqrt();
    let y = if y0 == f32::INFINITY { 0.0 } else { y0 };
    -(l2 * (y * y) - 1.0) * (0.5 * y) + y
}

/// The vector unit's length: the squared length (z + (y + x)) times its refined reciprocal root.
#[inline]
fn vlen(v: Vec3) -> f32 {
    let l2 = dot3(v, v);
    l2 * vrsqrt1(l2)
}

const UP: Vec3 = Vec3::Y;

// ---------------------------------------------------------------------------------------------

#[cfg(test)]
thread_local! {
    /// (tests) every push_goal's arguments, on this thread
    pub static GOAL_LOG: std::cell::RefCell<Vec<(Vec3, Vec3, f32, Vec3, f32, u32)>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[derive(Clone, Debug)]
pub struct Aiv {
    pub tun: AivTuning,
    veh: Veh,
    portals: Ring<Cell, 128>,
    planes_upto: i32,
    goals: Ring<Goal, 8>,
    obst: [Obstacle; 32],
    n_obst: usize,
    size: Vec3,
    pos: Vec3,
    h: Vec3,
    f: Vec3,
    vel: Vec3,
    ang_vel: Vec3,
    goal_dir: Vec3,
    v_fwd: f32,
    cap: f32,
    dt: f32,
    b1: [f32; 3],
    t1: f32,
    b2: [f32; 3],
    t2: f32,
    g: f32,
    inv_g: f32,
    right: Vec3,
    ln: Line,
    /// point -> goal tag (aiv+30496)
    pg: Ring<i32, 64>,
    last_reached: Vec3,
    j_prev: f32,
    cursor: i32,
    step: f32,
    mode: i32,
    progress: i32,
    /// obstacle tests, recomputed legs, forward-pass samples (aiv+30788..+30796)
    pub counters: [i32; 3],
    cached: i32,
    next_goal: i32,
    last_push_ok: bool,
    sc: Box<Scratch>,
    /// (the port's) a number no other state of any aiv has had: a new one at every change through a `&mut self` method
    /// (`touch`), kept by a copy - so an aiv whose number is the one a copy was taken at holds that copy's state
    /// (`ahead` / `take_ahead`). Every field change outside `impl Aiv` must go through a method that touches.
    epoch: u64,
}

impl Aiv {
    /// The vehicle tables' MaxSpeed (aivVehicleTuning [0]).
    pub fn max_speed(&self) -> f32 {
        self.veh.t.max_speed
    }

    /// F-36694 (with the vehicle tables of the car, aiv+0).
    pub fn new(tune: Arc<AiTune>) -> Aiv {
        let empty_line = Line { pts: Ring::new(Vec3::ZERO), pp: Ring::new(0), s: [Sample::default(); NS], n: 0, f: 0, j: 0.0, t: 0.0 };
        let mut a = Aiv {
            tun: AivTuning::default(),
            veh: Veh::new(tune),
            portals: Ring::new(Cell::default()),
            planes_upto: 0,
            goals: Ring::new(Goal::default()),
            obst: [Obstacle { pos: Vec3::ZERO, vel: Vec3::ZERO, dir: Vec3::X, extent: (0.0, 0.0), flags: 0 }; 32],
            n_obst: 0,
            size: Vec3::new(2.0, 1.5, 5.0),
            pos: Vec3::ZERO,
            h: Vec3::ZERO,
            f: Vec3::ZERO,
            vel: Vec3::ZERO,
            ang_vel: Vec3::ZERO,
            goal_dir: Vec3::ZERO,
            v_fwd: 0.0,
            cap: -1.0,
            dt: 1.0 / 60.0,
            b1: [2.0, 12.0, 2.5],
            t1: 0.0,
            b2: [2.0, 25.0, 2.5],
            t2: 0.0,
            g: -9.8,
            inv_g: -0.102_040_81,
            right: Vec3::ZERO,
            ln: empty_line,
            pg: Ring::new(-1),
            last_reached: Vec3::ZERO,
            j_prev: 1e8,
            cursor: 1,
            step: 1.0,
            mode: 0,
            progress: 1,
            counters: [0; 3],
            cached: 0,
            next_goal: 0,
            last_push_ok: false,
            sc: Box::new(Scratch { backup: [empty_line; 4], dirty: [0; 2], coef: [[Vec3::ZERO; 4]; 64], legs: 0 }),
            epoch: 0,
        };
        a.touch();
        a.reset();
        a
    }

    /// (the port's) a new state number (`epoch`).
    fn touch(&mut self) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        self.epoch = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// (the port's, GitHub #48) the racing brain's aiv steps of a frame - `set_obstacles`, `activate` when due, `update` -
    /// worked out on a copy, ahead of the frame and on another thread (vehicles.rs, `brain::ahead`): the optimiser is
    /// most of a racer's tick and each racer's is its own.
    pub fn ahead(&self, obstacles: &[Obstacle], car: &AivCar, activate: bool, dt: f32) -> AivAhead {
        let mut out = self.clone();
        out.set_obstacles(obstacles);
        if activate {
            out.activate(car);
        }
        out.update(car, dt);
        AivAhead { epoch: self.epoch, obstacles: obstacles.to_vec(), car: *car, activate, dt, out }
    }

    /// The frame's steps as `ahead` did them, when it did them from this state with these inputs, bit for bit (then the
    /// result is the steps' own: they read nothing else); false: not taken, the caller runs them.
    /// (MCLA_AIV_AHEAD_CHECK=1: a taken result is checked against the steps run here, a difference logged)
    pub fn take_ahead(&mut self, ahead: AivAhead, obstacles: &[Obstacle], car: &AivCar, activate: bool, dt: f32) -> bool {
        let same = ahead.epoch == self.epoch
            && ahead.activate == activate
            && ahead.dt.to_bits() == dt.to_bits()
            && aiv_car_bits(&ahead.car) == aiv_car_bits(car)
            && ahead.obstacles.len() == obstacles.len()
            && ahead.obstacles.iter().zip(obstacles).all(|(a, b)| obstacle_bits(a) == obstacle_bits(b));
        if !same {
            return false;
        }
        static CHECK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *CHECK.get_or_init(|| std::env::var("MCLA_AIV_AHEAD_CHECK").is_ok_and(|v| v == "1")) {
            let mut here = self.clone();
            here.set_obstacles(obstacles);
            if activate {
                here.activate(car);
            }
            here.update(car, dt);
            here.epoch = ahead.out.epoch;
            let (a, b) = (format!("{:?}", here), format!("{:?}", ahead.out));
            static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            static BAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let bad = BAD.load(std::sync::atomic::Ordering::Relaxed) + (a != b) as u64;
            BAD.store(bad, std::sync::atomic::Ordering::Relaxed);
            if a != b || n % 500 == 0 {
                log::info!("aiv ahead check: {n} taken, {bad} different from the steps run in place");
            }
        }
        *self = ahead.out;
        true
    }

    pub fn set_vehicle(&mut self, tune: Arc<AiTune>) {
        self.touch();
        self.veh = Veh::new(tune);
    }

    /// F-71106: drop corridor, goals and line.
    /// The racing brain's enable F-92836 (Activate): +25168 the car's size (2 x the bound's half extents), +25292 /
    /// +25308 the nitrous / slingshot tunes, +25324 g = -9.8 x the car's gravity factor, +25328 = 1.0 / g.
    pub fn activate(&mut self, car: &AivCar) {
        self.touch();
        self.size = car.size;
        self.b1 = car.nitro_tune;
        self.b2 = car.sling_tune;
        self.g = car.gravity;
        self.inv_g = 1.0 / car.gravity;
    }

    /// The aiv init's car values (written by its init, which BrainRacing_Reset F-69438 runs again): size (2, 1.5, 5),
    /// the tunes (2, 12, 2.5) / (2, 25, 2.5), g -9.8 and 1 / g.
    pub fn car_defaults(&mut self) {
        self.touch();
        self.size = Vec3::new(2.0, 1.5, 5.0);
        self.b1 = [2.0, 12.0, 2.5];
        self.b2 = [2.0, 25.0, 2.5];
        self.g = -9.8;
        self.inv_g = -0.102_040_81;
    }

    /// +25168, the car's size as the aiv holds it.
    pub fn size(&self) -> Vec3 {
        self.size
    }

    /// +25324 / +25328 and the two tunes (for tests).
    pub fn car_values(&self) -> (Vec3, [f32; 3], [f32; 3], f32, f32) {
        (self.size, self.b1, self.b2, self.g, self.inv_g)
    }

    pub fn reset(&mut self) {
        self.touch();
        self.next_goal = 0;
        self.cached = 0;
        self.planes_upto = 0;
        self.portals.clear();
        self.goals.clear();
        self.pg.clear();
        self.step = 1.0;
        self.ln.j = 1e8;
        self.progress = 1;
        self.ln.t = 1e8;
        self.cursor = 1;
        self.j_prev = 1e8;
        self.mode = 0;
        self.ln.f = 0;
        self.ln.pts.clear();
        self.ln.pp.clear();
        self.ln.n = 0;
    }

    // ---- sizes / queries --------------------------------------------------------------------

    pub const PORTAL_CAP: i32 = 128;
    pub const GOAL_CAP: i32 = 8;
    pub const POINT_CAP: i32 = 64;
    pub const OBSTACLE_CAP: usize = 32;
    pub fn portal_count(&self) -> i32 {
        self.portals.count
    }
    pub fn goal_count(&self) -> i32 {
        self.goals.count
    }
    pub fn point_count(&self) -> i32 {
        self.ln.pts.count
    }
    /// aiv+22992: goals popped so far (the goal ring's base)
    pub fn goals_done(&self) -> i32 {
        self.goals.base
    }
    /// aiv+22560 + aiv+22552: the portal ring's base + count
    pub fn portal_end(&self) -> i32 {
        self.portals.end()
    }
    /// The ends +0 / +16 of the ring slots (start + count - 1) and (start + count) mod 128 - the last two portals stored,
    /// read by slot as F-47058 reads them (whatever the slots hold when fewer are stored)
    pub fn last_two_ends(&self) -> ((Vec3, Vec3), (Vec3, Vec3)) {
        let e = self.portals.start + self.portals.count;
        let c = |k: i32| {
            let c = &self.portals.d[k.rem_euclid(128) as usize];
            (c.a, c.b)
        };
        (c(e - 1), c(e))
    }
    /// (diagnostics) the portal ring's start, count, base and planes-built index, and each portal's a, b, sa, sb, n, offroad
    pub fn portal_ring_dbg(&self) -> ((i32, i32, i32, i32), Vec<([Vec3; 5], bool)>) {
        let cells = (self.portals.base..self.portals.end()).map(|i| {
            let c = self.portals.get(i);
            ([c.a, c.b, c.sa, c.sb, c.n], c.offroad)
        });
        ((self.portals.start, self.portals.count, self.portals.base, self.planes_upto), cells.collect())
    }
    /// aiv+22560 + aiv+22552 != 0
    pub fn has_path(&self) -> bool {
        self.portals.base + self.portals.count != 0
    }
    /// F-65527
    pub fn line_used_up(&self) -> bool {
        self.progress >= self.ln.pts.end()
    }
    /// the line's progress (+30784), the point ring's base (+26480) and count (+26472)
    pub fn line_ring(&self) -> (i32, i32, i32) {
        (self.progress, self.ln.pts.base, self.ln.pts.count)
    }
    /// F-28749: the first control point ahead of the car
    pub fn next_point(&self) -> Vec3 {
        self.ln.pts.get(self.progress)
    }
    /// aivVehicleTuning MaxCurvature (the brain's turnaround circle, aiv tuning +8)
    pub fn max_curvature(&self) -> f32 {
        self.veh.t.max_curvature
    }
    /// the line heading H (aiv+25200)
    pub fn heading(&self) -> Vec3 {
        self.h
    }
    /// control points from the one behind the car on
    /// (diagnostics) the speed profile's samples from the car on: (kappa, table speed, grip, curve speed, allowed, v)
    pub fn opt_dbg(&self) -> String {
        let o: Vec<String> = self.obst[..self.n_obst].iter().map(|o| format!("{:.0},{:.0} v{:.0} e{:.1}x{:.1}", o.pos.x, o.pos.z, o.vel.length(), o.extent.0, o.extent.1)).collect();
        format!("J{:.3} pJ{:.3} c{} st{:.3} m{} p{} n{:?} obst {} [{}]", self.ln.j, self.j_prev, self.cursor, self.step, self.mode, self.progress, self.counters, self.n_obst, o.join("; "))
    }

    pub fn samples_dbg(&self) -> Vec<(f32, f32, f32, f32, f32, f32)> {
        let (f, n) = (self.ln.f.max(0) as usize, (self.ln.n.max(0) as usize).min(NS));
        (f..n).map(|i| { let s = &self.ln.s[i]; (s.kappa, s.v_table, s.a_max, s.v_curve, s.allowed, s.v) }).collect()
    }

    /// (diagnostics, work/ai-record: the replay of recorded states) Load the original's aiv object bytes
    /// (+0 .. +31232, read from the running recomp, big endian) into this one: the portal / goal / obstacle /
    /// point / sample rings, the car state, and the optimiser's cursor. Planes are recomputed.
    pub fn import_original(&mut self, b: &[u8]) {
        self.touch();
        let f = |o: usize| f32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let i = |o: usize| i32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let v3 = |o: usize| Vec3::new(f(o), f(o + 4), f(o + 8));
        let ring_hdr = |o: usize| (i(o + 4), i(o + 8), i(o + 16)); // start, count, base (tail +0, ? +12)
        // portals: 128 x 176 at +16, header +22544, flags +22576
        let (st, ct, bs) = ring_hdr(22544);
        self.portals.start = st;
        self.portals.count = ct;
        self.portals.base = bs;
        for slot in 0..128usize {
            let o = 16 + 176 * slot;
            // (the six cell planes at +80 .. +176 as held: F-68053 builds them once per pair, so they can be those of a portal
            // since popped and pushed again - F-82126 leaves the planes-built index past it)
            self.portals.d[slot] = Cell {
                a: v3(o), b: v3(o + 16), sa: v3(o + 32), sb: v3(o + 48), n: v3(o + 64),
                fwd: v3(o + 80), back: v3(o + 96), side_a: v3(o + 112), side_b: v3(o + 128), sside_a: v3(o + 144), sside_b: v3(o + 160),
                offroad: b[22576 + slot] & 0x80 != 0,
            };
        }
        self.planes_upto = i(30832);
        // goals: 8 x 32 at +22720, header +22976, words +23008
        let (st, ct, bs) = ring_hdr(22976);
        self.goals.start = st;
        self.goals.count = ct;
        self.goals.base = bs;
        for slot in 0..8usize {
            let o = 22720 + 32 * slot;
            self.goals.d[slot] = Goal { p0: v3(o), w: f(o + 12), p1: v3(o + 16), speed: f(o + 28), word: i(23008 + 4 * slot) };
        }
        // obstacles: 32 x 64 at +23056, count +25104, flags +25120
        self.n_obst = (i(25104).max(0) as usize).min(32);
        for k in 0..32usize {
            let o = 23056 + 64 * k;
            self.obst[k] = Obstacle { pos: v3(o), vel: v3(o + 16), dir: v3(o + 32), extent: (f(o + 48), f(o + 56)), flags: b[25120 + k] };
        }
        // car state
        self.size = v3(25168);
        self.pos = v3(25184);
        self.h = v3(25200);
        self.f = v3(25216);
        self.vel = v3(25232);
        self.ang_vel = v3(25248);
        self.goal_dir = v3(25264);
        self.v_fwd = f(25280);
        self.cap = f(25284);
        self.dt = f(25288);
        self.b1 = [f(25292), f(25296), f(25300)];
        self.t1 = f(25304);
        self.b2 = [f(25308), f(25312), f(25316)];
        self.t2 = f(25320);
        self.g = f(25324);
        self.inv_g = f(25328);
        self.right = v3(30816);
        // points / point->portal / samples
        let (st, ct, bs) = ring_hdr(26464);
        self.ln.pts.start = st;
        self.ln.pts.count = ct;
        self.ln.pts.base = bs;
        for slot in 0..64usize {
            self.ln.pts.d[slot] = v3(25440 + 16 * slot);
        }
        let (st, ct, bs) = (i(26756), i(26760), i(26764)); // (the 4-word header: tail, start, count, base)
        self.ln.pp.start = st;
        self.ln.pp.count = ct;
        self.ln.pp.base = bs;
        for slot in 0..64usize {
            self.ln.pp.d[slot] = i(26496 + 4 * slot);
        }
        for k in 0..NS {
            let o = 26768 + 56 * k;
            let w: Vec<f32> = (0..14).map(|q| f(o + 4 * q)).collect();
            self.ln.s[k] = Sample { v: w[0], allowed: w[1], v_curve: w[2], v_table: w[3], kv: w[4], kappa: w[5], g_ty: w[6], g_ly: w[7], a_max: w[8], ds: w[9], ty: w[10], xy: w[11], w: w[12], coll: w[13] };
        }
        self.ln.n = i(30464);
        self.ln.f = i(30468);
        self.ln.j = f(30472);
        self.ln.t = f(30476);
        self.last_reached = v3(30480);
        let (st, ct, bs) = (i(30756), i(30760), i(30764));
        self.pg.start = st;
        self.pg.count = ct;
        self.pg.base = bs;
        for slot in 0..64usize {
            self.pg.d[slot] = i(30496 + 4 * slot);
        }
        self.j_prev = f(30768);
        self.cursor = i(30772);
        self.step = f(30776);
        self.mode = i(30780);
        self.progress = i(30784);
        self.cached = i(30836);
        self.next_goal = i(30840);
    }

    /// (diagnostics) the car as the aiv's own copy holds it after F-12813 - the input of that update.
    pub fn car_as_held(&self) -> AivCar {
        AivCar {
            pos: self.pos, right: self.right, forward: self.f, vel: self.vel, ang_vel: self.ang_vel, nitro_tune: self.b1, nitro_t: self.t1,
            sling_tune: self.b2, sling_t: self.t2, speed_cap: self.cap, size: self.size, gravity: self.g,
        }
    }

    /// (diagnostics) the sample ring and line points as `samples_raw` / `points` give them, for comparing two states.
    pub fn line_state(&self) -> (Vec<Vec3>, Vec<[f32; 14]>, i32, i32, f32, i32) {
        let (f, n, j, _t, prog, s) = self.samples_raw();
        let _ = (f, n);
        (self.points().collect(), s, self.ln.f, self.ln.n, j, prog)
    }

    /// (diagnostics, work/ai-record) the sample ring as the original's bytes +26768.. hold it: first (+30468), end (+30464),
    /// cost J (+30472), time T (+30476), progress (+30784) and each sample's 14 floats from the first to the end.
    pub fn samples_raw(&self) -> (i32, i32, f32, f32, i32, Vec<[f32; 14]>) {
        let (f, n) = (self.ln.f.max(0) as usize, (self.ln.n.max(0) as usize).min(NS));
        let v = (f..n).map(|i| { let s = &self.ln.s[i]; [s.v, s.allowed, s.v_curve, s.v_table, s.kv, s.kappa, s.g_ty, s.g_ly, s.a_max, s.ds, s.ty, s.xy, s.w, s.coll] }).collect();
        (self.ln.f, self.ln.n, self.ln.j, self.ln.t, self.progress, v)
    }

    /// (diagnostics) the corridor's portals as held: the ends a / b and the shrunk ends sa / sb
    pub fn portal_ends(&self) -> impl Iterator<Item = (Vec3, Vec3, Vec3, Vec3)> + '_ {
        (self.portals.base..self.portals.end()).map(|i| {
            let c = self.portals.get(i);
            (c.a, c.b, c.sa, c.sb)
        })
    }

    /// (diagnostics) the goal ring as held: base, count, then per goal p0, w, p1, speed, word; and the exit direction
    pub fn goals_dbg(&self) -> String {
        let mut s = format!("{},{},{:.3},{:.3},{:.3}", self.goals.base, self.goals.count, self.goal_dir.x, self.goal_dir.y, self.goal_dir.z);
        for g in self.goals.base..self.goals.end() {
            let g = self.goals.get(g);
            s.push_str(&format!(",{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{}", g.p0.x, g.p0.y, g.p0.z, g.w, g.p1.x, g.p1.y, g.p1.z, g.speed, 0.0, g.word));
        }
        s
    }

    pub fn points(&self) -> impl Iterator<Item = Vec3> + '_ {
        (self.ln.pts.base..self.ln.pts.end()).map(|i| self.ln.pts.get(i))
    }

    /// F-43876: the corridor point ahead.
    pub fn corridor_ahead(&self) -> Vec3 {
        let b = self.portals.base;
        if b + self.portals.count == 2 {
            let (p, q) = (self.portals.get(b + 1), self.portals.get(b));
            p.a * 0.25 + p.b * 0.25 + q.a * 0.25 + q.b * 0.25
        } else {
            let p = self.portals.get(b + 1);
            (p.a + p.b) * 0.5
        }
    }

    /// F-13872: the car is off the planned line (clamped position more than 0.25 m away in
    /// plan, more than 5 m below it, or the next point not reachable through the portals).
    /// Returns the test and the clamped position. Checked against the original run offline (test
    /// off_line_matches_the_original, 6,000 cases).
    pub fn off_line(&self) -> (bool, Vec3) {
        if self.progress >= self.ln.pts.end() {
            return (false, self.pos);
        }
        let mut x = self.pos;
        let mut idx = self.ln.pp.get(self.progress - 1) >> 1;
        self.clamp_cell(&mut idx, &mut x);
        self.surface_y(idx, &mut x);
        let (dx, dz) = (x.x - self.pos.x, x.z - self.pos.z);
        // (dx dx rounded, then dz dz + it in one rounding: the original's fmuls / fmadds)
        if fma(dz, dz, dx * dx) > 0.0625 || self.pos.y < x.y - 5.0 {
            return (true, x);
        }
        let pb = self.ln.pp.get(self.progress) >> 1;
        (!self.reach(idx, x, pb, self.ln.pts.get(self.progress)), x)
    }

    // ---- input from the brain ---------------------------------------------------------------

    /// F-95594: append a portal (shrunk by half the car width + BufferZone).
    /// As generated: the width from fused squares (z, then y, then x), the direction times the reciprocal refined once
    /// (vrefp, vnmsubfp, vmaddfp), the limits fused (fmadds / fmsubs), the near ends as vnmsubfp: -(d k - a).
    pub fn push_portal(&mut self, p: &Portal) {
        self.touch();
        // (fmadds / fmsubs as the recomp evaluates them: fma in double, rounded to single)
        let fma = |x: f32, y: f32, z: f32| (x as f64).mul_add(y as f64, z as f64) as f32;
        let (mut a, mut b) = (p.a, p.b);
        let mut c = Cell { n: p.normal, offroad: p.offroad, ..Cell::default() };
        let d = b - a;
        let w = (fma(d.x, d.x, fma(d.y, d.y, d.z * d.z)) as f64).sqrt() as f32;
        if w > 0.2 {
            let (e1, e2) = p.ext;
            let r = 1.0 / w;
            let inv = r * -(r * w - 1.0) + r;
            let d = d * inv;
            let lim = fma(self.size.x, 0.5, self.tun.buffer_zone);
            let half = fma(w + e1 + e2, 0.5, -0.05);
            let m = if lim - half >= 0.0 { half } else { lim };
            c.sb = d * (e2 - m) + b;
            c.sa = -(d * (e1 - m) - a);
            if p.offroad {
                let h = fma(w, 0.5, -0.05);
                let m2 = if m - h >= 0.0 { h } else { m };
                a = d * m2 + a;
                b = -(d * m2 - b);
            }
        } else {
            c.sa = a;
            c.sb = b;
        }
        c.a = a;
        c.b = b;
        self.portals.push(c);
    }

    /// F-82126: drop the last portal.
    pub fn pop_portal(&mut self) {
        self.touch();
        if self.portals.count == 0 {
            return;
        }
        self.portals.pop_back();
        self.planes_upto = self.planes_upto.min(self.portals.end() + 1);
    }

    /// F-29767: append a goal. `w` = radius, or the gate fraction (flags bit 0 = gate,
    /// segment p0-p1); `dir` = exit direction; `speed` = target speed (-1 none); `flags` = the
    /// brain's flags/tag (low 13 bits kept).
    pub fn push_goal(&mut self, dir: Vec3, p0: Vec3, w: f32, p1: Vec3, speed: f32, flags: u32) {
        self.touch();
        #[cfg(test)]
        GOAL_LOG.with(|l| l.borrow_mut().push((dir, p0, w, p1, speed, flags)));
        self.goal_dir = dir;
        self.j_prev = 1e8;
        self.ln.j = 1e8;
        self.ln.t = 1e8;
        let end = self.portals.end();
        if end < 2 {
            return;
        }
        if self.ln.pts.count == 0 {
            self.last_reached = self.pos;
            self.ln.pts.push(self.pos);
            self.ln.pp.push(0);
            self.pg.push(-1);
        }
        let word = ((end << 13).wrapping_sub(8193) & !0x1FFF) | (flags as i32 & 0x1FFF);
        self.goals.push(Goal { p0, w, p1, speed, word });
        self.planes();
    }

    /// F-69761 + F-70624 for each (up to 32).
    pub fn set_obstacles(&mut self, list: &[Obstacle]) {
        self.touch();
        self.n_obst = 0;
        for o in list.iter().take(32) {
            let dir = norm_or(Vec3::new(o.dir.x, 0.0, o.dir.z), 1e-5, Vec3::X);
            self.obst[self.n_obst] = Obstacle { pos: o.pos, vel: o.vel, dir, extent: (o.extent.0 * 0.5, o.extent.1 * 0.5), flags: o.flags & 0xC0 };
            self.n_obst += 1;
        }
    }

    // ---- per frame --------------------------------------------------------------------------

    /// F-12813 (car state, one leg of line building, progress, trimming), then the
    /// optimiser (F-32780 -> F-62607) when the line is not used up.
    /// (approx.: the original runs the optimiser as an async job joined on the next update, and
    /// only when brain+253 is clear; here it runs at once)
    pub fn update(&mut self, car: &AivCar, dt: f32) {
        self.touch();
        self.update_state(car, dt);
        if !self.line_used_up() {
            self.optimise();
        }
    }

    /// F-12813 alone.
    /// (the car's size, g and boost tunes are not F-12813's: `activate` / `car_defaults` set them)
    pub fn update_state(&mut self, car: &AivCar, dt: f32) {
        self.touch();
        self.pos = car.pos;
        self.f = car.forward;
        self.vel = car.vel;
        self.ang_vel = car.ang_vel;
        // (fused as F-12813 forms it: z product rounded, then y and x added in fmadds)
        self.v_fwd = self.vel.x.mul_add(self.f.x, self.f.y.mul_add(self.vel.y, self.f.z * self.vel.z));
        self.dt = dt;
        self.t1 = car.nitro_t;
        self.t2 = car.sling_t;
        self.cap = car.speed_cap;
        self.right = car.right;
        // heading reset: h . v below 1 or |v|^2 not at least 25 (a NaN resets), both fused in F-12813's order
        let hv = self.h.x.mul_add(self.vel.x, self.h.z.mul_add(self.vel.z, self.h.y * self.vel.y));
        let v2 = self.vel.z.mul_add(self.vel.z, self.vel.x.mul_add(self.vel.x, self.vel.y * self.vel.y));
        if hv < 1.0 || !(v2 >= 25.0) {
            self.h = self.f;
        }
        self.build_line();
        let end = self.ln.pts.end();
        if end < 2 {
            return;
        }
        while self.progress < end {
            let pi = self.ln.pts.get(self.progress);
            let d = pi - self.last_reached;
            let q = pi - self.pos;
            let k = if self.progress == end - 1 { 0.1 } else { 0.75 };
            if d.dot(self.h) < 0.0 {
                break;
            }
            if q.dot(d) > d.length_squared() * k {
                break;
            }
            self.last_reached = pi;
            self.pg.set(self.progress, -1);
            self.ln.pts.pop_front();
            self.ln.pp.pop_front();
            self.pg.pop_front();
            self.progress += 1;
            self.cursor = self.cursor.max(self.progress);
        }
        if self.progress >= end {
            return;
        }
        if self.tun.restart_every_update {
            self.cursor = self.progress;
        }
        let k = self.progress - 1;
        self.ln.pts.set(k, self.pos);
        let old = self.ln.pp.get(k);
        let mut idx = old >> 1;
        self.walk(&mut idx, self.pos);
        self.ln.pp.set(k, (idx << 1) | (old & 1));
        self.point_height(k);
        let n = self.portals.get(self.ln.pp.get(k) >> 1).n;
        if n.y > 0.5 {
            // n x (h x n) in the vector unit's form (products rounded, vnmsubfp)
            self.h = Self::vcross(n, Self::vcross(self.h, n));
        }
        // drop portals and goals behind everything still referenced
        let mut lo_portal = self.planes_upto;
        let mut lo_goal = self.next_goal;
        for i in k..end {
            lo_portal = lo_portal.min(self.ln.pp.get(i) >> 1);
            let tag = self.pg.get(i);
            if tag >= 0 {
                lo_goal = lo_goal.min(tag);
                lo_portal = lo_portal.min(self.goals.get(tag).portal());
            }
        }
        while self.portals.base < lo_portal {
            self.portals.pop_front();
        }
        while self.goals.base < lo_goal {
            self.goals.pop_front();
        }
    }

    /// F-65301 -> F-41825 -> F-28541. `gear` = brain gear - 2 clamped >= 0;
    /// `min_wheel` is passed by the brain but not read; `rpm_ratio` = the f3 term of
    /// F-28541; `collision` = the brain's collision timer running.
    pub fn output(&mut self, gear: i32, _min_wheel: f32, rpm_ratio: f32, collision: bool) -> AivOut {
        self.touch();
        if self.progress >= self.ln.pts.end() {
            return AivOut { throttle: 0.0, steer: 0.0, brake: 1.0, handbrake: 0.0, valid: false };
        }
        let (kc, ac) = self.command(collision);
        let (steer, throttle, brake) = pedal_map(&self.veh, kc, ac, rpm_ratio, self.t1, self.b1[1], self.t2, self.b2[1], self.g, gear, self.vel, self.f, self.right);
        // (the handbrake branch after F-28541 never fires: it always writes 0)
        AivOut { throttle, steer, brake, handbrake: 0.0, valid: true }
    }

    /// F-28541 for a commanded curvature / acceleration, with this follower's car state
    /// (the brain also calls it directly).
    pub fn pedals(&self, kappa: f32, a_cmd: f32, gear: i32, rpm_ratio: f32) -> (f32, f32, f32) {
        pedal_map(&self.veh, kappa, a_cmd, rpm_ratio, self.t1, self.b1[1], self.t2, self.b2[1], self.g, gear, self.vel, self.f, self.right)
    }

    /// F-28541 with the car's state as its caller passes it (the cruising brain's slot 11, F-95280: velocity, forward,
    /// the boost terms and gravity from the car, not this follower's state).
    pub fn controller(&self, kappa: f32, a_cmd: f32, rpm_ratio: f32, boost: [f32; 4], g: f32, gear: i32, vel: Vec3, fwd: Vec3, right: Vec3) -> (f32, f32, f32) {
        pedal_map(&self.veh, kappa, a_cmd, rpm_ratio, boost[0], boost[1], boost[2], boost[3], g, gear, vel, fwd, right)
    }

    /// F-82077: nitro benefit (0..1) for the brain's nitro decision.
    pub fn nitro_benefit(&self) -> f32 {
        self.boost_benefit(self.b1[1], self.b1[2], false)
    }
    /// F-96973: slingshot benefit.
    pub fn sling_benefit(&self) -> f32 {
        self.boost_benefit(self.b2[1], self.b2[2], true)
    }

    // ---- corridor ---------------------------------------------------------------------------

    /// F-68053: the cell planes of every portal pair not yet done.
    fn planes(&mut self) {
        let base = self.portals.base;
        let end = base + self.portals.count;
        let mut i = self.planes_upto.max(base + 1);
        self.planes_upto = i;
        while i < end {
            let q = self.portals.get(i);
            let mut p = self.portals.get(i - 1);
            let c = (p.a + p.b + q.a + q.b) * 0.25;
            p.fwd = plane_n((q.a + q.b) * 0.5 - c, q.b - q.a);
            p.back = plane_n((p.a + p.b) * 0.5 - c, p.b - p.a);
            p.side_a = side_n(p.a, q.a, c);
            p.side_b = side_n(p.b, q.b, c);
            p.sside_a = side_n(p.sa, q.sa, c);
            p.sside_b = side_n(p.sb, q.sb, c);
            self.portals.set(i - 1, p);
            i += 1;
            self.planes_upto = i;
        }
    }

    /// F-84189: walk the cell index along the corridor until x is in it.
    fn walk(&self, idx: &mut i32, x: Vec3) -> bool {
        let lo = self.portals.base;
        let hi = lo + self.portals.count - 2;
        let start = if *idx < lo { lo } else if *idx > hi { hi } else { *idx };
        *idx = start;
        if start <= hi {
            loop {
                let c = self.portals.get(*idx);
                let nx = self.portals.get(*idx + 1);
                if dot3(c.fwd, x - nx.a) >= 1e-6 {
                    if *idx >= hi {
                        return false;
                    }
                    *idx += 1;
                } else {
                    break;
                }
            }
            if *idx != start {
                return true;
            }
        }
        while *idx >= lo {
            let c = self.portals.get(*idx);
            if dot3(c.back, x - c.a) >= 1e-6 {
                if *idx <= lo {
                    return false;
                }
                *idx -= 1;
            } else {
                break;
            }
        }
        true
    }

    /// F-43908: binary search of the cell of x between the cells a and b.
    fn bracket(&self, idx: &mut i32, a: i32, b: i32, x: Vec3) -> bool {
        let mut lo = a.min(b).max(self.portals.base);
        let mut hi = a.max(b).min(self.portals.end() - 2);
        *idx = if *idx < lo { lo } else if *idx > hi { hi } else { *idx };
        if lo == hi {
            return true;
        }
        loop {
            while *idx <= hi {
                let c = self.portals.get(*idx);
                let nx = self.portals.get(*idx + 1);
                // (the vector unit's three-term dot, (0 + z z') + (y y' + x x'))
                if dot3(c.fwd, x - nx.a) >= 1e-6 {
                    if *idx >= hi {
                        return false;
                    }
                    lo = *idx + 1;
                    *idx += (hi - *idx + 1) / 2;
                } else {
                    hi = *idx;
                    break;
                }
            }
            while *idx >= lo {
                let c = self.portals.get(*idx);
                if dot3(c.back, x - c.a) >= 1e-6 {
                    if *idx <= lo {
                        return false;
                    }
                    hi = *idx - 1;
                    *idx -= (*idx - lo + 1) / 2;
                } else {
                    lo = *idx;
                    break;
                }
            }
            if lo == hi {
                return true;
            }
        }
    }

    /// F-15125: clamp x into its corridor cell (the shrunk side planes, and the end planes
    /// of the first and last cell).
    fn clamp_cell(&self, idx: &mut i32, x: &mut Vec3) {
        self.walk(idx, *x);
        let base = self.portals.base;
        let last = base + self.portals.count - 2;
        if *idx == base {
            let c = self.portals.get(*idx);
            *x -= c.back * dot3(c.back, *x - c.a).max(0.0);
        } else if *idx == last {
            let c = self.portals.get(*idx);
            let nx = self.portals.get(*idx + 1);
            *x -= c.fwd * dot3(c.fwd, *x - nx.a).max(0.0);
        }
        let c = self.portals.get(*idx);
        *x -= c.sside_a * dot3(c.sside_a, *x - c.sa).max(0.0);
        *x -= c.sside_b * dot3(c.sside_b, *x - c.sb).max(0.0);
    }

    /// F-84036: height of x on the cell's surface triangle.
    fn surface_y(&self, idx: i32, x: &mut Vec3) {
        let c = self.portals.get(idx);
        let nx = self.portals.get(idx + 1);
        let a = c.a;
        let e1 = c.b - a;
        // the original's operation order: the products rounded, the sums fused where it fuses them
        let f10 = fma(e1.z * e1.z, 1e-4, (e1.x * e1.x) * 1e-4);
        let test = |e2: Vec3| {
            let f2 = fma(e2.z, e2.z, e2.x * e2.x) * f10;
            let thr = if 1e-12 - f2 >= 0.0 { 1e-12 } else { f2 };
            let cr = fms(e1.z, e2.x, e2.z * e1.x);
            let cr = cr.abs();
            (!(cr * cr < thr), cr)
        };
        let mut e2 = nx.a - a;
        let (mut ok, mut cr) = test(e2);
        if !ok {
            e2 = nx.b - a;
            (ok, cr) = test(e2);
            if !ok {
                x.y = a.y;
                return;
            }
        }
        let d = *x - a;
        let inv = 1.0 / cr;
        let u = fms(e2.z, d.x, d.z * e2.x).abs() * inv;
        let w = fms(e1.z, d.x, d.z * e1.x).abs() * inv;
        // (the selects are the original's fsel on the differences)
        let (hi, lo) = if -e1.y >= 0.0 { (0.0, e1.y) } else { (e1.y, 0.0) };
        let lo = if lo - e2.y >= 0.0 { e2.y } else { lo };
        let hi = if hi - e2.y >= 0.0 { hi } else { e2.y };
        let y = fma(w, e2.y, u * e1.y) + a.y;
        let (ymin, ymax) = (lo + a.y, hi + a.y);
        let y = if y - ymin >= 0.0 { y } else { ymin };
        x.y = if y - ymax >= 0.0 { ymax } else { y };
    }

    /// F-80559: can p (in cell b) be reached from x (in cell a), bending the line once
    /// round each shrunk portal end at most.
    fn reach(&self, a: i32, x: Vec3, b: i32, p: Vec3) -> bool {
        if b == a {
            return true;
        }
        let (dir, mut i, stop) = if b > a { (1, a + 1, b + 1) } else { (-1, a, b) };
        let mut tgt = p;
        let (mut ra, mut rb) = (false, false);
        while i != stop {
            let c = self.portals.get(i);
            if let Some((s, t)) = seg_portal(x, tgt, c.sa, c.sb, false) {
                if !(0.0..=1.0).contains(&s) {
                    return false;
                }
                if t < 0.0 {
                    if rb {
                        return false;
                    }
                    let v = tgt - x;
                    tgt = rot_y(v, ang_xz(v, c.sa - x)) + x;
                    ra = true;
                } else if t > 1.0 {
                    if ra {
                        return false;
                    }
                    let v = tgt - x;
                    tgt = rot_y(v, ang_xz(v, c.sb - x)) + x;
                    rb = true;
                }
            }
            i += dir;
        }
        true
    }

    /// F-42352: P -> M crosses every shrunk portal from..=to inside it (and is not too long).
    fn visible(&self, p: Vec3, m: Vec3, from: i32, to: i32) -> bool {
        let isl = self.tun.initial_segment_length;
        if isl > 1e-6 {
            let n = (((m - p).length() + 0.1) / isl + 0.5) as i32 + 1;
            if n > 32 {
                return false;
            }
        }
        for i in from..=to {
            let c = self.portals.get(i);
            match seg_portal(p, m, c.sa, c.sb, false) {
                Some((_, t)) if (0.0..=1.0).contains(&t) => {}
                _ => return false,
            }
        }
        true
    }

    /// F-51411: the furthest portal whose midpoint the last point sees (string pulling).
    fn furthest_visible(&self) -> i32 {
        let pend = self.portals.end();
        let gend = self.goals.end();
        if pend <= 2 {
            return pend;
        }
        let s = self.ln.pp.last() >> 1;
        if self.next_goal < gend && self.goals.get(self.next_goal).portal() <= s {
            return s + 1;
        }
        let p = self.ln.pts.last();
        let mut r = s + 2;
        while r < pend {
            let c = self.portals.get(r);
            let m = (c.a + c.b) * 0.5;
            if (m - p).length_squared() >= 1e-12 {
                if !self.visible(p, m, s + 1, r - 1) {
                    return r - 1;
                }
                if self.next_goal < gend && self.goals.get(self.next_goal).portal() == r {
                    let tgt = self.goals.get(self.next_goal).target();
                    return if self.visible(p, tgt, s + 1, r) { r + 1 } else { r };
                }
            }
            r += 1;
        }
        pend
    }

    /// F-84385: append a leg A -> B cut into pieces of about InitialSegmentLength.
    fn add_leg(&mut self, a: Vec3, b: Vec3, target: i32, tag: i32) -> bool {
        let pend = self.portals.end();
        let isl = self.tun.initial_segment_length;
        let n = if isl > 1e-6 { ((b - a).length() / isl + 0.5) as i32 + 1 } else { 1 };
        if n > 64 - self.ln.pts.count {
            return false;
        }
        let e2 = pend - 2;
        let nf = n as f32;
        for k in 1..n {
            let p = (b - a) * (k as f32 / nf) + a;
            self.ln.pts.push(p);
            self.pg.push(-1);
            let prev = self.ln.pp.last() >> 1;
            let mut idx = (prev + 1).min(target);
            if idx >= e2 {
                idx = e2;
            }
            self.walk(&mut idx, p);
            self.ln.pp.push(idx << 1);
            self.point_height(self.ln.pts.end() - 1);
        }
        self.ln.pts.push(b);
        self.pg.push(tag);
        let v = if target < 0 { 0 } else if target <= e2 { target } else { e2 };
        self.ln.pp.push(v << 1);
        true
    }

    /// F-85983: one leg per call, to the furthest visible portal or the next goal.
    fn build_line(&mut self) {
        let pend = self.portals.end();
        let gend = self.goals.end();
        if pend < 2 || self.ln.pts.count == 64 {
            return;
        }
        if self.cached == 0 {
            self.cached = self.furthest_visible();
        }
        let cached = self.cached;
        let mut g = self.next_goal;
        let mut tp = cached;
        if g < gend && cached > self.goals.get(g).portal() {
            tp = self.goals.get(g).portal();
        } else {
            g = -1;
        }
        if tp >= pend {
            return;
        }
        let target = if g == -1 {
            let c = self.portals.get(cached);
            (c.a + c.b) * 0.5
        } else {
            self.goals.get(g).target()
        };
        let li = self.ln.pts.end() - 1;
        let l = self.ln.pts.get(li);
        let d = target - l;
        let d2 = d.x.mul_add(d.x, d.z.mul_add(d.z, d.y * d.y)); // (fused in F-85983's order)
        let tagged = g != -1 && self.pg.get(li) == -1 && li >= self.progress && !self.goals.get(g).gate() && d2 < self.goals.get(g).w * self.goals.get(g).w;
        if tagged {
            self.pg.set(li, g);
        } else if d2 > 1e-12 && !self.add_leg(l, target, tp, g) {
            return;
        }
        if g != -1 {
            self.next_goal += 1;
        }
        self.cached = 0;
    }

    // ---- line state helpers -----------------------------------------------------------------

    fn window_end(&self) -> i32 {
        let mut e = self.ln.pts.end();
        if self.tun.max_steps_ahead >= 0 {
            e = e.min(self.progress + self.tun.max_steps_ahead);
        }
        e
    }

    fn port(&self, k: i32) -> i32 {
        self.ln.pp.get(k) >> 1
    }

    fn set_port(&mut self, k: i32, idx: i32) {
        let old = self.ln.pp.get(k);
        self.ln.pp.set(k, (idx << 1) | (old & 1));
    }

    fn set_coll(&mut self, k: i32) {
        let old = self.ln.pp.get(k);
        self.ln.pp.set(k, old | 1);
    }

    fn set_dirty(&mut self, bit: i32) {
        if (0..64).contains(&bit) {
            self.sc.dirty[(bit >> 5) as usize] |= 1 << (bit & 31);
        }
    }

    fn dirty(&self, bit: i32) -> bool {
        (0..64).contains(&bit) && self.sc.dirty[(bit >> 5) as usize] & (1 << (bit & 31)) != 0
    }

    /// F-77899
    fn point_height(&mut self, k: i32) {
        let mut p = self.ln.pts.get(k);
        self.surface_y(self.port(k), &mut p);
        self.ln.pts.set(k, p);
    }

    /// F-19562: the link a-b does not miss a shrunk portal it crosses.
    fn link_ok(&self, a: i32, b: i32) -> bool {
        let (pa, pb) = (self.port(a), self.port(b));
        if pa == pb {
            return true;
        }
        let (dir, mut i, stop) = if pb > pa { (1, pa + 1, pb + 1) } else { (-1, pa, pb) };
        let (xa, xb) = (self.ln.pts.get(a), self.ln.pts.get(b));
        while i != stop {
            let c = self.portals.get(i);
            if let Some((s, t)) = seg_portal(xa, xb, c.sa, c.sb, false) {
                // (compares as the original's: a NaN is neither below nor above the range)
                if !(s < 0.0 || s > 1.0) && (t < 0.0 || t > 1.0) {
                    return false;
                }
            }
            i += dir;
        }
        true
    }

    /// F-63779: a pushed point is still inside the corridor with valid links.
    fn push_ok(&self, k: i32, out: &mut i32) -> bool {
        *out = self.port(k);
        let p = self.ln.pts.get(k);
        if !self.walk(out, p) {
            return false;
        }
        let c = self.portals.get(*out);
        if dot3(c.sside_a, p - c.sa) >= 1e-6 || dot3(c.sside_b, p - c.sb) >= 1e-6 {
            return false;
        }
        if k >= self.progress && !self.link_ok(k - 1, k) {
            return false;
        }
        if k < self.ln.pts.end() - 1 && !self.link_ok(k + 1, k) {
            return false;
        }
        true
    }

    /// F-81109: keep point k on its goal and in the corridor; mark it dirty.
    fn constrain(&mut self, k: i32) {
        let tag = self.pg.get(k);
        if tag != -1 {
            let g = self.goals.get(tag);
            let p = self.ln.pts.get(k);
            if g.gate() {
                let e = g.p1 - g.p0;
                // (the scalar chains x x' + (y y' + z z') with the innermost product rounded)
                let s = dot_xyz(p - g.p0, e) / dot_xyz(e, e);
                let s = if s >= 0.0 { s } else { 0.0 };
                let s = if s - 1.0 >= 0.0 { 1.0 } else { s };
                self.ln.pts.set(k, e * s + g.p0);
            } else {
                let d = g.p0 - p;
                let d2 = fma(d.z, d.z, d.x * d.x);
                if d2 > g.w * g.w {
                    // frsqrte, two Newton steps in double, rounded to single (zero where -d2 >= 0)
                    let x = d2 as f64;
                    let y0 = (1.0 / x.sqrt()) as f32 as f64;
                    let e0 = -((y0 * y0).mul_add(x, -1.0));
                    let y1 = e0.mul_add(y0 * 0.5, y0);
                    let e1 = -((y1 * y1).mul_add(x, -1.0));
                    let rs = e1.mul_add(y1 * 0.5, y1) as f32;
                    let rs = if -d2 >= 0.0 { 0.0 } else { rs };
                    let f = fms(rs, d2, g.w) * rs;
                    self.ln.pts.set(k, d * f + p);
                }
            }
        }
        let mut idx = self.port(k);
        let mut p = self.ln.pts.get(k);
        self.clamp_cell(&mut idx, &mut p);
        self.ln.pts.set(k, p);
        self.set_port(k, idx);
        self.set_dirty(k - self.progress + 1);
    }

    /// F-29423: make the link a -> b pass the shrunk portals by rotating b about a (once
    /// round each end at most), then reconstrain b and set its height.
    fn fix_link(&mut self, a: i32, b: i32) -> bool {
        let (pa, pb) = (self.port(a), self.port(b));
        if pb == pa {
            return true;
        }
        let (dir, mut i, stop) = if pb > pa { (1, pa + 1, pb + 1) } else { (-1, pa, pb) };
        let (mut ra, mut rb) = (false, false);
        while i != stop {
            let c = self.portals.get(i);
            let xa = self.ln.pts.get(a);
            let xb = self.ln.pts.get(b);
            if let Some((s, t)) = seg_portal(xa, xb, c.sa, c.sb, false) {
                if (0.0..=1.0).contains(&s) {
                    if t < 0.0 {
                        if rb {
                            return false;
                        }
                        let v = xb - xa;
                        self.ln.pts.set(b, rot_y(v, ang_xz(v, c.sa - xa)) + xa);
                        ra = true;
                    } else if t > 1.0 {
                        if ra {
                            return false;
                        }
                        let v = xb - xa;
                        self.ln.pts.set(b, rot_y(v, ang_xz(v, c.sb - xa)) + xa);
                        rb = true;
                    }
                }
            }
            i += dir;
        }
        if ra || rb {
            self.constrain(b);
        }
        self.point_height(b);
        true
    }

    /// F-63252: rotate points k.. about point k-1, each following point by the falloff of
    /// the previous point's achieved angle.
    fn rotate_from(&mut self, k: i32, angle: f32) -> bool {
        let end = self.window_end();
        let last = end - 1;
        let mut i = k;
        if k < last {
            let mut a = angle;
            loop {
                let pivot = self.ln.pts.get(k - 1);
                let d = self.ln.pts.get(i) - pivot;
                // the rotation as this function carries it inline (F-57591 with the x row's fused multiply on the
                // other product: x' = z sin + (x cos), z' = z cos - x sin)
                let s = guest_sin(a as f64) as f32;
                let c = guest_cos(a as f64) as f32;
                let r = Vec3::new(fma(d.z, s, d.x * c), d.y, fms(d.z, c, d.x * s));
                self.ln.pts.set(i, r + pivot);
                self.constrain(i);
                if !self.fix_link(i - 1, i) {
                    return false;
                }
                let d2 = self.ln.pts.get(i) - self.ln.pts.get(k - 1);
                a = self.tun.angular_adjustment_falloff * ang_xz(d, d2);
                if a.abs() / angle.abs() < self.tun.angular_adjustment_threshold {
                    break;
                }
                i += 1;
                if i >= last {
                    break;
                }
            }
        }
        if end > 2 { self.fix_link(last, end - 2) } else { true }
    }

    /// the evaluation chain of F-21477 / F-62607
    fn evaluate(&mut self) {
        self.ln.j = 0.0;
        self.obstacles();
        self.spline();
        self.samples();
        let d = self.backward();
        self.forward(d);
    }

    /// F-21477: try point k moved by +-step (mode 0 rotate, 1 along, 2 across); keep the best.
    fn try_move(&mut self, k: i32, recurse: bool) {
        let (o, b) = if recurse { (0, 2) } else { (1, 3) };
        self.sc.backup[o] = self.ln;
        self.sc.backup[b] = self.ln;
        let mode = self.mode;
        let mut dv = Vec3::ZERO;
        if mode != 0 {
            let d = self.ln.pts.get(k + 1) - self.ln.pts.get(k - 1);
            let d = Vec3::new(d.x, 0.0, d.z);
            // the vector unit's normalisation (dot3, the refined reciprocal root, one multiply), the unit x where the
            // squared length is not above 1e-12
            let l2 = dot3(d, d);
            dv = if l2.abs() > 1e-12 { d * vrsqrt1(l2) } else { Vec3::X };
            if mode == 2 {
                dv = Vec3::new(-dv.z, 0.0, dv.x);
            }
        }
        let step = self.step;
        for s in [step, -step] {
            let ok = if self.mode == 0 {
                self.rotate_from(k, self.tun.angular_adjustment * s * 0.017_453_292)
            } else {
                let p = self.ln.pts.get(k);
                self.ln.pts.set(k, dv * (self.tun.linear_adjustment * s) + p);
                self.constrain(k);
                self.fix_link(k - 1, k) && self.fix_link(k + 1, k)
            };
            if ok {
                self.evaluate();
                if self.mode != 0 && recurse {
                    self.recurse(k);
                }
                if self.ln.j < self.sc.backup[b].j {
                    self.sc.backup[b] = self.ln;
                }
            }
            self.ln = self.sc.backup[o];
        }
        self.ln = self.sc.backup[b];
    }

    /// F-34153: re-try the neighbours with a falling step.
    fn recurse(&mut self, k: i32) {
        let msr = self.tun.max_steps_to_recurse;
        let mut hi = self.window_end() - 1;
        if msr >= 0 {
            hi = hi.min(k + msr + 1);
        }
        let s0 = self.step;
        for i in k + 1..hi {
            self.step *= self.tun.recursive_falloff;
            self.try_move(i, false);
        }
        self.step = s0;
        if self.mode != 0 {
            let mut lo = self.progress;
            if msr >= 0 {
                lo = lo.max(k - msr);
            }
            let mut i = k - 1;
            while i >= lo {
                self.step *= self.tun.recursive_falloff;
                self.try_move(i, false);
                i -= 1;
            }
            self.step = s0;
        }
    }

    /// F-62607: evaluate the line, then coordinate descent over the control points.
    pub fn optimise(&mut self) {
        self.touch();
        let end = self.window_end();
        if self.tun.restart_every_update {
            self.cursor = self.progress;
        }
        self.ln.s = [Sample::default(); NS];
        self.ln.n = 0;
        self.sc.dirty[0] = 0x3F_FFFF;
        self.evaluate();
        if self.progress >= end - 1 {
            return;
        }
        let (s0, m0) = (self.step, self.mode);
        self.step = self.tun.adjustment_reset_threshold;
        self.mode = 2;
        self.try_move(self.progress, true);
        self.step = s0;
        self.mode = m0;
        self.counters = [0; 3];
        let mut calls = 1;
        loop {
            self.try_move(self.cursor, true);
            self.mode += 1;
            if self.mode >= 3 {
                self.mode = 0;
                self.cursor += 1;
                if self.cursor >= end - 1 {
                    let (jp, j) = (self.j_prev, self.ln.j);
                    self.cursor = self.progress;
                    if (jp - j).abs() / jp < self.tun.adjustment_falloff_trigger {
                        self.step *= self.tun.adjustment_falloff;
                        if self.step < self.tun.adjustment_reset_threshold {
                            self.step = 1.0;
                        }
                    }
                    self.j_prev = j;
                }
            }
            let c = calls;
            calls += 1;
            if c >= 100 || self.counters[0] >= 1200 || self.counters[1] >= 400 || self.counters[2] >= 5000 {
                break;
            }
        }
    }

    // ---- obstacles --------------------------------------------------------------------------

    /// F-70569: push the line's points sideways off the predicted obstacles.
    fn obstacles(&mut self) {
        let end = self.window_end();
        let v = self.v_fwd;
        let inv = 1.0 / if v - 10.0 >= 0.0 { v } else { 10.0 };
        let ex = self.size.x * 0.5 + self.tun.buffer_zone;
        let ez = self.size.z * 0.5 + self.tun.buffer_zone;
        let mohd = self.tun.max_obstacle_height_delta;
        let mofd2 = self.tun.max_obstacle_flat_delta * self.tun.max_obstacle_flat_delta;
        let mopd2 = self.tun.max_obstacle_prediction_dist * self.tun.max_obstacle_prediction_dist;
        let olt = self.tun.obstacle_lookahead_time;
        for i in self.progress - 1..end {
            let p = self.ln.pp.get(i);
            self.ln.pp.set(i, p & !1);
        }
        let mut i = self.progress;
        let mut t = 0.0f32;
        let (mut s_neg, mut s_pos) = (false, false);
        let xz2 = |v: Vec3| v.x * v.x + v.z * v.z;
        while i < end {
            let p0 = self.ln.pts.get(i - 1);
            let dvec = if i == end - 1 { self.ln.pts.get(i) - p0 } else { self.ln.pts.get(i + 1) - p0 };
            let dxz = Vec3::new(dvec.x, 0.0, dvec.z);
            let l2 = dot3(dxz, dxz);
            let dn = if l2.abs() > 1e-12 { dxz * vrsqrt1(l2) } else { Vec3::ZERO };
            let lt = vlen(self.ln.pts.get(i) - p0) * inv;
            // (up x dn) as the vector unit forms it: products with the unit vector's zeros kept, so the zeros keep their signs
            let lat = Vec3::new(0.0 * dn.y - dn.z, 0.0 * dn.z - 0.0 * dn.x, dn.x - 0.0 * dn.y);
            let (mut e_neg, mut e_pos) = (false, false);
            for o in 0..self.n_obst {
                let ob = self.obst[o];
                if ob.flags & OBST_FIRST_LEG != 0 && i > self.progress {
                    continue;
                }
                let te = t + lt;
                let mut a_end = self.ln.pts.get(i) - (ob.vel * te + ob.pos);
                let mut a_start = self.ln.pts.get(i - 1) - (ob.vel * t + ob.pos);
                if (ob.vel * t).length_squared() >= mopd2 {
                    continue;
                }
                let (dmin, dmax) = (a_end.y.min(a_start.y), a_end.y.max(a_start.y));
                if -mohd > dmax || dmin > mohd || xz2(a_end).min(xz2(a_start)) > mofd2 {
                    continue;
                }
                self.counters[0] += 1;
                let r = Vec3::new(0.0 * ob.dir.y - ob.dir.z, 0.0 * ob.dir.z - 0.0 * ob.dir.x, ob.dir.x - 0.0 * ob.dir.y);
                let (hx, hz) = ob.extent;
                let c1 = r * hx + ob.dir * hz;
                let c2 = ob.dir * hz - r * hx;
                if ob.flags & OBST_FIRST_LEG == 0 {
                    let ed = dn.dot(c1).abs().max(dn.dot(c2).abs()) + ez;
                    let a = dn.dot(a_end);
                    let b = dn.dot(a_start);
                    if -ed > a.max(b) || a.min(b) > ed {
                        continue;
                    }
                    let den = b - a;
                    // the reciprocal estimate (1/x) refined by one Newton step; f32::MAX for a zero denominator
                    let r0 = 1.0 / den;
                    let rcp = if den == 0.0 { f32::MAX } else { r0 * (-(r0 * den - 1.0)) + r0 };
                    let u = (rcp * b).min(1.0).max(0.0);
                    let pc = ob.vel * (lt * u + t) + ob.pos;
                    a_end = self.ln.pts.get(i) - pc;
                    a_start = self.ln.pts.get(i - 1) - pc;
                }
                let el = lat.dot(c1).abs().max(lat.dot(c2).abs()) + ex;
                let s = lat.dot(a_start);
                let e = lat.dot(a_end);
                if s >= 0.0 {
                    if i > self.progress && !(s >= el) {
                        self.push_point(i - 1, lat * (el - s), i - self.progress, s_neg);
                        s_pos |= self.last_push_ok;
                    }
                    if i >= end - 1 {
                        continue;
                    }
                    if !(e >= el) {
                        self.push_point(i, lat * (el - e), i - self.progress + 1, e_neg);
                        e_pos |= self.last_push_ok;
                    }
                } else {
                    let nel = -el;
                    if i > self.progress && s > nel {
                        self.push_point(i - 1, lat * (nel - s), i - self.progress, s_pos);
                        s_neg |= self.last_push_ok;
                    }
                    if i >= end - 1 {
                        continue;
                    }
                    if e > nel {
                        self.push_point(i, lat * (nel - e), i - self.progress + 1, e_pos);
                        e_neg |= self.last_push_ok;
                    }
                }
            }
            t += lt;
            s_neg = e_neg;
            s_pos = e_pos;
            if t > olt {
                break;
            }
            i += 1;
        }
    }

    // ---- spline, samples, speed profile -------------------------------------------------------

    /// F-86861: Catmull-Rom coefficients of the legs in the window.
    fn spline(&mut self) {
        let end = self.window_end();
        let pr = self.progress;
        let n = end - pr + 1;
        self.sc.legs = (n - 1).max(0);
        if n < 2 {
            return;
        }
        let p = |s: &Self, i: i32| s.ln.pts.get(i);
        let v = if n > 2 { self.h * vlen(p(self, pr + 1) - p(self, pr - 1)) } else { Vec3::ZERO };
        let ph = if dot3(v, v) > 1e-12 { p(self, pr) - v } else { (p(self, pr - 1) - p(self, pr)) * 2.0 + p(self, pr) };
        let mut w = [ph, p(self, pr - 1), p(self, pr), Vec3::ZERO];
        let mut k = 0usize;
        let mut r = pr - 1;
        while r < end - 2 {
            w[3] = p(self, r + 2);
            if k < 64 {
                self.sc.coef[k] = cr(&w);
            }
            k += 1;
            w = [w[1], w[2], w[3], Vec3::ZERO];
            r += 1;
        }
        let v = if end < self.ln.pts.end() && n > 2 {
            p(self, end) - p(self, end - 2)
        } else if pr < end - 1 && n > 2 && dot3(self.goal_dir, self.goal_dir) > 1e-12 {
            // the goal direction times (the length of the chord times its refined reciprocal root)
            self.goal_dir * (vlen(p(self, end - 1) - p(self, end - 3)) * vrsqrt1(dot3(self.goal_dir, self.goal_dir)))
        } else {
            Vec3::ZERO
        };
        w[3] = if dot3(v, v) > 1e-12 { v + p(self, end - 2) } else { (p(self, end - 1) - p(self, end - 2)) * 2.0 + p(self, end - 2) };
        if k < 64 {
            self.sc.coef[k] = cr(&w);
        }
    }

    fn dp4(a: [f32; 4], b: [f32; 4]) -> f32 {
        (a[3] * b[3] + a[2] * b[2]) + (a[1] * b[1] + a[0] * b[0])
    }

    /// The cross product a x b as F-27864's vector code forms it: the products of the shuffled operands are rounded, then
    /// `vnmsubfp` gives -(p - q), so an exact cancellation leaves -0 (a plain difference would leave +0).
    #[inline]
    fn vcross(a: Vec3, b: Vec3) -> Vec3 {
        let nm = |p: f32, q: f32| -(p - q);
        Vec3::new(nm(b.y * a.z, b.z * a.y), nm(b.z * a.x, b.x * a.z), nm(b.x * a.y, b.y * a.x))
    }

    /// The curvature speed table read as F-27864 reads it (the class's tables at [1939 + i] / [2003 + i], blob scales [8..10]):
    /// the index by the add / subtract floor, the fraction from the unclamped argument, the interpolation fused, floored at 0.
    #[inline]
    fn curve_speed_exact(&self, k_abs: f32) -> f32 {
        let (row, x) = if k_abs < self.veh.knee { (&self.veh.t.speed_low_curv, self.veh.low_k * k_abs) } else { (&self.veh.t.speed_high_curv, (k_abs - self.veh.knee) * self.veh.high_k) };
        let c = fsel(x - 62.1, 62.1, x);
        let m = fsel(c, -8388608.0, 8388608.0);
        let r = (c - m) + m;
        let fl = fsel(c - r, r, r - 1.0);
        let i = fl as i32;
        let frac = x - fl;
        let i = (i.max(0) as usize).min(row.len().saturating_sub(2));
        let (lo, hi) = (row[i], row[i + 1]);
        let v = fma(hi - lo, frac, lo);
        fsel(-v, 0.0, v)
    }

    /// F-27864: sample the dirty legs (curvature, grip, table speeds, weights).
    fn samples(&mut self) {
        let end = self.window_end();
        let pr = self.progress;
        let sps = self.tun.samples_per_segment;
        let isl = self.tun.initial_segment_length;
        let n = end - pr + 1;
        // the first leg's sample count: the length of the first segment (the squared length (dy², then dz², then dx², fused),
        // the single-precision root) times samples per segment over the nominal length, plus a half, cut to a whole number
        let p0 = self.ln.pts.get(pr - 1);
        let p1 = self.ln.pts.get(pr);
        let (dx, dy, dz) = (p1.x - p0.x, p1.y - p0.y, p1.z - p0.z);
        let len2 = fma(dx, dx, fma(dz, dz, dy * dy));
        let len = (len2 as f64).sqrt() as f32;
        let n0 = fma(len, sps as f32 / isl, 0.5) as i32;
        let n0 = if n0 < 1 { 1 } else if n0 > 2 * sps { 2 * sps } else { n0 };
        self.ln.f = 2 * sps - n0;
        self.ln.n = sps * n + 1;
        let inv0 = 1.0 / n0 as f32;
        let inv_s = 1.0 / sps as f32;
        let t_tab = self.veh.t.clone();
        let g = self.g;
        let msc = self.tun.max_speed_scalar;
        let port0 = self.port(pr - 1);
        let nrm0 = self.portals.get(port0).n;
        let l0 = Self::vcross(self.h, nrm0);
        let mut th = ang3(UP, self.h);
        let f = self.ln.f as usize;
        if f < NS {
            let s = &mut self.ln.s[f];
            s.kappa = 0.0;
            s.kv = 0.0;
            s.ds = 0.0;
            s.v_curve = t_tab.max_speed;
            s.v_table = t_tab.max_speed;
            s.g_ty = g * self.h.y;
            s.g_ly = g * l0.y;
            s.a_max = fsel(1e-6 - nrm0.y, 1e-6, nrm0.y) * t_tab.max_total_accel;
            s.coll = -1.0;
            s.ty = self.h.y;
            s.xy = self.pos.y;
            s.w = 1.0;
        }
        let mut cursor = self.ln.f + 1;
        let mut cnt = n0;
        let mut inv = inv0;
        let mut cont = true;
        let (mut xp, mut tp, mut thp): (Vec3, Vec3, f32);
        let mut x = self.pos;
        let mut tn = self.h;
        let mut port = port0;
        let mut lo_port: i32;
        let mut r = pr - 1;
        while r < end - 1 {
            let k = r - pr + 1;
            let any = self.dirty((k - 1).max(0)) || self.dirty(k) || self.dirty((k + 1).min(n - 1)) || self.dirty((k + 2).min(n - 1));
            if !any {
                cursor += cnt;
                cont = false;
                cnt = sps;
            } else {
                self.counters[1] += 1;
                let first = cursor;
                cursor += cnt;
                for j in 0..cnt {
                    if cont {
                        xp = x;
                        tp = tn;
                        thp = th;
                        lo_port = port;
                    } else {
                        lo_port = self.port(r);
                        xp = self.ln.pts.get(r);
                        tp = self.ln.pts.get(r + 1) - self.ln.pts.get(r - 1);
                        thp = ang3(UP, tp);
                        cont = true;
                    }
                    let last_j = j == cnt - 1;
                    let tv;
                    if last_j && r < end - 2 {
                        x = self.ln.pts.get(r + 1);
                        tv = self.ln.pts.get(r + 2) - self.ln.pts.get(r);
                        port = self.port(r + 1);
                    } else {
                        // the parameter: the reciprocal of the leg's count times the sample number (rounded to single)
                        let tt = inv * (j + 1) as f32;
                        let tt2 = tt * tt;
                        let tt3 = tt2 * tt;
                        let wp = [tt3, tt2, tt, 1.0];
                        let wt = Vec3::new(tt2 * 3.0, tt * 2.0, 1.0);
                        let c = self.sc.coef[(k as usize).min(63)];
                        // position: the four-term dot of the powers with each column of the coefficient rows; tangent:
                        // the three-term dot of (3 t^2, 2 t, 1) with the first three rows
                        x = Vec3::new(
                            Self::dp4(wp, [c[0].x, c[1].x, c[2].x, c[3].x]),
                            Self::dp4(wp, [c[0].y, c[1].y, c[2].y, c[3].y]),
                            Self::dp4(wp, [c[0].z, c[1].z, c[2].z, c[3].z]),
                        );
                        tv = Vec3::new(
                            dot3(wt, Vec3::new(c[0].x, c[1].x, c[2].x)),
                            dot3(wt, Vec3::new(c[0].y, c[1].y, c[2].y)),
                            dot3(wt, Vec3::new(c[0].z, c[1].z, c[2].z)),
                        );
                        let mut idx = port;
                        self.bracket(&mut idx, lo_port, self.port(r + 1), x);
                        port = idx;
                    }
                    // the unit tangent: the refined reciprocal root times the vector where the squared length exceeds 1e-12
                    let l2 = dot3(tv, tv);
                    let rs = vrsqrt1(l2);
                    tn = if l2 > 1e-12 { Vec3::new(rs * tv.x, rs * tv.y, rs * tv.z) } else { Vec3::ZERO };
                    let cell = self.portals.get(port);
                    let l = Self::vcross(tn, cell.n);
                    th = ang3(UP, tn);
                    // the step: squared length (dy^2, dz^2, dx^2 fused), the scalar reciprocal root (frsqrte as 1/sqrt rounded
                    // to single, two Newton steps in double) and its product with the squared length
                    let (ex, ey, ez) = (x.x - xp.x, x.y - xp.y, x.z - xp.z);
                    let d2 = fma(ex, ex, fma(ez, ez, ey * ey));
                    let xd = d2 as f64;
                    let y0 = (1.0 / xd.sqrt()) as f32 as f64;
                    let e0 = -((y0 * y0).mul_add(xd, -1.0));
                    let y1 = e0.mul_add(y0 * 0.5, y0);
                    let e1 = -((y1 * y1).mul_add(xd, -1.0));
                    let rs_d = e1.mul_add(y1 * 0.5, y1) as f32;
                    let rs_d = fsel(-d2, 0.0, rs_d);
                    let inv_ds = fsel(d2 - 1e-12, rs_d, 1e8);
                    let ds = fsel(d2 - 1e-12, inv_ds * d2, 1e-6);
                    let kappa = ang_xz(tp, tn) * inv_ds;
                    // the heading change, wrapped into [-pi, pi] by two fsel pairs
                    let dth0 = th - thp;
                    let dth1 = fsel(-(dth0 - std::f32::consts::PI), dth0, dth0 - std::f32::consts::TAU);
                    let dth = fsel(dth1 + std::f32::consts::PI, dth1, dth1 + std::f32::consts::TAU);
                    let kv = -(dth * inv_ds);
                    let coll = if last_j && self.ln.pp.get(r + 1) & 1 != 0 { 1.0 } else { -1.0 };
                    let tag = if last_j { self.pg.get(r + 1) } else { -1 };
                    let goal_speed = if tag != -1 { self.goals.get(tag).speed } else { -1.0 };
                    let k_abs = kappa.abs();
                    let vt = self.curve_speed_exact(k_abs);
                    let v_table = msc * vt;
                    let big = k_abs - t_tab.min_curvature;
                    let a_lim = (vt * vt) * k_abs;
                    let a_max = fsel(big, fsel(a_lim - t_tab.max_total_accel, t_tab.max_total_accel, a_lim) * fsel(1e-6 - cell.n.y, 1e-6, cell.n.y), t_tab.max_total_accel);
                    // the speed the grip allows: sqrt(a_max / (|kappa| + a_max kv / g)) times the scalar
                    let den0 = fma(a_max * self.inv_g, kv, k_abs);
                    let den = fsel(1e-6 - den0, 1e-6, den0);
                    let rcp = (1.0f64 / den as f64) as f32;
                    let vv = sqrt_nr(rcp * a_max) * msc;
                    let v_curve = fsel(big, fsel(v_table - vv, vv, v_table), v_table);
                    let mut w = 1.0;
                    if cell.offroad && (dot3(cell.side_a, x - cell.a) >= 1e-6 || dot3(cell.side_b, x - cell.b) >= 1e-6) {
                        w = self.tun.off_road_weight + 1.0;
                    }
                    let si = (first + j) as usize;
                    if si < NS {
                        let s = &mut self.ln.s[si];
                        s.ds = ds;
                        s.kappa = kappa;
                        s.kv = kv;
                        s.g_ty = g * tn.y;
                        s.g_ly = g * l.y;
                        s.ty = tn.y;
                        s.xy = x.y;
                        s.w = w;
                        s.coll = coll;
                        s.allowed = goal_speed;
                        s.v_table = v_table;
                        s.a_max = a_max;
                        s.v_curve = v_curve;
                    }
                }
                cnt = sps;
            }
            inv = inv_s;
            r += 1;
        }
        self.sc.dirty[0] = 0;
    }

    /// F-31929: backward braking pass; returns the mean sample spacing.
    fn backward(&mut self) -> f32 {
        let (n, f) = (self.ln.n, self.ln.f);
        let v0 = self.v_fwd;
        // the first sample's speed is the car's, floored at 0.1 (an fsel pair: NaN takes the second operand)
        if (f as usize) < NS {
            self.ln.s[f as usize].v = fsel(0.1 - v0, 0.1, v0);
        }
        self.ln.t = 0.0;
        if n - f <= 1 || n as usize > NS {
            return 0.0;
        }
        let cap = self.cap;
        // allowed = min(v_curve, goal, cap), each by an fsel pair as the original's code does
        let lim = |vc: f32, goal: f32| {
            let m = fsel(vc - goal, goal, vc);
            let a = fsel(goal, m, vc);
            let b = fsel(cap - a, a, cap);
            fsel(cap, b, a)
        };
        let last = (n - 1) as usize;
        let s = self.ln.s[last];
        self.ln.s[last].allowed = lim(s.v_curve, s.allowed);
        let mtds = self.tun.max_total_decel_scalar;
        let mbd = self.veh.t.max_brake_decel;
        let mbds = self.tun.max_brake_decel_scalar;
        let mut sum = 0.0f32;
        let mut j = last;
        while j as i32 > f {
            let s = self.ln.s[j];
            let p = self.ln.s[j - 1];
            let v = s.allowed;
            let vv = v * v;
            // lateral part of the available grip, the grip circle left for braking (a friction circle of radius n a_max)
            let alat = fma(s.kappa * v, v, s.g_ly);
            let nn0 = fnms(s.kv * self.inv_g * v, v, 1.0);
            let nn1 = fsel(nn0, nn0, 0.0);
            let nn = fsel(nn1 - 1.0, 1.0, nn1);
            let gl = nn * s.a_max * mtds;
            let lng = sqrt_nr(fsel(0.1 - fms(gl, gl, alat * alat), 0.1, fms(gl, gl, alat * alat)));
            let cap_dec = fms(mbd, mbds, s.g_ty);
            let dec = fsel(cap_dec - lng, lng, cap_dec);
            let vi = sqrt_nr(fsel(0.1 - fma(dec * s.ds, 2.0, vv), 0.1, fma(dec * s.ds, 2.0, vv)));
            let m = fsel(p.v_curve - vi, vi, p.v_curve);
            let m2 = fsel(m - p.allowed, p.allowed, m);
            let a = fsel(p.allowed, m2, m);
            let b = fsel(cap - a, a, cap);
            self.ln.s[j - 1].allowed = fsel(cap, b, a);
            sum += s.ds;
            j -= 1;
        }
        sum / (n - 1) as f32
    }

    /// F-39661: forward acceleration pass and the cost J.
    fn forward(&mut self, d: f32) {
        let (n, f) = (self.ln.n, self.ln.f);
        if n - f <= 1 || n as usize > NS {
            return;
        }
        // The original's registers are doubles that hold single values; each operation below is the
        // original's instruction (the comment names it), in its order, with its rounding.
        let r = |x: f64| x as f32 as f64; // frsp / the single-precision form of the instruction
        let fsel = |c: f64, a: f64, b: f64| if c >= 0.0 { a } else { b };
        let fma = |a: f64, b: f64, c: f64| r(a.mul_add(b, c)); // fmadds
        let fms = |a: f64, b: f64, c: f64| r(a.mul_add(b, -c)); // fmsubs
        let fnms = |a: f64, b: f64, c: f64| r(-a.mul_add(b, -c)); // fnmsubs
        let rsqrte = |x: f64| ((1.0 / x.sqrt()) as f32) as f64; // frsqrte
        let fres = |x: f64| ((1.0 / x) as f32) as f64; // fres
        let ctiw = |x: f64| -> i32 { if x.is_nan() { i32::MIN } else { x as i32 } }; // fctiwz
        let t = self.veh.t.clone();
        // constants of the image (1.0, 0.0, 126.1, +-2^23, 2.0, 0.1, 1e-6, 0.5)
        let (f12, f13) = (1.0f64, 0.0f64);
        let f9 = 126.1f32 as f64;
        let c_neg23 = -8388608.0f64;
        let c_pos23 = 8388608.0f64;
        let two = 2.0f64;
        let f0 = 0.1f32 as f64;
        let eps = 9.999999974752427e-07f64;
        let half = 0.5f64;
        let one_d = 1.0f64;
        let (mut f1, mut f2, mut f3, mut f4, mut f5, mut f6, mut f7, mut f8);
        let (mut f10, mut f11);
        let (mut f14, mut f15, mut f16, mut f17, mut f18, mut f19, mut f20, mut f21, mut f22, mut f23);
        let (mut f24, mut f25, mut f26, mut f27, mut f28, mut f29, mut f30, mut f31);
        let d_arg = d as f64;
        // prologue: the index of the first sample's speed in the acceleration table

        f4 = self.ln.s[f as usize].v as f64;
        f3 = r(f4 * self.veh.spd_k as f64);
        f2 = r(f12 / t.max_curvature as f64);
        let m208 = f2;
        f11 = self.t1 as f64;
        f10 = self.t2 as f64;
        f6 = f13;
        f8 = f13;
        let m192 = c_neg23;
        let m188 = c_pos23;
        f5 = -f3;
        f4 = fsel(f5, f13, f3);
        f3 = fsel(f4, f4, f13);
        f2 = r(f3 - f9);
        f5 = fsel(f2, f9, f3);
        f3 = fsel(f5, c_neg23, c_pos23);
        f2 = r(f5 - f3);
        let f0b = r(f2 + f3);
        f7 = r(f5 - f0b);
        f5 = r(f0b - f12);
        f3 = fsel(f7, f0b, f5);
        let mut r9 = ctiw(f3);
        f7 = r(f4 - f3);
        let mut ptr = (f + 1) as usize;
        let mut left = n - (f + 1);
        // keep the compiler's view of the registers the loop leaves behind
        f1 = d_arg;
        f14 = 0.0; f15 = 0.0; f16 = 0.0; f17 = 0.0; f18 = 0.0; f19 = 0.0; f20 = 0.0; f21 = 0.0;
        f22 = 0.0; f23 = 0.0; f24 = 0.0; f25 = 0.0; f26 = 0.0; f27 = 0.0; f28 = 0.0; f29 = 0.0;
        f30 = 0.0; f31 = 0.0;
        let _ = (f1, f14, f15, f16, f17, f18, f19, f20, f21, f22, f23, f24, f25, f26, f27, f28, f29, f30, f31, one_d);
        while left > 0 {
            let s = self.ln.s[ptr];
            let pv = self.ln.s[ptr - 1].v as f64;
            f31 = self.inv_g as f64;
            f2 = -f11;
            f3 = s.kv as f64;
            f30 = -f10;
            f3 = r(f3 * f31);
            f31 = pv;
            f29 = s.kappa as f64;
            f28 = r(f29 * f31);
            f23 = t.drag[1] as f64;
            f29 = f29.abs();
            f24 = t.drag[2] as f64;
            f14 = m208;
            f20 = self.b1[1] as f64;
            f26 = s.g_ly as f64;
            f22 = t.drag[0] as f64;
            f21 = r(f31 * f31);
            f2 = fsel(f2, f13, f11);
            f19 = self.b2[1] as f64;
            f30 = fsel(f30, f13, f10);
            f27 = self.tun.max_total_accel_scalar as f64;
            f3 = r(f3 * f31);
            // acceleration table at the index of the previous sample's speed
            f18 = t.accel[r9 as usize] as f64;
            f17 = t.accel[r9 as usize + 1] as f64;
            f17 = r(f17 - f18);
            f25 = s.a_max as f64;
            f24 = fma(f29, f24, f23);
            f16 = s.ds as f64;
            f29 = fms(f29, f14, f12);
            f15 = self.tun.collision_speed_penalty as f64;
            f28 = fma(f28, f31, f26);
            f26 = s.g_ty as f64;
            f23 = s.coll as f64;
            f1 = r(f16 - d_arg);
            f23 = fsel(f23, f15, f13);
            f15 = s.allowed as f64;
            f20 = r(f20 * f2);
            f14 = r(f30 * f30);
            f3 = fnms(f3, f31, f12);
            f18 = fma(f17, f7, f18);
            f7 = two;
            f31 = fma(f24, f31, f22);
            f28 = r(f28 * f28);
            f2 = r(f20 * f2);
            f30 = r(f14 * f30);
            f3 = fsel(f3, f3, f13);
            f24 = r(f0 - f18);
            f22 = -f31;
            f2 = fma(f30, f19, f2);
            f30 = r(f3 - f12);
            f24 = fsel(f24, f0, f18);
            f31 = fsel(f22, f13, f31);
            f3 = fsel(f30, f12, f3);
            f2 = r(f2 + f24);
            f30 = r(f31 * f31);
            f3 = r(f3 * f27);
            f2 = r(f2 + f26);
            f30 = r(f30 * f31);
            f3 = r(f3 * f25);
            f2 = fnms(f30, f31, f2);
            f3 = fms(f3, f3, f28);
            f31 = r(f0 - f3);
            f3 = fsel(f31, f0, f3);
            // square root: frsqrte, one refinement step, frsp
            f31 = rsqrte(f3);
            f30 = f31 * f31;
            f28 = f31 * half;
            f30 = -f30.mul_add(f3, -one_d);
            f31 = f30.mul_add(f28, f31);
            f3 = f31 * f3;
            f3 = r(f3);
            f31 = r(f2 - f3);
            f2 = fsel(f31, f3, f2);
            f3 = r(f2 * f16);
            f2 = fma(f3, f7, f21);
            f3 = r(f0 - f2);
            f2 = fsel(f3, f0, f2);
            f3 = rsqrte(f2);
            f1 = f1.abs();
            f30 = m188;
            f27 = f3 * f3;
            f31 = m192;
            f26 = f3 * half;
            f28 = -f29;
            f6 = r(f1 + f6);
            f1 = -f27.mul_add(f2, -one_d);
            f29 = fsel(f28, f13, f29);
            f3 = f1.mul_add(f26, f3);
            f2 = f3 * f2;
            f1 = r(f2);
            f3 = r(f1 - f15);
            f2 = fsel(f3, f15, f1);
            f1 = r(f2 - f23);
            f3 = r(f0 - f1);
            f2 = fsel(f3, f0, f1);
            self.ln.s[ptr].v = f2 as f32;
            let vnew = f2;
            f1 = self.veh.spd_k as f64;
            f3 = pv;
            f28 = self.ln.t as f64;
            f27 = self.ln.j as f64;
            f3 = r(f2 + f3);
            f2 = r(f2 * f1);
            self.counters[2] += 1;
            f1 = r(f0 - f3);
            f26 = -f2;
            f3 = fsel(f1, f0, f3);
            f2 = fsel(f26, f13, f2);
            f1 = fres(f3);
            f3 = fsel(f2, f2, f13);
            f1 = r(f1 * f16);
            f26 = r(f3 - f9);
            f7 = r(f1 * f7);
            f3 = fsel(f26, f9, f3);
            f1 = r(f28 + f7);
            self.ln.t = f1 as f32;
            f1 = s.w as f64;
            f1 = fma(f1, f7, f27);
            self.ln.j = f1 as f32;
            f31 = fsel(f3, f31, f30);
            f30 = vnew;
            f11 = r(f11 - f7);
            f28 = self.tun.max_curvature_penalty as f64;
            f1 = fma(f29, f28, f1);
            f29 = r(f3 - f31);
            self.ln.j = f1 as f32;
            f10 = r(f10 - f7);
            f8 = fma(f7, f30, f8);
            ptr += 1;
            left -= 1;
            f7 = r(f29 + f31);
            f3 = r(f3 - f7);
            f1 = r(f7 - f12);
            f7 = fsel(f3, f7, f1);
            r9 = ctiw(f7);
            f1 = d_arg;
            f7 = r(f2 - f7);
        }
        // epilogue: the average-speed term and the length-variance factor
        f3 = f13;
        let f13t = self.ln.t as f64;
        f11 = r(eps - f13t);
        f10 = fsel(f11, eps, f13t);
        let f13v = r(f8 / f10);
        if f13v > eps {
            f11 = self.tun.average_speed_weight as f64;
            f10 = r(f11 / f13v);
            f3 = self.ln.j as f64;
            f5 = (n - 1) as f64;
            f5 = r(f5);
            f4 = r(f10 * f5);
            f3 = fma(f4, f1, f3);
            self.ln.j = f3 as f32;
        }
        f11 = r(self.tun.length_variance_weight as f64 * f6);
        f10 = self.ln.j as f64;
        f6 = r((n - 2) as f64);
        f6 = r(f6 * f1);
        f5 = r(eps - f6);
        f4 = fsel(f5, eps, f6);
        f3 = r(f11 / f4);
        f2 = r(f3 + f12);
        f1 = r(f2 * f10);
        self.ln.j = f1 as f32;
    }

    /// F-41825: commanded curvature and acceleration at the next sample; turns H.
    fn command(&mut self, collision: bool) -> (f32, f32) {
        let (n, f) = (self.ln.n, self.ln.f);
        if n - f <= 1 || (f + 1) as usize >= NS {
            return (0.0, -1e8);
        }
        let r = self.ln.s[(f + 1) as usize];
        let fv = norm_or(Vec3::new(self.f.x, 0.0, self.f.z), 1e-5, Vec3::ZERO);
        let vv = norm_or(Vec3::new(self.vel.x, 0.0, self.vel.z), 1.0, Vec3::ZERO);
        let hv = norm_or(Vec3::new(self.h.x, 0.0, self.h.z), 1e-5, Vec3::ZERO);
        let speed = fma(self.vel.z, self.vel.z, fma(self.vel.x, self.vel.x, self.vel.y * self.vel.y)).sqrt();
        let vh = fma(self.h.z, self.vel.z, self.vel.x * self.h.x);
        let mut a1 = ang_xz_unit(fv, hv);
        let fdv = fma(vv.x, fv.x, fma(vv.z, fv.z, vv.y * fv.y));
        let a2 = ang_xz_unit(vv, hv) * if -fdv >= 0.0 { 0.0 } else { fdv };
        if f == 2 * self.tun.samples_per_segment - 1 && speed < 10.0 {
            let d = self.ln.pts.get(self.progress) - self.pos;
            let d = norm_or(Vec3::new(d.x, 0.0, d.z), 1e-5, Vec3::ZERO);
            a1 += ang_xz_unit(fv, d);
        }
        let mut kc = (a2 + a1) / r.ds + r.kappa;
        if !collision {
            kc += r.kappa - self.ang_vel.y / speed.max(5.0);
        }
        let ac = fms(r.allowed, r.allowed, self.v_fwd * self.v_fwd) / (r.ds.max(1.0) * 2.0);
        let lim = 1.0 / self.veh.steer_scalar(self.v_fwd);
        let kk = r.kappa.max(-lim).min(lim);
        // (made unit by the vector unit: vmsum3fp, vrsqrtefp with infinity to 0, one Newton step)
        let h = rot_y(self.h, kk * self.dt * vh);
        let d = dot3(h, h);
        let e = 1.0 / d.sqrt();
        let e = if e == f32::INFINITY { 0.0 } else { e };
        self.h = h * ((1.0 - d * (e * e)) * (0.5 * e) + e);
        (kc, ac)
    }

    /// F-82077 / F-96973 on this follower's line and state (the boost's c and duration).
    fn boost_benefit(&self, c: f32, dur: f32, cubic: bool) -> f32 {
        self.veh.boost_benefit(&self.ln.s, self.ln.f, self.ln.n, self.v_fwd, c, dur, self.inv_g, self.ln.t, cubic)
    }

    /// push point k by `dv` (F-70569's push), keeping it when F-63779 accepts it.
    fn push_point(&mut self, k: i32, dv: Vec3, dirty_bit: i32, other_side: bool) {
        let old = self.ln.pts.get(k);
        self.ln.pts.set(k, dv + old);
        let mut out = 0;
        if !self.push_ok(k, &mut out) {
            self.ln.pts.set(k, old);
            self.set_coll(k);
            self.last_push_ok = false;
        } else {
            self.set_port(k, out);
            self.set_dirty(dirty_bit);
            if other_side {
                self.set_coll(k);
            }
            self.last_push_ok = true;
        }
    }
}

/// F-68053's side planes: the edge p -> q, from its midpoint (a short edge keeps midpoint - c unprojected, as plane_n does).
fn side_n(p: Vec3, q: Vec3, c: Vec3) -> Vec3 {
    plane_n((p + q) * 0.5 - c, q - p)
}

/// F-96281 with the Catmull-Rom basis at 0x828386B0 (set up in F-86861).
fn cr(w: &[Vec3; 4]) -> [Vec3; 4] {
    [
        w[0] * -0.5 + w[1] * 1.5 + w[2] * -1.5 + w[3] * 0.5,
        w[0] * 1.0 + w[1] * -2.5 + w[2] * 2.0 + w[3] * -0.5,
        w[0] * -0.5 + w[2] * 0.5,
        w[1],
    ]
}

/// F-28541: steer, throttle, brake from the commanded curvature and acceleration.
#[allow(clippy::too_many_arguments)]
fn pedal_map(veh: &Veh, kappa: f32, a_cmd: f32, f3: f32, t1: f32, c1: f32, t2: f32, c2: f32, g: f32, gear: i32, vel: Vec3, fwd: Vec3, right: Vec3) -> (f32, f32, f32) {
    let vf = fma(fwd.x, vel.x, fma(vel.z, fwd.z, vel.y * fwd.y));
    let ss = veh.steer_scalar(vf);
    let steer = fsel(-(ss * kappa) - -1.0, -(ss * kappa), -1.0);
    let steer = fsel(steer - 1.0, 1.0, steer);
    let mut ae = veh.accel(gear, vf);
    let ad = veh.decel(gear, vf);
    let t = &veh.t;
    let keff = (steer / ss).abs();
    let drag = fma(fma(keff, t.drag[2], t.drag[1]), vf, t.drag[0]);
    let drag = fsel(-drag, 0.0, drag);
    let areq = fnms(g, fwd.y, fma(drag * drag * drag, drag, a_cmd));
    let (b1, b2) = (fsel(-t1, 0.0, t1), fsel(-t2, 0.0, t2));
    let ab = fma(b2 * b2 * b2, c2, b1 * b1 * c1);
    if f3 != 0.0 && vf > 1.0 {
        // (made unit by the vector unit, as in F-41825)
        let d = dot3(vel, vel);
        let e = 1.0 / d.sqrt();
        let e = if e == f32::INFINITY { 0.0 } else { e };
        let vh = vel * ((1.0 - d * (e * e)) * (0.5 * e) + e);
        let x = fma(right.x, vh.x, fma(right.y, vh.y, right.z * vh.z)).abs();
        let x = fsel(x, x, 0.0);
        let x = fsel(x - 1.0, 1.0, x);
        let s = sin_ramp(x);
        ae *= fma(fsel(s - steer.abs(), s, steer.abs()), f3, 1.0);
    }
    let (thr, brk) = if !(areq < ab * 0.5) {
        ((ae + areq) / fma(ae, 2.0, ab), 0.0)
    } else if !(areq < -ad) {
        ((ad + areq) / fma(ad, 2.0, ab), 0.0)
    } else {
        (0.0, -((ad + areq) / t.max_brake_decel))
    };
    let c = |x: f32| {
        let x = fsel(x, x, 0.0);
        fsel(x - 1.0, 1.0, x)
    };
    (steer, c(thr), c(brk))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field the follower's per-frame update writes, as bit patterns, in the original's order of the aiv bytes.
    fn follow_fields(a: &Aiv) -> Vec<(String, u32)> {
        let mut o: Vec<(String, u32)> = Vec::new();
        let mut v = |n: &str, x: Vec3| {
            for (q, c) in [x.x, x.y, x.z].into_iter().enumerate() {
                o.push((format!("{n}.{q}"), c.to_bits()));
            }
        };
        v("pos", a.pos);
        v("h", a.h);
        v("f", a.f);
        v("vel", a.vel);
        v("ang_vel", a.ang_vel);
        v("right", a.right);
        v("last_reached", a.last_reached);
        let mut s = |n: &str, x: u32| o.push((n.to_string(), x));
        s("v_fwd", a.v_fwd.to_bits());
        s("cap", a.cap.to_bits());
        s("dt", a.dt.to_bits());
        s("t1", a.t1.to_bits());
        s("t2", a.t2.to_bits());
        s("progress", a.progress as u32);
        s("cursor", a.cursor as u32);
        s("cached", a.cached as u32);
        s("next_goal", a.next_goal as u32);
        for (n, (st, ct, bs)) in [
            ("portals", (a.portals.start, a.portals.count, a.portals.base)),
            ("goals", (a.goals.start, a.goals.count, a.goals.base)),
            ("pts", (a.ln.pts.start, a.ln.pts.count, a.ln.pts.base)),
            ("pp", (a.ln.pp.start, a.ln.pp.count, a.ln.pp.base)),
            ("pg", (a.pg.start, a.pg.count, a.pg.base)),
        ] {
            s(&format!("{n}.start"), st as u32);
            s(&format!("{n}.count"), ct as u32);
            s(&format!("{n}.base"), bs as u32);
        }
        for i in a.ln.pts.base..a.ln.pts.end() {
            let p = a.ln.pts.get(i);
            for (q, c) in [p.x, p.y, p.z].into_iter().enumerate() {
                o.push((format!("pt[{i}].{q}"), c.to_bits()));
            }
            o.push((format!("pp[{i}]"), a.ln.pp.get(i) as u32));
            o.push((format!("pg[{i}]"), a.pg.get(i) as u32));
        }
        o
    }

    /// The follower's per-frame update F-12813 (with F-85983 build_line, F-84385 add_leg, F-51411 furthest_visible,
    /// F-42352 visible, the cell walk F-84189 / surface height F-84036 and F-77899) run offline on its generated code
    /// (work/racerai/follow: follow.py, gen_in.py, run.py) on the 201 recorded R8 aiv states of the 1600 m bend, each with the car of the
    /// next recorded frame, on 1,010 varied states (car moved along / off the line, slow, reversed, other caps / boost timers) and on
    /// 603 recorded states with the point ring cut short (so the line is laid anew): every field it writes, bit for bit
    /// (docs/reference/ai/verify_follower.md).
    /// F-82077 / F-96973 against the original run offline (tools/boost_benefit_reference.py): 8,000 random lines (0 .. 66
    /// samples), the R8's tune with the drag, speed index and acceleration row varied, speeds at and around the 0.1 floor,
    /// zero durations, line times at and around the runs' times; both answers equal bit for bit.
    #[test]
    fn boost_benefit_matches_the_original() {
        let Ok(tb) = std::fs::read("D:/MCLA/work/ai-verify/scalar/tune_r8.bin") else {
            println!("tune_r8.bin missing: skipped");
            return;
        };
        let base: Vec<f32> = tb.chunks_exact(4).map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
        let text = include_str!("testdata/boost_benefit_ref.txt");
        let hex = |w: &str| u32::from_str_radix(&w.replace('_', ""), 16).unwrap();
        let mut tabs: Vec<Vec<f32>> = Vec::new();
        let (mut ran, mut bad, mut mid) = (0, 0, 0);
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            if w[0] == "table" {
                tabs.push(w[2..].iter().map(|x| f32::from_bits(hex(x))).collect());
                continue;
            }
            let arrow = w.iter().position(|x| *x == "->").unwrap();
            let v: Vec<u32> = w[1..arrow].iter().map(|x| hex(x)).collect();
            let want = [hex(w[arrow + 1]), hex(w[arrow + 2])];
            let fv = |i: usize| f32::from_bits(v[i]);
            let mut fl = base.clone();
            fl[19..147].copy_from_slice(&tabs[v[0] as usize]);
            fl[5] = fv(1);
            fl[6] = fv(2);
            fl[7] = fv(3);
            let mut veh = Veh::new(Arc::new(AiTune::from_floats(&fl).unwrap()));
            veh.spd_k = fv(4);
            let (n, f) = (v[11] as i32, v[12] as i32);
            let mut ss = [Sample::default(); NS];
            for j in 0..(n - f - 1).max(0) as usize {
                let q = &v[14 + 7 * j..14 + 7 * j + 7];
                let g = |i: usize| f32::from_bits(q[i]);
                ss[(f + 1) as usize + j] = Sample { allowed: g(0), kv: g(1), kappa: g(2), g_ty: g(3), g_ly: g(4), a_max: g(5), ds: g(6), ..Default::default() };
            }
            let got = [
                veh.boost_benefit(&ss, f, n, fv(5), fv(6), fv(7), fv(10), fv(13), false).to_bits(),
                veh.boost_benefit(&ss, f, n, fv(5), fv(8), fv(9), fv(10), fv(13), true).to_bits(),
            ];
            ran += 1;
            mid += want.iter().filter(|&&x| x != 0 && x != 0x3F80_0000).count();
            if got != want {
                if bad < 5 {
                    println!("{line}{}  port {:08x} {:08x}", char::from(10), got[0], got[1]);
                }
                bad += 1;
            }
        }
        println!("boost benefit: {ran} cases, {mid} answers strictly inside 0 .. 1, {bad} differ");
        assert!(ran >= 8000);
        assert_eq!(bad, 0);
    }

    /// F-70624 (one obstacle appended) run offline whole with its normalise F-41334: 5,000 cases
    /// (tools/obstacle_add_reference.py, testdata/obstacle_add_ref.txt; directions down to below the 1e-5 fallback and
    /// zero, the two bools, any old flag byte). The stored position, velocity, direction (its y set to 0, normalised),
    /// half extent (lanes 0 and 2) and the flag byte's two top bits equal bit for bit. (The byte's low 6 bits are the
    /// slot's old ones, which only this writes and nothing reads; the port keeps them 0.)
    /// F-80559 run offline (tools/aiv_reach_reference.py, seed 80559, 6,000 cases: corridors of 2 .. 12 portals with bends,
    /// reversed, degenerate and random ones, the ring's start 0 .. 127 so it wraps, b above, below and equal to a, x and p near
    /// their cells or anywhere; 3,345 true): `reach` equals the original on every case. On the same ring the corridor point
    /// ahead F-43876 (904 cases on the two-portal mean): `corridor_ahead` equal to the bit.
    #[test]
    fn reach_matches_the_original() {
        let text = include_str!("testdata/aiv_reach_ref.txt");
        let tune = Arc::new(mcla_formats::rage::aitune::AiTune::from_floats(&vec![0.0; 4096]).unwrap());
        let (mut n, mut bad, mut bad_c) = (0, 0, 0);
        for line in text.lines().filter(|l| !l.is_empty()) {
            let w: Vec<&str> = line.split_whitespace().collect();
            let int = |i: usize| w[i].parse::<i32>().unwrap();
            let f = |i: usize| f32::from_bits(u32::from_str_radix(&w[i].replace('_', ""), 16).unwrap());
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let (start, base, k, ca, cb) = (int(1), int(2), int(3), int(4), int(5));
            let mut a = Aiv::new(tune.clone());
            a.portals.start = start;
            a.portals.base = base;
            a.portals.count = k;
            for j in 0..k {
                let at = 12 + 12 * j as usize;
                a.portals.set(base + j, Cell { sa: v(at), sb: v(at + 3), a: v(at + 6), b: v(at + 9), ..Cell::default() });
            }
            let r = w.len() - 4;
            n += 1;
            if a.reach(base + ca, v(6), base + cb, v(9)) != (w[r] == "1") {
                bad += 1;
            }
            let c = a.corridor_ahead();
            if [c.x, c.y, c.z].map(f32::to_bits) != [f(r + 1), f(r + 2), f(r + 3)].map(f32::to_bits) {
                bad_c += 1;
            }
        }
        assert!(n == 6000 && bad == 0 && bad_c == 0, "reach {bad} / corridor {bad_c} of {n} differ");
    }

    /// F-13872 run offline (tools/aiv_off_line_reference.py, seed 13872, 6,000 cases: corridors of 2 .. 10 portals with
    /// their planes, 1 .. 10 control points, the car near the line, about 0.25 m off it, 5 m below it or anywhere, the line
    /// used up in some): `off_line`'s test and its clamped position equal the original's on every case (the position only
    /// where the original writes it: the line not used up, the out pointer given); the getters line used up F-65527 and
    /// next point F-28749, run on the same aiv, equal too.
    #[test]
    fn off_line_matches_the_original() {
        let text = include_str!("testdata/aiv_off_line_ref.txt");
        let tune = Arc::new(mcla_formats::rage::aitune::AiTune::from_floats(&vec![0.0; 4096]).unwrap());
        let (mut n, mut bad, mut bad_x, mut bad_g) = (0, 0, 0, 0);
        for line in text.lines().filter(|l| !l.is_empty()) {
            let w: Vec<&str> = line.split_whitespace().collect();
            let int = |i: usize| w[i].parse::<i32>().unwrap();
            let f = |i: usize| f32::from_bits(u32::from_str_radix(&w[i].replace('_', ""), 16).unwrap());
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let (ps, pb, k, qs, qb, m, ws, prog, out) = (int(1), int(2), int(3), int(4), int(5), int(6), int(7), int(8), int(9) != 0);
            let mut a = Aiv::new(tune.clone());
            a.pos = v(10);
            (a.portals.start, a.portals.count, a.portals.base) = (ps, k, pb);
            let mut at = 13;
            for j in 0..k {
                let c = Cell { a: v(at), b: v(at + 3), sa: v(at + 6), sb: v(at + 9), n: v(at + 12), fwd: v(at + 15), back: v(at + 18), side_a: v(at + 21), side_b: v(at + 24), sside_a: v(at + 27), sside_b: v(at + 30), ..Cell::default() };
                a.portals.set(pb + j, c);
                at += 33;
            }
            (a.ln.pts.start, a.ln.pts.count, a.ln.pts.base) = (qs, m, qb);
            for i in 0..m {
                a.ln.pts.set(qb + i, v(at));
                at += 3;
            }
            (a.ln.pp.start, a.ln.pp.count, a.ln.pp.base) = (ws, m, qb);
            for i in 0..m {
                a.ln.pp.set(qb + i, int(at));
                at += 1;
            }
            a.progress = prog;
            assert_eq!(w[at], "->");
            let (got, x) = a.off_line();
            n += 1;
            if got != (w[at + 1] == "1") {
                bad += 1;
            }
            if out && prog < qb + m && [x.x, x.y, x.z].map(f32::to_bits) != [f(at + 2), f(at + 3), f(at + 4)].map(f32::to_bits) {
                bad_x += 1;
            }
            // (the getters F-65527 line used up, F-28749 next point)
            let np = a.next_point();
            if a.line_used_up() != (w[at + 5] == "1") || [np.x, np.y, np.z].map(f32::to_bits) != [f(at + 6), f(at + 7), f(at + 8)].map(f32::to_bits) {
                bad_g += 1;
            }
        }
        assert!(n == 6000 && bad == 0 && bad_x == 0 && bad_g == 0, "test {bad} / position {bad_x} / getters {bad_g} of {n} differ");
    }

    #[test]
    fn obstacle_add_matches_the_original() {
        let text = include_str!("testdata/obstacle_add_ref.txt");
        let tune = Arc::new(mcla_formats::rage::aitune::AiTune::from_floats(&vec![0.0; 4096]).unwrap());
        let mut n = 0;
        for line in text.lines() {
            let w: Vec<u32> = line.split_whitespace().filter(|x| *x != "case" && *x != "->").map(|x| u32::from_str_radix(&x.replace('_', ""), 16).unwrap()).collect();
            assert_eq!(w.len(), 39);
            let f = |i: usize| f32::from_bits(w[i]);
            let v = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let o = Obstacle { pos: v(0), vel: v(4), dir: v(8), extent: (f(12), f(14)), flags: ((w[16] as u8) << 7) | ((w[17] as u8) << 6) };
            let mut a = Aiv::new(tune.clone());
            a.set_obstacles(&[o]);
            let g = a.obst[0];
            let r = &w[20..];
            let got = [g.pos.x, g.pos.y, g.pos.z, g.vel.x, g.vel.y, g.vel.z, g.dir.x, g.dir.y, g.dir.z, g.extent.0, g.extent.1].map(|x| x.to_bits());
            let want = [r[0], r[1], r[2], r[4], r[5], r[6], r[8], r[9], r[10], r[12], r[14]];
            assert_eq!(got, want, "case {n}: {line}");
            assert_eq!(g.flags, r[16] as u8 & 0xC0, "case {n} flags");
            assert_eq!((r[17], r[18]), (w[18] + 1, w[18] + 1), "case {n} counts");
            assert_eq!(a.n_obst, 1);
            n += 1;
        }
        assert_eq!(n, 5000);
    }

    /// The aiv's car values in a race (+25168 size, +25292 / +25308 the tunes, +25324 g, +25328 1 / g) as the live R8
    /// race holds them (work/racerai/follow/in_rec.bin, 201 states): the aiv init's, the enable F-92836's values put
    /// back by BrainRacing_Reset F-69438; `car_defaults` and a fresh aiv equal them bit for bit, F-12813 keeps them.
    #[test]
    fn race_car_values_are_the_recorded_ones() {
        let tune = Arc::new(mcla_formats::rage::aitune::AiTune::from_floats(&vec![0.0; 4096]).unwrap());
        let bits = |v: (Vec3, [f32; 3], [f32; 3], f32, f32)| {
            let mut w: Vec<u32> = v.0.to_array().iter().chain(&v.1).chain(&v.2).map(|x| x.to_bits()).collect();
            w.extend([v.3.to_bits(), v.4.to_bits()]);
            w
        };
        let fresh = Aiv::new(tune.clone());
        assert_eq!(fresh.inv_g.to_bits(), 0xbdd0_fac6);
        assert_eq!(fresh.g.to_bits(), 0xc11c_cccd);
        let mut a = Aiv::new(tune.clone());
        let car = AivCar {
            pos: Vec3::ZERO, right: Vec3::X, forward: Vec3::Z, vel: Vec3::ZERO, ang_vel: Vec3::ZERO, nitro_tune: [1.0; 3], nitro_t: 0.0,
            sling_tune: [3.0; 3], sling_t: 0.0, speed_cap: -1.0, size: Vec3::splat(7.0), gravity: -13.72,
        };
        a.update_state(&car, 1.0 / 60.0);
        assert_eq!(bits(a.car_values()), bits(fresh.car_values()), "F-12813 keeps the car values");
        a.activate(&car);
        assert_eq!(a.size(), Vec3::splat(7.0));
        assert_eq!(a.inv_g, 1.0 / -13.72f32);
        a.car_defaults();
        assert_eq!(bits(a.car_values()), bits(fresh.car_values()));
        const BS: usize = 31232;
        let Ok(inp) = std::fs::read("D:/MCLA/work/racerai/follow/in_rec.bin") else {
            println!("recording missing: skipped");
            return;
        };
        let n = inp.len() / (BS + 96);
        for k in 0..n {
            let mut r = Aiv::new(tune.clone());
            r.import_original(&inp[k * (BS + 96)..k * (BS + 96) + BS]);
            assert_eq!(bits(r.car_values()), bits(fresh.car_values()), "recorded state {k}");
        }
        println!("{n} recorded states hold the defaults");
        assert!(n >= 200);
    }

    #[test]
    fn follower_update_is_the_originals() {
        let dir = "D:/MCLA/work/racerai/follow";
        let tb = std::fs::read("D:/MCLA/work/ai-verify/scalar/tune_r8.bin");
        let Ok(tb) = tb else {
            println!("follower fixtures missing: skipped");
            return;
        };
        let floats: Vec<f32> = tb.chunks_exact(4).map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
        let tune = Arc::new(mcla_formats::rage::aitune::AiTune::from_floats(&floats).unwrap());
        const BS: usize = 31232;
        let mut fails = 0;
        let mut ran = 0;
        for set in ["rec", "var", "trim"] {
            let (Ok(inp), Ok(out)) = (std::fs::read(format!("{dir}/in_{set}.bin")), std::fs::read(format!("{dir}/out_{set}.bin"))) else {
                println!("{set}: fixtures missing: skipped");
                continue;
            };
            let n = inp.len() / (BS + 96);
            assert_eq!(out.len(), n * BS);
            let (mut bad, mut first) = (0, String::new());
            let mut per: std::collections::BTreeMap<String, usize> = Default::default();
            for k in 0..n {
                let r = &inp[k * (BS + 96)..(k + 1) * (BS + 96)];
                let g = |i: usize| f32::from_le_bytes(r[BS + 4 * i..BS + 4 * i + 4].try_into().unwrap());
                let v = |i: usize| Vec3::new(g(i), g(i + 1), g(i + 2));
                let mut a = Aiv::new(tune.clone());
                a.import_original(&r[..BS]);
                a.planes();
                // the harness hands the original the matrix row 2 = 0 - forward and stores 0 - row 2 (signed zeros as the original's)
                let fw = v(4);
                let fwd = Vec3::new(0.0 - (0.0 - fw.x), 0.0 - (0.0 - fw.y), 0.0 - (0.0 - fw.z));
                let car = AivCar {
                    pos: v(8), right: v(0), forward: fwd, vel: v(12), ang_vel: v(16), nitro_tune: a.b1, nitro_t: g(20),
                    sling_tune: a.b2, sling_t: g(21), speed_cap: g(22), size: a.size, gravity: a.g,
                };
                a.update_state(&car, g(23));
                let mut w = Aiv::new(tune.clone());
                w.import_original(&out[k * BS..(k + 1) * BS]);
                let (pf, wf) = (follow_fields(&a), follow_fields(&w));
                if std::env::var("AIV_FOLLOW_CASE").ok() == Some(format!("{set}{k}")) {
                    for (x, y) in pf.iter().zip(&wf) {
                        if x != y {
                            println!("  {} port {:08x} {} / original {} {:08x}", x.0, x.1, f32::from_bits(x.1), y.0, y.1);
                        }
                    }
                }
                if pf != wf {
                    bad += 1;
                    let names_p: Vec<&String> = pf.iter().map(|x| &x.0).collect();
                    let names_w: Vec<&String> = wf.iter().map(|x| &x.0).collect();
                    if names_p != names_w {
                        *per.entry("(point count)".into()).or_default() += 1;
                        if first.is_empty() {
                            first = format!("case {k}: points port {} original {}", a.ln.pts.count, w.ln.pts.count);
                        }
                    } else {
                        for (x, y) in pf.iter().zip(&wf) {
                            if x.1 != y.1 {
                                let key = x.0.split(['[', '.']).next().unwrap().to_string();
                                *per.entry(key).or_default() += 1;
                                if first.is_empty() {
                                    first = format!("case {k}: {} port {:08x} original {:08x}", x.0, x.1, y.1);
                                }
                            }
                        }
                    }
                }
            }
            println!("{set}: {n} cases, {bad} differ; fields {per:?}; first {first}");
            fails += bad;
            ran += n;
        }
        assert!(ran == 0 || fails == 0);
    }

    /// The six cell planes F-68053 builds per portal pair (+80 .. +176 of each 176-byte portal: forward, back, side a / b, shrunk
    /// side a / b), its generated code run offline (work/racerai/follow/planes.py) on the 201 recorded R8 aivs and the 1,010 varied
    /// states with the planes-built index (+30832) reset to 0: `planes()` from the same state gives every plane's xyz and the
    /// index bit for bit (docs/reference/ai/verify_follower.md section 6).
    #[test]
    fn cell_planes_are_the_originals() {
        let dir = "D:/MCLA/work/racerai/follow";
        let tune = tune();
        let mut ran = 0;
        for (set, inp, stride) in [("rec", format!("{dir}/in_rec.bin"), 31232 + 96), ("var", "D:/MCLA/work/ai-verify/scalar/chain_in_var.bin".to_string(), 31232)] {
            let (Ok(inp), Ok(out)) = (std::fs::read(&inp), std::fs::read(format!("{dir}/planes_{set}.bin"))) else {
                println!("{set}: fixtures missing: skipped");
                continue;
            };
            let n = inp.len() / stride;
            assert_eq!(out.len(), n * 31232);
            let (mut cells, mut bad, mut first) = (0, 0, String::new());
            for k in 0..n {
                let mut a = Aiv::new(tune.clone());
                a.import_original(&inp[k * stride..k * stride + 31232]);
                a.planes_upto = 0;
                a.planes();
                let mut w = Aiv::new(tune.clone());
                w.import_original(&out[k * 31232..(k + 1) * 31232]);
                if a.planes_upto != w.planes_upto {
                    bad += 1;
                    if first.is_empty() {
                        first = format!("case {k}: planes built to {} / {}", a.planes_upto, w.planes_upto);
                    }
                }
                for i in a.portals.base..a.portals.end() - 1 {
                    let (c, d) = (a.portals.get(i), w.portals.get(i));
                    cells += 1;
                    let pc = [c.fwd, c.back, c.side_a, c.side_b, c.sside_a, c.sside_b];
                    let pd = [d.fwd, d.back, d.side_a, d.side_b, d.sside_a, d.sside_b];
                    for j in 0..6 {
                        if pc[j].to_array().map(f32::to_bits) != pd[j].to_array().map(f32::to_bits) {
                            bad += 1;
                            if first.is_empty() {
                                first = format!("case {k} portal {i} plane {j}: port {:?} original {:?}", pc[j], pd[j]);
                            }
                        }
                    }
                }
            }
            println!("{set}: {n} states, {cells} cells x 6 planes, {bad} differ {first}");
            assert_eq!(bad, 0);
            ran += cells;
        }
        let _ = ran;
    }

    /// F-29767 (push a goal) run offline (work/racerai/follow/goal.py) on the 201 recorded R8 aivs and the 1,010 variants with
    /// random goals (15 % with the point rings emptied, 7 % with the portal end below 2): the goal direction, J / T / previous J,
    /// the goal ring, the point / point-portal / point-goal rings, the last point reached, the planes-built index and every
    /// portal's planes equal `push_goal`'s (docs/reference/ai/verify_follower.md section 11).
    #[test]
    fn push_goal_is_the_originals() {
        let dir = "D:/MCLA/work/racerai/follow";
        let tune = tune();
        const BS: usize = 31232;
        for set in ["rec", "var"] {
            let Ok(b) = std::fs::read(format!("{dir}/goal_{set}.bin")) else {
                println!("{set}: fixtures missing: skipped");
                continue;
            };
            let stride = 2 * BS + 64;
            assert_eq!(b.len() % stride, 0);
            let n = b.len() / stride;
            let (mut bad, mut first, mut modes) = (0, String::new(), [0; 3]);
            for k in 0..n {
                let r = &b[k * stride..(k + 1) * stride];
                let w = |q: usize| u32::from_be_bytes(r[2 * BS + 4 * q..2 * BS + 4 * q + 4].try_into().unwrap());
                let f = |q: usize| f32::from_bits(w(q));
                let mut a = Aiv::new(tune.clone());
                a.import_original(&r[..BS]);
                a.push_goal(Vec3::new(f(0), f(1), f(2)), Vec3::new(f(3), f(4), f(5)), f(6), Vec3::new(f(7), f(8), f(9)), f(10), w(11));
                modes[w(12) as usize] += 1;
                let mut o = Aiv::new(tune.clone());
                o.import_original(&r[BS..2 * BS]);
                let pa = format!("{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}", a.goal_dir, a.j_prev, a.ln.j, a.ln.t, a.goals, a.ln.pts, a.ln.pp, a.pg, a.last_reached, a.planes_upto, a.portals);
                let po = format!("{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}", o.goal_dir, o.j_prev, o.ln.j, o.ln.t, o.goals, o.ln.pts, o.ln.pp, o.pg, o.last_reached, o.planes_upto, o.portals);
                if pa != po {
                    bad += 1;
                    if first.is_empty() {
                        let i = pa.bytes().zip(po.bytes()).position(|(x, y)| x != y).unwrap_or(0);
                        first = format!("case {k} mode {}: port ..{}.. original ..{}..", w(12), &pa[i.saturating_sub(60)..(i + 60).min(pa.len())], &po[i.saturating_sub(60)..(i + 60).min(po.len())]);
                    }
                }
            }
            println!("{set}: {n} cases (modes {modes:?}), {bad} differ {first}");
            assert_eq!(bad, 0);
        }
    }

    /// F-82126 (drop the last portal) run offline (work/racerai/follow/pop.py) on the 201 recorded R8 aivs and the 1,010 variants,
    /// the planes-built index set to the portal end + 4 first: the ring's start / count / base and the built index equal
    /// `pop_portal`'s, and the original's own tail words (portals +22544, flag bytes +22704 / +22712) stay the ring's start + count
    /// (docs/reference/ai/verify_follower.md section 8).
    #[test]
    fn pop_portal_is_the_originals() {
        let dir = "D:/MCLA/work/racerai/follow";
        let tune = tune();
        for (set, inp, stride) in [("rec", format!("{dir}/in_rec.bin"), 31232 + 96), ("var", "D:/MCLA/work/ai-verify/scalar/chain_in_var.bin".to_string(), 31232)] {
            let (Ok(inp), Ok(out)) = (std::fs::read(&inp), std::fs::read(format!("{dir}/pop_{set}.bin"))) else {
                println!("{set}: fixtures missing: skipped");
                continue;
            };
            let n = inp.len() / stride;
            assert_eq!(out.len(), n * 31232);
            let mut bad = 0;
            for k in 0..n {
                let mut a = Aiv::new(tune.clone());
                a.import_original(&inp[k * stride..k * stride + 31232]);
                a.planes_upto = a.portals.end() + 4;
                a.pop_portal();
                let o = &out[k * 31232..(k + 1) * 31232];
                let i = |off: usize| i32::from_be_bytes(o[off..off + 4].try_into().unwrap());
                let mut w = Aiv::new(tune.clone());
                w.import_original(o);
                let same = (a.portals.start, a.portals.count, a.portals.base, a.planes_upto) == (w.portals.start, w.portals.count, w.portals.base, w.planes_upto);
                let tails = i(22544) == (i(22548) + i(22552)).rem_euclid(128) && i(22704) == (i(22708) + i(22712)).rem_euclid(128) && i(22712) == i(22552);
                if !(same && tails) {
                    bad += 1;
                }
            }
            println!("{set}: {n} cases, {bad} differ");
            assert_eq!(bad, 0);
        }
    }

    /// The aivRaceLineTuning (F-46101's defaults, aiv +25332, 108 bytes) as the running original holds it in a race: the R8's aiv
    /// in all 202 recorded snapshots of work/ai-record/fixture/r7_r8_run0.bin (one block, these words; RestartEveryUpdate's byte 0,
    /// the rest of its word padding). `AivTuning::default` equals it field by field (docs/reference/ai/verify_aitune.md).
    #[test]
    fn race_line_tuning_is_the_recorded() {
        let t = AivTuning::default();
        let rec: [u32; 27] = [
            0x14, 0, 1, 0x3f000000, 0x41c80000, 3, 0x3e4ccccd, 0x3dcccccd, 0x40a00000, 0x3f800000, 0x41200000, 0x3dcccccd, 0x3f400000, 0x3dcccccd,
            0x3f800000, 0x3dcac083, 0x3f800000, 0x40c00000, 0x3f800000, 0x3f800000, 0x3f400000, 0x3f800000, 0x3f000000, 0x40a00000, 0x41c80000,
            0x42c80000, 0x40d68f5c,
        ];
        let f = [
            t.recursive_falloff, t.initial_segment_length, t.length_variance_weight, t.off_road_weight, t.max_curvature_penalty, t.linear_adjustment,
            t.angular_adjustment, t.angular_adjustment_threshold, t.angular_adjustment_falloff, t.adjustment_falloff, t.adjustment_falloff_trigger,
            t.adjustment_reset_threshold, t.buffer_zone, t.obstacle_lookahead_time, t.max_speed_scalar, t.max_total_accel_scalar,
            t.max_total_decel_scalar, t.max_brake_decel_scalar, t.average_speed_weight, t.max_obstacle_height_delta, t.max_obstacle_flat_delta,
            t.max_obstacle_prediction_dist, t.collision_speed_penalty,
        ];
        let got: Vec<u32> = [t.max_steps_ahead as u32, t.restart_every_update as u32, t.max_steps_to_recurse as u32]
            .into_iter()
            .chain(f[..2].iter().map(|x| x.to_bits()))
            .chain([t.samples_per_segment as u32])
            .chain(f[2..].iter().map(|x| x.to_bits()))
            .collect();
        assert_eq!(got, rec);
    }

    /// A plain car: tables flat so the decoded rules can be checked by hand.
    fn tune() -> Arc<AiTune> {
        let row = |v: f32| vec![v; 128];
        // speed tables against curvature: v = sqrt(8 / k) capped at 60 (8 m/s^2 lateral)
        let maxc = 0.2f32;
        let knee = maxc * 0.1;
        let low: Vec<f32> = (0..64).map(|i| { let k = i as f32 * knee / 64.0; if k > 0.0 { (8.0 / k).sqrt().min(60.0) } else { 60.0 } }).collect();
        let high: Vec<f32> = (0..64).map(|i| { let k = knee + i as f32 * (maxc - knee) / 63.0; (8.0 / k).sqrt().min(60.0) }).collect();
        Arc::new(AiTune {
            max_speed: 60.0,
            min_curvature: 0.005,
            max_curvature: maxc,
            max_total_accel: 30.0,
            max_brake_decel: 20.0,
            idle_speed: [0.0; 7],
            accel: row(6.0),
            decel: (0..7).map(|_| row(3.0)).collect(),
            speed_high_curv: high,
            speed_low_curv: low,
            steer_scalar: row(4.0),
            drag: [0.0; 3],
            accel_rows: (0..8).map(|_| row(6.0)).collect(),
        })
    }

    fn car(pos: Vec3, fwd: Vec3, speed: f32) -> AivCar {
        AivCar {
            pos,
            right: Vec3::new(-fwd.z, 0.0, fwd.x),
            forward: fwd,
            vel: fwd * speed,
            ang_vel: Vec3::ZERO,
            nitro_tune: [2.0, 12.0, 2.5],
            nitro_t: 0.0,
            sling_tune: [2.0, 25.0, 2.5],
            sling_t: 0.0,
            speed_cap: -1.0,
            size: Vec3::new(2.0, 1.5, 5.0),
            gravity: -9.8,
        }
    }

    fn portal(a: Vec3, b: Vec3) -> Portal {
        Portal { a, b, normal: Vec3::Y, offroad: false, ext: (0.0, 0.0) }
    }

    /// Run the follower for a few frames so every leg gets built.
    fn settle(aiv: &mut Aiv, c: &AivCar, frames: usize) {
        for _ in 0..frames {
            aiv.update(c, 1.0 / 60.0);
        }
    }

    /// (GitHub #48) the frame's steps worked out ahead on another thread are the steps run in place, bit for bit, and are
    /// taken only from the very state and inputs they were worked out from.
    #[test]
    fn steps_worked_out_ahead_are_the_steps_in_place() {
        let state = |a: &Aiv| {
            let mut c = a.clone();
            c.epoch = 0;
            format!("{c:?}")
        };
        let mut aiv = Aiv::new(tune());
        let c = car(Vec3::ZERO, Vec3::Z, 12.0);
        aiv.update_state(&c, 1.0 / 60.0);
        for i in 0..61 {
            let z = i as f32 * 10.0;
            let x = (z * 0.02).sin() * 8.0;
            aiv.push_portal(&portal(Vec3::new(x + 6.0, 0.0, z), Vec3::new(x - 6.0, 0.0, z)));
        }
        aiv.push_goal(Vec3::Z, Vec3::new(0.0, 0.0, 600.0), 5.0, Vec3::ZERO, -1.0, 0);
        settle(&mut aiv, &c, 20);
        let obst = [Obstacle { pos: Vec3::new(1.0, 0.0, 30.0), vel: Vec3::new(0.0, 0.0, 6.0), dir: Vec3::Z, extent: (2.0, 4.5), flags: 0 }];
        let c2 = car(Vec3::new(0.2, 0.0, 3.0), Vec3::new(0.05, 0.0, 1.0).normalize(), 14.0);
        let dt = 1.0 / 30.0;
        let mut here = aiv.clone();
        here.set_obstacles(&obst);
        here.activate(&c2);
        here.update(&c2, dt);
        let ahead = || std::thread::scope(|s| s.spawn(|| aiv.ahead(&obst, &c2, true, dt)).join().unwrap());
        let mut a = aiv.clone();
        assert!(a.take_ahead(ahead(), &obst, &c2, true, dt));
        assert_eq!(state(&a), state(&here));
        // (not taken: the aiv changed since, another obstacle - a -0 for a 0 - another car value, no activation)
        let mut b = aiv.clone();
        b.push_goal(Vec3::Z, Vec3::new(0.0, 0.0, 600.0), 5.0, Vec3::ZERO, -1.0, 0);
        assert!(!b.take_ahead(ahead(), &obst, &c2, true, dt));
        let mut o2 = obst;
        o2[0].pos.y = -0.0;
        assert!(!aiv.clone().take_ahead(ahead(), &o2, &c2, true, dt));
        let mut c3 = c2;
        c3.speed_cap = 30.0;
        assert!(!aiv.clone().take_ahead(ahead(), &obst, &c3, true, dt));
        assert!(!aiv.clone().take_ahead(ahead(), &obst, &c2, false, dt));
    }

    #[test]
    fn straight_corridor_line_in_the_middle_and_top_speed() {
        let mut aiv = Aiv::new(tune());
        let c = car(Vec3::ZERO, Vec3::Z, 0.0);
        aiv.update_state(&c, 1.0 / 60.0);
        // 600 m long, 12 m wide, along +z
        for i in 0..61 {
            let z = i as f32 * 10.0;
            aiv.push_portal(&portal(Vec3::new(6.0, 0.0, z), Vec3::new(-6.0, 0.0, z)));
        }
        aiv.push_goal(Vec3::Z, Vec3::new(0.0, 0.0, 600.0), 5.0, Vec3::ZERO, -1.0, 0);
        settle(&mut aiv, &c, 40);
        let pts: Vec<Vec3> = aiv.points().collect();
        assert!(pts.len() > 10, "{}", pts.len());
        for p in &pts {
            assert!(p.x.abs() < 0.05, "point off the middle {p:?}");
        }
        // pieces of about InitialSegmentLength
        assert!((pts[2].z - pts[1].z - 25.0).abs() < 3.0, "{:?}", &pts[..4]);
        // the speed profile: from the car's 0.1 m/s up at 6 m/s^2 to the table's 60
        let s = &aiv.ln.s;
        let (f, n) = (aiv.ln.f as usize, aiv.ln.n as usize);
        assert_eq!(s[f].v, 0.1);
        let vmax = s[f..n].iter().map(|s| s.v).fold(0.0, f32::max);
        assert!((vmax - 60.0).abs() < 1e-3, "top speed {vmax}");
        assert!(s[f + 1..n].iter().all(|s| (s.v_curve - 60.0).abs() < 1e-3));
        // first sample: v^2 = 0.01 + 2 a ds
        let v1 = (0.01 + 2.0 * 6.0 * s[f + 1].ds).sqrt();
        assert!((s[f + 1].v - v1).abs() < 1e-3, "{} {}", s[f + 1].v, v1);
        // output: full throttle from a standstill, no steer
        let o = aiv.output(0, 0.0, 0.0, false);
        assert!(o.valid);
        assert!(o.steer.abs() < 1e-3 && o.throttle > 0.9 && o.brake == 0.0, "{o:?}");
    }

    #[test]
    fn corner_cuts_inside_and_apex_speed_from_the_table() {
        let mut aiv = Aiv::new(tune());
        let c = car(Vec3::new(0.0, 0.0, 0.0), Vec3::Z, 20.0);
        aiv.update_state(&c, 1.0 / 60.0);
        // 10 m wide road up +z to z = 100, then a 90 degree turn to +x (inner corner at (5, 95)...)
        // straight part: x in [-5, 5]
        for i in 0..10 {
            let z = i as f32 * 10.0;
            aiv.push_portal(&portal(Vec3::new(5.0, 0.0, z), Vec3::new(-5.0, 0.0, z)));
        }
        // the corner square (x in [-5, 5], z in [95, 105]) as a fan of portals from the inner corner
        let inner = Vec3::new(5.0, 0.0, 95.0);
        for k in 1..4 {
            let a = (k as f32) * std::f32::consts::FRAC_PI_2 / 4.0;
            // outer point on the square's far sides
            let dir = Vec3::new(-a.cos(), 0.0, a.sin());
            let t = (10.0 / dir.x.abs()).min(10.0 / dir.z.abs());
            aiv.push_portal(&portal(inner, inner + dir * t));
        }
        // then along +x: z in [95, 105]
        for i in 0..10 {
            let x = 5.0 + i as f32 * 10.0;
            aiv.push_portal(&portal(Vec3::new(x, 0.0, 95.0), Vec3::new(x, 0.0, 105.0)));
        }
        aiv.push_goal(Vec3::X, Vec3::new(95.0, 0.0, 100.0), 5.0, Vec3::ZERO, -1.0, 0);
        settle(&mut aiv, &c, 60);
        let pts: Vec<Vec3> = aiv.points().collect();
        // every point inside the shrunk road (car half width 1 + buffer 1 = 2 m from the edges)
        for p in &pts {
            let in_straight = p.z <= 95.0 && p.x >= -3.0 - 1e-3 && p.x <= 3.0 + 1e-3;
            let in_corner = p.z > 95.0 - 1e-3 && p.z <= 103.0 + 1e-3 && p.x >= -3.0 - 1e-3;
            assert!(in_straight || in_corner, "point outside the shrunk corridor {p:?} {pts:?}");
        }
        // the line cuts to the inside: the point nearest the inner corner is on its inner half
        let near = pts.iter().min_by(|a, b| a.distance(inner).partial_cmp(&b.distance(inner)).unwrap()).unwrap();
        assert!(near.distance(inner) < 6.0, "nearest point to the inside corner {near:?} {pts:?}");
        // apex speed: at the tightest sample the curve speed is the table's
        let (f, n) = (aiv.ln.f as usize, aiv.ln.n as usize);
        let s = &aiv.ln.s[f + 1..n];
        let apex = s.iter().max_by(|a, b| a.kappa.abs().partial_cmp(&b.kappa.abs()).unwrap()).unwrap();
        let k = apex.kappa.abs();
        assert!(k > 0.02, "no bend found {k}");
        let vt = aiv.veh.curve_speed(k);
        assert!((apex.v_table - vt).abs() < 1e-4);
        // (v_t^2 k = 8 < MaxTotalAccel, so A = 8 and sqrt(A / k) = v_t when kv = 0)
        assert!((apex.v_curve - vt).abs() < 0.05 * vt, "{} {}", apex.v_curve, vt);
        assert!(apex.allowed <= apex.v_curve + 1e-4 && apex.v <= apex.allowed + 1e-4);
        // and the slowest allowed speed of the profile is the bend's
        let vmin = s.iter().map(|s| s.allowed).fold(f32::MAX, f32::min);
        assert!(vmin < 30.0 && vmin <= apex.v_curve + 1e-4, "{vmin}");
    }

    /// F-67089 (steer scalar), F-45930 (engine accel by gear), F-71244 (engine decel by gear) against the original's code run offline
    /// on random racer tunes (tools/people_small_reference.py -> testdata/people_small_ref.txt). The tune's derived scale at word 11 is
    /// 127 / MaxSpeed as `Veh::new` derives it (the setup that writes it is not part of the comparison).
    #[test]
    fn tune_lookups_match_the_original() {
        let hx = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut veh: Option<Veh> = None;
        let mut n = 0;
        for line in include_str!("testdata/people_small_ref.txt").lines() {
            let w: Vec<&str> = line.split(' ').collect();
            match w[0] {
                "TU" => {
                    let f: Vec<f32> = w[1..].iter().map(|x| f32::from_bits(hx(x))).collect();
                    assert_eq!(f[11], 127.0 / f[0]);
                    veh = Some(Veh::new(Arc::new(AiTune::from_floats(&f).unwrap())));
                }
                "ST" => {
                    let got = veh.as_ref().unwrap().steer_scalar(f32::from_bits(hx(w[1])));
                    assert_eq!(got.to_bits(), hx(w[3]), "{line}");
                    n += 1;
                }
                "AC" | "DC" => {
                    let (g, v) = (w[1].parse::<i32>().unwrap(), f32::from_bits(hx(w[2])));
                    let got = if w[0] == "AC" { veh.as_ref().unwrap().accel(g, v) } else { veh.as_ref().unwrap().decel(g, v) };
                    assert_eq!(got.to_bits(), hx(w[4]), "{line}");
                    n += 1;
                }
                _ => {}
            }
        }
        assert_eq!(n, 1080);
    }

    /// F-83981 (the stock / upgraded tune blend) with F-49493 (its post-load step: best-gear row 0, the derived scales at words 8 - 11)
    /// against the original's code run offline (tools/people_tune_reference.py -> testdata/people_tune_ref.txt; inputs from a seed by the
    /// xorshift below). Every word the original writes is compared: 0 .. 7, the rows, the tables, row 0, and 8 .. 11 against `Veh::new`;
    /// the idle speeds (words 12 .. 18) are the one block the original does not write (the port blends them).
    #[test]
    fn tune_blend_matches_the_original() {
        let words = |seed: u32| -> Vec<f32> {
            let mut x = seed;
            (0..2195)
                .map(|i| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    if i == 0 { 20.0 + ((x >> 20) as f32 / 4096.0) * 80.0 } else { ((x >> 8) as f32 / 16777216.0) * 30.0 }
                })
                .collect()
        };
        let mut n = 0;
        for line in include_str!("testdata/people_tune_ref.txt").lines() {
            let (head, out) = line.split_once(" -> ").unwrap();
            let h: Vec<&str> = head.split(' ').collect();
            let (a, b) = (words(h[1].parse().unwrap()), words(h[2].parse().unwrap()));
            let t = f32::from_bits(u32::from_str_radix(h[3], 16).unwrap());
            let want: Vec<u32> = out.split(' ').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            assert_eq!(want.len(), 2195);
            let blended = AiTune::lerp(&AiTune::from_floats(&a).unwrap(), &AiTune::from_floats(&b).unwrap(), t);
            let veh = Veh::new(Arc::new(blended.clone()));
            let mut got = vec![0u32; 2195];
            let put = |got: &mut Vec<u32>, at: usize, v: &[f32]| v.iter().enumerate().for_each(|(i, x)| got[at + i] = x.to_bits());
            put(&mut got, 0, &[blended.max_speed, blended.min_curvature, blended.max_curvature, blended.max_total_accel, blended.max_brake_decel]);
            put(&mut got, 5, &blended.drag);
            put(&mut got, 8, &[veh.low_k, veh.knee, veh.high_k, veh.spd_k]);
            for (r, row) in blended.accel_rows.iter().enumerate() {
                put(&mut got, 19 + 128 * r, row);
            }
            for (r, row) in blended.decel.iter().enumerate().skip(1) {
                put(&mut got, 915 + 128 * r, row);
            }
            put(&mut got, 1939, &blended.speed_high_curv);
            put(&mut got, 2003, &blended.speed_low_curv);
            put(&mut got, 2067, &blended.steer_scalar);
            assert_eq!(blended.accel, blended.accel_rows[0]);
            assert_eq!(blended.decel[0], blended.accel_rows[7]);
            for i in (0..2195).filter(|i| !(12..=18).contains(i)) {
                assert_eq!(got[i], want[i], "word {i} of {head}");
            }
            n += 1;
        }
        assert_eq!(n, 7);
    }

    #[test]
    fn pedal_mapping_cases() {
        let veh = Veh::new(tune());
        let (vel, fwd, right) = (Vec3::Z * 10.0, Vec3::Z, Vec3::X);
        // ae = 6, ad = 3, no boost, flat, no drag
        let (s, t, b) = pedal_map(&veh, 0.0, 2.0, 0.0, 0.0, 12.0, 0.0, 25.0, -9.8, 0, vel, fwd, right);
        assert_eq!((s, b), (0.0, 0.0));
        assert!((t - (2.0 + 6.0) / 12.0).abs() < 1e-6, "{t}");
        // a_req in [-ad, 0.5 ab): (a + ad) / (2 ad + ab)
        let (_, t, b) = pedal_map(&veh, 0.0, -1.0, 0.0, 0.0, 12.0, 0.0, 25.0, -9.8, 0, vel, fwd, right);
        assert!((t - 2.0 / 6.0).abs() < 1e-6 && b == 0.0, "{t}");
        // below -ad: brake -(a + ad) / MaxBrakeDecel
        let (_, t, b) = pedal_map(&veh, 0.0, -13.0, 0.0, 0.0, 12.0, 0.0, 25.0, -9.8, 0, vel, fwd, right);
        assert!(t == 0.0 && (b - 10.0 / 20.0).abs() < 1e-6, "{b}");
        // boost term in the throttle denominator: ab = c1 t1^2 = 12 x 0.25 = 3
        let (_, t, _) = pedal_map(&veh, 0.0, 4.0, 0.0, 0.5, 12.0, 0.0, 25.0, -9.8, 0, vel, fwd, right);
        assert!((t - 10.0 / 15.0).abs() < 1e-6, "{t}");
        // uphill: -g F.y adds to the request
        let up = Vec3::new(0.0, 0.1, 1.0).normalize();
        let (_, t, _) = pedal_map(&veh, 0.0, 0.0, 0.0, 0.0, 12.0, 0.0, 25.0, -9.8, 0, up * 10.0, up, right);
        assert!((t - (9.8 * up.y + 6.0) / 12.0).abs() < 1e-5, "{t}");
        // steer = clamp(-SteerScalar x kappa)
        let (s, _, _) = pedal_map(&veh, 0.1, 0.0, 0.0, 0.0, 12.0, 0.0, 25.0, -9.8, 0, vel, fwd, right);
        assert!((s + 0.4).abs() < 1e-6);
        let (s, _, _) = pedal_map(&veh, -1.0, 0.0, 0.0, 0.0, 12.0, 0.0, 25.0, -9.8, 0, vel, fwd, right);
        assert_eq!(s, 1.0);
    }

    /// The cars' .r50 copies of DragConstant are already scaled (x 1e-16 at PostLoad).
    #[test]
    fn real_tables_drag_scaled() {
        let dir = std::path::Path::new(r"D:\MCLA\extracted\cache\resources\vehicle\vp_mit_3000gt_99");
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("r50") {
                let Ok(res) = crate::assets::load_resource_file(&p) else { return };
                let Some(t) = AiTunes::parse(&res) else { return };
                assert!((t.base.drag[0] - 0.4012).abs() < 1e-3, "{:?}", t.base.drag);
            }
        }
    }
    use mcla_formats::rage::aitune::AiTunes;

    /// The recorded states of the original's aiv (work/ai-record/fixture, 2026-10-26: R8 on AROUND HOLLYWOOD,
    /// `mkfix.py`): every snapshot is an aiv after one update; the port's update from snapshot k with the car
    /// of snapshot k + 1 must give snapshot k + 1's line. Prints the errors (`--nocapture`); returns at once without
    /// the fixture.
    #[test]
    fn replay_recorded_updates() {
        let dir = "D:/MCLA/work/ai-record/fixture";
        let (Ok(fix), Ok(tb)) = (std::fs::read(format!("{dir}/r7_r8_run0.bin")), std::fs::read(format!("{dir}/aivtune_r8.bin"))) else { return };
        let floats: Vec<f32> = tb.chunks_exact(4).map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
        let tune = Arc::new(mcla_formats::rage::aitune::AiTune::from_floats(&floats).unwrap());
        let n = u32::from_le_bytes(fix[8..12].try_into().unwrap()) as usize;
        let rec = 8 + 31232 + 1024;
        let snaps: Vec<(f64, &[u8])> = (0..n).map(|k| {
            let o = 12 + k * rec;
            (f64::from_le_bytes(fix[o..o + 8].try_into().unwrap()), &fix[o + 8..o + 8 + 31232])
        }).collect();
        let (mut tested, mut exact, mut pts_equal, mut j_close, mut lockstep) = (0, 0, 0, 0, 0);
        for k in 0..n - 1 {
            if snaps[k + 1].0 - snaps[k].0 > 0.06 {
                continue;
            }
            let mut a = Aiv::new(tune.clone());
            a.import_original(snaps[k].1);
            let mut b = Aiv::new(tune.clone());
            b.import_original(snaps[k + 1].1);
            // brain-side pushes between the two (portals / goals appended) are not replayed: skip those pairs
            if b.portals.end() > a.portals.end() || b.goals.end() > a.goals.end() {
                continue;
            }
            a.n_obst = b.n_obst;
            a.obst = b.obst;
            a.planes(); // (the cell planes are derived from the portals; the original holds them, an import does not)
            let car = b.car_as_held();
            let (a0c, a0m, a0s) = (a.cursor, a.mode, a.step);
            a.update_state(&car, b.dt);
            if !a.line_used_up() {
                a.optimise();
            }
            let (pa, sa, fa, na, ja, pra) = a.line_state();
            let (pb, sb, fb, nb, jb, prb) = b.line_state();
            tested += 1;
            let pe = if pa.len() == pb.len() { pa.iter().zip(&pb).map(|(x, y)| (*x - *y).length()).fold(0.0f32, f32::max) } else { -1.0 };
            let se = if sa.len() == sb.len() { sa.iter().zip(&sb).map(|(x, y)| x.iter().zip(y).map(|(p, q)| (p - q).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max) } else { -1.0 };
            if pe == 0.0 && se == 0.0 {
                exact += 1;
            }
            pts_equal += (pe >= 0.0 && pe < 1e-4) as i32;
            j_close += ((ja - jb).abs() < 1e-3) as i32;
            lockstep += (pra == prb && fa == fb && na == nb && a.cursor == b.cursor && a.mode == b.mode) as i32;
            if pe < 0.01 && se > 0.05 && std::env::var("MCLA_REPLAY_DIFF").is_ok() {
                const NAMES: [&str; 14] = ["v", "allowed", "v_curve", "v_table", "kv", "kappa", "g_ty", "g_ly", "a_max", "ds", "ty", "xy", "w", "coll"];
                let mut shown = 0;
                for (i, (x, y)) in sa.iter().zip(&sb).enumerate() {
                    for q in 0..14 {
                        if (x[q] - y[q]).abs() > 1e-3 && shown < 10 {
                            shown += 1;
                            println!("    sample {} (first {fa}) {}: port {} original {}", i, NAMES[q], x[q], y[q]);
                        }
                    }
                }
            }
            if pe > 0.04 && std::env::var("MCLA_REPLAY_PTS").is_ok() {
                let (c0, m0, s0) = (a0c, a0m, a0s);
                let diffs: Vec<String> = pa.iter().zip(&pb).enumerate().filter(|(_, (x, y))| (**x - **y).length() > 1e-4).map(|(i, (x, y))| format!("p{} ({:.3},{:.3},{:.3})", i as i32 + a.ln.pts.base, x.x - y.x, x.y - y.y, x.z - y.z)).collect();
                println!("    pair start: cursor {c0} mode {m0} step {s0} progress {pra}; point diffs port-orig: {}", diffs.join(" "));
            }
            println!("k {k:3} t {:.3} progress {pra}/{prb} pts {}/{} first/end {fa}/{fb} {na}/{nb} J {ja:.4}/{jb:.4} max point err {pe:.4} max sample err {se:.4} cursor {}/{} mode {}/{}", snaps[k + 1].0, pa.len(), pb.len(), a.cursor, b.cursor, a.mode, b.mode);
        }
        println!("tested {tested} pairs: points equal {pts_equal}, J within 0.001 {j_close}, progress / window / cursor / mode equal {lockstep}, all equal {exact}");
        // (2026-10-26, 191 pairs: 69 / 144 / 186. 2026-11-07, 194 pairs, with the follower update and the planes exact: 190 / 189 / 191,
        // 189 all equal; of the 5 left 3 hold a snapshot read between the original's asynchronous line job and its join
        // (docs/reference/ai/ai_record.md), 2 differ in the optimiser's moves: docs/reference/ai/verify_follower.md section 3)
        assert!(tested >= 150 && lockstep * 100 >= tested * 95 && j_close * 100 >= tested * 70 && pts_equal * 100 >= tested * 30);
    }

    /// Re-run the evaluation chain (planes, spline, samples, backward, forward) over recorded post-update states of the original,
    /// everything dirty, with the car heading H turned back by the commanded step (the recording holds H after the output's turn,
    /// F-41825; the line was evaluated with the H before it): the cost J and the first leg's curvature must come out as stored
    /// (work/ai-record: 25 of 26 states J within 0.002 - the 26th was read mid-update -, sample 1's curvature within 1e-5).
    #[test]
    fn reevaluate_recorded_state() {
        let dir = "D:/MCLA/work/ai-record/fixture";
        let (Ok(fix), Ok(tb)) = (std::fs::read(format!("{dir}/r7_r8_run0.bin")), std::fs::read(format!("{dir}/aivtune_r8.bin"))) else { return };
        let floats: Vec<f32> = tb.chunks_exact(4).map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
        let tune = Arc::new(mcla_formats::rage::aitune::AiTune::from_floats(&floats).unwrap());
        let n = u32::from_le_bytes(fix[8..12].try_into().unwrap()) as usize;
        let rec = 8 + 31232 + 1024;
        let (mut tested, mut j_ok, mut k_ok) = (0, 0, 0);
        for k in (0..n).step_by(8) {
            let o = 12 + k * rec;
            let snap = &fix[o + 8..o + 8 + 31232];
            let mut a = Aiv::new(tune.clone());
            a.import_original(snap);
            let (_, _, js, _, _, ss) = a.samples_raw();
            let kk0 = a.ln.s[(a.ln.f + 1) as usize].kappa;
            let lim = 1.0 / a.veh.steer_scalar(a.v_fwd);
            let kk = kk0.max(-lim).min(lim);
            let vh = a.vel.z * a.h.z + a.vel.x * a.h.x;
            a.h = norm0(rot_y(a.h, -(kk * a.dt * vh)));
            a.sc.dirty = [u32::MAX; 2];
            a.ln.j = 0.0;
            a.planes(); // (derived from the portals; the original holds them, an import does not)
            a.spline();
            a.samples();
            let d = a.backward();
            a.forward(d);
            let (_, _, jr, _, _, sr) = a.samples_raw();
            tested += 1;
            j_ok += ((jr - js).abs() < 0.002) as i32;
            k_ok += ((sr[1][5] - ss[1][5]).abs() < 1e-5) as i32;
        }
        println!("re-evaluated {tested} recorded states: J within 0.002 in {j_ok}, first curvature within 1e-5 in {k_ok}");
        assert!(tested >= 20 && j_ok >= tested - 2 && k_ok >= tested - 2);
    }

    /// The original's pedal output (F-41825 command + F-28541 mapping) against its recorded steer target (mcOpponentInput +24, written by the
    /// brain each frame), on the post-update snapshots of `work/ai-record/fixture/r7_r8_inp.bin` (`work/ai-record/mkfix2.py`: the R8 of work/ai-record r7 with its
    /// input and brain). Per snapshot k, with H turned back by the output's own step: the line samples of snapshot k (`own`), or those of
    /// snapshot k - 1 with the car of k (`prev`: the asynchronous job's samples of the previous frame). Prints the counts (`--nocapture`);
    /// returns at once without the fixture.
    #[test]
    fn recorded_pedal_output() {
        let (Ok(fix), Ok(tb)) = (std::fs::read("D:/MCLA/work/ai-record/fixture/r7_r8_inp.bin"), std::fs::read("D:/MCLA/work/ai-record/fixture/aivtune_r8.bin")) else { return };
        let floats: Vec<f32> = tb.chunks_exact(4).map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
        let tune = Arc::new(mcla_formats::rage::aitune::AiTune::from_floats(&floats).unwrap());
        let n = u32::from_le_bytes(fix[8..12].try_into().unwrap()) as usize;
        let rec = 8 + 31232 + 1024 + 192 + 576;
        let be = |b: &[u8], o: usize| f32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let snap = |k: usize| {
            let o = 12 + k * rec;
            let t = f64::from_le_bytes(fix[o..o + 8].try_into().unwrap());
            let (al, inp, br) = (&fix[o + 8..o + 8 + 31232], &fix[o + 8 + 31232 + 1024..o + 8 + 31232 + 1024 + 192], &fix[o + 8 + 31232 + 1024 + 192..o + rec]);
            (t, al, inp, br)
        };
        let unturn = |a: &mut Aiv| {
            let kk0 = a.ln.s[(a.ln.f + 1) as usize].kappa;
            let lim = 1.0 / a.veh.steer_scalar(a.v_fwd);
            let kk = kk0.max(-lim).min(lim);
            let vh = a.vel.z * a.h.z + a.vel.x * a.h.x;
            a.h = norm0(rot_y(a.h, -(kk * a.dt * vh)));
        };
        let steer_of = |a: &mut Aiv, coll: bool| {
            let (kc, ac) = a.command(coll);
            let (s, t, b) = a.pedals(kc, ac, 2, 0.0);
            (s, t, b, kc, ac)
        };
        let (mut tested, mut own_ok, mut prev_ok, mut either, mut clamped) = (0, 0, 0, 0, 0);
        let mut worst = (0.0f32, 0usize);
        let mut hist = [0; 5];
        let mut xt = [[0; 2]; 2];
        for k in 1..n {
            let (t, al, inp, br) = snap(k);
            let (tp, alp, _, _) = snap(k - 1);
            let (want, coll) = (be(inp, 24), be(br, 296) > 0.0);
            let mut a = Aiv::new(tune.clone());
            a.import_original(al);
            if a.ln.n - a.ln.f <= 1 {
                continue;
            }
            let car = (a.pos, a.h, a.f, a.vel, a.ang_vel, a.v_fwd, a.dt);
            unturn(&mut a);
            let (so, ..) = steer_of(&mut a, coll);
            let mut ok_prev = false;
            if t - tp < 0.06 {
                let mut p = Aiv::new(tune.clone());
                p.import_original(alp);
                (p.pos, p.h, p.f, p.vel, p.ang_vel, p.v_fwd, p.dt) = car;
                if p.ln.n - p.ln.f > 1 {
                    unturn(&mut p);
                    let (sp, ..) = steer_of(&mut p, coll);
                    ok_prev = (sp - want).abs() < 1e-5;
                }
            }
            // (the recorded steer is mcOpponentInput's slewed one: from the previous frame's value at 4/s back towards 0, 6/s out, x dt)
            let (_, _, pinp, _) = snap(k - 1);
            let frames = ((t - tp) * 30.0).round() as f32;
            let so = if frames >= 1.0 && frames <= 3.0 {
                let cur = be(pinp, 24);
                let rate = if (cur - so) * cur >= 0.0 { 4.0 } else { 6.0 } * a.dt * frames;
                cur + (so - cur).max(-rate).min(rate)
            } else {
                so
            };
            let ok_own = (so - want).abs() < 1e-5;
            // is the snapshot's line the one its own car state gives (re-evaluated as in reevaluate_recorded_state)? else it was read mid-job
            let consistent = {
                let mut r = Aiv::new(tune.clone());
                r.import_original(al);
                let i = (r.ln.f + 1) as usize;
                let (k0, a0, d0) = (r.ln.s[i].kappa, r.ln.s[i].allowed, r.ln.s[i].ds);
                unturn(&mut r);
                r.sc.dirty = [u32::MAX; 2];
                r.ln.j = 0.0;
                r.planes();
                r.spline();
                r.samples();
                let d = r.backward();
                r.forward(d);
                (r.ln.s[i].kappa - k0).abs() < 1e-5 && (r.ln.s[i].allowed - a0).abs() < 1e-3 && (r.ln.s[i].ds - d0).abs() < 1e-4
            };
            xt[consistent as usize][ok_own as usize] += 1;
            tested += 1;
            own_ok += ok_own as i32;
            prev_ok += ok_prev as i32;
            either += (ok_own || ok_prev) as i32;
            clamped += (want.abs() >= 1.0) as i32;
            let e = (so - want).abs();
            hist[if e < 1e-5 { 0 } else if e < 1e-4 { 1 } else if e < 1e-3 { 2 } else if e < 1e-2 { 3 } else { 4 }] += 1;
            if !ok_own && (so - want).abs() > worst.0 {
                worst = ((so - want).abs(), k);
            }
            if !ok_own && !ok_prev && std::env::var("MCLA_PEDAL_DIFF").is_ok() {
                println!("k {k:3} t {t:.3} steer port {so:.6} original {want:.6} coll {coll} f {} n {}", a.ln.f, a.ln.n);
            }
        }
        println!("pedal output, {tested} snapshots: steer within 1e-5 with the own samples {own_ok}, with the previous frame's {prev_ok}, either {either}; clamped {clamped}; worst own {:.6} at {}; |err| <1e-5 / <1e-4 / <1e-3 / <1e-2 / more: {hist:?}; [line not / is the car's own][steer off / exact]: {xt:?}", worst.0, worst.1);
        // (2026-11-03, 464 snapshots: 189 within 1e-5, 136 more within 1e-4, 97 within 1e-3, 42 beyond. The output is the original's code word for word
        // (output_is_the_originals), so the misses are the recording's timing, not the rule: docs/reference/ai/verify_pedal_output.md)
        assert!(tested >= 400 && hist[0] * 100 >= tested * 35 && (hist[0] + hist[1] + hist[2]) * 100 >= tested * 85);
    }

    /// The original's follower output (F-65301 with F-41825 and F-28541), run offline on each of the 465 recorded aiv states of
    /// `work/ai-record/fixture/r7_r8_inp.bin` in 4 variants of gear / rpm term / collision (`work/racerai/pedal`, written to orig_out.bin):
    /// steer, throttle, brake, handbrake and the turned H, word for word. Returns at once without the files.
    #[test]
    fn output_is_the_originals() {
        let (Ok(fix), Ok(tb), Ok(orig)) = (
            std::fs::read("D:/MCLA/work/ai-record/fixture/r7_r8_inp.bin"),
            std::fs::read("D:/MCLA/work/ai-record/fixture/aivtune_r8.bin"),
            std::fs::read("D:/MCLA/work/racerai/pedal/orig_out.bin"),
        ) else { return };
        let floats: Vec<f32> = tb.chunks_exact(4).map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
        let tune = Arc::new(mcla_formats::rage::aitune::AiTune::from_floats(&floats).unwrap());
        let n = u32::from_le_bytes(fix[8..12].try_into().unwrap()) as usize;
        let rec = 8 + 31232 + 1024 + 192 + 576;
        let (gears, rpms) = ([2, 0, 3, 1], [0.0f32, 0.5, 1.0, 0.25]);
        let (mut cases, mut bad, mut bad_cmd) = (0, 0, 0);
        for k in 0..n {
            let al = &fix[12 + k * rec + 8..12 + k * rec + 8 + 31232];
            for v in 0..4 {
                let o = (k * 4 + v) * 44;
                let w = |i: usize| u32::from_le_bytes(orig[o + 4 * i..o + 4 * i + 4].try_into().unwrap());
                let mut a = Aiv::new(tune.clone());
                a.import_original(al);
                if a.ln.n - a.ln.f > 1 {
                    // (F-41825 alone: the commanded curvature and acceleration)
                    let mut c = Aiv::new(tune.clone());
                    c.import_original(al);
                    let (kc, ac) = c.command(w(8) != 0);
                    if (kc.to_bits(), ac.to_bits()) != (w(9), w(10)) {
                        if bad_cmd < 10 {
                            println!("k {k} v {v}: command port {:08x} {:08x} original {:08x} {:08x}", kc.to_bits(), ac.to_bits(), w(9), w(10));
                        }
                        bad_cmd += 1;
                    }
                }
                let out = a.output(gears[v], 0.0, rpms[v], w(8) != 0);
                let got = [out.valid as u32, out.steer.to_bits(), out.throttle.to_bits(), out.brake.to_bits(), out.handbrake.to_bits(), a.h.x.to_bits(), a.h.y.to_bits(), a.h.z.to_bits()];
                let want: Vec<u32> = (0..8).map(w).collect();
                cases += 1;
                if got[..] != want[..] {
                    if bad < 10 {
                        println!("k {k} v {v}: port {got:08x?} original {want:08x?}");
                    }
                    bad += 1;
                }
            }
        }
        println!("follower output vs the original's code: {cases} cases, {bad} differ ({bad_cmd} in the command alone)");
        assert!(cases >= 1800 && bad == 0 && bad_cmd == 0);
    }

    /// 12,000 cases of the original's scalar geometry helpers (F-41334 norm_or, F-88594 ang3, F-32060 ang_xz,
    /// F-52074 ang_xz_unit, F-57591 rot_y, F-17888 ease, F-91226 seg_portal), run offline on their
    /// generated code with the callee acos / sin / cos (work/ai-verify/scalar/cases.py): per case, bit for bit.
    #[test]
    fn scalar_helpers_are_the_originals() {
        let text = include_str!("aiv_scalar_cases.txt");
        let f = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap_or_else(|_| panic!("token {t:?}")));
        let v3 = |w: &[&str]| Vec3::new(f(w[0]), f(w[1]), f(w[2]));
        let mut bad: std::collections::BTreeMap<String, (usize, usize, String)> = Default::default();
        for line in text.lines() {
            let (i, o) = line.split_once(" => ").unwrap();
            let iw: Vec<&str> = i.split(' ').collect();
            let ow: Vec<&str> = o.split(' ').collect();
            let mode = iw[0];
            let a = &iw[1..];
            let (ok, got): (bool, String) = match mode {
                "norm" => {
                    let r = norm_or(v3(&a[0..3]), f(a[3]), v3(&a[4..7]));
                    let want = v3(&ow[1..4]);
                    (r.to_array().map(f32::to_bits) == want.to_array().map(f32::to_bits), format!("{:08x} {:08x} {:08x}", r.x.to_bits(), r.y.to_bits(), r.z.to_bits()))
                }
                "ang3" | "angxz" | "angu" => {
                    let (x, y) = (v3(&a[0..3]), v3(&a[3..6]));
                    let r = match mode {
                        "ang3" => ang3(x, y),
                        "angxz" => ang_xz(x, y),
                        _ => ang_xz_unit(x, y),
                    };
                    (format!("{:08x}", r.to_bits()) == ow[0], format!("{:08x}", r.to_bits()))
                }
                "roty" => {
                    let r = rot_y(v3(&a[0..3]), f(a[3]));
                    let want = v3(&ow[0..3]);
                    (r.to_array().map(f32::to_bits) == want.to_array().map(f32::to_bits), format!("{:08x} {:08x} {:08x}", r.x.to_bits(), r.y.to_bits(), r.z.to_bits()))
                }
                "ease" => {
                    let r = sin_ramp(f(a[0]));
                    (format!("{:08x}", r.to_bits()) == ow[0], format!("{:08x}", r.to_bits()))
                }
                "segp" => {
                    let r = seg_portal(v3(&a[0..3]), v3(&a[3..6]), v3(&a[6..9]), v3(&a[9..12]), a[12] != "0");
                    let want_ok = ow[0] == "1";
                    match r {
                        Some((s, t)) => (want_ok && format!("{:08x}", s.to_bits()) == ow[1] && format!("{:08x}", t.to_bits()) == ow[2], format!("{:08x} {:08x}", s.to_bits(), t.to_bits())),
                        None => (!want_ok, "none".into()),
                    }
                }
                _ => panic!("{mode}"),
            };
            let e = bad.entry(mode.to_string()).or_insert((0, 0, String::new()));
            e.1 += 1;
            if !ok {
                e.0 += 1;
                if e.2.is_empty() {
                    e.2 = format!("{line} (port {got})");
                }
            }
        }
        let mut fails = 0;
        for (m, (b, n, first)) in &bad {
            println!("{m}: {b} of {n} differ {first}");
            fails += b;
        }
        assert_eq!(fails, 0);
    }

    /// The original's evaluation chain (F-70569 obstacles, F-86861 spline, F-27864 samples, F-31929 backward, F-39661
    /// forward) run offline on its generated code on the 202 recorded aiv snapshots of the R8 at the bend
    /// (work/ai-verify/scalar/chain.py, stage s = the chain up to and including step s), against the port's chain run
    /// the same way: every control point, every sample field, J, n, first, and the spline's coefficients.
    #[test]
    fn evaluation_chain_is_the_originals() {
        let dir = "D:/MCLA/work/ai-verify/scalar";
        let sfx = std::env::var("AIV_SET").map(|v| format!("_{v}")).unwrap_or_default();
        let Ok(inp) = std::fs::read(format!("{dir}/chain_in{sfx}.bin")) else {
            println!("chain fixtures missing: skipped");
            return;
        };
        let tb = std::fs::read(format!("{dir}/tune_r8.bin")).unwrap();
        let floats: Vec<f32> = tb.chunks_exact(4).map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
        let tune = Arc::new(mcla_formats::rage::aitune::AiTune::from_floats(&floats).unwrap());
        const BS: usize = 31232;
        const SS: usize = 21696;
        let n = inp.len() / BS;
        let mut total_bad = 0;
        let only: Option<usize> = std::env::var("AIV_STAGE").ok().and_then(|v| v.parse().ok());
        for stage in 0..6usize {
            if only.is_some_and(|o| o != stage) || (only.is_none() && matches!(stage, 0 | 5)) {
                // (stages 0 and 5 - the obstacle pass and the whole optimiser - run only when asked: AIV_STAGE=0 / 5)
                continue;
            }
            let out = std::fs::read(format!("{dir}/chain_out_{stage}{sfx}.bin")).unwrap();
            let (mut same, mut first_bad) = (0, String::new());
            let (mut pts_bad, mut fn_bad, mut j_bad, mut coef_bad, mut sc_bad) = (0, 0, 0, 0, 0);
            let mut fields = [0usize; 14];
            for k in 0..n {
                let b = &inp[k * BS..(k + 1) * BS];
                let o = &out[k * (BS + SS)..k * (BS + SS) + BS];
                let scr = &out[k * (BS + SS) + BS..(k + 1) * (BS + SS)];
                // each stage starts from the original's state after the stage before (decoupled), the first from the snapshot
                let prev = if stage == 0 || stage == 5 { None } else { Some(std::fs::read(format!("{dir}/chain_out_{}{sfx}.bin", stage - 1)).unwrap()) };
                let (pb, pscr): (&[u8], Option<&[u8]>) = match &prev {
                    Some(pv) => (&pv[k * (BS + SS)..k * (BS + SS) + BS], Some(&pv[k * (BS + SS) + BS..(k + 1) * (BS + SS)])),
                    None => (b, None),
                };
                let mut a = Aiv::new(tune.clone());
                a.import_original(pb);
                a.planes();
                if stage == 5 {
                    // the whole optimiser from the snapshot (it sets up its own scratch)
                    a.optimise();
                } else if stage == 0 {
                    a.ln.s = [Sample::default(); NS];
                    a.ln.n = 0;
                    a.sc.dirty[0] = 0x3F_FFFF;
                    a.ln.j = 0.0;
                } else if let Some(sc) = pscr {
                    let f = |o: usize| f32::from_be_bytes(sc[o..o + 4].try_into().unwrap());
                    a.sc.dirty[0] = u32::from_be_bytes(sc[20160..20164].try_into().unwrap());
                    a.sc.dirty[1] = u32::from_be_bytes(sc[20164..20168].try_into().unwrap());
                    a.sc.legs = i32::from_be_bytes(sc[21584..21588].try_into().unwrap());
                    for l in 0..22usize {
                        for row in 0..4 {
                            a.sc.coef[l][row] = Vec3::new(f(20176 + 64 * l + 16 * row), f(20176 + 64 * l + 16 * row + 4), f(20176 + 64 * l + 16 * row + 8));
                        }
                    }
                }
                let mut d = 0.0;
                match stage {
                    0 => a.obstacles(),
                    1 => a.spline(),
                    2 => a.samples(),
                    3 => d = a.backward(),
                    5 => {} // (the whole optimiser has run above)
                    _ => {
                        // forward takes backward's result in f1: the original's own, recovered by running the port's backward
                        // on the original's samples first (checked as stage 3)
                        let mut t = Aiv::new(tune.clone());
                        t.import_original(pb);
                        t.planes();
                        d = t.backward();
                        a.forward(d);
                    }
                }
                let _ = d;
                let mut r = Aiv::new(tune.clone());
                r.import_original(o);
                let mut ok = true;
                if stage == 5 {
                    let rd = |off: usize| i32::from_be_bytes(o[off..off + 4].try_into().unwrap());
                    let want = [rd(30772), rd(30780), rd(25440 + 5348), rd(25440 + 5352), rd(25440 + 5356)];
                    let got = [a.cursor, a.mode, a.counters[0], a.counters[1], a.counters[2]];
                    if want != got || a.step.to_bits() != r.step.to_bits() || a.j_prev.to_bits() != r.j_prev.to_bits() || a.progress != r.progress {
                        sc_bad += 1;
                        ok = false;
                        if first_bad.is_empty() {
                            first_bad = format!("snapshot {k}: cursor/mode/counters port {got:?} original {want:?}; step {} / {}, j_prev {} / {}", a.step, r.step, a.j_prev, r.j_prev);
                        }
                    }
                }
                let (pp, ps, _, _, pj, _) = a.line_state();
                let (rp, rs, _, _, rj, _) = r.line_state();
                let (pf, pn, ..) = a.samples_raw();
                let (rf, rn, ..) = r.samples_raw();
                if pp.iter().zip(&rp).any(|(x, y)| x.to_array().map(f32::to_bits) != y.to_array().map(f32::to_bits)) {
                    pts_bad += 1;
                    if first_bad.is_empty() {
                        let i = pp.iter().zip(&rp).position(|(x, y)| x.to_array().map(f32::to_bits) != y.to_array().map(f32::to_bits)).unwrap();
                        first_bad = format!("snapshot {k} point {i} (of {} from {}): port {:?} original {:?}; obstacles {}", pp.len(), a.ln.pts.start, pp[i], rp[i], a.n_obst);
                    }
                    ok = false;
                }
                if stage >= 2 {
                    if (pf, pn) != (rf, rn) {
                        fn_bad += 1;
                        ok = false;
                    }
                    if stage >= 4 && pj.to_bits() != rj.to_bits() {
                        j_bad += 1;
                        ok = false;
                    }
                    for (i, (x, y)) in ps.iter().zip(&rs).enumerate() {
                        for q in 0..14 {
                            if x[q].to_bits() != y[q].to_bits() {
                                fields[q] += 1;
                                if ok && first_bad.is_empty() {
                                    first_bad = format!("snapshot {k} sample {i} field {q}: port {} original {}", x[q], y[q]);
                                }
                                ok = false;
                            }
                        }
                    }
                }
                if stage >= 1 {
                    let legs = u32::from_be_bytes(scr[21584..21588].try_into().unwrap()) as usize;
                    let f = |o: usize| f32::from_be_bytes(scr[o..o + 4].try_into().unwrap());
                    let mut bad = legs as i32 != a.sc.legs;
                    for l in 0..legs.min(a.sc.legs.max(0) as usize) {
                        for row in 0..4 {
                            let c = a.sc.coef[l][row];
                            for (q, v) in [c.x, c.y, c.z].into_iter().enumerate() {
                                if v.to_bits() != f(20176 + 64 * l + 16 * row + 4 * q).to_bits() {
                                    bad = true;
                                    if first_bad.is_empty() {
                                        first_bad = format!("snapshot {k} leg {l} coef row {row}.{q}: port {v} original {}", f(20176 + 64 * l + 16 * row + 4 * q));
                                    }
                                }
                            }
                        }
                    }
                    if bad {
                        coef_bad += 1;
                        ok = false;
                    }
                }
                if ok {
                    same += 1;
                }
            }
            println!("stage {stage}: {same} of {n} snapshots equal; points differ {pts_bad}, coefficients {coef_bad}, n/first {fn_bad}, J {j_bad}, cursor/mode/step/counters {sc_bad}; per sample field {fields:?}; first: {first_bad}");
            total_bad += n - same;
        }
        assert_eq!(total_bad, 0);
    }
}
