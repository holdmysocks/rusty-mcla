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
    /// the heading the helicopter turns its offset by: acos(c.z), negative when c.x < 0
    fn heading(&self) -> f32 {
        let c = self.c();
        let h = c.z.clamp(-1.0, 1.0).acos();
        if c.x < 0.0 { -h } else { h }
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
    /// [0, 1)
    fn unit(&mut self) -> f32 {
        self.u23() * 1.192_09e-7
    }
    fn normal(&mut self) -> f32 {
        if let Some(s) = self.spare.take() {
            return s;
        }
        loop {
            let u = self.u23() * 2.384_19e-7 - 1.0;
            let v = self.u23() * 2.384_19e-7 - 1.0;
            let s = v * v + u * u;
            if s < 1.0 && s != 0.0 {
                let k = (s.ln() / s * -2.0).sqrt();
                self.spare = Some(k * u);
                return k * v;
            }
        }
    }
    /// a shot's length: average, variance, least, most
    fn length(&mut self, c: [f32; 4]) -> f32 {
        (self.normal() * c[1] + c[0]).max(c[2]).min(c[3])
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

/// about y (F-57591): x' = x cos + z sin, z' = z cos - x sin
fn rot_y(v: Vec3, a: f32) -> Vec3 {
    let (s, c) = a.sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
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

/// mcReplayCameraHeli (state 18).
#[derive(Clone, Debug, Default)]
struct Heli {
    place: Vec3,
    vel: Vec3,
    rot: Quat,
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
/// `r` from `a` to `b` through the city and gives the last place it is free when it touches.
pub trait Probe {
    fn sweep(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<Vec3>;
}

impl<F: FnMut(Vec3, Vec3, f32) -> Option<Vec3>> Probe for F {
    fn sweep(&mut self, a: Vec3, b: Vec3, r: f32) -> Option<Vec3> {
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
    /// the drive view's place the frame before (the test of the reverse camera)
    pub drive_eye: Option<Vec3>,
}

impl Director {
    /// as mcPlayerCamera's reset leaves it (F-80435): BUMPER, switching on
    pub fn new(tune: &CamTune, seed: u32, car: &CarView) -> Director {
        let mut d = Director { state: 0, time: 0.0, switching: true, next: false, rng: Mwc::new(seed), heli: Heli::default(), polar: Polar::default(), cut: true, aspect: 16.0 / 9.0, drive_eye: None };
        d.polar_activate(tune);
        d.heli_activate(tune, car);
        d.time = d.rng.length(tune.cut);
        d
    }

    /// the ball the tests sweep: the near plane's corner
    fn near_r(&self, fov: f32) -> f32 {
        let t = (fov.to_radians() * 0.5).tan();
        NEAR * (t * t + (t * self.aspect).powi(2)).sqrt()
    }

    fn polar_activate(&mut self, tune: &CamTune) {
        // (F-82095: a side, then 22.5 .. 67.5 degrees)
        let sign = if self.rng.u23() * 2.384_19e-7 - 1.0 >= 0.0 { 1.0 } else { -1.0 };
        let a = self.rng.u23() * 9.362_68e-8 + 0.392_699;
        self.polar = Polar { pitch: 0.0, yaw: a * sign, fov: tune.polar_fov_range[2] };
    }

    fn heli_activate(&mut self, tune: &CamTune, car: &CarView) {
        // (F-79727: the polar camera's first, then its own)
        self.polar_activate(tune);
        let t = car.at(TARGET);
        let angle = 0.078_539_8 + self.rng.u23() * 3.183_31e-7;
        let (lo, hi) = (tune.heli_fov[0], tune.heli_fov[1]);
        let fov = lo + (hi - lo) * self.rng.unit();
        let side = self.rng.next() & 1 != 0;
        let a = if side { angle } else { -angle };
        let place = t + rot_y(HELI_OFFSET, car.heading() + a);
        let zoom = self.rng.length(tune.heli_cut);
        self.heli = Heli { place, vel: car.vel, rot: look_rot(place - t), fov, goal: fov, angle, side, zoom };
    }

    fn heli_update(&mut self, tune: &CamTune, car: &CarView, dt: f32) {
        let h = &mut self.heli;
        h.zoom -= dt;
        if h.zoom < 0.0 {
            h.zoom = self.rng.length(tune.heli_cut);
            // (a new goal away from the old one: one of the lens range's halves)
            let (lo, hi) = (tune.heli_fov[0], tune.heli_fov[1]);
            let half = (hi - lo) * 0.5;
            let mut g = lo + half * self.rng.unit();
            if g > h.goal - (hi - lo) * 0.25 {
                g += half;
            }
            h.goal = g;
        }
        let t = car.at(TARGET);
        let a_row = car.rot * Vec3::X;
        h.side = a_row.dot(h.vel + (h.place - car.pos)) > 0.0;
        let a = if h.side { h.angle } else { -h.angle };
        let want = t + rot_y(HELI_OFFSET, car.heading() + a);
        let mut acc = (car.vel - h.vel) * tune.heli_match[0] + (want - h.place) * tune.heli_match[1] - h.vel * tune.heli_match[2];
        let n = acc.length();
        acc *= HELI_ACCEL / n.max(HELI_ACCEL);
        h.vel += acc * dt;
        h.place += h.vel * dt;
        let goal = look_rot(h.place - t);
        h.rot = h.rot.slerp(goal, (tune.heli_rates[1] * dt).min(1.0));
        if h.fov < h.goal {
            h.fov = (h.fov + tune.heli_rates[0] * dt).min(h.goal);
        } else if h.fov > h.goal {
            h.fov = (h.fov - tune.heli_rates[0] * dt).max(h.goal);
        }
    }

    /// the polar camera's place: about the car in the world's frame, pushed in by the city
    fn polar_place(&self, car: &CarView, probe: &mut dyn Probe) -> (Vec3, Quat) {
        let t = car.at(TARGET);
        let mut place = t + rot_y(rot_x(POLAR_OFFSET, -self.polar.pitch), self.polar.yaw);
        if let Some(p) = probe.sweep(t, place, self.near_r(self.polar.fov) + 0.25) {
            place = p;
        }
        (place, look_rot(place - t))
    }

    /// The right stick and the triggers while the polar camera is on (F-63907): x, y in
    /// -1..1 past the dead zone, `zoom` = (LT - RT) in -1..1.
    pub fn polar_stick(&mut self, tune: &CamTune, x: f32, y: f32, zoom: f32, dt: f32) {
        if self.state != POLAR {
            return;
        }
        let p = &mut self.polar;
        p.pitch = (p.pitch - y * 1.25 * dt).clamp(0.0, 1.256_64);
        p.yaw += x * 2.0 * dt;
        p.fov = (p.fov * (1.0 + zoom * dt)).clamp(tune.polar_fov_range[0], tune.polar_fov_range[1]);
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
                // (F-77143: down the car's length, level about its line of sight)
                let (o, back) = Self::mount(tune, state);
                let c = if back { -car.c() } else { car.c() };
                Some((car.at(o), look_rot(c), tune.fixed_fov))
            }
            HELI => Some((self.heli.place, self.heli.rot, self.heli.fov)),
            POLAR => {
                let (p, r) = self.polar_place(car, probe);
                Some((p, r, self.polar.fov))
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
        let (eye, fov) = match self.own(tune, car, self.state, probe) {
            Some((e, _, f)) => (e, f),
            // (the reverse camera: where it stood the frame before; none yet - taken)
            None => match self.drive_eye {
                Some(e) if !self.cut => (e, 60.0),
                _ => return true,
            },
        };
        let r = self.near_r(fov);
        if probe.sweep(eye, eye, r).is_some() {
            return false;
        }
        if ahead && probe.sweep(eye, eye + car.vel * tune.cut[2], r).is_some() {
            return false;
        }
        if self.state == HELI && probe.sweep(eye, car.pos + Vec3::Y * car.top, 0.5).is_some() {
            return false;
        }
        true
    }

    /// choose() (F-53721): NEXT CAMERA steps to the next state; otherwise any other
    /// state but the polar one with equal chance (a chase view never after a chase view),
    /// up to six tries until one passes its test.
    fn choose(&mut self, tune: &CamTune, car: &CarView, dt: f32, probe: &mut dyn Probe) {
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
            } else if self.state == HELI && tries > 3 && self.time >= 0.0 {
                self.heli_activate(tune, car);
            } else {
                mask &= !(1 << self.state);
                let n = mask.count_ones().max(1);
                let k = (self.rng.next() & 0x7FFF_FFFF) % n;
                let mut left = k;
                self.state = (0..STATES).find(|&s| mask & (1 << s) != 0 && { let hit = left == 0; if !hit { left -= 1 } hit }).unwrap_or(0);
                self.activate(tune, car);
            }
            let t = tries;
            tries -= 1;
            if t <= 0 || self.state < 5 || self.shot_ok(tune, car, true, dt, probe) {
                break;
            }
        }
        self.next = false;
        self.time = self.rng.length(tune.cut);
    }

    /// One frame (F-19930); `dt` 0 while the replay is paused.
    pub fn update(&mut self, tune: &CamTune, car: &CarView, dt: f32, probe: &mut dyn Probe) -> Shot {
        self.cut = false;
        if self.switching && !self.next {
            self.time -= dt;
        } else if self.next {
            self.time = self.rng.length(tune.cut);
        }
        let ok = !self.next && self.time >= 0.0 && (self.state < 5 || self.shot_ok(tune, car, false, dt, probe));
        if !ok {
            self.choose(tune, car, dt, probe);
        }
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
        let dead = |v: f32| {
            let v = v.clamp(-1.0, 1.0);
            if v > 0.3 { (v - 0.3) / 0.7 } else if v < -0.3 { (v + 0.3) / 0.7 } else { 0.0 }
        };
        if let Some(d) = self.director.as_mut() {
            d.polar_stick(&self.tune, dead(x), dead(y), zoom, dt);
        }
    }

    /// The camera of this frame (None while the opening runs: the race start's own cameras).
    /// `car`: the player's car. The director's time is the replay clock's step since its last
    /// frame.
    pub fn camera(&mut self, car: &CarView, aspect: f32, probe: &mut dyn Probe) -> Option<Shot> {
        if self.opening != 0 {
            self.last_t = None;
            return None;
        }
        let dt = self.last_t.map_or(0.0, |l| (self.t - l).max(0.0));
        self.last_t = Some(self.t);
        let tune = &self.tune;
        let d = self.director.get_or_insert_with(|| Director::new(tune, self.seed, car));
        d.aspect = aspect;
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
    fn open(_: Vec3, _: Vec3, _: f32) -> Option<Vec3> {
        None
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
        let mut wall = |a: Vec3, _: Vec3, _: f32| Some(a);
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
