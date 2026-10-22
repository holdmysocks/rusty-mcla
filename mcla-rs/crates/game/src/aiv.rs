//! The rage `aiv` race-line follower of the computer racers, ported 1:1 from the executable
//! (decode: work/racerai/raceline.md; aiv code 0x8254A000-0x82550000, optimiser 0x8216C900-0x821712F0).
//!
//! The object holds a corridor of nav-mesh portals (ring of 128), the brain's goals (ring of 8),
//! the race line's control points (ring of 64, Catmull-Rom knots), up to 32 obstacles and the
//! aivRaceLineTuning. Each frame [`Aiv::update`] takes the car state (sub_8254D570, which also
//! builds one leg of the line, sub_8254D160) and runs the optimiser (sub_82171078, the job
//! "UpdateRaceLine", here synchronously); [`Aiv::output`] (sub_8254DCD0) gives the chassis pedals.
//!
//! Ring indices are absolute, as in the original: element `i` sits in slot
//! `(i - base + start + 1) mod N`. The line state that the optimiser saves and restores
//! (the 5040-byte block aiv+25440..+30480) is [`Line`].
//!
//! Runtime-initialised vector constants (zero in the image), decided from their initialisers
//! (static constructors 0x827A9108..0x827A9458) and their use:
//! 0x82860930 = (0,0,0,1) (copy of 0x827D8440), 0x82860950 = xyz lane mask, 0x82860910 /
//! 0x82860A00 = x,z(,w) lane mask, 0x828609E0 = +inf splat (rsqrte guard). 0x828608E0 and
//! 0x828608C0 have no initialiser in the static-constructor block: read as the zero vector
//! (they are the fallback of the normalisations and the floor of the half-space clamps
//! vmaxfp(dot, c) in sub_8216D5C0, where only 0 makes sense).
#![allow(dead_code)] // (wired into the racers by brain.rs)
use glam::Vec3;
use mcla_formats::rage::aitune::AiTune;
use std::sync::Arc;

/// aivRaceLineTuning (registration sub_8254A5F0, member table 0x82810A38); `Default` = the code
/// defaults of sub_825498C8. No tune file sets them in the game.
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

/// One nav-mesh portal as the brain passes it to sub_8254ADF0: the shared edge A-B of two
/// consecutive path polygons, the polygon normal, the polygon flag bit 0x2 ("off-road") and the
/// two extensions (0 for polygon edges; the path element's +28/+32 in sub_822BC508).
#[derive(Debug, Clone, Copy)]
pub struct Portal {
    pub a: Vec3,
    pub b: Vec3,
    pub normal: Vec3,
    pub offroad: bool,
    pub ext: (f32, f32),
}

/// One obstacle as the brain passes it to sub_8254B030 (the selection, sub_822BBB60, is the
/// brain's). `extent` is what the brain passes; sub_8254B030 stores half of it (x along `dir`'s
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

/// The car state of sub_8254D570 plus the values the brain sets once (sub_822BB47C).
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
    /// 2 x the bound's half extents (sub_8255B9A8(car+8)+16 x 2)
    pub size: Vec3,
    /// g = -9.8 x car vfunc 44 (negative)
    pub gravity: f32,
}

/// The chassis pedals of sub_8254A280 as written (before mcOpponentInput's smoothing).
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
/// of the cell to the next portal (sub_8254B8B8), plus the flag byte (0x80 = off-road).
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

/// The job's scratch (sub_8254DDD0): backup states, dirty bits, spline coefficients.
struct Scratch {
    backup: [Line; 4],
    dirty: [u32; 2],
    coef: [[Vec3; 4]; 64],
    legs: i32,
}

/// The vehicle tables (aivVehicleTuning) with the derived scales of sub_82549770.
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
    /// The class's read of a 128-entry speed table at x = speed index (unclamped fraction).
    fn read(row: &[f32], x: f32) -> f32 {
        if row.len() < 2 {
            return 0.0;
        }
        let x = if x > 0.0 { x } else { 0.0 };
        let i = x.min(126.1).floor();
        let k = (i as usize).min(row.len() - 2);
        let (a, b) = (row[k], row[k + 1]);
        (b - a) * (x - i) + a
    }
    /// sub_82549B78: max(0.1, MaxEngineAccelByGear [147 + 128 g + i]) = accel row g + 1.
    fn accel(&self, g: i32, v: f32) -> f32 {
        let r = (g.max(0) as usize + 1).min(self.t.accel_rows.len().saturating_sub(1));
        Self::read(self.t.accel_rows.get(r).map_or(&[][..], |r| &r[..]), self.spd_k * v).max(0.1)
    }
    /// sub_82549C20: max(0.1, MaxEngineDecelByGear [1043 + 128 g + i]) = decel row g + 1.
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
    /// sub_82549CC8
    fn steer_scalar(&self, v: f32) -> f32 {
        Self::read(&self.t.steer_scalar, v.abs() * self.spd_k)
    }
    /// the curvature speed tables as sub_8216E408 reads them (max 0)
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

/// sub_82165300: normalise if |v|^2 > eps, else the fallback.
fn norm_or(v: Vec3, eps: f32, fb: Vec3) -> Vec3 {
    let l2 = v.length_squared();
    if l2 > eps { v * (1.0 / l2.sqrt()) } else { fb }
}

/// rsqrte-guarded normalisation (vrsqrtefp + one Newton step; zero stays zero)
fn norm0(v: Vec3) -> Vec3 {
    let l2 = v.length_squared();
    if l2 > 0.0 { v * (1.0 / l2.sqrt()) } else { Vec3::ZERO }
}

/// sub_8254AD68: signed XZ angle between two unit vectors.
fn ang_xz_unit(a: Vec3, b: Vec3) -> f32 {
    let dot = (a.x * b.x + b.z * a.z).clamp(-1.0, 1.0);
    let s = b.x * a.z - a.x * b.z;
    dot.acos() * if s >= 0.0 { 1.0 } else { -1.0 }
}

/// sub_8216C9C0: signed XZ angle from a to b (0 when either is degenerate).
pub fn ang_xz(a: Vec3, b: Vec3) -> f32 {
    let norm = ((b.x * b.x + b.z * b.z) * (a.x * a.x + a.z * a.z)).sqrt();
    if norm - 1e-12 >= 0.0 {
        let c = ((b.z * a.z + b.x * a.x) / norm).clamp(-1.0, 1.0);
        let s = a.z * b.x - a.x * b.z;
        c.acos() * if s >= 0.0 { 1.0 } else { -1.0 }
    } else {
        0.0
    }
}

/// sub_8216C900: unsigned 3D angle.
fn ang3(a: Vec3, b: Vec3) -> f32 {
    let norm = (b.length_squared() * a.length_squared()).sqrt();
    if norm - 1e-12 >= 0.0 { (a.dot(b) / norm).clamp(-1.0, 1.0).acos() } else { 0.0 }
}

/// sub_8216C890: rotation about y.
fn rot_y(v: Vec3, a: f32) -> Vec3 {
    let (s, c) = a.sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
}

/// sub_8216CA78: segment P0-P1 against portal Q0-Q1 in XZ -> (s along the segment, t along the portal).
fn seg_portal(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3, one_sided: bool) -> Option<(f32, f32)> {
    let dqx = q1.x - q0.x;
    let dpz = p1.z - p0.z;
    let dpx = p1.x - p0.x;
    let dqz = q1.z - q0.z;
    let den = dqz * dpx - dqx * dpz;
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
        let t = (s * dpz + p0.z - q0.z) / dqz;
        return Some((s, t));
    }
    if dqz.abs() < 1e-5 {
        if dpz.abs() < 1e-5 {
            return None;
        }
        let s = (q0.z - p0.z) / dpz;
        let t = (s * dpx + p0.x - q0.x) / dqx;
        return Some((s, t));
    }
    let s = ((p0.z - q0.z) * dqx + (q0.x - p0.x) * dqz) / den;
    let t = (s * dpx + p0.x - q0.x) / dqx;
    Some((s, t))
}

/// the plane-normal construction of sub_8254B8B8: v made perpendicular to e (when |e|^2 > 1e-4),
/// normalised (fallback 0)
fn plane_n(v: Vec3, e: Vec3) -> Vec3 {
    let l2 = e.length_squared();
    let v = if l2 > 1e-4 { v - e * (v.dot(e) / l2) } else { v };
    norm_or(v, 1e-12, Vec3::ZERO)
}

/// sub_82227C18
fn sin_ramp(x: f32) -> f32 {
    if x <= 0.0 {
        0.0
    } else if x >= 1.0 {
        1.0
    } else {
        (x * 1.570_796_4).sin()
    }
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

    /// sub_8254CE70 (with the vehicle tables of the car, aiv+0).
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

    /// sub_8254B768: drop corridor, goals and line.
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
    /// sub_8254CA78
    pub fn line_used_up(&self) -> bool {
        self.progress >= self.ln.pts.end()
    }
    /// sub_8254C888: the first control point ahead of the car
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
    /// (diagnostics) the corridor's portals as held: the ends a / b and the shrunk ends sa / sb
    pub fn portal_ends(&self) -> impl Iterator<Item = (Vec3, Vec3, Vec3, Vec3)> + '_ {
        (self.portals.base..self.portals.end()).map(|i| {
            let c = self.portals.get(i);
            (c.a, c.b, c.sa, c.sb)
        })
    }

    pub fn points(&self) -> impl Iterator<Item = Vec3> + '_ {
        (self.ln.pts.base..self.ln.pts.end()).map(|i| self.ln.pts.get(i))
    }

    /// sub_8254C8C0: the corridor point ahead.
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

    /// sub_8254C6E0: the car is off the planned line (clamped position more than 0.25 m away in
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

    /// sub_8254ADF0: append a portal (shrunk by half the car width + BufferZone).
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

    /// sub_8254B828: drop the last portal.
    pub fn pop_portal(&mut self) {
        if self.portals.count == 0 {
            return;
        }
        self.portals.pop_back();
        self.planes_upto = self.planes_upto.min(self.portals.end() + 1);
    }

    /// sub_8254CFA0: append a goal. `w` = radius, or the gate fraction (flags bit 0 = gate,
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

    /// sub_8254B020 + sub_8254B030 for each (up to 32).
    pub fn set_obstacles(&mut self, list: &[Obstacle]) {
        self.n_obst = 0;
        for o in list.iter().take(32) {
            let dir = norm_or(Vec3::new(o.dir.x, 0.0, o.dir.z), 1e-5, Vec3::X);
            self.obst[self.n_obst] = Obstacle { pos: o.pos, vel: o.vel, dir, extent: (o.extent.0 * 0.5, o.extent.1 * 0.5), flags: o.flags & 0xC0 };
            self.n_obst += 1;
        }
    }

    // ---- per frame --------------------------------------------------------------------------

    /// sub_8254D570 (car state, one leg of line building, progress, trimming), then the
    /// optimiser (sub_8254DEA0 -> sub_82171078) when the line is not used up.
    /// (approx.: the original runs the optimiser as an async job joined on the next update, and
    /// only when brain+253 is clear; here it runs at once)
    pub fn update(&mut self, car: &AivCar, dt: f32) {
        self.update_state(car, dt);
        if !self.line_used_up() {
            self.optimise();
        }
    }

    /// sub_8254D570 alone.
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

    /// sub_8254DCD0 -> sub_8254CAA0 -> sub_8254A280. `gear` = brain gear - 2 clamped >= 0;
    /// `min_wheel` is passed by the brain but not read; `rpm_ratio` = the f3 term of
    /// sub_8254A280; `collision` = the brain's collision timer running.
    pub fn output(&mut self, gear: i32, _min_wheel: f32, rpm_ratio: f32, collision: bool) -> AivOut {
        if self.progress >= self.ln.pts.end() {
            return AivOut { throttle: 0.0, steer: 0.0, brake: 1.0, handbrake: 0.0, valid: false };
        }
        let (kc, ac) = self.command(collision);
        let (steer, throttle, brake) = pedal_map(&self.veh, kc, ac, rpm_ratio, self.t1, self.b1[1], self.t2, self.b2[1], self.g, gear, self.vel, self.f, self.right);
        // (the handbrake branch after sub_8254A280 never fires: it always writes 0)
        AivOut { throttle, steer, brake, handbrake: 0.0, valid: true }
    }

    /// sub_8254A280 for a commanded curvature / acceleration, with this follower's car state
    /// (the brain also calls it directly).
    pub fn pedals(&self, kappa: f32, a_cmd: f32, gear: i32, rpm_ratio: f32) -> (f32, f32, f32) {
        pedal_map(&self.veh, kappa, a_cmd, rpm_ratio, self.t1, self.b1[1], self.t2, self.b2[1], self.g, gear, self.vel, self.f, self.right)
    }

    /// sub_8254B110: nitro benefit (0..1) for the brain's nitro decision.
    pub fn nitro_benefit(&self) -> f32 {
        self.boost_benefit(self.b1[1], self.b1[2], false)
    }
    /// sub_8254B438: slingshot benefit.
    pub fn sling_benefit(&self) -> f32 {
        self.boost_benefit(self.b2[1], self.b2[2], true)
    }

    // ---- corridor ---------------------------------------------------------------------------

    /// sub_8254B8B8: the cell planes of every portal pair not yet done.
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

    /// sub_8216D1C8: walk the cell index along the corridor until x is in it.
    fn walk(&self, idx: &mut i32, x: Vec3) -> bool {
        let lo = self.portals.base;
        let hi = lo + self.portals.count - 2;
        let start = if *idx < lo { lo } else if *idx > hi { hi } else { *idx };
        *idx = start;
        if start <= hi {
            loop {
                let c = self.portals.get(*idx);
                let nx = self.portals.get(*idx + 1);
                if c.fwd.dot(x - nx.a) >= 1e-6 {
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
            if c.back.dot(x - c.a) >= 1e-6 {
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

    /// sub_8216D390: binary search of the cell of x between the cells a and b.
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
                if c.fwd.dot(x - nx.a) >= 1e-6 {
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
                if c.back.dot(x - c.a) >= 1e-6 {
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

    /// sub_8216D5C0: clamp x into its corridor cell (the shrunk side planes, and the end planes
    /// of the first and last cell).
    fn clamp_cell(&self, idx: &mut i32, x: &mut Vec3) {
        self.walk(idx, *x);
        let base = self.portals.base;
        let last = base + self.portals.count - 2;
        if *idx == base {
            let c = self.portals.get(*idx);
            *x -= c.back * c.back.dot(*x - c.a).max(0.0);
        } else if *idx == last {
            let c = self.portals.get(*idx);
            let nx = self.portals.get(*idx + 1);
            *x -= c.fwd * c.fwd.dot(*x - nx.a).max(0.0);
        }
        let c = self.portals.get(*idx);
        *x -= c.sside_a * c.sside_a.dot(*x - c.sa).max(0.0);
        *x -= c.sside_b * c.sside_b.dot(*x - c.sb).max(0.0);
    }

    /// sub_8216DB50: height of x on the cell's surface triangle.
    fn surface_y(&self, idx: i32, x: &mut Vec3) {
        let c = self.portals.get(idx);
        let nx = self.portals.get(idx + 1);
        let a = c.a;
        let e1 = c.b - a;
        let f10 = e1.z * e1.z * 1e-4 + e1.x * e1.x * 1e-4;
        let test = |e2: Vec3| {
            let thr = ((e2.z * e2.z + e2.x * e2.x) * f10).max(1e-12);
            let cr = e1.z * e2.x - e2.z * e1.x;
            (cr * cr >= thr, cr.abs())
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
        let u = (e2.z * d.x - d.z * e2.x).abs() * inv;
        let w = (e1.z * d.x - d.z * e1.x).abs() * inv;
        let y = w * e2.y + u * e1.y + a.y;
        let ymin = e1.y.min(0.0).min(e2.y) + a.y;
        let ymax = e1.y.max(0.0).max(e2.y) + a.y;
        x.y = y.max(ymin).min(ymax);
    }

    /// sub_8216D7B0: can p (in cell b) be reached from x (in cell a), bending the line once
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

    /// sub_8254BDD8: P -> M crosses every shrunk portal from..=to inside it (and is not too long).
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

    /// sub_8254BF98: the furthest portal whose midpoint the last point sees (string pulling).
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

    /// sub_8254C348: append a leg A -> B cut into pieces of about InitialSegmentLength.
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

    /// sub_8254D160: one leg per call, to the furthest visible portal or the next goal.
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

    /// sub_8216F898
    fn point_height(&mut self, k: i32) {
        let mut p = self.ln.pts.get(k);
        self.surface_y(self.port(k), &mut p);
        self.ln.pts.set(k, p);
    }

    /// sub_8216D9A0: the link a-b does not miss a shrunk portal it crosses.
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
                if (0.0..=1.0).contains(&s) && !(0.0..=1.0).contains(&t) {
                    return false;
                }
            }
            i += dir;
        }
        true
    }

    /// sub_8216F288: a pushed point is still inside the corridor with valid links.
    fn push_ok(&self, k: i32, out: &mut i32) -> bool {
        *out = self.port(k);
        let p = self.ln.pts.get(k);
        if !self.walk(out, p) {
            return false;
        }
        let c = self.portals.get(*out);
        if c.sside_a.dot(p - c.sa) >= 1e-6 || c.sside_b.dot(p - c.sb) >= 1e-6 {
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

    /// sub_8216F4B8: keep point k on its goal and in the corridor; mark it dirty.
    fn constrain(&mut self, k: i32) {
        let tag = self.pg.get(k);
        if tag != -1 {
            let g = self.goals.get(tag);
            let p = self.ln.pts.get(k);
            if g.gate() {
                let e = g.p1 - g.p0;
                let s = (p - g.p0).dot(e) / e.length_squared();
                let s = if s >= 0.0 { s } else { 0.0 };
                let s = if s - 1.0 >= 0.0 { 1.0 } else { s };
                self.ln.pts.set(k, e * s + g.p0);
            } else {
                let d = g.p0 - p;
                let d2 = d.x * d.x + d.z * d.z;
                if d2 > g.w * g.w {
                    let rs = if d2 > 0.0 { 1.0 / d2.sqrt() } else { 0.0 };
                    let f = (rs * d2 - g.w) * rs;
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

    /// sub_821705F0: make the link a -> b pass the shrunk portals by rotating b about a (once
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

    /// sub_82170918: rotate points k.. about point k-1, each following point by the falloff of
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
                self.ln.pts.set(i, rot_y(d, a) + pivot);
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

    /// the evaluation chain of sub_82170B98 / sub_82171078
    fn evaluate(&mut self) {
        self.ln.j = 0.0;
        self.obstacles();
        self.spline();
        self.samples();
        let d = self.backward();
        self.forward(d);
    }

    /// sub_82170B98: try point k moved by +-step (mode 0 rotate, 1 along, 2 across); keep the best.
    fn try_move(&mut self, k: i32, recurse: bool) {
        let (o, b) = if recurse { (0, 2) } else { (1, 3) };
        self.sc.backup[o] = self.ln;
        self.sc.backup[b] = self.ln;
        let mode = self.mode;
        let mut dv = Vec3::ZERO;
        if mode != 0 {
            let d = self.ln.pts.get(k + 1) - self.ln.pts.get(k - 1);
            let d = Vec3::new(d.x, 0.0, d.z);
            dv = if d.length_squared() > 1e-12 { norm0(d) } else { Vec3::X };
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

    /// sub_82170F38: re-try the neighbours with a falling step.
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

    /// sub_82171078: evaluate the line, then coordinate descent over the control points.
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

    /// sub_8216F900: push the line's points sideways off the predicted obstacles.
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
            let dn = if dxz.length_squared() > 1e-12 { norm0(dxz) } else { Vec3::ZERO };
            let lt = (self.ln.pts.get(i) - p0).length() * inv;
            let lat = Vec3::new(-dn.z, 0.0, dn.x);
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
                let r = Vec3::new(-ob.dir.z, 0.0, ob.dir.x);
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
                    let rcp = if den == 0.0 { f32::MAX } else { 1.0 / den };
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

    /// sub_8216DD38: Catmull-Rom coefficients of the legs in the window.
    fn spline(&mut self) {
        let end = self.window_end();
        let pr = self.progress;
        let n = end - pr + 1;
        self.sc.legs = (n - 1).max(0);
        if n < 2 {
            return;
        }
        let p = |s: &Self, i: i32| s.ln.pts.get(i);
        let v = if n > 2 { self.h * (p(self, pr + 1) - p(self, pr - 1)).length() } else { Vec3::ZERO };
        let ph = if v.length_squared() > 1e-12 { p(self, pr) - v } else { (p(self, pr - 1) - p(self, pr)) * 2.0 + p(self, pr) };
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
        } else if pr < end - 1 && n > 2 && self.goal_dir.length_squared() > 1e-12 {
            norm0(self.goal_dir) * (p(self, end - 1) - p(self, end - 3)).length()
        } else {
            Vec3::ZERO
        };
        w[3] = if v.length_squared() > 1e-12 { v + p(self, end - 2) } else { (p(self, end - 1) - p(self, end - 2)) * 2.0 + p(self, end - 2) };
        if k < 64 {
            self.sc.coef[k] = cr(&w);
        }
    }

    /// sub_8216E408: sample the dirty legs (curvature, grip, table speeds, weights).
    fn samples(&mut self) {
        let end = self.window_end();
        let pr = self.progress;
        let sps = self.tun.samples_per_segment;
        let isl = self.tun.initial_segment_length;
        let n = end - pr + 1;
        let p0 = self.ln.pts.get(pr - 1);
        let p1 = self.ln.pts.get(pr);
        let n0 = ((p1 - p0).length() * (sps as f32 / isl) + 0.5) as i32;
        let n0 = if n0 < 1 { 1 } else if n0 > 2 * sps { 2 * sps } else { n0 };
        self.ln.f = 2 * sps - n0;
        self.ln.n = sps * n + 1;
        let inv0 = 1.0 / n0 as f32;
        let inv_s = 1.0 / sps as f32;
        let t_tab = self.veh.t.clone();
        let g = self.g;
        let port0 = self.port(pr - 1);
        let nrm0 = self.portals.get(port0).n;
        let l0 = self.h.cross(nrm0);
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
            s.a_max = nrm0.y.max(1e-6) * t_tab.max_total_accel;
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
                        let tt = (j + 1) as f32 * inv;
                        let c = self.sc.coef[(k as usize).min(63)];
                        x = c[0] * (tt * tt * tt) + c[1] * (tt * tt) + c[2] * tt + c[3];
                        tv = c[0] * (3.0 * tt * tt) + c[1] * (2.0 * tt) + c[2];
                        let mut idx = port;
                        self.bracket(&mut idx, lo_port, self.port(r + 1), x);
                        port = idx;
                    }
                    tn = if tv.length_squared() > 1e-12 { norm0(tv) } else { Vec3::ZERO };
                    let cell = self.portals.get(port);
                    let l = tn.cross(cell.n);
                    th = ang3(UP, tn);
                    let d2 = (x - xp).length_squared();
                    let (ds, inv_ds) = if d2 >= 1e-12 { let rs = 1.0 / d2.sqrt(); (rs * d2, rs) } else { (1e-6, 1e8) };
                    let kappa = ang_xz(tp, tn) * inv_ds;
                    let mut dth = th - thp;
                    if dth - std::f32::consts::PI > 0.0 {
                        dth -= std::f32::consts::TAU;
                    }
                    if dth + std::f32::consts::PI < 0.0 {
                        dth += std::f32::consts::TAU;
                    }
                    let kv = -(dth * inv_ds);
                    let coll = if last_j && self.ln.pp.get(r + 1) & 1 != 0 { 1.0 } else { -1.0 };
                    let tag = if last_j { self.pg.get(r + 1) } else { -1 };
                    let goal_speed = if tag != -1 { self.goals.get(tag).speed } else { -1.0 };
                    let k_abs = kappa.abs();
                    let vt = self.veh.curve_speed(k_abs);
                    let v_table = self.tun.max_speed_scalar * vt;
                    let a_max = if k_abs - t_tab.min_curvature >= 0.0 { (vt * vt * k_abs).min(t_tab.max_total_accel) * cell.n.y.max(1e-6) } else { t_tab.max_total_accel };
                    let v_curve = if k_abs - t_tab.min_curvature >= 0.0 {
                        let den = (a_max * self.inv_g * kv + k_abs).max(1e-6);
                        let vv = (a_max * (1.0 / den)).sqrt() * self.tun.max_speed_scalar;
                        v_table.min(vv)
                    } else {
                        v_table
                    };
                    let mut w = 1.0;
                    if cell.offroad && (cell.side_a.dot(x - cell.a) >= 1e-6 || cell.side_b.dot(x - cell.b) >= 1e-6) {
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

    /// sub_8216CBC0: backward braking pass; returns the mean sample spacing.
    fn backward(&mut self) -> f32 {
        let (n, f) = (self.ln.n, self.ln.f);
        let t = self.veh.t.clone();
        let v0 = self.v_fwd;
        if (f as usize) < NS {
            self.ln.s[f as usize].v = if 0.1 - v0 >= 0.0 { 0.1 } else { v0 };
        }
        self.ln.t = 0.0;
        if n - f <= 1 || n as usize > NS {
            return 0.0;
        }
        let cap = self.cap;
        let lim = |a: f32, goal: f32| {
            let a = if goal >= 0.0 { a.min(goal) } else { a };
            if cap >= 0.0 { a.min(cap) } else { a }
        };
        let last = (n - 1) as usize;
        let s = self.ln.s[last];
        self.ln.s[last].allowed = lim(s.v_curve, s.allowed);
        let mtds = self.tun.max_total_decel_scalar;
        let brake = t.max_brake_decel * self.tun.max_brake_decel_scalar;
        let mut sum = 0.0;
        let mut j = last;
        while j as i32 > f {
            let s = self.ln.s[j];
            let v = s.allowed;
            let nn = (1.0 - s.kv * self.inv_g * v * v).clamp(0.0, 1.0);
            let alat = s.kappa * v * v + s.g_ly;
            let gl = nn * s.a_max * mtds;
            let lng = (gl * gl - alat * alat).max(0.1).sqrt();
            let dec = (brake - s.g_ty).min(lng);
            let vi = (dec * s.ds * 2.0 + v * v).max(0.1).sqrt();
            let p = self.ln.s[j - 1];
            self.ln.s[j - 1].allowed = lim(p.v_curve.min(vi), p.allowed);
            sum += s.ds;
            j -= 1;
        }
        sum / (n - 1) as f32
    }

    /// sub_8216CDD0: forward acceleration pass and the cost J.
    fn forward(&mut self, d: f32) {
        let (n, f) = (self.ln.n, self.ln.f);
        if n - f <= 1 || n as usize > NS {
            return;
        }
        let t = self.veh.t.clone();
        let (d1, d2, d3) = (t.drag[0], t.drag[1], t.drag[2]);
        let inv_maxc = 1.0 / t.max_curvature;
        let (c1, c2) = (self.b1[1], self.b2[1]);
        let (mut t1, mut t2) = (self.t1, self.t2);
        let mtas = self.tun.max_total_accel_scalar;
        let csp = self.tun.collision_speed_penalty;
        let mcp = self.tun.max_curvature_penalty;
        let (mut sumabs, mut svdt) = (0.0f32, 0.0f32);
        for i in (f + 1) as usize..n as usize {
            let s = self.ln.s[i];
            let vp = self.ln.s[i - 1].v;
            let b1 = t1.max(0.0);
            let b2 = t2.max(0.0);
            let nn = (1.0 - s.kv * self.inv_g * vp * vp).clamp(0.0, 1.0);
            let alat = s.kappa * vp * vp + s.g_ly;
            let drag = ((s.kappa.abs() * d3 + d2) * vp + d1).max(0.0);
            let acc = self.veh.accel0(vp);
            let mut a = b2 * b2 * b2 * c2 + b1 * b1 * c1 + acc + s.g_ty - drag * drag * drag * drag;
            let gl = nn * mtas * s.a_max;
            a = a.min((gl * gl - alat * alat).max(0.1).sqrt());
            let mut vi = (a * s.ds * 2.0 + vp * vp).max(0.1).sqrt().min(s.allowed);
            if s.coll >= 0.0 {
                vi -= csp;
            }
            let vi = vi.max(0.1);
            self.ln.s[i].v = vi;
            // (approx.: fres, the hardware reciprocal estimate, taken as 1/x)
            let dt = (1.0 / (vi + vp).max(0.1)) * s.ds * 2.0;
            self.ln.t += dt;
            self.ln.j += s.w * dt;
            self.ln.j += (s.kappa.abs() * inv_maxc - 1.0).max(0.0) * mcp;
            t1 -= dt;
            t2 -= dt;
            self.counters[2] += 1;
            svdt += dt * vi;
            sumabs += (s.ds - d).abs();
        }
        let vbar = svdt / self.ln.t.max(1e-6);
        if vbar > 1e-6 {
            self.ln.j += self.tun.average_speed_weight / vbar * (n - 1) as f32 * d;
        }
        self.ln.j *= self.tun.length_variance_weight * sumabs / ((n - 2) as f32 * d).max(1e-6) + 1.0;
    }

    /// sub_8254CAA0: commanded curvature and acceleration at the next sample; turns H.
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

    /// sub_8254B110 / sub_8254B438: fraction of the time gain of a boost that the grip-limited
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

    /// push point k by `dv` (sub_8216F900's push), keeping it when sub_8216F288 accepts it.
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

/// sub_8215E000 with the Catmull-Rom basis at 0x828386B0 (set up in sub_8216DD38).
fn cr(w: &[Vec3; 4]) -> [Vec3; 4] {
    [
        w[0] * -0.5 + w[1] * 1.5 + w[2] * -1.5 + w[3] * 0.5,
        w[0] * 1.0 + w[1] * -2.5 + w[2] * 2.0 + w[3] * -0.5,
        w[0] * -0.5 + w[2] * 0.5,
        w[1],
    ]
}

/// sub_8254A280: steer, throttle, brake from the commanded curvature and acceleration.
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
}
