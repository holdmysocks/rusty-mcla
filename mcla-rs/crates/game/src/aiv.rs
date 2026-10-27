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

#[derive(Clone, Copy)]
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
#[derive(Clone, Copy, Default)]
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

#[derive(Clone, Copy, Default)]
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
#[derive(Clone, Copy)]
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
struct Scratch {
    backup: [Line; 4],
    dirty: [u32; 2],
    coef: [[Vec3; 4]; 64],
    legs: i32,
}

/// The vehicle tables (aivVehicleTuning) with the derived scales of F-49493.
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
    /// the best-gear row 0 ([19..]) as the forward pass reads it
    fn accel0(&self, v: f32) -> f32 {
        Self::read(&self.t.accel, self.spd_k * v).max(0.1)
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
fn norm_or(v: Vec3, eps: f32, fb: Vec3) -> Vec3 {
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
fn acos32(c: f32) -> f32 {
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
fn ang_xz_unit(a: Vec3, b: Vec3) -> f32 {
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
fn ang3(a: Vec3, b: Vec3) -> f32 {
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
fn rot_y(v: Vec3, a: f32) -> Vec3 {
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

/// the plane-normal construction of F-68053: v made perpendicular to e (when |e|^2 > 1e-4),
/// normalised (fallback 0)
fn plane_n(v: Vec3, e: Vec3) -> Vec3 {
    let l2 = e.length_squared();
    let v = if l2 > 1e-4 { v - e * (v.dot(e) / l2) } else { v };
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
fn vrsqrt1(l2: f32) -> f32 {
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
        };
        a.reset();
        a
    }

    pub fn set_vehicle(&mut self, tune: Arc<AiTune>) {
        self.veh = Veh::new(tune);
    }

    /// F-71106: drop corridor, goals and line.
    pub fn reset(&mut self) {
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
    /// aiv+22560 + aiv+22552 != 0
    pub fn has_path(&self) -> bool {
        self.portals.base + self.portals.count != 0
    }
    /// F-65527
    pub fn line_used_up(&self) -> bool {
        self.progress >= self.ln.pts.end()
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
            self.portals.d[slot] = Cell {
                a: v3(o), b: v3(o + 16), sa: v3(o + 32), sb: v3(o + 48), n: v3(o + 64),
                offroad: b[22576 + slot] & 0x80 != 0,
                ..Cell::default()
            };
        }
        self.planes_upto = 0;
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
    /// Returns the test and the clamped position.
    pub fn off_line(&self) -> (bool, Vec3) {
        if self.progress >= self.ln.pts.end() {
            return (false, self.pos);
        }
        let mut x = self.pos;
        let mut idx = self.ln.pp.get(self.progress - 1) >> 1;
        self.clamp_cell(&mut idx, &mut x);
        self.surface_y(idx, &mut x);
        let (dx, dz) = (x.x - self.pos.x, x.z - self.pos.z);
        if dz * dz + dx * dx > 0.0625 || self.pos.y < x.y - 5.0 {
            return (true, x);
        }
        let pb = self.ln.pp.get(self.progress) >> 1;
        (!self.reach(idx, x, pb, self.ln.pts.get(self.progress)), x)
    }

    // ---- input from the brain ---------------------------------------------------------------

    /// F-95594: append a portal (shrunk by half the car width + BufferZone).
    pub fn push_portal(&mut self, p: &Portal) {
        let (mut a, mut b) = (p.a, p.b);
        let mut c = Cell { n: p.normal, offroad: p.offroad, ..Cell::default() };
        let d = b - a;
        let w = d.length();
        if w > 0.2 {
            let (e1, e2) = p.ext;
            let d = d * (1.0 / w);
            let lim = self.size.x * 0.5 + self.tun.buffer_zone;
            let half = (w + e1 + e2) * 0.5 - 0.05;
            let m = if lim - half >= 0.0 { half } else { lim };
            c.sb = d * (e2 - m) + b;
            c.sa = a - d * (e1 - m);
            if p.offroad {
                let h = w * 0.5 - 0.05;
                let m2 = if m - h >= 0.0 { h } else { m };
                a = d * m2 + a;
                b = b - d * m2;
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
        self.update_state(car, dt);
        if !self.line_used_up() {
            self.optimise();
        }
    }

    /// F-12813 alone.
    pub fn update_state(&mut self, car: &AivCar, dt: f32) {
        self.size = car.size;
        self.g = car.gravity;
        self.inv_g = 1.0 / car.gravity;
        self.b1 = car.nitro_tune;
        self.b2 = car.sling_tune;
        self.pos = car.pos;
        self.f = car.forward;
        self.vel = car.vel;
        self.ang_vel = car.ang_vel;
        self.v_fwd = self.f.x * self.vel.x + self.f.y * self.vel.y + self.f.z * self.vel.z;
        self.dt = dt;
        self.t1 = car.nitro_t;
        self.t2 = car.sling_t;
        self.cap = car.speed_cap;
        self.right = car.right;
        if self.h.dot(self.vel) < 1.0 || self.vel.length_squared() < 25.0 {
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
            self.h = n.cross(self.h.cross(n));
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
        let d2 = (target - l).length_squared();
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
        let speed = self.vel.length();
        let vh = self.vel.z * self.h.z + self.vel.x * self.h.x;
        let mut a1 = ang_xz_unit(fv, hv);
        let fdv = fv.dot(vv);
        let a2 = ang_xz_unit(vv, hv) * if fdv > 0.0 { fdv } else { 0.0 };
        if f == 2 * self.tun.samples_per_segment - 1 && speed < 10.0 {
            let d = self.ln.pts.get(self.progress) - self.pos;
            let d = norm_or(Vec3::new(d.x, 0.0, d.z), 1e-5, Vec3::ZERO);
            a1 += ang_xz_unit(fv, d);
        }
        let mut kc = (a2 + a1) / r.ds + r.kappa;
        if !collision {
            kc += r.kappa - self.ang_vel.y / speed.max(5.0);
        }
        let ac = (r.allowed * r.allowed - self.v_fwd * self.v_fwd) / (r.ds.max(1.0) * 2.0);
        let lim = 1.0 / self.veh.steer_scalar(self.v_fwd);
        let kk = r.kappa.max(-lim).min(lim);
        self.h = norm0(rot_y(self.h, kk * self.dt * vh));
        (kc, ac)
    }

    /// F-82077 / F-96973: fraction of the time gain of a boost that the grip-limited
    /// line keeps, against an unlimited run.
    fn boost_benefit(&self, c: f32, dur: f32, cubic: bool) -> f32 {
        let (n, f) = (self.ln.n, self.ln.f);
        let mut va = if 0.1 - self.v_fwd >= 0.0 { 0.1 } else { self.v_fwd };
        let mut vb = va;
        let (mut ta, mut tb) = (0.0f32, 0.0f32);
        let t = &self.veh.t;
        let (d1, d2, d3) = (t.drag[0], t.drag[1], t.drag[2]);
        let inv_t = 1.0 / dur;
        let pw = |b: f32| if cubic { b * b * b } else { b * b };
        let mut i = f + 1;
        while i < n && (i as usize) < NS {
            let s = self.ln.s[i as usize];
            let ba = (1.0 - inv_t * ta).max(0.0);
            let bb = (1.0 - inv_t * tb).max(0.0);
            let k = s.kappa.abs();
            let nn = (1.0 - s.kv * self.inv_g * va * va).clamp(0.0, 1.0);
            let alat = s.kappa * va * va + s.g_ly;
            let ga = nn * s.a_max;
            let lim = (ga * ga - alat * alat).max(0.0).sqrt();
            let dra = ((k * d3 + d2) * va + d1).max(0.0);
            let mut a = c * pw(ba) + (self.veh.accel0(va) - dra * dra * dra * dra) + s.g_ty;
            a = a.min(lim);
            let na = (a * s.ds * 2.0 + va * va).max(0.1).sqrt().min(s.allowed);
            let drb = ((k * d3 + d2) * vb + d1).max(0.0);
            let ab = c * pw(bb) + self.veh.accel0(vb) - drb * drb * drb * drb + s.g_ty;
            let nb = (ab * s.ds * 2.0 + vb * vb).max(0.1).sqrt();
            ta += s.ds * 2.0 / (na + va);
            tb += s.ds * 2.0 / (nb + vb);
            va = na;
            vb = nb;
            i += 1;
        }
        let tt = self.ln.t;
        let r = if tt - tb > 1e-6 { (tt - ta) / (tt - tb) } else { 0.0 };
        r.max(0.0).min(1.0)
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

fn side_n(p: Vec3, q: Vec3, c: Vec3) -> Vec3 {
    let e = q - p;
    if e.length_squared() > 1e-4 { plane_n((p + q) * 0.5 - c, e) } else { norm_or(p - c, 1e-12, Vec3::ZERO) }
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
    let vf = vel.x * fwd.x + vel.y * fwd.y + vel.z * fwd.z;
    let ss = veh.steer_scalar(vf);
    let steer = (-(ss * kappa)).max(-1.0).min(1.0);
    let mut ae = veh.accel(gear, vf);
    let ad = veh.decel(gear, vf);
    let t = &veh.t;
    let keff = (steer / ss).abs();
    let drag = ((keff * t.drag[2] + t.drag[1]) * vf + t.drag[0]).max(0.0);
    let areq = drag * drag * drag * drag + a_cmd - g * fwd.y;
    let (b1, b2) = (t1.max(0.0), t2.max(0.0));
    let ab = b2 * b2 * b2 * c2 + b1 * b1 * c1;
    if f3 != 0.0 && vf > 1.0 {
        let vh = norm0(vel);
        let x = right.dot(vh).abs().max(0.0).min(1.0);
        ae *= sin_ramp(x).max(steer.abs()) * f3 + 1.0;
    }
    let (thr, brk) = if areq >= ab * 0.5 {
        ((ae + areq) / (ae * 2.0 + ab), 0.0)
    } else if areq >= -ad {
        ((ad + areq) / (ad * 2.0 + ab), 0.0)
    } else {
        (0.0, -((ad + areq) / t.max_brake_decel))
    };
    let c = |x: f32| if x >= 0.0 { x.min(1.0) } else { 0.0 };
    (steer, c(thr), c(brk))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // (2026-10-26, 191 pairs: 69 / 144 / 186. The other pairs hold a snapshot read between the original's asynchronous line job and its join:
        // docs/reference/ai/ai_record.md)
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
