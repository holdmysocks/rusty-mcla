//! VIEW REPLAY (the race-over menu's last row): the race just run, played again from its
//! grid with the replay's own cameras and the legend of the prompt movie
//! (`template_Replay`; references `work/ui/ref/y_replay*.png`, `vr_*.png`).
//!
//! The race's cars are recorded as they are simulated - thirty states a second each - and
//! put back state by state; nothing is simulated again. The traffic about the player is
//! recorded too (as poses) and the cars that have left the road are made again for the replay.
//!
//! The cameras are the original's replay director (mcReplayCamera, `Director`; rule
//! `docs/reference/cameras/replay_camera_rule.md`): the player's five drive views and its
//! reverse camera, twelve places on the car (the six of
//! `tune/camera/<car>_replay.mcreplaycamera`, each also mirrored), a helicopter and a polar
//! camera (`.mcreplaycamera{fixed,heli,polar}`); a shot lasts a normal N(5, 3) s in [1, 10]
//! and the next one is drawn at random among those whose camera stands clear of the city.

use crate::physics::car::CarState;
use crate::vehicles::CarInstance;
use glam::{Quat, Vec3};

/// seconds between two recorded states
pub const STEP: f32 = 1.0 / 30.0;
/// the longest race kept (states of one car)
const MOST: usize = 30 * 60 * 12;
/// traffic further than this from the player is not recorded (it is not in a replay
/// camera's sight: the helicopter stands about 40 m off)
const TRAFFIC_RANGE: f32 = 250.0;
/// the tag of a car made again for a replay (taken off when it ends)
pub const GHOST: &str = "replay:";

/// A traffic car's pose at one step: what is drawn of it.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub steer: f32,
    /// per wheel: the hub in the car's axes, its turn, its steer, its compression
    pub wheels: [(Vec3, f32, f32, f32); 4],
}

impl Pose {
    fn of(s: &CarState) -> Pose {
        let inv = s.rot.inverse();
        let w = |k: usize| (inv * (s.wheels[k].world_pos - s.pos), s.wheels[k].spin, s.wheels[k].steer, s.wheels[k].compression);
        Pose { pos: s.pos, rot: s.rot, vel: s.vel, steer: s.steer, wheels: [w(0), w(1), w(2), w(3)] }
    }
    fn lerp(&self, b: &Pose, f: f32) -> Pose {
        let mut p = *self;
        p.pos = self.pos.lerp(b.pos, f);
        p.rot = self.rot.slerp(b.rot, f);
        p.vel = self.vel.lerp(b.vel, f);
        p.steer += (b.steer - self.steer) * f;
        for k in 0..4 {
            let (a, w) = (self.wheels[k], b.wheels[k]);
            p.wheels[k] = (a.0.lerp(w.0, f), a.1 + (w.1 - a.1) * f, a.2 + (w.2 - a.2) * f, a.3 + (w.3 - a.3) * f);
        }
        p
    }
    fn put(&self, s: &mut CarState) {
        s.pos = self.pos;
        s.rot = self.rot;
        s.vel = self.vel;
        s.steer = self.steer;
        for k in 0..4 {
            let (h, spin, steer, comp) = self.wheels[k];
            let w = &mut s.wheels[k];
            w.world_pos = self.pos + self.rot * h;
            w.spin = spin;
            w.steer = steer;
            w.compression = comp;
        }
    }
}

/// One car's stretch of the recording: a racer's (and the player's) whole states, or a traffic
/// car's poses while it was on the road near the player.
pub struct Track {
    pub uid: u32,
    /// the step of its first state
    pub first: usize,
    pub full: Vec<(CarState, f32)>,
    pub poses: Vec<Pose>,
    /// to make the car again when it has gone by the replay's time
    pub model: std::sync::Arc<crate::vehicles::CarModel>,
    pub color: [f32; 4],
}

impl Track {
    fn len(&self) -> usize {
        self.full.len().max(self.poses.len())
    }
    /// Puts the car where it was at step `x` (fractional); false outside the stretch.
    fn put(&self, x: f32, c: &mut CarInstance) -> bool {
        let x = x - self.first as f32;
        let Some(last) = self.len().checked_sub(1) else { return false };
        if x < 0.0 || x > last as f32 + 0.5 {
            return false;
        }
        let x = x.min(last as f32);
        if !self.full.is_empty() {
            let Some((s, lean)) = Recording::sample(&self.full, x * STEP) else { return false };
            c.state = s;
            c.lean = lean;
        } else {
            let i = (x.floor() as usize).min(last);
            let j = (i + 1).min(last);
            self.poses[i].lerp(&self.poses[j], x - i as f32).put(&mut c.state);
        }
        c.pose_history.begin(&c.state, c.lean);
        true
    }
}

#[derive(Default)]
pub struct Recording {
    pub tracks: Vec<Track>,
    /// the tracks still being added to, by car
    open: std::collections::HashMap<u32, usize>,
    /// steps taken
    steps: usize,
    acc: f32,
    /// the race clock when the last state was taken (a clock that runs back is a new race)
    clock: f32,
    /// the replay was left: the cars go back to where the race ended
    pub restore: bool,
    /// the race's countdown when the recording began (it runs down with the recording)
    pub countdown0: f32,
    /// every car as it stood when the replay began (put back when it is left)
    pub saved: Vec<(u32, CarState, f32)>,
}

impl Recording {
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.open.clear();
        self.steps = 0;
        self.acc = 0.0;
        self.clock = 0.0;
    }

    pub fn frames(&self) -> usize {
        self.steps
    }

    /// length in seconds
    pub fn len(&self) -> f32 {
        self.steps.saturating_sub(1) as f32 * STEP
    }

    /// One simulated step of a race: `clock` its time, `who` the cars that take part (indices
    /// into `cars`). The racers and the player are kept whole; the traffic about the player as
    /// poses, each car's stretch while it is near.
    pub fn record(&mut self, dt: f32, clock: f32, countdown: f32, cars: &[CarInstance], who: &[usize]) {
        if clock < self.clock || self.steps == 0 {
            self.clear();
            self.countdown0 = countdown.max(0.0);
            self.acc = STEP;
        }
        self.clock = clock;
        self.acc += dt;
        if self.acc < STEP || self.steps >= MOST {
            return;
        }
        self.acc = (self.acc - STEP).min(STEP);
        let step = self.steps;
        self.steps += 1;
        let me = cars.iter().find(|c| c.is_player).map(|c| c.state.pos);
        let mut seen = std::collections::HashSet::new();
        for (i, c) in cars.iter().enumerate() {
            let whole = c.is_player || who.contains(&i);
            if c.driver.starts_with(GHOST) || (!whole && me.map_or(true, |p| p.distance(c.state.pos) > TRAFFIC_RANGE)) {
                continue;
            }
            seen.insert(c.uid);
            let k = match self.open.get(&c.uid) {
                Some(&k) => k,
                None => {
                    self.tracks.push(Track { uid: c.uid, first: step, full: Vec::new(), poses: Vec::new(), model: c.model.clone(), color: c.color });
                    self.open.insert(c.uid, self.tracks.len() - 1);
                    self.tracks.len() - 1
                }
            };
            let t = &mut self.tracks[k];
            if whole {
                t.full.push((c.state.clone(), c.lean));
                // (a car just put on the grid has its wheels where they were before - the world's
                // origin - until its first simulated step: those states take the first good
                // state's wheels on the car)
                let off = |s: &CarState| s.wheels.iter().any(|w| w.world_pos.distance(s.pos) > 10.0);
                if !off(&c.state) {
                    let inv = c.state.rot.inverse();
                    let hubs: Vec<Vec3> = c.state.wheels.iter().map(|w| inv * (w.world_pos - c.state.pos)).collect();
                    for (s, _) in t.full.iter_mut().rev().skip(1).take_while(|x| off(&x.0)) {
                        for (w, h) in s.wheels.iter_mut().zip(&hubs) {
                            w.world_pos = s.pos + s.rot * *h;
                        }
                    }
                }
            } else {
                t.poses.push(Pose::of(&c.state));
            }
        }
        // (a car gone from the road, or out of range, ends its stretch)
        self.open.retain(|uid, _| seen.contains(uid));
    }

    /// The state of a recorded car `t` seconds into its stretch.
    pub fn sample(states: &[(CarState, f32)], t: f32) -> Option<(CarState, f32)> {
        let last = states.len().checked_sub(1)?;
        let x = (t / STEP).clamp(0.0, last as f32);
        let i = (x.floor() as usize).min(last);
        let j = (i + 1).min(last);
        let f = x - i as f32;
        let (a, b) = (&states[i], &states[j]);
        let mut s = a.0.clone();
        s.pos = a.0.pos.lerp(b.0.pos, f);
        s.rot = a.0.rot.slerp(b.0.rot, f);
        s.vel = a.0.vel.lerp(b.0.vel, f);
        s.steer = a.0.steer + (b.0.steer - a.0.steer) * f;
        s.rpm = a.0.rpm + (b.0.rpm - a.0.rpm) * f;
        for k in 0..4 {
            let (wa, wb) = (&a.0.wheels[k], &b.0.wheels[k]);
            let w = &mut s.wheels[k];
            w.compression = wa.compression + (wb.compression - wa.compression) * f;
            w.spin = wa.spin + (wb.spin - wa.spin) * f;
            w.steer = wa.steer + (wb.steer - wa.steer) * f;
            w.world_pos = wa.world_pos.lerp(wb.world_pos, f);
            w.contact = wa.contact.lerp(wb.contact, f);
        }
        Some((s, a.1 + (b.1 - a.1) * f))
    }

    /// Is this car in the recording?
    pub fn has(&self, uid: u32) -> bool {
        self.tracks.iter().any(|t| t.uid == uid)
    }

    /// The recorded cars that are no longer on the road (made again for the replay).
    pub fn gone<'a>(&'a self, cars: &[CarInstance]) -> Vec<&'a Track> {
        let mut out: Vec<&Track> = Vec::new();
        for t in &self.tracks {
            if !cars.iter().any(|c| c.uid == t.uid) && !out.iter().any(|o| o.uid == t.uid) {
                out.push(t);
            }
        }
        out
    }

    /// Puts the recorded cars where they were `t` seconds in; a car the recording does not
    /// have on the road then is put out of sight.
    pub fn apply(&self, t: f32, cars: &mut [CarInstance]) {
        let x = (t / STEP).clamp(0.0, self.steps.saturating_sub(1) as f32);
        for c in cars.iter_mut() {
            let mut known = false;
            let mut shown = false;
            let uid = c.uid;
            for tr in self.tracks.iter().filter(|tr| tr.uid == uid) {
                known = true;
                if tr.put(x, c) {
                    shown = true;
                    break;
                }
            }
            if known && !shown {
                c.state.pos.y = -1.0e4;
                c.pose_history.begin(&c.state, c.lean);
            }
        }
    }
}

/// The car's replay camera files.
#[derive(Clone, Debug)]
pub struct CamTune {
    /// SideFwd, SideRear, WheelFwd, WheelRear, Roof, HoodLookback: x, y up, z to the rear
    pub mounts: [Vec3; 6],
    /// average, variance, least, most seconds of a view
    pub cut: [f32; 4],
    /// the polar camera's lens: least, most, usual
    pub polar_fov_range: [f32; 3],
    pub fixed_fov: f32,
    /// the helicopter's lens: least, most, usual
    pub heli_fov: [f32; 3],
    /// its zoom's time: average, variance, least, most
    pub heli_cut: [f32; 4],
    /// MatchSpeedFactor, MatchPositionFactor, LinearDampingFactor
    pub heli_match: [f32; 3],
    /// FOVApproachRate (degrees a second), LookAtApproachRate
    pub heli_rates: [f32; 2],
}

impl Default for CamTune {
    /// (the Audi R8's files)
    fn default() -> Self {
        CamTune {
            mounts: [
                Vec3::new(-1.199245, 0.8420561, 0.2),
                Vec3::new(-1.099245, 0.8420561, 2.05057),
                Vec3::new(-1.199245, 0.52649701, 0.0),
                Vec3::new(-1.15, 0.52649701, 2.05057),
                Vec3::new(-0.99924529, 1.1916997, 1.025285),
                Vec3::new(-0.99924529, 1.1916997, -2.3400674),
            ],
            cut: [5.0, 3.0, 1.0, 10.0],
            polar_fov_range: [25.0, 60.0, 47.0],
            fixed_fov: 65.0,
            heli_fov: [4.0, 20.0, 7.0],
            heli_cut: [15.0, 5.0, 5.0, 30.0],
            heli_match: [2.0, 1.0, 1.0],
            heli_rates: [20.0, 5.0],
        }
    }
}

impl CamTune {
    pub fn load(data_dir: &str, car: &str) -> CamTune {
        // (the cut GT-R has no camera files: the R8's, catalog::camera_car)
        let car = crate::catalog::camera_car(car);
        let mut t = CamTune::default();
        let read = |ext: &str| -> Vec<(String, Vec<f32>)> {
            // (a mod car: its own file, then its manifest's camera donor - modfs.rs; the name hash ignores case)
            let text = crate::modfs::car_camera_text(data_dir, car, &format!("_replay.mcreplaycamera{ext}"), crate::modfs::Donor::Cameras).unwrap_or_default();
            text.lines()
                .filter_map(|l| {
                    let mut it = l.split_whitespace();
                    let key = it.next()?.to_string();
                    let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
                    (!v.is_empty()).then_some((key, v))
                })
                .collect()
        };
        const MOUNTS: [&str; 6] = ["SideFwdOffset", "SideRearOffset", "WheelFwdOffset", "WheelRearOffset", "RoofOffset", "HoodLookbackOffset"];
        const CUT: [&str; 4] = ["AverageCutDuration", "CutDurationVariance", "MinCutDuration", "MaxCutDuration"];
        for (k, v) in read("") {
            if let (Some(i), true) = (MOUNTS.iter().position(|m| *m == k), v.len() >= 3) {
                t.mounts[i] = Vec3::new(v[0], v[1], v[2]);
            } else if let Some(i) = CUT.iter().position(|m| *m == k) {
                t.cut[i] = v[0];
            }
        }
        for (k, v) in read("polar") {
            match k.as_str() {
                "MinFOV" => t.polar_fov_range[0] = v[0],
                "MaxFOV" => t.polar_fov_range[1] = v[0],
                "DefaultFOV" => t.polar_fov_range[2] = v[0],
                _ => {}
            }
        }
        for (k, v) in read("fixed") {
            if k == "CameraFOV" {
                t.fixed_fov = v[0];
            }
        }
        for (k, v) in read("heli") {
            match k.as_str() {
                "MinFOV" => t.heli_fov[0] = v[0],
                "MaxFOV" => t.heli_fov[1] = v[0],
                "DefaultFOV" => t.heli_fov[2] = v[0],
                "MatchSpeedFactor" => t.heli_match[0] = v[0],
                "MatchPositionFactor" => t.heli_match[1] = v[0],
                "LinearDampingFactor" => t.heli_match[2] = v[0],
                "FOVApproachRate" => t.heli_rates[0] = v[0],
                "LookAtApproachRate" => t.heli_rates[1] = v[0],
                _ => {
                    if let Some(i) = CUT.iter().position(|m| *m == k) {
                        t.heli_cut[i] = v[0];
                    }
                }
            }
        }
        t
    }
}

/// The replay director's states (mcReplayCamera +28; `docs/reference/cameras/replay_camera_rule.md`):
/// 0..4 the player's drive views (bumper, POV, boom 1, 2, 3), 5 the reverse camera, 6..17 the
/// twelve mounts on the car, 18 the helicopter, 19 the polar camera (NEXT CAMERA only).
pub const STATES: usize = 20;
pub const REVERSE: usize = 5;
pub const HELI: usize = 18;
pub const POLAR: usize = 19;
/// the state of mount `k` (0 SideFwd .. 5 HoodLookback) as the file has it; + 1 mirrored
pub fn mount_state(k: usize) -> usize {
    6 + 2 * k.min(5)
}

/// What the director needs of the player's car this frame.
#[derive(Clone, Copy, Debug)]
pub struct CarView {
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    /// the top of its bound box above its origin
    pub top: f32,
}

impl CarView {
    pub fn of(s: &CarState, top: f32) -> CarView {
        CarView { pos: s.pos, rot: s.rot, vel: s.vel, top }
    }
    /// a point given in the car's own axes (the original's car matrix * (x, y, z, 1))
    fn at(&self, o: Vec3) -> Vec3 {
        self.pos + self.rot * o
    }
    /// the matrix's row c (the car's rear)
    fn c(&self) -> Vec3 {
        self.rot * Vec3::Z
    }
}

/// The original's random numbers (0x82860880): a multiply-with-carry generator; a shot's
/// length is a normal deviate by the polar method (F-66400, the spare one kept).
#[derive(Clone, Debug)]
struct Mwc {
    x: u32,
    c: u32,
    spare: Option<f32>,
}

impl Mwc {
    fn new(seed: u32) -> Mwc {
        Mwc { x: seed | 1, c: 0x1234_5678, spare: None }
    }
    fn next(&mut self) -> u32 {
        let t = self.x as u64 * 0x5CDC_FAA7 + self.c as u64;
        self.x = t as u32;
        self.c = (t >> 32) as u32;
        self.x
    }
    fn u23(&mut self) -> f32 {
        (self.next() & 0x7F_FFFF) as f32
    }
    /// F-66400 with its mean and deviation: the original's arithmetic (2^-22, s = fma(second, second, first^2), the game's own logarithm,
    /// the result fma(value, deviation, mean)); the same routine as `VehState::random_normal`, checked there against the original's code
    fn normal(&mut self, mean: f32, dev: f32) -> f32 {
        if let Some(s) = self.spare.take() {
            return s.mul_add(dev, mean);
        }
        loop {
            let u = self.u23() * (1.0 / 4_194_304.0) - 1.0;
            let v = self.u23() * (1.0 / 4_194_304.0) - 1.0;
            let s = v.mul_add(v, u * u);
            if s < 1.0 && s != 0.0 {
                let k = ((crate::caraudio::ln_guest(s as f64) as f32 / s) * -2.0).sqrt();
                self.spare = Some(k * u);
                return (k * v).mul_add(dev, mean);
            }
        }
    }
    /// a shot's length: average, variance, least, most
    fn length(&mut self, c: [f32; 4]) -> f32 {
        self.normal(c[0], c[1]).max(c[2]).min(c[3])
    }
}

/// The k-th set bit of a mask, counted from the lowest (F-48190: its bytes from the lowest, a byte's bits from the top down);
/// -1 when the mask has k or fewer.
fn kth_bit(mask: u32, k: u32) -> i32 {
    let mut seen = 0u32;
    for byte in 0..4 {
        let b = (mask >> (8 * byte)) & 0xFF;
        seen += b.count_ones();
        if seen > k {
            let mut left = seen - k;
            for bit in (0..8).rev() {
                if b & (1 << bit) != 0 {
                    left -= 1;
                    if left == 0 {
                        return (8 * byte + bit) as i32;
                    }
                }
            }
        }
    }
    -1
}

/// The pad's stick byte as the original reads it (F-63907): (b - 127.5) / 127, within -1 .. 1.
#[cfg(test)]
fn stick_byte(b: u8) -> f32 {
    ((b as f32 - 127.5) * (1.0 / 127.0)).max(-1.0).min(1.0)
}

/// fmadds as the recomp evaluates it: the fused product-sum in double, then rounded to single
fn fmadds(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// vmsum3fp128 as the recomp's dpps: (0 + z) + (y + x)
fn dp3(a: Vec3, b: Vec3) -> f32 {
    (0.0 + a.z * b.z) + (a.y * b.y + a.x * b.x)
}

/// a point (w = 1) through matrix rows by vmsum4fp128 over the transposed rows: (place + z c) + (y b + x a)
fn xform(p: Vec3, m: &[Vec3; 4]) -> Vec3 {
    let k = |i: usize| (m[3][i] + p.z * m[2][i]) + (p.y * m[1][i] + p.x * m[0][i]);
    Vec3::new(k(0), k(1), k(2))
}

/// a x b as the vector unit forms it: -(b.yzx a.zxy - b.zxy a.yzx) (vpermwi products, vnmsubfp: the difference negated,
/// which keeps the original's signs of zero)
fn vcross(a: Vec3, b: Vec3) -> Vec3 {
    -(Vec3::new(b.y * a.z, b.z * a.x, b.x * a.y) - Vec3::new(b.z * a.y, b.x * a.z, b.y * a.x))
}

/// the scalar unit's reciprocal square root: the recomp's estimate (exact 1 / sqrt, single) and two Newton steps in double; 0 for x <= 0
fn rsqrt2(x: f32) -> f32 {
    let nd = x as f64;
    let y0 = (1.0 / nd.sqrt()) as f32 as f64;
    let y1 = (-(y0 * y0).mul_add(nd, -1.0)).mul_add(y0 * 0.5, y0);
    let y2 = (-(y1 * y1).mul_add(nd, -1.0)).mul_add(y1 * 0.5, y1);
    if -nd >= 0.0 { 0.0 } else { y2 as f32 }
}

/// F-97951 look-at (rows a, b, c from a direction and an up): c = dir by the vector unit's one-step reciprocal square root
/// (render::plane_scale), a = up x c the same, b = c x a
fn look_rows(dir: Vec3, up: Vec3) -> [Vec3; 3] {
    let c = dir * crate::render::plane_scale(dp3(dir, dir));
    let a0 = vcross(up, c);
    let a = a0 * crate::render::plane_scale(dp3(a0, a0));
    [a, vcross(c, a), c]
}

/// the squared length in the two orders the re-orthonormaliser uses: (y y, + x x, + z z) and (z z, + y y, + x x)
fn len2_yxz(v: Vec3) -> f32 {
    fmadds(v.z, v.z, fmadds(v.x, v.x, v.y * v.y))
}
fn len2_zyx(v: Vec3) -> f32 {
    fmadds(v.x, v.x, fmadds(v.y, v.y, v.z * v.z))
}

/// F-41334: normalise with a floor (1e-5 on the squared length): `fallback` below it
fn norm_or(v: Vec3, fallback: Vec3) -> Vec3 {
    let n = len2_yxz(v);
    if n > f32::from_bits(0x3727_c5ac) { v * rsqrt2(n) } else { fallback }
}

/// F-46571: the rows made orthonormal again: c normalised (else z), a = b x c normalised (else y x c, else b x z
/// normalised, else x), b = c x a
fn orthonormal(m: &mut [Vec3; 3]) {
    let eps = f32::from_bits(0x3727_c5ac);
    let c = norm_or(m[2], Vec3::Z);
    let a0 = vcross(m[1], c);
    let n = len2_yxz(a0);
    let a = if n > eps {
        a0 * rsqrt2(n)
    } else {
        let a1 = vcross(Vec3::Y, c);
        let n1 = len2_zyx(a1);
        if n1 > eps { a1 * rsqrt2(n1) } else { norm_or(vcross(m[1], Vec3::Z), Vec3::X) }
    };
    *m = [a, vcross(c, a), c];
}

/// F-77143: a fixed mount's camera: the eye is the mount through the car's rows, the rows the look-at along the car's c row
/// (0 - c when it looks back), made orthonormal
fn fixed_rows(o: Vec3, back: bool, car: &HeliCar) -> ([Vec3; 3], Vec3) {
    let eye = xform(o, &car.m);
    let c = car.m[2];
    let mut m = look_rows(if back { Vec3::ZERO - c } else { c }, Vec3::Y);
    orthonormal(&mut m);
    (m, eye)
}

/// the car as the helicopter reads it: its matrix rows (a, b, c, place) and velocity (car +400)
struct HeliCar {
    m: [Vec3; 4],
    vel: Vec3,
}

impl HeliCar {
    fn of(car: &CarView) -> HeliCar {
        HeliCar { m: [car.rot * Vec3::X, car.rot * Vec3::Y, car.rot * Vec3::Z, car.pos], vel: car.vel }
    }
    /// acos(c.z) through the guest's double arccosine (F-68873), negative when c.x < 0
    fn heading(&self) -> f32 {
        let h = crate::physics::exact::guest_acos(self.m[2].z as f64) as f32;
        if self.m[2].x < 0.0 { -h } else { h }
    }
}

impl Heli {
    fn rows16(&self) -> [f32; 16] {
        let mut r = [0.0; 16];
        for i in 0..3 {
            r[4 * i..4 * i + 3].copy_from_slice(&self.m[i].to_array());
        }
        r[12..15].copy_from_slice(&self.place.to_array());
        r
    }

    /// F-79727 (activate): the polar camera's activate on this object first (its two draws; what it sets is replaced),
    /// the angle fma(u, k, pi/40), the lens fma((most - least) u, 2^-23, least), the side bit, the place about the target
    /// along the car's heading, the look-at rows made orthonormal, then the zoom timer
    fn activate(rng: &mut Mwc, tune: &CamTune, car: &HeliCar) -> Heli {
        rng.next();
        rng.next();
        let t = xform(TARGET, &car.m);
        let angle = fmadds(rng.u23(), f32::from_bits(0x34aa_e714), f32::from_bits(0x3da0_d97c));
        let (lo, hi) = (tune.heli_fov[0], tune.heli_fov[1]);
        let fov = fmadds((hi - lo) * rng.u23(), f32::from_bits(0x3400_0000), lo);
        let side = rng.next() & 1 != 0;
        let a = if side { angle } else { -angle };
        let place = crate::aiv::rot_y(HELI_OFFSET, a + car.heading()) + t;
        let mut m = look_rows(place - t, Vec3::Y);
        orthonormal(&mut m);
        let zoom = rng.length(tune.heli_cut);
        Heli { m, place, vel: car.vel, fov, goal: fov, angle, side, zoom }
    }

    /// F-48462 (update): the zoom timer and a new lens goal, the side from the car's a row, the chase acceleration
    /// capped at HELI_ACCEL, the turn as a slerp of the rows' quaternions towards the look-at, the lens towards its goal
    fn update(&mut self, rng: &mut Mwc, tune: &CamTune, car: &HeliCar, dt: f32) {
        self.zoom -= dt;
        if self.zoom < 0.0 {
            self.zoom = rng.length(tune.heli_cut);
            let (lo, hi) = (tune.heli_fov[0], tune.heli_fov[1]);
            let q = (hi - lo) * 0.25;
            let half = q * 2.0;
            let floor = self.goal - q;
            let g = fmadds(((hi - half) - lo) * rng.u23(), f32::from_bits(0x3400_0000), lo);
            self.goal = if g > floor { g + half } else { g };
        }
        let m = &car.m;
        let d = self.place - m[3];
        self.side = dp3(m[0], self.vel) + dp3(m[0], d) > 0.0;
        let t = xform(TARGET, m);
        let a = if self.side { self.angle } else { -self.angle };
        let want = crate::aiv::rot_y(HELI_OFFSET, car.heading() + a) + t;
        let k = tune.heli_match;
        let acc = (want - self.place) * k[1] + (car.vel - self.vel) * k[0] - self.vel * k[2];
        let n = fmadds(acc.x, acc.x, fmadds(acc.y, acc.y, acc.z * acc.z)).sqrt();
        let s = HELI_ACCEL / if HELI_ACCEL - n >= 0.0 { HELI_ACCEL } else { n };
        self.vel = (acc * s) * dt + self.vel;
        self.place = self.vel * dt + self.place;
        let q0 = crate::cine::mat_quat(&self.rows16());
        let look = look_rows(self.place - t, Vec3::Y);
        let mut r = self.rows16();
        for i in 0..3 {
            r[4 * i..4 * i + 3].copy_from_slice(&look[i].to_array());
        }
        let mut q1 = crate::cine::mat_quat(&r);
        let u = tune.heli_rates[1] * dt;
        let u = if 1.0 - u >= 0.0 { u } else { 1.0 };
        // (F-37101: the shorter way: the goal negated when fma(x, x, fma(y, y, fma(w, w, z z))) < 0)
        if fmadds(q0[0], q1[0], fmadds(q0[1], q1[1], fmadds(q0[3], q1[3], q0[2] * q1[2]))) < 0.0 {
            q1 = q1.map(|v| -v);
        }
        crate::cine::quat_mat(&mut r, crate::cine::slerp(q0, q1, u));
        self.m = [Vec3::new(r[0], r[1], r[2]), Vec3::new(r[4], r[5], r[6]), Vec3::new(r[8], r[9], r[10])];
        orthonormal(&mut self.m);
        let rate = tune.heli_rates[0];
        if self.fov < self.goal {
            self.fov = fmadds(rate, dt, self.fov);
            if self.fov > self.goal {
                self.fov = self.goal;
            }
        } else if self.fov > self.goal {
            self.fov = fmadds(-rate, dt, self.fov);
            if self.fov < self.goal {
                self.fov = self.goal;
            }
        }
    }
}

/// The camera's turn from its row c (the way it looks away from) and the world's up
/// (F-97951: a = up x c, b = c x a).
fn look_rot(c: Vec3) -> Quat {
    let c = c.normalize_or(Vec3::Z);
    let a = Vec3::Y.cross(c).normalize_or(Vec3::X);
    let b = c.cross(a);
    Quat::from_mat3(&glam::Mat3::from_cols(a, b, c))
}

/// about x (F-43547): y' = y cos - z sin, z' = z cos + y sin
fn rot_x(v: Vec3, a: f32) -> Vec3 {
    // (the guest's sin and cos rounded to f32, the two fused sums of the asm)
    let s = crate::physics::exact::guest_sin(a as f64) as f32;
    let c = crate::physics::exact::guest_cos(a as f64) as f32;
    Vec3::new(v.x, v.y.mul_add(c, -(v.z * s)), v.z.mul_add(c, v.y * s))
}

/// the point on the car the helicopter and the polar camera look at (+240)
const TARGET: Vec3 = Vec3::new(0.0, 0.65, 0.0);
/// the helicopter's offset from it (+256), turned about the car's heading
const HELI_OFFSET: Vec3 = Vec3::new(0.0, 30.0, 25.0);
/// the polar camera's (+256), turned in the world's frame
const POLAR_OFFSET: Vec3 = Vec3::new(0.0, 1.0, 7.0);
/// the near plane of every replay camera (CameraNear)
const NEAR: f32 = 0.2;
/// (approx.) the helicopter's most acceleration (+332 = car +92 -> +124 * 9.8; the factor was
/// not read: taken as the sim's gravity scale 1.4)
const HELI_ACCEL: f32 = 1.4 * 9.8;

/// mcReplayCameraHeli (state 18): its matrix rows (+16 a, +32 b, +48 c, +64 place) as the original keeps them.
#[derive(Clone, Debug, Default)]
struct Heli {
    m: [Vec3; 3],
    place: Vec3,
    vel: Vec3,
    fov: f32,
    goal: f32,
    angle: f32,
    side: bool,
    zoom: f32,
}

/// mcReplayCameraPolar (state 19).
#[derive(Clone, Debug, Default)]
struct Polar {
    pitch: f32,
    yaw: f32,
    fov: f32,
}

/// What the replay shows this frame.
#[derive(Clone, Copy, Debug)]
pub enum Shot {
    /// the player's own camera: view 0 bumper .. 4 boom 3 (the original's numbering), `back`
    /// the reverse camera (state 5, from boom 1's place)
    Drive { view: u8, back: bool },
    /// the director's own: the place, the camera's axes (it looks along -c) and its lens in degrees
    Own { eye: Vec3, rot: Quat, fov: f32 },
}

impl Shot {
    /// eye, a point it looks at, up, lens
    pub fn view(&self) -> Option<(Vec3, Vec3, Vec3, f32)> {
        match *self {
            Shot::Own { eye, rot, fov } => Some((eye, eye - rot * Vec3::Z, rot * Vec3::Y, fov)),
            Shot::Drive { .. } => None,
        }
    }
}

/// The car's world, as the director's tests see it: `probe(a, b, r)` sweeps a ball of radius
/// `r` from `a` to `b` through the city (a == b: the ball at a) and gives its first touch: the
/// contact point and the normal, as the original's query result holds them.
pub trait Probe {
    fn sweep(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<(Vec3, Vec3)>;
    /// the heli's test of its view of the car: the city only, undirected (F-80278 with r10 0)
    fn heli(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<(Vec3, Vec3)> {
        self.sweep(a, b, r)
    }
    /// the same sweep against the watched car's own bound alone (F-38041's query: only that
    /// instance, directed)
    fn own(&mut self, _a: Vec3, _b: Vec3, _r: f32) -> Option<(Vec3, Vec3)> {
        None
    }
}

impl<F: FnMut(Vec3, Vec3, f32) -> Option<(Vec3, Vec3)>> Probe for F {
    fn sweep(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<(Vec3, Vec3)> {
        self(a, b, r)
    }
}

/// mcReplayCamera: which camera the replay shows and when it cuts (F-19930, choose
/// F-53721; `docs/reference/cameras/replay_camera_rule.md`).
#[derive(Clone, Debug)]
pub struct Director {
    pub state: usize,
    /// seconds left of the shot (+32)
    time: f32,
    /// DISABLE / ENABLE CAMERA SWITCHING (+36)
    pub switching: bool,
    /// NEXT CAMERA asked (+37)
    pub next: bool,
    rng: Mwc,
    heli: Heli,
    polar: Polar,
    /// the camera was changed this frame (the drive views start afresh)
    pub cut: bool,
    /// the screen's width over its height (the near plane's corner)
    pub aspect: f32,
    /// the lens (degrees) the shared viewport holds: the one last drawn. The tests' ball is sized by it, not by the lens
    /// of the camera being tested (F-51014 / F-67414 / F-38041 read the viewport's +704 near, +712 / +716 tangents)
    pub lens: f32,
    /// the drive view's place the frame before (the test of the reverse camera)
    pub drive_eye: Option<Vec3>,
}

impl Director {
    /// as mcPlayerCamera's reset leaves it (F-80435): BUMPER, switching on
    pub fn new(tune: &CamTune, seed: u32, car: &CarView) -> Director {
        let mut d = Director { state: 0, time: 0.0, switching: true, next: false, rng: Mwc::new(seed), heli: Heli::default(), polar: Polar::default(), cut: true, aspect: 16.0 / 9.0, lens: 65.0, drive_eye: None };
        d.polar_activate(tune);
        d.heli_activate(tune, car);
        d.time = d.rng.length(tune.cut);
        d
    }

    /// the ball the tests sweep: the near plane's corner, from the viewport (F-90812: +712 = tan(lens x pi/360) through
    /// the C library's double tangent, +716 = +712 x aspect; the tests: sqrt(fma(+712 n, +712 n, (+716 n)^2)), n the near
    /// +704). (The host's tangent stands for the C library's F-55967; both are rounded to single.)
    fn near_r(&self) -> f32 {
        let t = ((self.lens * f32::from_bits(0x3c0e_fa35)) as f64).tan() as f32;
        let u = t * self.aspect;
        let (a, b) = (u * NEAR, t * NEAR);
        fmadds(b, b, a * a).sqrt()
    }

    fn polar_activate(&mut self, tune: &CamTune) {
        // (F-82095: a side from fms(u, 2^-22, 1), then 22.5 .. 67.5 degrees, fma(u, k, pi/8))
        let sign = if fmadds(self.rng.u23(), f32::from_bits(0x3480_0000), -1.0) >= 0.0 { 1.0 } else { -1.0 };
        let a = fmadds(self.rng.u23(), f32::from_bits(0x33c9_0fda), f32::from_bits(0x3ec9_0fdb));
        self.polar = Polar { pitch: 0.0, yaw: a * sign, fov: tune.polar_fov_range[2] };
    }

    fn heli_activate(&mut self, tune: &CamTune, car: &CarView) {
        self.heli = Heli::activate(&mut self.rng, tune, &HeliCar::of(car));
    }

    fn heli_update(&mut self, tune: &CamTune, car: &CarView, dt: f32) {
        self.heli.update(&mut self.rng, tune, &HeliCar::of(car), dt);
    }

    /// The polar camera's update F-72161: place = rot_y(yaw, rot_x(-pitch, (0, 1, 7))) + T (T = the car's matrix times
    /// (0, 0.65, 0)), the look-at rows (F-97951); then the world collision F-67414: a ball (the viewport's near corner +
    /// 0.25) swept T -> place, on a touch place + ((contact - place) + normal r); after a touch only, the push-out F-38041 ->
    /// the car's own bound: the ball swept from 100 (place - T) + T to the place, on a touch normal r + contact; the look-at
    /// rows of the place made orthonormal (F-46571).
    fn polar_place(&self, car: &HeliCar, probe: &mut dyn Probe) -> (Vec3, [Vec3; 3]) {
        let t = xform(TARGET, &car.m);
        let mut place = crate::aiv::rot_y(rot_x(POLAR_OFFSET, -self.polar.pitch), self.polar.yaw) + t;
        let r = self.near_r() + 0.25;
        if let Some((c, n)) = probe.sweep(t, place, r) {
            place = place + (n * r + (c - place));
            if let Some((c, n)) = probe.own((place - t) * 100.0 + t, place, r) {
                place = n * r + c;
            }
        }
        let mut m = look_rows(place - t, Vec3::Y);
        orthonormal(&mut m);
        (place, m)
    }

    /// The right stick and the triggers while the polar camera is on (F-63907): x, y in
    /// -1..1 past the dead zone, `zoom` = (LT - RT) in -1..1.
    pub fn polar_stick(&mut self, tune: &CamTune, x: f32, y: f32, zoom: f32, dt: f32) {
        if self.state != POLAR {
            return;
        }
        let p = &mut self.polar;
        // (the original's own order: fma(dt * -y, 1.25, pitch), fma(dt * x, 2, yaw), fma(dt, zoom, 1) * lens; the least first, then the most)
        p.pitch = fmadds(dt * -y, 1.25, p.pitch).max(0.0).min(1.256_637_1);
        p.yaw = fmadds(dt * x, 2.0, p.yaw);
        p.fov = (fmadds(dt, zoom, 1.0) * p.fov).max(tune.polar_fov_range[0]).min(tune.polar_fov_range[1]);
    }

    /// state 6..17: the place on the car and whether it looks back
    fn mount(tune: &CamTune, state: usize) -> (Vec3, bool) {
        let k = (state - 6) / 2;
        let mut o = tune.mounts[k];
        if (state - 6) % 2 == 1 {
            o.x = -o.x;
        }
        (o, k == 5)
    }

    /// The camera of a state as it stands (the director's own; None for the drive views).
    fn own(&self, tune: &CamTune, car: &CarView, state: usize, probe: &mut dyn Probe) -> Option<(Vec3, Quat, f32)> {
        match state {
            6..=17 => {
                let (o, back) = Self::mount(tune, state);
                let (m, eye) = fixed_rows(o, back, &HeliCar::of(car));
                Some((eye, Quat::from_mat3(&glam::Mat3::from_cols(m[0], m[1], m[2])), tune.fixed_fov))
            }
            HELI => Some((self.heli.place, Quat::from_mat3(&glam::Mat3::from_cols(self.heli.m[0], self.heli.m[1], self.heli.m[2])), self.heli.fov)),
            POLAR => {
                let (p, m) = self.polar_place(&HeliCar::of(car), probe);
                Some((p, Quat::from_mat3(&glam::Mat3::from_cols(m[0], m[1], m[2])), self.polar.fov))
            }
            _ => None,
        }
    }

    /// activate(state) (F-82762): the camera starts afresh and is cut to
    fn activate(&mut self, tune: &CamTune, car: &CarView) {
        match self.state {
            HELI => self.heli_activate(tune, car),
            POLAR => self.polar_activate(tune),
            _ => {}
        }
        self.cut = true;
    }

    /// Can the shot be used (F-51014)? Its camera is updated, then a ball the size of the
    /// near plane's corner must stand free at its place; `ahead`: and stay free swept along the
    /// car's velocity for MinCutDuration; the helicopter must see the car's roof.
    fn shot_ok(&mut self, tune: &CamTune, car: &CarView, ahead: bool, dt: f32, probe: &mut dyn Probe) -> bool {
        if self.state == HELI {
            self.heli_update(tune, car, dt);
        }
        let eye = match self.own(tune, car, self.state, probe) {
            Some((e, _, _)) => e,
            // (the reverse camera: where it stood the frame before; none yet - taken)
            None => match self.drive_eye {
                Some(e) if !self.cut => e,
                _ => return true,
            },
        };
        let r = self.near_r();
        if probe.sweep(eye, eye, r).is_some() {
            return false;
        }
        if ahead && probe.sweep(eye, eye + car.vel * tune.cut[2], r).is_some() {
            return false;
        }
        if self.state == HELI && probe.heli(eye, car.pos + Vec3::Y * car.top, 0.5).is_some() {
            return false;
        }
        true
    }

    /// choose() (F-53721): NEXT CAMERA steps to the next state; otherwise any other
    /// state but the polar one with equal chance (a chase view never after a chase view),
    /// up to six tries until one passes its test.
    /// `test(self, ahead)` is the shot test F-51014.
    fn choose(&mut self, tune: &CamTune, car: &CarView, test: &mut dyn FnMut(&mut Director, bool) -> bool) {
        let mut mask: u32 = 0xF_FFFF & !(1 << POLAR);
        if (2..=4).contains(&self.state) {
            mask &= !0b1_1100;
        }
        let mut tries = 5;
        loop {
            if self.next {
                self.state = (self.state + 1) % STATES;
                self.activate(tune, car);
            } else if self.state == POLAR && tries > 3 {
                self.polar_activate(tune);
            } else if self.state == HELI && tries > 3 && !(self.time < 0.0) {
                self.heli_activate(tune, car);
            } else {
                mask &= !(1 << self.state);
                let n = mask.count_ones().max(1);
                let k = (self.rng.next() & 0x7FFF_FFFF) % n;
                self.state = kth_bit(mask, k).max(0) as usize;
                self.activate(tune, car);
            }
            let t = tries;
            tries -= 1;
            // (a pick from an empty mask traps in the original - divw by 0 - and cannot happen: every state but the
            // last one tried stays in it)
            if t <= 0 || self.state < 5 || test(self, true) {
                break;
            }
        }
        self.next = false;
        self.time = self.rng.length(tune.cut);
    }

    /// The replay camera's frame update F-19930: the shot's time runs down while switching is on, NEXT CAMERA starts a
    /// new one; a choice when NEXT CAMERA was asked, the time ran out, or a director's own shot fails its test.
    fn direct(&mut self, tune: &CamTune, car: &CarView, dt: f32, test: &mut dyn FnMut(&mut Director, bool) -> bool) {
        self.cut = false;
        if self.switching && !self.next {
            self.time -= dt;
        } else if self.next {
            self.time = self.rng.length(tune.cut);
        }
        let ok = !self.next && !(self.time < 0.0) && (self.state < 5 || test(self, false));
        if !ok {
            self.choose(tune, car, test);
        }
    }

    /// One frame (F-19930); `dt` 0 while the replay is paused.
    pub fn update(&mut self, tune: &CamTune, car: &CarView, dt: f32, probe: &mut dyn Probe) -> Shot {
        self.direct(tune, car, dt, &mut |d: &mut Director, ahead| d.shot_ok(tune, car, ahead, dt, probe));
        // (the camera manager updates its camera as well: the helicopter takes two steps a
        // frame - its zoom measured at 37..40 degrees a second against FOVApproachRate 20,
        // `work/cameras2/cap2/replay3.txt`)
        if self.state == HELI {
            self.heli_update(tune, car, dt);
        }
        self.shot(tune, car, probe)
    }

    /// The shot as it stands (no time passes).
    pub fn shot(&mut self, tune: &CamTune, car: &CarView, probe: &mut dyn Probe) -> Shot {
        match self.own(tune, car, self.state, probe) {
            Some((eye, rot, fov)) => Shot::Own { eye, rot, fov },
            None if self.state == REVERSE => Shot::Drive { view: 2, back: true },
            None => Shot::Drive { view: self.state as u8, back: false },
        }
    }

    /// seconds left of the shot
    pub fn time_left(&self) -> f32 {
        self.time
    }

    /// State `s` at once (test hooks, the idle camera).
    pub fn force(&mut self, tune: &CamTune, car: &CarView, s: usize) {
        self.state = s % STATES;
        self.activate(tune, car);
        self.time = self.rng.length(tune.cut);
    }
}

/// A mount's camera, held (the idle camera): eye, a point it looks at, up, lens.
pub fn mount_view(tune: &CamTune, car: &CarView, k: usize) -> (Vec3, Vec3, Vec3, f32) {
    let (o, back) = Director::mount(tune, mount_state(k));
    let c = if back { -car.c() } else { car.c() };
    let rot = look_rot(c);
    let eye = car.at(o);
    (eye, eye - rot * Vec3::Z, rot * Vec3::Y, tune.fixed_fov)
}

pub struct Playback {
    pub t: f32,
    pub paused: bool,
    /// Back steps: the whole legend, the one line HIDE HUD, nothing at all
    pub legend: u8,
    /// the views change by themselves (DISABLE / ENABLE CAMERA SWITCHING)
    pub auto_cam: bool,
    /// the replay's PAUSE panel and its cursor
    pub menu: Option<usize>,
    pub tune: CamTune,
    /// the other traffic was taken off the road
    pub started: bool,
    seed: u32,
    pub director: Option<Director>,
    /// NEXT CAMERA pressed (taken by the director's next frame)
    next: bool,
    /// a state asked for by a test hook
    force: Option<usize>,
    /// the replay's clock at the director's last frame (it runs on the replay's own time:
    /// slow motion and FRAME ADVANCE included)
    last_t: Option<f32>,
    /// the race's own start played again before the replay's clock runs (the warp onto the
    /// grid, the start scene with its badge and 3-2-1 - `work/cameras2/cap2`, opening): 1
    /// asked for, 2 running, 0 over
    pub opening: u8,
    /// frames of the opening so far
    pub opening_frames: u32,
}

impl Playback {
    pub fn new(tune: CamTune, seed: u32) -> Playback {
        Playback { t: 0.0, paused: false, legend: 0, auto_cam: true, menu: None, tune, started: false, seed, director: None, next: false, force: None, last_t: None, opening: 1, opening_frames: 0 }
    }

    pub fn restart(&mut self) {
        self.t = 0.0;
        self.opening = 1;
        self.auto_cam = true;
        self.paused = false;
        self.menu = None;
        self.director = None;
        self.last_t = None;
    }

    /// a state of the director at once (test hook)
    pub fn set_view(&mut self, v: usize) {
        self.force = Some(v % STATES);
    }

    /// NEXT CAMERA
    pub fn next_view(&mut self) {
        self.next = true;
    }

    /// the director's state (0..19), once it runs
    pub fn view(&self) -> Option<usize> {
        self.director.as_ref().map(|d| d.state)
    }

    /// The right stick (x, y in -1..1) and the triggers (`zoom` = LT - RT): the polar camera
    /// turns and zooms (F-63907: past a dead zone of 0.3).
    pub fn stick(&mut self, x: f32, y: f32, zoom: f32, dt: f32) {
        let dead = |v: f32| crate::boomcam::dead_zone(v.max(-1.0).min(1.0), 0.3);
        if let Some(d) = self.director.as_mut() {
            d.polar_stick(&self.tune, dead(x), dead(y), zoom, dt);
        }
    }

    /// The camera of this frame (None while the opening runs: the race start's own cameras).
    /// `car`: the player's car. The director's time is the replay clock's step since its last
    /// frame.
    /// `lens`: the lens (degrees) the last frame was drawn with (the viewport's).
    pub fn camera(&mut self, car: &CarView, aspect: f32, lens: f32, probe: &mut dyn Probe) -> Option<Shot> {
        if self.opening != 0 {
            self.last_t = None;
            return None;
        }
        let dt = self.last_t.map_or(0.0, |l| (self.t - l).max(0.0));
        self.last_t = Some(self.t);
        let tune = &self.tune;
        let d = self.director.get_or_insert_with(|| Director::new(tune, self.seed, car));
        d.aspect = aspect;
        d.lens = lens;
        d.switching = self.auto_cam;
        if let Some(s) = self.force.take() {
            d.force(tune, car, s);
        }
        if std::mem::take(&mut self.next) {
            d.next = true;
        }
        if dt <= 0.0 {
            // (F-10533: paused, only NEXT CAMERA runs the director)
            if d.next {
                return Some(d.update(tune, car, 0.0, probe));
            }
            d.cut = false;
            return Some(d.shot(tune, car, probe));
        }
        Some(d.update(tune, car, dt, probe))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-43547 (rotation about x) against the original's own code on 6,000 cases
    /// (`work/triage-cameras/verify/b1.py`; `MCLA_CAMERA_REF_DIR` = its `ref` folder).
    #[test]
    fn rot_x_is_the_originals() {
        let Ok(dir) = std::env::var("MCLA_CAMERA_REF_DIR") else { return };
        let text = std::fs::read_to_string(std::path::Path::new(&dir).join("b1_ref.txt")).unwrap();
        let mut n = 0;
        for l in text.lines().filter(|l| l.starts_with("rx ")) {
            let w: Vec<u32> = l.split(' ').skip(1).map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
            let v = rot_x(Vec3::new(0.25, f32::from_bits(w[0]), f32::from_bits(w[1])), f32::from_bits(w[2]));
            assert_eq!((v.y.to_bits(), v.z.to_bits()), (w[3], w[4]), "case {w:x?}");
            n += 1;
        }
        assert!(n >= 6000);
    }

    fn car() -> CarView {
        CarView { pos: Vec3::new(10.0, 2.0, -5.0), rot: Quat::from_rotation_y(0.7), vel: Vec3::new(3.0, 0.0, -12.0), top: 1.3 }
    }

    /// nothing in the way
    fn open(_: Vec3, _: Vec3, _: f32) -> Option<(Vec3, Vec3)> {
        None
    }

    /// a world that answers the polar camera's two sweeps (contact, normal) and keeps what it was asked
    struct PolarWorld {
        world: Option<(Vec3, Vec3)>,
        own: Option<(Vec3, Vec3)>,
        asked_own: Vec<(Vec3, Vec3, f32)>,
    }

    impl Probe for PolarWorld {
        fn sweep(&mut self, _: Vec3, _: Vec3, _: f32) -> Option<(Vec3, Vec3)> {
            self.world
        }
        fn own(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<(Vec3, Vec3)> {
            self.asked_own.push((a, b, r));
            self.own
        }
    }

    /// F-72161 -> F-38041: the car's own bound is swept only after the city hit, from 100 x
    /// (place - T) + T to the place, with the city sweep's ball; the places from the touches'
    /// contact and normal.
    #[test]
    fn polar_push_out() {
        let tune = CamTune::default();
        let c = car();
        let d = Director::new(&tune, 3, &c);
        let t = xform(TARGET, &HeliCar::of(&c).m);
        let r = d.near_r() + 0.25;
        let free = crate::aiv::rot_y(rot_x(POLAR_OFFSET, -d.polar.pitch), d.polar.yaw) + t;
        // no city hit: the car's bound is not asked
        let mut w = PolarWorld { world: None, own: Some((Vec3::ZERO, Vec3::Y)), asked_own: vec![] };
        let (p, _) = d.polar_place(&HeliCar::of(&c), &mut w);
        assert!(w.asked_own.is_empty());
        assert_eq!(p, free);
        // a city hit without a car hit: off the contact by the ball along the normal
        let (hit, n) = (t + Vec3::new(0.5, 0.2, 1.0), Vec3::new(0.0, 0.6, 0.8));
        let at = free + (n * r + (hit - free));
        let mut w = PolarWorld { world: Some((hit, n)), own: None, asked_own: vec![] };
        assert_eq!(d.polar_place(&HeliCar::of(&c), &mut w).0, at);
        assert_eq!(w.asked_own, vec![((at - t) * 100.0 + t, at, r)]);
        // both: the car's
        let (hit2, n2) = (t + Vec3::new(0.9, 0.4, 1.8), Vec3::Z);
        let mut w = PolarWorld { world: Some((hit, n)), own: Some((hit2, n2)), asked_own: vec![] };
        let (p, m) = d.polar_place(&HeliCar::of(&c), &mut w);
        assert_eq!(p, n2 * r + hit2);
        let mut want = look_rows(p - t, Vec3::Y);
        orthonormal(&mut want);
        assert_eq!(m, want);
    }

    /// The own-bound sweep on a hull: a place inside the car is moved to where the ball from
    /// outside first touches it; a place clear of it is left alone.
    #[test]
    fn own_hull_sweep() {
        use crate::physics::camprobe::CamObj;
        use crate::physics::hull::HullShape;
        let verts = vec![
            Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 0.0, -2.0), Vec3::new(1.0, 1.0, -2.0), Vec3::new(-1.0, 1.0, -2.0),
            Vec3::new(-1.0, 0.0, 2.0), Vec3::new(1.0, 0.0, 2.0), Vec3::new(1.0, 1.0, 2.0), Vec3::new(-1.0, 1.0, 2.0),
        ];
        // outward-facing (counter-clockwise seen from outside)
        let q = |a: u16, b: u16, c: u16, d: u16| [[a, b, c], [a, c, d]];
        let tris: Vec<[u16; 3]> = [q(0, 3, 2, 1), q(4, 5, 6, 7), q(0, 1, 5, 4), q(3, 7, 6, 2), q(0, 4, 7, 3), q(1, 2, 6, 5)].concat();
        let hull = HullShape::new(verts, &tris);
        let o = CamObj::of_hull(&hull, Vec3::ZERO, Quat::IDENTITY);
        let t = Vec3::new(0.0, 0.65, 0.0);
        let r = 0.3;
        // a place inside the box, behind it: the ball stops 0.3 m behind the rear face
        let place = Vec3::new(0.0, 0.65, 1.5);
        let (s, n) = o.sweep(t + (place - t) * 100.0, place, r).unwrap();
        let a = t + (place - t) * 100.0;
        let got = a + (place - a) * s;
        assert!((got.z - 2.3).abs() < 1e-3 && n.z > 0.99, "{got:?} {n:?}");
        // a place 1 m clear of the box: no touch
        let place = Vec3::new(0.0, 0.65, 3.3);
        assert!(o.sweep(t + (place - t) * 100.0, place, r).is_none());
    }

    #[test]
    fn shot_lengths_are_normal_and_clamped() {
        let mut r = Mwc::new(7);
        let v: Vec<f32> = (0..20000).map(|_| r.length([5.0, 3.0, 1.0, 10.0])).collect();
        assert!(v.iter().all(|x| (1.0..=10.0).contains(x)));
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean - 5.0).abs() < 0.15, "mean {mean}");
        // (a normal with sd 3 clamped at 1: about 9 % of the draws at the floor)
        let floor = v.iter().filter(|x| **x == 1.0).count() as f32 / v.len() as f32;
        assert!((floor - 0.091).abs() < 0.02, "floor {floor}");
    }

    #[test]
    fn mounts_look_down_the_car() {
        let t = CamTune::default();
        let c = car();
        let mut d = Director::new(&t, 3, &c);
        let mut probe = open;
        for k in 0..6 {
            for mirror in 0..2 {
                d.force(&t, &c, mount_state(k) + mirror);
                let Shot::Own { eye, rot, fov } = d.shot(&t, &c, &mut probe) else { panic!() };
                let mut o = t.mounts[k];
                if mirror == 1 {
                    o.x = -o.x;
                }
                assert!((eye - (c.pos + c.rot * o)).length() < 1e-4);
                let look = -(rot * Vec3::Z);
                let fwd = c.rot * -Vec3::Z;
                // (HoodLookback looks back at the car, the others forward)
                assert!(look.dot(if k == 5 { -fwd } else { fwd }) > 0.999, "k {k} look {look:?}");
                assert_eq!(fov, 65.0);
            }
        }
    }

    #[test]
    fn choice_and_next_camera() {
        let t = CamTune::default();
        let c = car();
        let mut d = Director::new(&t, 11, &c);
        assert_eq!(d.state, 0);
        let mut probe = open;
        let mut seen = [0usize; STATES];
        let mut last = d.state;
        for _ in 0..4000 {
            d.time = -1.0;
            d.update(&t, &c, 1.0 / 30.0, &mut probe);
            assert_ne!(d.state, last, "a timed-out shot always changes");
            assert_ne!(d.state, POLAR, "the polar camera is never drawn");
            assert!(!((2..=4).contains(&last) && (2..=4).contains(&d.state)), "chase after chase");
            seen[d.state] += 1;
            last = d.state;
        }
        assert!(seen[..POLAR].iter().all(|n| *n > 50), "{seen:?}");
        // NEXT CAMERA: the next state in order, the polar one included
        d.force(&t, &c, 17);
        for want in [HELI, POLAR, 0, 1] {
            d.next = true;
            d.update(&t, &c, 1.0 / 30.0, &mut probe);
            assert_eq!(d.state, want);
        }
    }

    #[test]
    fn blocked_shots_are_passed_over() {
        let t = CamTune::default();
        let c = car();
        let mut d = Director::new(&t, 5, &c);
        // a wall everywhere: only the drive views stay (they take no test)
        let mut wall = |a: Vec3, _: Vec3, _: f32| Some((a, Vec3::Y));
        for _ in 0..300 {
            d.time = -1.0;
            d.update(&t, &c, 1.0 / 30.0, &mut wall);
        }
        // (six tries; the sixth is kept whatever it shows - a mount can still come up)
        let mut drive = 0;
        for _ in 0..300 {
            d.time = -1.0;
            d.update(&t, &c, 1.0 / 30.0, &mut wall);
            // (the reverse camera has no place before its first frame: taken as well)
            drive += (d.state <= REVERSE) as usize;
        }
        assert!(drive > 225, "{drive}");
    }

    #[test]
    fn helicopter_place() {
        let t = CamTune::default();
        let c = CarView { rot: Quat::IDENTITY, vel: Vec3::ZERO, ..car() };
        let mut d = Director::new(&t, 9, &c);
        let mut probe = open;
        d.force(&t, &c, HELI);
        let h = d.heli.clone();
        let rel = h.place - c.at(TARGET);
        // 30 m up, 25 m out at 4.5 .. 157.5 degrees off straight behind (+z)
        assert!((rel.y - 30.0).abs() < 1e-3);
        let a = rel.x.atan2(rel.z).abs();
        assert!((0.0785..=2.75).contains(&a), "{a}");
        assert!((t.heli_fov[0]..=t.heli_fov[1]).contains(&h.fov));
        // it looks at the car
        let Shot::Own { eye, rot, .. } = d.shot(&t, &c, &mut probe) else { panic!() };
        let to = (c.at(TARGET) - eye).normalize();
        assert!((-(rot * Vec3::Z)).dot(to) > 0.999);
    }
}

/// F-66400 as the replay director's shot lengths use it (`Mwc::normal`) against the original's own code run offline
/// (tools/random_normal_reference.py -> testdata/random_normal_ref.txt, 288 sequences of 16 calls).
#[cfg(test)]
mod original_normal_tests {
    use super::*;

    #[test]
    fn normal_deviate_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut seqs = 0;
        for line in include_str!("testdata/random_normal_ref.txt").lines() {
            let (a, b) = line.strip_prefix("NRM ").unwrap().split_once(" -> ").unwrap();
            let (head, calls) = a.split_once(" | ").unwrap();
            let hd: Vec<&str> = head.split(' ').collect();
            let mut g = Mwc { x: h(hd[0]), c: h(hd[1]), spare: None };
            let w: Vec<&str> = calls.split(' ').collect();
            for (i, row) in b.split(" | ").enumerate() {
                let r = g.normal(f32::from_bits(h(w[2 * i])), f32::from_bits(h(w[2 * i + 1])));
                let o: Vec<&str> = row.split(' ').collect();
                assert_eq!(r.to_bits(), h(o[0]), "call {i} of {line}");
                assert_eq!((g.x, g.c), (h(o[3]), h(o[4])), "generator, call {i} of {line}");
                assert_eq!(g.spare.map(f32::to_bits), if o[1] == "1" { Some(h(o[2])) } else { None }, "spare, call {i} of {line}");
            }
            seqs += 1;
        }
        assert_eq!(seqs, 288);
    }
}


/// The director's small routines against the original's code (tools/replay_director_reference.py, testdata/replay_director_ref.txt):
/// the shot length F-66204 and the helicopter's zoom timer F-16218 (300 sequences of 12 calls, generator and spare after each),
/// the k-th set bit F-48190 (4,000 masks), the polar camera's stick F-63907 (6,000 cases from the pad's bytes) and its activate
/// F-82095 (3,000 generators), the fixed mounts' camera F-77143 (3,000 car matrices, a third looking back) and the mount
/// setter F-31661 (3,000 states and mount tables; outside 6..17 the original only clears the look-back byte, where the port
/// does not call it).
#[cfg(test)]
mod original_director_tests {
    use super::*;

    fn h(x: &str) -> u32 {
        u32::from_str_radix(x, 16).unwrap()
    }

    #[test]
    fn director_routines_match_the_original() {
        let (mut len, mut kth, mut pol, mut pact, mut fix, mut mnt) = (0, 0, 0, 0, 0, 0);
        for line in include_str!("testdata/replay_director_ref.txt").lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            match w[0] {
                "LEN" | "ZOOM" => {
                    let mut g = Mwc { x: h(w[1]), c: h(w[2]), spare: None };
                    let calls = &w[4..];
                    for (i, row) in b.split(" | ").enumerate() {
                        let c = [0, 1, 2, 3].map(|j| f32::from_bits(h(calls[4 * i + j])));
                        let r = g.length(c);
                        let o: Vec<&str> = row.split(' ').collect();
                        assert_eq!(r.to_bits(), h(o[0]), "call {i} of {line}");
                        assert_eq!((g.x, g.c), (h(o[3]), h(o[4])), "generator, call {i} of {line}");
                        assert_eq!(g.spare.map(f32::to_bits), if o[1] == "1" { Some(h(o[2])) } else { None }, "spare, call {i} of {line}");
                    }
                    len += 1;
                }
                "KTH" => {
                    assert_eq!(kth_bit(h(w[1]), h(w[2])), b.parse::<i32>().unwrap(), "{line}");
                    kth += 1;
                }
                "POL" => {
                    let f: Vec<f32> = w[1..7].iter().map(|x| f32::from_bits(h(x))).collect();
                    let by: Vec<u8> = w[7..11].iter().map(|x| h(x) as u8).collect();
                    let mut tune = CamTune::default();
                    tune.polar_fov_range[0] = f[3];
                    tune.polar_fov_range[1] = f[4];
                    let car = CarView { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, top: 1.0 };
                    let mut d = Director::new(&tune, 1, &car);
                    d.state = POLAR;
                    d.polar = Polar { pitch: f[1], yaw: f[2], fov: f[0] };
                    let dead = |b: u8| crate::boomcam::dead_zone(stick_byte(b), 0.3);
                    let k = 1.0 / 255.0;
                    let zoom = (by[2] as f32).mul_add(k, -(by[3] as f32 * k));
                    d.polar_stick(&tune, dead(by[0]), dead(by[1]), zoom, f[5]);
                    let o: Vec<u32> = b.split(' ').map(h).collect();
                    assert_eq!([d.polar.fov.to_bits(), d.polar.pitch.to_bits(), d.polar.yaw.to_bits()], [o[0], o[1], o[2]], "{line}");
                    pol += 1;
                }
                "FIX" => {
                    let v: Vec<f32> = w[1..16].iter().map(|x| f32::from_bits(h(x))).collect();
                    let r = |i: usize| Vec3::new(v[i], v[i + 1], v[i + 2]);
                    let car = HeliCar { m: [r(0), r(3), r(6), r(9)], vel: Vec3::ZERO };
                    let (m, eye) = fixed_rows(r(12), w[16] == "1", &car);
                    let got: Vec<u32> = m.iter().chain([eye].iter()).flat_map(|x| x.to_array().map(f32::to_bits)).collect();
                    let want: Vec<u32> = b.split_whitespace().map(h).collect();
                    assert_eq!(got, want, "{line}");
                    fix += 1;
                }
                "PACT" => {
                    let mut tune = CamTune::default();
                    tune.polar_fov_range[2] = f32::from_bits(h(w[3]));
                    let car = CarView { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, top: 1.0 };
                    let mut d = Director::new(&tune, 1, &car);
                    d.rng = Mwc { x: h(w[1]), c: h(w[2]), spare: None };
                    d.polar_activate(&tune);
                    let o: Vec<u32> = b.split(' ').map(h).collect();
                    assert_eq!([d.polar.fov.to_bits(), d.polar.pitch.to_bits(), d.polar.yaw.to_bits(), d.rng.x, d.rng.c], [o[0], o[1], o[2], o[3], o[4]], "{line}");
                    pact += 1;
                }
                "MNT" => {
                    let state = h(w[1]) as usize;
                    let v: Vec<f32> = w[2..20].iter().map(|x| f32::from_bits(h(x))).collect();
                    let mut tune = CamTune::default();
                    for k in 0..6 {
                        tune.mounts[k] = Vec3::new(v[3 * k], v[3 * k + 1], v[3 * k + 2]);
                    }
                    let o: Vec<u32> = b.split(' ').map(h).collect();
                    if (6..=17).contains(&state) {
                        let (m, back) = Director::mount(&tune, state);
                        assert_eq!([m.x.to_bits(), m.y.to_bits(), m.z.to_bits(), back as u32], [o[0], o[1], o[2], o[3]], "{line}");
                    } else {
                        assert_eq!(o, [0xA5A5_A5A5, 0xA5A5_A5A5, 0xA5A5_A5A5, 0], "{line}");
                    }
                    mnt += 1;
                }
                _ => panic!("{line}"),
            }
        }
        assert!(len >= 290 && kth >= 3980 && pol >= 5990 && pact >= 2970 && fix >= 2970 && mnt >= 2970, "{len} {kth} {pol} {pact} {fix} {mnt}");
    }

    /// The helicopter's activate F-79727 and update F-48462 against the original's code (tools/replay_heli_reference.py,
    /// testdata/replay_heli_ref.txt): a moving car, 25 steps a sequence, the camera's whole state and the generator after each.
    /// The frame update F-19930 and choose F-53721 against the original's code (tools/replay_choose_reference.py,
    /// testdata/replay_choose_ref.txt): 40 frames of a set-up director per case (the state, the shot's time, the generator,
    /// the helicopter's and the polar camera's fields after each), with the shot test F-51014 answering from a list in both
    /// (its calls and whether they look ahead compared), and the TRANSITIONMOVIE a cut to bumper / cockpit starts (main.rs).
    #[test]
    fn choose_matches_the_original() {
        let mut cases = 0;
        for line in include_str!("testdata/replay_choose_ref.txt").lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split_whitespace().collect();
            let f = |x: &str| f32::from_bits(h(x));
            let v: Vec<f32> = w[4..44].iter().map(|x| f(x)).collect();
            let mut tune = CamTune::default();
            tune.cut = [v[0], v[1], v[2], v[3]];
            tune.heli_fov[0] = v[4];
            tune.heli_fov[1] = v[5];
            tune.heli_match = [v[6], v[7], v[8]];
            tune.heli_rates = [v[9], v[10]];
            tune.heli_cut = [v[11], v[12], v[13], v[14]];
            tune.polar_fov_range[2] = v[15];
            for k in 0..6 {
                tune.mounts[k] = Vec3::new(v[22 + 3 * k], v[23 + 3 * k], v[24 + 3 * k]);
            }
            let car = CarView { pos: Vec3::new(v[16], v[17], v[18]), rot: Quat::IDENTITY, vel: Vec3::new(v[19], v[20], v[21]), top: 1.0 };
            let mut d = Director::new(&tune, 1, &car);
            d.rng = Mwc { x: h(w[1]), c: h(w[2]), spare: None };
            d.polar_activate(&tune);
            d.heli_activate(&tune, &car);
            d.time = d.rng.length(tune.cut);
            d.state = h(w[3]) as usize;
            let n: usize = w[44].parse().unwrap();
            let answers: Vec<bool> = w[45..45 + n].iter().map(|x| *x == "1").collect();
            let mut qi = 0;
            let nf: usize = w[45 + n].parse().unwrap();
            let fr = &w[46 + n..];
            let rows: Vec<&str> = b.split(" | ").collect();
            assert_eq!(rows.len(), nf);
            for (s, row) in rows.iter().enumerate() {
                d.switching = fr[3 * s + 1] == "1";
                if fr[3 * s + 2] == "1" {
                    d.next = true;
                }
                let mut ev: Vec<String> = Vec::new();
                d.direct(&tune, &car, f(fr[3 * s]), &mut |d: &mut Director, ahead| {
                    ev.push(format!("S{},{}", d.state, ahead as u8));
                    qi += 1;
                    answers.get(qi - 1).copied().unwrap_or(true)
                });
                if d.cut && d.state < 2 {
                    ev.push(format!("T{:08x},{:08x},{:08x}", 0.01f32.to_bits(), 0.25f32.to_bits(), 0.01f32.to_bits()));
                }
                let o: Vec<&str> = row.split(' ').filter(|x| !x.is_empty()).collect();
                let want_ev: Vec<&str> = o.iter().copied().filter(|x| x.starts_with('S') || x.starts_with('T')).collect();
                assert_eq!(ev, want_ev, "frame {s} of {line}");
                let o: Vec<&str> = o.into_iter().filter(|x| !x.starts_with(['S', 'T', 'D', 'R'])).collect();
                let hl = &d.heli;
                let got = [
                    d.state as u32, d.time.to_bits(), d.next as u32, d.rng.x, d.rng.c, d.rng.spare.is_some() as u32, d.rng.spare.map_or(0, f32::to_bits),
                    hl.fov.to_bits(), hl.angle.to_bits(), hl.side as u32, hl.zoom.to_bits(), hl.place.x.to_bits(), hl.place.y.to_bits(), hl.place.z.to_bits(),
                    d.polar.fov.to_bits(), d.polar.pitch.to_bits(), d.polar.yaw.to_bits(),
                ];
                let want: Vec<u32> = o.iter().map(|x| h(x)).collect();
                // (the spare's value is only compared while it is held)
                let mut want = want;
                if want[5] == 0 {
                    want[6] = 0;
                }
                assert_eq!(got.to_vec(), want, "frame {s} of {line}");
            }
            cases += 1;
        }
        assert!(cases >= 380, "{cases}");
    }

    /// The shot test F-51014 against the original's code (tools/replay_shot_reference.py, testdata/replay_shot_ref.txt): the
    /// fixed and helicopter states, the probes answering from a list in both; what each probe is given (the ball's place and
    /// radius from the viewport's lens, the look-ahead's end, the helicopter's look at the car's top), the answer, the generator and
    /// the helicopter's lens and zoom timer. (The include mask is logged by the original only: 0x300 below 22.35 m/s, else 0x100;
    /// no object is known to carry 0x200.)
    #[test]
    fn shot_test_matches_the_original() {
        struct Rec<'a> {
            ans: &'a [bool],
            calls: Vec<String>,
        }
        impl Rec<'_> {
            fn take(&mut self, a: Vec3) -> Option<(Vec3, Vec3)> {
                let i = self.calls.len() - 1;
                self.ans.get(i).copied().unwrap_or(false).then_some((a, Vec3::Y))
            }
        }
        impl Probe for Rec<'_> {
            fn sweep(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<(Vec3, Vec3)> {
                let k = if self.calls.is_empty() { format!("P{:08x},{:08x},{:08x},{:08x}", a.x.to_bits(), a.y.to_bits(), a.z.to_bits(), r.to_bits()) } else {
                    format!("W{:08x},{:08x},{:08x},{:08x},{:08x},{:08x},{:08x}", a.x.to_bits(), a.y.to_bits(), a.z.to_bits(), b.x.to_bits(), b.y.to_bits(), b.z.to_bits(), r.to_bits())
                };
                self.calls.push(k);
                self.take(a)
            }
            fn heli(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<(Vec3, Vec3)> {
                self.calls.push(format!("H{:08x},{:08x},{:08x},{:08x},{:08x},{:08x},{:08x}", a.x.to_bits(), a.y.to_bits(), a.z.to_bits(), b.x.to_bits(), b.y.to_bits(), b.z.to_bits(), r.to_bits()));
                self.take(a)
            }
        }
        let mut cases = 0;
        for line in include_str!("testdata/replay_shot_ref.txt").lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split_whitespace().collect();
            let f = |x: &str| f32::from_bits(h(x));
            let state = h(w[1]) as usize;
            let ahead = w[2] == "1";
            let v: Vec<f32> = w[3..35].iter().map(|x| f(x)).collect();
            let ht: Vec<f32> = w[37..49].iter().map(|x| f(x)).collect();
            let mut tune = CamTune::default();
            tune.cut = [v[10], v[11], v[12], v[13]];
            for k in 0..6 {
                tune.mounts[k] = Vec3::new(v[14 + 3 * k], v[15 + 3 * k], v[16 + 3 * k]);
            }
            tune.heli_fov[0] = ht[0];
            tune.heli_fov[1] = ht[1];
            tune.heli_match = [ht[2], ht[3], ht[4]];
            tune.heli_rates = [ht[5], ht[6]];
            tune.heli_cut = [ht[7], ht[8], ht[9], ht[10]];
            let car = CarView { pos: Vec3::new(v[2], v[3], v[4]), rot: Quat::IDENTITY, vel: Vec3::new(v[5], v[6], v[7]), top: v[8] };
            let mut d = Director::new(&tune, 1, &car);
            d.rng = Mwc { x: h(w[35]), c: h(w[36]), spare: None };
            if state == HELI {
                d.heli_activate(&tune, &car);
            }
            d.state = state;
            d.lens = v[0];
            d.aspect = v[1];
            d.cut = false;
            let n: usize = w[49].parse().unwrap();
            let ans: Vec<bool> = w[50..50 + n].iter().map(|x| *x == "1").collect();
            let mut rec = Rec { ans: &ans, calls: Vec::new() };
            let ok = d.shot_ok(&tune, &car, ahead, ht[11], &mut rec);
            let o: Vec<&str> = b.split_whitespace().collect();
            // (the original's calls without their include masks)
            let want: Vec<String> = o.iter().filter(|x| x.starts_with(['P', 'W', 'H'])).map(|x| x.rsplit_once(',').unwrap().0.to_string()).collect();
            assert_eq!(rec.calls, want, "{line}");
            let rest: Vec<u32> = o.iter().filter(|x| !x.starts_with(['P', 'W', 'H'])).map(|x| h(x)).collect();
            let got = [ok as u32, d.rng.x, d.rng.c, if state == HELI { d.heli.fov.to_bits() } else { 0 }, if state == HELI { d.heli.zoom.to_bits() } else { 0 }];
            assert_eq!(got.to_vec(), rest, "{line}");
            cases += 1;
        }
        assert!(cases >= 3900, "{cases}");
    }

    /// The polar camera's update F-72161 with its world collision F-67414 against the original's code
    /// (tools/replay_polar_reference.py, testdata/replay_polar_ref.txt): 6,000 car matrices, lenses, pitches and yaws, the
    /// sweep answering with a contact and normal (or nothing) in both; the sweep's arguments, whether the push-out F-38041 is
    /// asked, the rows shown and the place. (The push-out itself is stubbed: it leaves the place.)
    #[test]
    fn polar_update_matches_the_original() {
        struct W {
            hit: Option<(Vec3, Vec3)>,
            calls: Vec<String>,
        }
        impl Probe for W {
            fn sweep(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<(Vec3, Vec3)> {
                self.calls.push(format!("W{:08x},{:08x},{:08x},{:08x},{:08x},{:08x},{:08x}", a.x.to_bits(), a.y.to_bits(), a.z.to_bits(), b.x.to_bits(), b.y.to_bits(), b.z.to_bits(), r.to_bits()));
                self.hit
            }
            fn own(&mut self, _: Vec3, _: Vec3, _: f32) -> Option<(Vec3, Vec3)> {
                self.calls.push("O".into());
                None
            }
        }
        let mut cases = 0;
        for line in include_str!("testdata/replay_polar_ref.txt").lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split_whitespace().collect();
            let v: Vec<f32> = w[1..].iter().map(|x| f32::from_bits(h(x))).collect();
            let r3 = |i: usize| Vec3::new(v[i], v[i + 1], v[i + 2]);
            let car = HeliCar { m: [r3(4), r3(7), r3(10), r3(13)], vel: Vec3::ZERO };
            let hit = (w[17] == "1").then(|| (r3(17), r3(20)));
            let tune = CamTune::default();
            let cv = CarView { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, top: 1.0 };
            let mut d = Director::new(&tune, 1, &cv);
            d.lens = v[0];
            d.aspect = v[1];
            d.polar.pitch = v[2];
            d.polar.yaw = v[3];
            let mut probe = W { hit, calls: Vec::new() };
            let (place, m) = d.polar_place(&car, &mut probe);
            let o: Vec<&str> = b.split_whitespace().collect();
            let want_calls: Vec<&str> = o.iter().copied().filter(|x| x.starts_with(['W', 'O'])).collect();
            assert_eq!(probe.calls, want_calls, "{line}");
            let want: Vec<u32> = o.iter().filter(|x| !x.starts_with(['W', 'O'])).map(|x| h(x)).collect();
            let mut got: Vec<u32> = m.iter().chain([place].iter()).flat_map(|x| x.to_array().map(f32::to_bits)).collect();
            got.push(hit.is_some() as u32);
            assert_eq!(got, want, "{line}");
            cases += 1;
        }
        assert!(cases >= 5900, "{cases}");
    }

    #[test]
    fn heli_matches_the_original() {
        let mut seqs = 0;
        for line in include_str!("testdata/replay_heli_ref.txt").lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let f = |x: &str| f32::from_bits(h(x));
            let mut g = Mwc { x: h(w[1]), c: h(w[2]), spare: None };
            let t: Vec<f32> = w[3..14].iter().map(|x| f(x)).collect();
            let mut tune = CamTune::default();
            tune.heli_fov[0] = t[0];
            tune.heli_fov[1] = t[1];
            tune.heli_match = [t[2], t[3], t[4]];
            tune.heli_rates = [t[5], t[6]];
            tune.heli_cut = [t[7], t[8], t[9], t[10]];
            let n: usize = w[14].parse().unwrap();
            let mut heli = Heli::default();
            for (s, row) in b.split(" | ").enumerate().take(n) {
                let st: Vec<f32> = w[15 + 16 * s..15 + 16 * (s + 1)].iter().map(|x| f(x)).collect();
                let v = |i: usize| Vec3::new(st[i], st[i + 1], st[i + 2]);
                let car = HeliCar { m: [v(1), v(4), v(7), v(10)], vel: v(13) };
                if s == 0 {
                    heli = Heli::activate(&mut g, &tune, &car);
                } else {
                    heli.update(&mut g, &tune, &car, st[0]);
                }
                let o: Vec<&str> = row.split(' ').collect();
                let mut got: Vec<u32> = Vec::new();
                for r in heli.m.iter().chain([heli.place, heli.vel].iter()) {
                    got.extend(r.to_array().map(f32::to_bits));
                }
                got.extend([heli.fov, heli.goal, heli.angle, heli.zoom].map(f32::to_bits));
                let want: Vec<u32> = o[..19].iter().map(|x| h(x)).collect();
                assert_eq!(got, want, "step {s} of sequence {seqs}");
                assert_eq!(heli.side, o[19] == "1", "side, step {s} of sequence {seqs}");
                assert_eq!((g.x, g.c), (h(o[20]), h(o[21])), "generator, step {s} of sequence {seqs}");
                assert_eq!(g.spare.map(f32::to_bits), if o[22] == "1" { Some(h(o[23])) } else { None }, "spare, step {s} of sequence {seqs}");
            }
            seqs += 1;
        }
        assert!(seqs >= 110, "{seqs}");
    }
}
