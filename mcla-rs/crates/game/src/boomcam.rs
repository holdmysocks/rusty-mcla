//! The game's driving cameras (`camBoomCS`, tune/camera/<car>_boom1..3, _POV, _bumper,
//! _reverse): a boom from a focus point on the car to the camera, in a frame that follows
//! the car's heading and the ground with a lag. The boom is bent by how the car moves:
//! its speed, turn rate and accelerations, each normalised by a range from the file, move
//! the camera and the point it looks at, and a gear change kicks it. The lens widens and
//! the boom shortens with speed.
use glam::{Mat3, Quat, Vec3};
use std::collections::HashMap;
use std::path::Path;

/// One camera's parameters (the text file's fields by name).
#[derive(Clone, Debug)]
pub struct BoomTune {
    f: HashMap<String, Vec<f32>>,
}

impl BoomTune {
    pub fn parse(text: &str) -> Option<BoomTune> {
        if !text.contains("camBoomCS") {
            return None;
        }
        let mut f = HashMap::new();
        for l in text.lines() {
            let mut it = l.split_whitespace();
            let Some(k) = it.next() else { continue };
            let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
            if !v.is_empty() {
                f.insert(k.to_string(), v);
            }
        }
        Some(BoomTune { f })
    }
    fn s(&self, k: &str) -> f32 {
        self.f.get(k).and_then(|v| v.first()).copied().unwrap_or(0.0)
    }
    fn v3(&self, k: &str) -> Vec3 {
        match self.f.get(k) {
            Some(v) if v.len() >= 3 => Vec3::new(v[0], v[1], v[2]),
            _ => Vec3::ZERO,
        }
    }
    /// CollideType (camBoomCS tune +132; 2 in every shipped file)
    pub fn collide_type(&self) -> i32 {
        self.s("CollideType") as i32
    }
    fn range(&self, k: &str, i: usize) -> f32 {
        self.f.get(k).and_then(|v| v.get(i)).copied().unwrap_or(0.0)
    }
    pub fn near(&self) -> f32 {
        let n = self.s("CameraNear");
        if n > 0.0 {
            n
        } else {
            0.2
        }
    }
}

/// A car's cameras.
pub struct BoomSet {
    /// boom1, boom2, boom3, POV, bumper
    pub views: Vec<BoomTune>,
    /// the look-behind camera
    pub reverse: Option<BoomTune>,
}

impl BoomSet {
    pub fn load(data_dir: &str, car: &str) -> Option<BoomSet> {
        // (the cut GT-R has no camera files: the R8's, catalog::camera_car)
        let car = crate::catalog::camera_car(car);
        // (a mod car: its own file, then its manifest's camera donor - modfs.rs)
        let get = |suffix: &str| crate::modfs::car_camera_text(data_dir, car, &format!("_{suffix}.camboomcs"), crate::modfs::Donor::Cameras).and_then(|t| BoomTune::parse(&t));
        let views: Vec<BoomTune> = ["boom1", "boom2", "boom3", "POV", "bumper"].iter().filter_map(|s| get(s)).collect();
        if views.is_empty() {
            return None;
        }
        Some(BoomSet { views, reverse: get("reverse") })
    }
}

/// What the camera needs to know of the car.
pub struct BoomTarget {
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub ang_vel: Vec3,
    pub grounded: usize,
    pub gear: i32,
    /// look along the Rev* boom (the look back, +208)
    pub rev: bool,
    pub nitro: bool,
    /// the car's nitrous lens: degrees added, seconds to open, seconds to close (mcCarNitroTune)
    pub nitro_fov: (f32, f32, f32),
    /// a running special ability's change to the lens (degrees) and to the distance (scale)
    pub ability: (f32, f32),
    /// the slipstream turbo's lens (mcCarSSTurbo +20, degrees added) and distance scale (+24) - the chase
    /// cameras add the one and multiply by the other (mcTrackCS F-92343, camDynamicBoomCS F-44852)
    pub sst: (f32, f32),
    /// the look-back camera's turn about the car: yaw and pitch, radians (camBoomCS +452 /
    /// +456, set by mcPlayerCamera: the look-back's turn, or the look-around's)
    pub pan: (f32, f32),
    /// the look-around's rise (+460, m)
    pub rise: f32,
    /// the bumper and cockpit mounts (+668): they turn in place
    pub fixed_mount: bool,
    /// mcCarSimTune ModelOffset (chassis +368 -> +32), the point the chase views turn about
    pub model_offset: Vec3,
}

pub struct BoomOut {
    pub pos: Vec3,
    pub look: Vec3,
    pub up: Vec3,
    /// vertical field of view, degrees
    pub fov: f32,
}

#[derive(Default, Clone)]
pub struct BoomCam {
    started: bool,
    /// the frame: x, up, back
    up: Vec3,
    dir: Vec3,
    /// heading taken from the velocity while the car tumbles (0..1)
    tumble: f32,
    /// filtered velocity, acceleration, turn rate and turn acceleration in the frame
    vel: Vec3,
    acc: Vec3,
    ang: Vec3,
    ang_acc: Vec3,
    /// camera and focus in the frame last frame, the roll, and the camera in the world
    off: Vec3,
    foc: Vec3,
    roll: f32,
    world: Vec3,
    /// gear change kick: time left (1..0), its size, and its value this frame
    shift: f32,
    shift_size: f32,
    shift_val: f32,
    last_gear: i32,
    pub gz: f32,
    nitro: f32,
    /// the move-speed limit in use (+656)
    rate: f32,
    /// the ability's lens offset (+224 = min(+224, ab +36), F-38892; 0 again when the view is set)
    ab_fov: f32,
}

fn norm_range(v: f32, range: f32) -> f32 {
    if range > 0.0 {
        (v / range).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// vertical movement is eased: sin(x pi/2), keeping the sign
fn ease(v: f32) -> f32 {
    (v.abs().min(1.0) * std::f32::consts::FRAC_PI_2).sin() * v.signum()
}

impl BoomCam {
    /// Forget the camera's history: it starts at rest behind the car.
    pub fn reset(&mut self) {
        self.started = false;
    }

    /// Where the camera's collision probe starts (F-20517): the file's Collide point in
    /// this frame's boom frame, P = origin + frame * (Collide - Center), origin = the car's
    /// place + car * Center. Call after `update`.
    pub fn collide_from(&self, t: &BoomTune, car: &BoomTarget) -> Vec3 {
        let car_m = Mat3::from_quat(car.rot);
        let x = self.up.cross(self.dir).normalize_or(car_m.x_axis);
        let z = x.cross(self.up);
        let frame = Mat3::from_cols(x, self.up, z);
        car.pos + car_m * t.v3("Center") + frame * (t.v3("Collide") - t.v3("Center"))
    }
}

/// The boom camera's collision (camBoomCS CollideType 2, F-12669 -> F-20517 for the
/// chase and reverse views, F-77832 for POV / bumper; rules in
/// `docs/reference/cameras/boom_collision_rule.md` and `probe_rule.md`). A ball the size of the
/// near plane's corner (near * sqrt(tanX^2 + tanY^2), + 0.01; POV / bumper twice the corner) at the
/// probe point `p`: if it touches anything (the city, other cars; up to ten contacts) each contact
/// puts the camera at p + n depth and the pushes are summed by F-87913 in the list's order, one
/// pass; else the ball is swept from `p` to the camera `c` and the camera stands where it first
/// touches. POV / bumper sweep only. The focus moves with it (the look keeps its direction);
/// every frame, no smoothing, the boom's own state is not changed. Returns the camera's move, or
/// None when it is clear.
pub fn boom_collide(p: Vec3, c: Vec3, r: f32, inside_car: bool, world: SphereTest) -> Option<Vec3> {
    if !inside_car {
        let k = world.contacts(p, r, true, 10);
        if !k.is_empty() {
            let mut acc = Vec3::ZERO;
            for (d, n) in k {
                acc = accumulate(acc, p + n * d - c);
            }
            return Some(acc);
        }
    }
    let (t, _) = world.sweep(p, c, r, true)?;
    Some(p + (c - p) * t - c)
}

/// F-87913: the next push adds only its part not along the sum so far.
fn accumulate(acc: Vec3, mut v: Vec3) -> Vec3 {
    let dd = acc.dot(v);
    let aa = acc.length_squared();
    if dd > 1e-12 && aa > 1e-12 {
        v -= acc * (dd / aa);
    }
    acc + v
}

impl BoomCam {
    pub fn update(&mut self, t: &BoomTune, car: &BoomTarget, dt: f32) -> BoomOut {
        let dt = dt.clamp(1.0 / 600.0, 0.1);
        // the filters are per call of the original's camera update: once a game frame, 1/30 s
        // (G+8 read 0.0333 in the running recomp; camera_leftovers.md 5)
        let frames = dt * 30.0;
        let per_frame = |f: f32| 1.0 - (1.0 - f.clamp(0.0, 1.0)).powf(frames);
        let car_m = Mat3::from_quat(car.rot);
        let (car_up, car_back) = (car_m.y_axis, car_m.z_axis);
        let snap = !self.started || (self.world - car.pos).length_squared() > 80.0 * 80.0;
        if snap {
            *self = BoomCam { started: true, last_gear: car.gear, ..Default::default() };
        }

        // ---- the frame: up leans part of the way with the ground, the heading follows the car ----
        let ground = if car.grounded >= 3 { car_up } else { Vec3::Y };
        let up_target = Vec3::Y.lerp(ground, t.s("OrientationFactor")).normalize_or(Vec3::Y);
        let speed_h = (car.vel.x * car.vel.x + car.vel.z * car.vel.z).sqrt();
        // a car on its side or roof is followed along its path instead
        let rising = if speed_h < 5.0 || car_up.y > 0.7 {
            Some(false)
        } else if speed_h > 15.0 || car_up.y < 0.3 {
            Some(true)
        } else {
            None
        };
        match rising {
            Some(true) => self.tumble = (self.tumble + dt).min(1.0),
            Some(false) => self.tumble = (self.tumble - dt).max(0.0),
            None => {}
        }
        let mut dir_target = car_back;
        if self.tumble > 0.0 && speed_h > 0.1 {
            let along = (car_back + Vec3::new(-car.vel.x, 0.0, -car.vel.z)).normalize_or(car_back);
            dir_target = car_back.lerp(along, self.tumble).normalize_or(car_back);
        }
        if snap {
            self.up = up_target;
            self.dir = dir_target;
        } else {
            self.up = self.up.lerp(up_target, per_frame(t.s("OrientationFilter"))).normalize_or(up_target);
            self.dir = self.dir.lerp(dir_target, per_frame(t.s("DirectionFilter"))).normalize_or(dir_target);
        }
        let x = self.up.cross(self.dir).normalize_or(car_m.x_axis);
        let z = x.cross(self.up);
        let frame = Mat3::from_cols(x, self.up, z);
        let origin = car.pos + car_m * t.v3("Center");
        let to_frame = frame.transpose();

        // ---- how the car moves, in the frame ----
        let vel = to_frame * car.vel;
        let ang = to_frame * car.ang_vel;
        if snap {
            self.vel = vel;
            self.ang = ang;
        } else {
            // (the VelocityFilter is halved below 60 frames a second - always, at the game's 30:
            // A-12557)
            let (kv, ka) = (per_frame(t.s("VelocityFilter") * 0.5), per_frame(t.s("AccelFilter")));
            let (v0, w0) = (self.vel, self.ang);
            self.vel = v0.lerp(vel, kv);
            self.ang = w0.lerp(ang, kv);
            self.acc = self.acc.lerp((self.vel - v0) / dt, ka);
            self.ang_acc = self.ang_acc.lerp((self.ang - w0) / dt, ka);
        }
        let fore_aft = |v: f32, k: &str| -norm_range(v, if v > 0.0 { t.range(k, 3) } else { t.range(k, 2) });
        let speed = Vec3::new(norm_range(self.vel.x, t.range("LinVelRange", 0)), ease(norm_range(self.vel.y, t.range("LinVelRange", 1))), fore_aft(self.vel.z, "LinVelRange"));
        let turn = Vec3::new(norm_range(self.ang.x, t.range("AngVelRange", 0)), -norm_range(self.ang.y, t.range("AngVelRange", 1)), -norm_range(self.ang.z, t.range("AngVelRange", 2)));
        let mut gs = Vec3::new(norm_range(self.acc.x, t.range("LinAccRange", 0)), ease(norm_range(self.acc.y, t.range("LinAccRange", 1))), fore_aft(self.acc.z, "LinAccRange"));
        let asx = Vec3::new(
            norm_range(self.ang_acc.x, t.range("AngAccRange", 0)),
            -norm_range(self.ang_acc.y, t.range("AngAccRange", 1)),
            -norm_range(self.ang_acc.z, t.range("AngAccRange", 2)),
        );
        if gs.z < 0.0 {
            // braking does not pull the camera in while the tail is out
            gs.z *= 1.0 - turn.y.abs() * t.s("FishtailFactor");
        }
        self.gz = gs.z;

        // ---- the gear change kick ----
        if car.gear > self.last_gear && car.gear > 2 && self.shift == 0.0 {
            self.shift = 1.0;
            self.shift_size = gs.z.max(0.01);
        }
        self.last_gear = car.gear;
        self.shift_val = 0.0;
        if self.shift > 0.0 {
            let p = 1.0 - self.shift;
            let keys = [(0.0, 0.0), (t.s("ShiftTime1"), t.s("ShiftValue1")), (t.s("ShiftTime2"), t.s("ShiftValue2")), (t.s("ShiftTime3"), t.s("ShiftValue3")), (1.0, 0.0)];
            let mut val = 0.0;
            for w in keys.windows(2) {
                if p >= w[0].0 && p <= w[1].0 {
                    let span = (w[1].0 - w[0].0).max(1e-4);
                    val = w[0].1 + (w[1].1 - w[0].1) * (p - w[0].0) / span;
                    break;
                }
            }
            self.shift_val = t.s("ShiftMagnitude") * self.shift_size * val;
            self.shift = (self.shift - dt / t.s("ShiftDuration").max(0.05)).max(0.0);
        }

        // ---- the lens and the boom length follow the speed ----
        let ss = ((self.vel.x * self.vel.x + self.vel.z * self.vel.z).sqrt() / t.range("LinVelRange", 2).max(1.0)).clamp(0.0, 1.0);
        let k = (1.0 - (ss * std::f32::consts::PI).cos()) * 0.5;
        // nitrous opens the lens by NitroFOV over FovOutTime and lets it close over FovInTime
        if car.nitro {
            self.nitro = (self.nitro + dt / car.nitro_fov.1.max(0.05)).min(1.0);
        } else {
            self.nitro = (self.nitro - dt / car.nitro_fov.2.max(0.05)).max(0.0);
        }
        // (the ability's terms reach the bumper and cockpit only: the lens offset held at its lowest
        // since the view was set, the distance scale as it is - F-38892; the chase views go to the
        // ability camera instead, `AbilityCam`)
        let (ab_fov, ab_dist) = if car.fixed_mount {
            self.ab_fov = self.ab_fov.min(car.ability.0);
            (self.ab_fov, car.ability.1)
        } else {
            (0.0, 1.0)
        };
        let fov = t.s("SSFovMin") + (t.s("SSFovMax") - t.s("SSFovMin")) * k + car.nitro_fov.0 * self.nitro + car.sst.0 + ab_fov;
        let dist_scale = (1.0 + (t.s("SSDistScale") - 1.0) * k) * car.sst.1 * ab_dist;

        // ---- where the camera and its focus want to be ----
        let (off0, foc0) = if car.rev { (t.v3("RevOffset"), t.v3("RevFocus")) } else { (t.v3("Offset"), t.v3("Focus")) };
        let (off0, foc0) = (off0 - t.v3("Center"), foc0 - t.v3("Center"));
        // (the distance scale shortens the offset's own length from Center, along the boom from the focus:
        // off - normalize(off - focus) x |off| x (1 - scale), A-81111..A-85412)
        let boom = off0 - foc0;
        let mut off = off0 - boom.normalize_or_zero() * off0.length() * (1.0 - dist_scale);
        // (accelerating and braking have their own sets)
        let (gz_off_z, gz_off_y, gz_foc_y, gz_foc_z) = if gs.z >= 0.0 {
            (t.s("GsZOffsetZ") * gs.z, t.s("GsZOffsetY") * gs.z, t.s("GsZFocusY") * gs.z, 0.0)
        } else {
            (-t.s("GsWOffsetZ") * gs.z, -t.s("GsWOffsetY") * gs.z, -t.s("GsWFocusY") * gs.z, -t.s("GsWFocusZ") * gs.z)
        };
        let mut d_off = Vec3::new(
            t.s("AsYOffsetX") * asx.y + t.s("GsXOffsetX") * gs.x + t.s("TurnYOffsetX") * turn.y + t.s("SpeedXOffsetX") * speed.x,
            t.s("AsYOffsetY") * asx.x + t.s("GsYOffsetY") * gs.y + t.s("TurnXOffsetY") * turn.x + speed.y.abs() * t.s("SpeedYOffsetY") + turn.y.abs() * t.s("TurnYOffsetY") + speed.x.abs() * t.s("SpeedXOffsetY") + gz_off_y + speed.z.abs() * t.s("SpeedZOffsetY") + t.s("ShiftZOffsetY") * self.shift_val,
            asx.y.abs() * t.s("AsYOffsetZ") + gs.x.abs() * t.s("GsXOffsetZ") + turn.y.abs() * t.s("TurnYOffsetZ") + speed.x.abs() * t.s("SpeedXOffsetZ") + gz_off_z + speed.z.abs() * t.s("SpeedZOffsetZ") + t.s("ShiftZOffsetZ") * self.shift_val,
        );
        let d_foc = Vec3::new(
            t.s("AsYFocusX") * asx.y + t.s("GsXFocusX") * gs.x + t.s("TurnYFocusX") * turn.y + t.s("SpeedXFocusX") * speed.x,
            t.s("AsYFocusY") * asx.x + t.s("GsYFocusY") * gs.y + t.s("TurnXFocusY") * turn.x + speed.y.abs() * t.s("SpeedYFocusY") + turn.y.abs() * t.s("TurnYFocusY") + speed.x.abs() * t.s("SpeedXFocusY") + gz_foc_y + speed.z.abs() * t.s("SpeedZFocusY") + t.s("ShiftZFocusY") * self.shift_val,
            asx.y.abs() * t.s("AsYFocusZ") + gs.x.abs() * t.s("GsXFocusZ") + turn.y.abs() * t.s("TurnYFocusZ") + speed.x.abs() * t.s("SpeedXFocusZ") + gz_foc_z,
        );
        // the frame's nose pointing up
        if z.y < 0.0 {
            d_off.z += t.s("AngleXOffsetZ") * z.y;
            d_off.y -= t.s("AngleXOffsetY") * z.y;
        }
        // (a turned look-back camera moves less with the car: the yaw takes the side and
        // fore-aft terms by 1 - min(yaw^2, 1), the pitch the height terms - A-27754..A-63970)
        let mut d_foc = d_foc;
        let (pan_yaw, pan_pitch) = car.pan;
        if pan_yaw != 0.0 {
            let k = 1.0 - (pan_yaw * pan_yaw).min(1.0);
            d_off.x *= k;
            d_off.z *= k;
            d_foc.x *= k;
            d_foc.z *= k;
        }
        if pan_pitch != 0.0 {
            let k = 1.0 - (pan_pitch * pan_pitch).min(1.0);
            d_off.y *= k;
            d_foc.y *= k;
        }
        off += d_off;
        let mut foc_target = foc0 + d_foc;
        // the look-around's rise lifts camera and focus together (+460)
        off.y += car.rise;
        foc_target.y += car.rise;
        // and turns its boom: pitch about x, then yaw about y (A-96068..A-15453), about the
        // car tune's ModelOffset for the chase views (chassis +368 -> +32; here less Center, as
        // offset and focus are), in place for the bumper and the cockpit (last frame's place, +416)
        if pan_yaw != 0.0 || pan_pitch != 0.0 {
            let r = Mat3::from_rotation_y(pan_yaw) * Mat3::from_rotation_x(pan_pitch);
            let pivot = if car.fixed_mount {
                if self.started { self.off } else { Vec3::ZERO }
            } else {
                car.model_offset - t.v3("Center")
            };
            off = pivot + r * (off - pivot);
            foc_target = pivot + r * (foc_target - pivot);
        }
        // on a slope the boom swings up to keep the road in view
        let slope = z.y.clamp(-1.0, 1.0).asin();
        let (pitch_dn, pitch_up) = if car.rev { (-t.s("HillPitchUp"), -t.s("HillPitchDn")) } else { (t.s("HillPitchDn"), t.s("HillPitchUp")) };
        let swing = slope.abs() * if slope >= 0.0 { pitch_dn } else { pitch_up };
        if swing != 0.0 {
            let (s, c) = swing.sin_cos();
            off = Vec3::new(off.x, off.y * c + off.z * s, off.z * c - off.y * s);
        }
        let roll_target = t.s("AsZRollZ") * asx.z + t.s("GsXRollZ") * gs.x + t.s("TurnZRollZ") * turn.z + t.s("SpeedXRollZ") * speed.x + t.s("TurnYRollZ") * turn.y;

        // ---- the camera trails: its last place, seen from the new frame, homes on the target ----
        let mut foc = foc_target;
        let mut roll = roll_target;
        if !snap {
            // (A-29979: once a game frame, 1/30 s, the last place seen from the new frame goes h of the
            // way to the target, homed = last + (off - last) h, then takes the target's length. The lag
            // that leaves is fed by the car's move over the frame, so at another step the place keeps the
            // original's decay q = (1 - h)^(dt 30) and takes the move at the original's weight
            // k = (1 - q)(1 - h) / (h dt 30) - at dt = 1/30 the original's step exactly, at any other its
            // time constant and its lag at a steady speed (GitHub #9: the plain per-step factor left the
            // cockpit four times as far behind))
            let homing = t.v3("HomingTime");
            if homing != Vec3::ZERO {
                let len = off.length();
                let last = to_frame * (self.world - origin);
                let moved = last - self.off;
                let keep = |h: f32| {
                    let h = h.clamp(1e-4, 1.0);
                    let q = (1.0 - h).powf(frames);
                    (q, (1.0 - q) * (1.0 - h) / (h * frames))
                };
                let (kx, ky, kz) = (keep(homing.x), keep(homing.y), keep(homing.z));
                let homed = off + Vec3::new(kx.0, ky.0, kz.0) * (self.off - off) + Vec3::new(kx.1, ky.1, kz.1) * moved;
                off = homed.normalize_or(off.normalize_or(Vec3::Z)) * len;
            }
            // and nothing moves faster than the approach rate (+656: 25 m/s while the camera is
            // turned, else the file's ApproachRate, moving there at 50 per second; none when the
            // file has none - A-69633)
            let file_rate = t.s("ApproachRate");
            let goal = if pan_yaw != 0.0 || pan_pitch != 0.0 { 25.0 } else { file_rate };
            // (approx.: a fresh camera starts at the file's rate)
            let cur = if self.rate > 0.0 { self.rate } else { file_rate };
            self.rate = if file_rate > 0.0 { cur + (goal - cur).clamp(-50.0 * dt, 50.0 * dt) } else { 0.0 };
            let rate = self.rate;
            if rate > 0.0 {
                let step = rate * dt;
                let mut share = 1.0f32;
                for (a, b) in [(off.x, self.off.x), (off.y, self.off.y), (off.z, self.off.z), (foc.x, self.foc.x), (foc.y, self.foc.y), (foc.z, self.foc.z)] {
                    let d = (a - b).abs();
                    if d > step {
                        share = share.min(step / d);
                    }
                }
                off = self.off + (off - self.off) * share;
                foc = self.foc + (foc - self.foc) * share;
                roll = self.roll + (roll - self.roll) * share;
            }
        }
        self.off = off;
        self.foc = foc;
        self.roll = roll;
        let pos = origin + frame * off;
        let look = origin + frame * foc;
        self.world = pos;
        // the roll turns the horizon about the line of sight
        let view = (look - pos).normalize_or(-z);
        let up = Quat::from_axis_angle(view, roll) * self.up;
        BoomOut { pos, look, up, fov }
    }
}

// ---- the player's choice of view (mcPlayerCamera, F-89557 / F-67519) ----
//
// The original keeps its views in a list: 0 Bumper (+44), 1 POV (+48), 2 Boom 1 (+52), 3 Boom 2,
// 4 Boom 3 (+60); index at +80. The port's `cam_mode` keeps its own order (0 boom1, 1 boom2,
// 2 boom3, 3 POV, 4 bumper; 5 = the event camera, only by the test hook).

/// The port's view for each of the original's list places.
pub const PORT_OF_VIEW: [u8; 5] = [4, 3, 0, 1, 2];
/// The original's list place of each port view (5, the event camera, counts as Boom 3).
pub const VIEW_OF_PORT: [usize; 6] = [2, 3, 4, 1, 0, 4];
/// The names the HUD shows (`_global.cameras`, F-13512): string table keys of the list
/// places 0..4, then the overhead view of the race editor (`Cheat_RE_OverHeadCam`).
pub const VIEW_KEYS: [&str; 6] = ["Bumper", "POV", "Boom 1", "Boom 2", "Boom 3", "Cheat_RE_OverHeadCam"];

/// The camera buttons outside the race editor (F-67519, A-11943..A-79101):
/// CAM_UP takes the list's previous place, CAM_DOWN the next; both wrap within the five.
/// The event camera is never a place of the list.
pub fn next_view(port: u8, cam_up: bool) -> u8 {
    let v = VIEW_OF_PORT[(port as usize).min(5)];
    PORT_OF_VIEW[if cam_up { (v + 4) % 5 } else { (v + 1) % 5 }]
}

/// A change between two views is a cut (transition type 4) when either is Bumper or POV,
/// else a blend (type 3) over the new view's own BlendTime (F-67519 -> F-41662).
pub fn is_cut(from_port: u8, to_port: u8) -> bool {
    VIEW_OF_PORT[(from_port as usize).min(5)] <= 1 || VIEW_OF_PORT[(to_port as usize).min(5)] <= 1
}

impl BoomTune {
    /// `BlendTime` of the file (camBoomCS +160).
    pub fn blend_time(&self) -> f32 {
        self.s("BlendTime")
    }
    /// `BlendGoal` of the file (+164).
    pub fn blend_goal(&self) -> f32 {
        self.f.get("BlendGoal").and_then(|v| v.first()).copied().unwrap_or(1.0)
    }
}

/// The blend from one view to the next (the camera manager's MCCAMERATRANSITION, F-18583):
/// both cameras keep running; the share of the new one is s = 1 - goal * left / time, eased
/// (1 - cos(pi s)) / 2 (type 3), and position, aim and lens go from the old to the new by it.
#[derive(Default)]
pub struct ViewBlend {
    from: Option<(BoomTune, BoomCam)>,
    left: f32,
    time: f32,
    goal: f32,
}

impl ViewBlend {
    pub fn start(&mut self, tune: BoomTune, cam: BoomCam, time: f32, goal: f32) {
        if time > 0.0 {
            self.from = Some((tune, cam));
            self.left = time;
            self.time = time;
            self.goal = goal;
        } else {
            self.stop();
        }
    }
    pub fn stop(&mut self) {
        self.from = None;
    }
    pub fn active(&self) -> bool {
        self.from.is_some()
    }
    /// Run the old view for this frame and mix it into `out` (the new view's output).
    pub fn apply(&mut self, car: &BoomTarget, out: BoomOut, dt: f32) -> BoomOut {
        let Some((tune, cam)) = self.from.as_mut() else { return out };
        let old = cam.update(tune, car, dt);
        self.left -= dt;
        if self.left <= 0.0 && self.goal == 1.0 {
            self.from = None;
            return out;
        }
        let s = (1.0 - self.goal * (self.left.max(0.0) / self.time.max(1e-4))).clamp(0.0, 1.0);
        let w = (1.0 - (std::f32::consts::PI * s).cos()) * 0.5;
        BoomOut {
            pos: old.pos.lerp(out.pos, w),
            look: old.look.lerp(out.look, w),
            up: old.up.lerp(out.up, w).normalize_or(out.up),
            fov: old.fov + (out.fov - old.fov) * w,
        }
    }
}

/// The black mask of a cut (TRANSITIONMOVIE as F-28091 starts it: mask_color 0,
/// fade_speed_in, fade_speed_out, wait_delay). The movie (30 frames a second) adds
/// 3.333 / fade_speed_in to its alpha each frame up to 100, waits until 3.333 / wait_delay a
/// frame has counted to 100, then takes 3.333 / fade_speed_out a frame off; all three can
/// happen on the same frame. A view change asks for (0.01, 0.25, 0.01), a look back for
/// (0.01, 0.1, 0.01).
#[derive(Default)]
pub struct CutMask {
    alpha: f32,
    wait: f32,
    ready: u8,
    fade_in: f32,
    fade_out: f32,
    delay: f32,
    on: bool,
    clock: f32,
}

impl CutMask {
    pub fn start(&mut self, fade_in: f32, fade_out: f32, delay: f32) {
        // (only when no transition is running: F-28091 reads StartOfTransitionin and
        // TransitionOutisReady first)
        if self.on {
            return;
        }
        // (the movie's next frame is the first: the mask is up on the frame of the cut)
        *self = CutMask { fade_in, fade_out, delay, on: true, clock: -1.0, ..Default::default() };
    }
    /// The fade up of a held mask (game/misctasks/screen_fade_to_scene and garage_core's own:
    /// StartOfTransitionin 0, TransitionOutisReady 2, StartOfTransitionout 1, fade_speed_out `fade_out`):
    /// no more fading in, the alpha goes down from where it is.
    pub fn release(&mut self, fade_out: f32) {
        if self.on {
            self.fade_out = fade_out;
            self.ready = 2;
        }
    }
    pub fn is_on(&self) -> bool {
        self.on
    }
    /// Faded in all the way (the movie's StartOfTransitionin cleared: ready 1).
    pub fn full(&self) -> bool {
        self.on && self.ready >= 1
    }
    /// The mask's opacity 0..1 after `dt` seconds.
    pub fn update(&mut self, dt: f32) -> f32 {
        if !self.on {
            return 0.0;
        }
        if self.clock < 0.0 {
            // (the first frame steps at once, then one every 1/30 s)
            self.clock = 1.0 / 30.0;
        } else {
            self.clock += dt;
        }
        while self.on && self.clock >= 1.0 / 30.0 {
            self.clock -= 1.0 / 30.0;
            if self.ready == 0 {
                self.alpha += 3.333 / self.fade_in.max(1e-4);
                if self.alpha >= 100.0 {
                    self.alpha = 100.0;
                    self.ready = 1;
                }
            }
            if self.ready == 1 {
                self.wait += 3.333 / self.delay.max(1e-4);
                if self.wait >= 100.0 {
                    self.ready = 2;
                }
            }
            if self.ready == 2 {
                self.alpha -= 3.333 / self.fade_out.max(1e-4);
                if self.alpha <= 0.0 {
                    *self = CutMask::default();
                }
            }
        }
        self.alpha.clamp(0.0, 100.0) / 100.0
    }
}

/// The camera file a view runs and whether it looks along its Rev* boom: looking back from a
/// chase view cuts to the car's `_reverse` camera; from Bumper or POV the same camera turns
/// round (F-39925: vt+80(1) on the camera in use). Backing up changes nothing: the update
/// takes RevOffset / RevFocus only on +208 (A-47729), which only vt+80 (F-38722) sets - at
/// the start for the `_reverse` camera (F-42437) and for the look back (GitHub #10).
pub fn tune_for(set: &BoomSet, view: u8, back: bool) -> (&BoomTune, bool) {
    let i = (view as usize).min(set.views.len() - 1);
    let chase = VIEW_OF_PORT[(view as usize).min(5)] >= 2;
    match (back, chase) {
        (true, true) => (set.reverse.as_ref().unwrap_or(&set.views[0]), true),
        (true, false) => (&set.views[i], true),
        _ => (&set.views[i], false),
    }
}

/// What the player's camera does when the view or the look back changes (mcPlayerCamera:
/// F-67519 the buttons, F-39925 the look back, F-38892 its turning).
#[derive(Default)]
pub struct Switch {
    pub blend: ViewBlend,
    pub mask: CutMask,
    last: Option<(u8, bool)>,
    /// the look-back camera's turn: yaw, pitch (+452, +456)
    pub pan: (f32, f32),
    /// CAM_PAN_LEFT/RIGHT (-1 / +1) and CAM_PAN_UP/DOWN (+1 / -1) held, from the pad
    pub pan_in: (f32, f32),
    seed: u32,
    /// the look back went off this frame (F-39925: the look-back slow motion is released)
    pub look_off: bool,
}

impl Switch {
    /// Forget the last view (the next frame starts without a blend or a mask).
    pub fn reset(&mut self) {
        self.last = None;
        self.blend.stop();
        self.pan = (0.0, 0.0);
    }

    /// The original's random draw: 23 bits of a linear congruential generator.
    fn rand23(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 9) as f32
    }

    /// Each frame before the view runs. `cinematic`: the CINEMATIC CAMERA option (+88);
    /// `moving`: the car's speed above zero. On a change of view or of the look back the
    /// camera's history goes (the new camera is started afresh, vt+36), the old view is
    /// blended out or the mask is put up.
    pub fn frame(&mut self, set: &BoomSet, view: u8, back: bool, cinematic: bool, moving: bool, cam: &mut BoomCam, dt: f32) {
        self.look_off = false;
        let Some((was_view, was_back)) = self.last.replace((view, back)) else { return };
        let chase = VIEW_OF_PORT[(view as usize).min(5)] >= 2;
        self.look_off = was_back && !back;
        if back != was_back {
            // (both ways a cut with the mask's quick fade: 0.01, 0.1, 0.01)
            self.blend.stop();
            self.mask.start(0.01, 0.1, 0.01);
            cam.reset();
            self.pan = (0.0, 0.0);
            if back && chase && cinematic && moving {
                // the look back of the cinematic camera stands off to one side: 22.5 .. 67.5
                // degrees either way (A-94637..A-37192)
                let sign = if self.rand23() * 2.384_185_8e-7 - 1.0 >= 0.0 { 1.0 } else { -1.0 };
                self.pan.0 = sign * (0.392_699 + self.rand23() * 9.362_68e-8);
            }
        } else if view != was_view && !back {
            if is_cut(was_view, view) {
                self.blend.stop();
                self.mask.start(0.01, 0.25, 0.01);
            } else {
                let (old, _) = tune_for(set, was_view, false);
                let (new, _) = tune_for(set, view, false);
                self.blend.start(old.clone(), cam.clone(), new.blend_time(), new.blend_goal());
            }
            cam.reset();
        }
        // turning the look-back camera with CAM_PAN_*: 2 rad/s round, 1.25 rad/s up, the
        // pitch kept in 0 .. 1.2566 (A-28508..A-14649)
        if back && chase {
            self.pan.0 += self.pan_in.0 * 2.0 * dt;
            self.pan.1 = (self.pan.1 + self.pan_in.1 * 1.25 * dt).clamp(0.0, 1.256_64);
        }
    }

    /// The pan to hand the camera this frame.
    pub fn pan_for(&self, view: u8, back: bool) -> (f32, f32) {
        if back && VIEW_OF_PORT[(view as usize).min(5)] >= 2 {
            self.pan
        } else {
            (0.0, 0.0)
        }
    }
}

/// The camera effects tune (`camera/default` = `tune/camera/default.camfx`, class camFX,
/// F-53685 into the player camera's +24 object): the times and thresholds the event
/// camera's chooser reads.
#[derive(Clone, Debug)]
pub struct CamFx {
    /// seconds without input before the Idle view (+36)
    pub idle_delay: f32,
    /// yaw rate (rad/s) that makes a burnout the Burnout view (+44)
    pub burnout_turn: f32,
    /// the look-around (F-36289): the chase views' yaw at full stick left / right (+60,
    /// +64), the cockpit's (+68, +72) and its pitch limits up / down (+76, +80), the chase
    /// views' rise at full CAMERA_UP (+16, m), the cockpit's lens at full CAMERA_UP (+32,
    /// degrees; not in the file: the constructor's 15) and the ease per call (+48, 0.2: idem)
    pub pan: (f32, f32),
    pub limit: (f32, f32, f32, f32),
    pub rise_up: f32,
    pub zoom_in: f32,
    pub look_lerp: f32,
    /// LookaroundDelayTime (+40): the cockpit's parked free look waits this long with no pedal
    /// and the car still (approx.: the constructor's own value not read; the file's 3)
    pub look_delay: f32,
}

impl Default for CamFx {
    fn default() -> CamFx {
        // (the constructor's defaults, F-25608)
        CamFx { idle_delay: 10.0, burnout_turn: 1.5, pan: (1.0472, 1.0472), limit: (1.0, 1.0, 0.471_239, 0.471_239), rise_up: 0.75, zoom_in: 15.0, look_lerp: 0.2, look_delay: 3.0 }
    }
}

impl CamFx {
    pub fn load(data_dir: &str) -> CamFx {
        let h = mcla_formats::hash::joaat("default.camfx");
        let text = std::fs::read_to_string(Path::new(data_dir).join("tune/camera").join(format!("0x{h:08x}"))).unwrap_or_default();
        let mut fx = CamFx::default();
        for l in text.lines() {
            let mut it = l.split_whitespace();
            match (it.next(), it.next().and_then(|v| v.parse::<f32>().ok())) {
                (Some("IdleDelayTime"), Some(v)) => fx.idle_delay = v,
                (Some("BurnoutTurnThreshold"), Some(v)) => fx.burnout_turn = v,
                (Some("CamRiseUp"), Some(v)) => fx.rise_up = v,
                (Some("CamZoomIn"), Some(v)) => fx.zoom_in = v,
                (Some("LookaroundLerpFactor"), Some(v)) => fx.look_lerp = v,
                (Some("LookaroundDelayTime"), Some(v)) => fx.look_delay = v,
                (Some("LookaroundPanLeft"), Some(v)) => fx.pan.0 = v,
                (Some("LookaroundPanRight"), Some(v)) => fx.pan.1 = v,
                (Some("LookaroundLimitLeft"), Some(v)) => fx.limit.0 = v,
                (Some("LookaroundLimitRight"), Some(v)) => fx.limit.1 = v,
                (Some("LookaroundLimitUp"), Some(v)) => fx.limit.2 = v,
                (Some("LookaroundLimitDown"), Some(v)) => fx.limit.3 = v,
                _ => {}
            }
        }
        fx
    }
}

/// One of camFX's five shake blocks (camFX + 96 + 224 i: Shake, Rumble, DrivingRumble,
/// BurnoutRumble, DriftRumble; rule in `docs/reference/cameras/shake_rule.md`).
#[derive(Clone, Debug, Default)]
struct ShakeBlock {
    /// CarRot, Rot, Tran: amplitude, frequency (rad/s), random, phase
    parts: [[Vec3; 4]; 3],
    timer: f32,
    magnitude: f32,
    scale: f32,
    /// 0 off, 1 one-shot (the timer runs down), 2 continuous
    state: u8,
}

/// The camera shake (camFX blocks, F-10707 applied to the camera in use each frame).
#[derive(Clone, Debug, Default)]
pub struct CamShake {
    blocks: Vec<ShakeBlock>,
    rng: (u32, u32),
}

pub const SHAKE_DRIVING: usize = 2;
pub const SHAKE_BURNOUT: usize = 3;
pub const SHAKE_DRIFT: usize = 4;

impl CamShake {
    pub fn load(data_dir: &str) -> CamShake {
        let h = mcla_formats::hash::joaat("default.camfx");
        let text = std::fs::read_to_string(Path::new(data_dir).join("tune/camera").join(format!("0x{h:08x}"))).unwrap_or_default();
        // (the constructor's: frequency 50, magnitude 3; mcPlayerCamera puts 0.01 into Rumble and
        // DrivingRumble before the file)
        let blocks = ["Shake", "Rumble", "DrivingRumble", "BurnoutRumble", "DriftRumble"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let b = track_block(&text, Some(name));
                let v = |k: &str, d: f32| b.get(k).filter(|v| v.len() >= 3).map_or(Vec3::splat(d), |v| Vec3::new(v[0], v[1], v[2]));
                let part = |p: &str| [v(&format!("{p}Amp"), 0.0), v(&format!("{p}Freq"), 50.0), v(&format!("{p}Random"), 0.0), Vec3::ZERO];
                let mag = b.get("Magnitude").and_then(|v| v.first()).copied().unwrap_or(if i == 1 || i == 2 { 0.01 } else { 3.0 });
                ShakeBlock { parts: [part("CarRot"), part("Rot"), part("Tran")], magnitude: mag, scale: 2.0, ..Default::default() }
            })
            .collect();
        CamShake { blocks, rng: (0x1234_5678, 0) }
    }

    /// Every block off (F-29203: the phases are kept).
    pub fn reset(&mut self) {
        for b in &mut self.blocks {
            b.timer = 0.0;
            b.state = 0;
        }
    }

    /// A one-shot on the Shake block (F-68650 -> F-48972): `fixed_mount` (bumper /
    /// cockpit) takes 0.4 of the scale; a stronger shake still running is kept.
    pub fn jolt(&mut self, duration: f32, scale: f32, fixed_mount: bool) {
        let scale = if fixed_mount { scale * 0.4 } else { scale };
        let Some(b) = self.blocks.get_mut(0) else { return };
        if b.magnitude * duration * scale <= b.magnitude * b.scale * b.timer {
            return;
        }
        b.state = 1;
        b.scale = scale;
        b.timer = duration;
    }

    /// A continuous block's strength (F-54070).
    pub fn set(&mut self, i: usize, scale: f32) {
        if let Some(b) = self.blocks.get_mut(i) {
            b.timer = 1.0;
            b.scale = scale;
            b.state = 2;
        }
    }

    /// 23 bits of the game's multiply-with-carry generator (0x82860880).
    fn rand23(&mut self) -> f32 {
        let x = self.rng.0 as u64 * 0x5CDC_FAA7 + self.rng.1 as u64;
        self.rng = (x as u32, (x >> 32) as u32);
        (self.rng.0 & 0x7F_FFFF) as f32 * (1.0 / 8_388_608.0)
    }

    /// Shakes the camera (eye `pos`, `look`, `up`) about the car at `car` (F-10707).
    pub fn apply(&mut self, dt: f32, pos: &mut Vec3, look: &mut Vec3, up: &mut Vec3, car: Vec3) {
        let reach = pos.distance(*look).max(0.1);
        // the camera's rows: a right, b up, c back
        let mut c = (*pos - *look).normalize_or(Vec3::Z);
        let mut a = up.cross(c).normalize_or(Vec3::X);
        let mut b = c.cross(a);
        let mut d = *pos;
        for i in 0..self.blocks.len() {
            if self.blocks[i].state == 0 {
                continue;
            }
            if self.blocks[i].state == 1 {
                self.blocks[i].timer -= dt;
                if self.blocks[i].timer <= 0.0 {
                    self.blocks[i].state = 0;
                    continue;
                }
            }
            let m = self.blocks[i].scale * self.blocks[i].magnitude * self.blocks[i].timer;
            if m == 0.0 {
                continue;
            }
            // the oscillator (F-17992): phase += freq dt; (amp sin phase + uniform(-r, r)) m
            let mut out = [Vec3::ZERO; 3];
            for k in 0..3 {
                let [amp, freq, rnd, ph] = self.blocks[i].parts[k];
                let ph = ph + freq * dt;
                self.blocks[i].parts[k][3] = ph;
                let u = Vec3::new(self.rand23(), self.rand23(), self.rand23());
                out[k] = (amp * Vec3::new(ph.x.sin(), ph.y.sin(), ph.z.sin()) + (-rnd + rnd * 2.0 * u)) * m;
            }
            let [cr, rt, tr] = out;
            // CarRot: the camera orbits the car, turning with it
            if cr.length_squared() != 0.0 && (d - car).length_squared() > 0.02 {
                // (F = look-at frame along car - camera, world up: its c from the camera to the car)
                let dist = (car - d).length();
                let fc = (car - d).normalize_or(-c);
                let fa = Vec3::Y.cross(fc).normalize_or(a);
                let fb = fc.cross(fa);
                let (mut fa, mut fb, mut fc) = (fa, fb, fc);
                let rot_local_x = |b: &mut Vec3, c: &mut Vec3, t: f32| {
                    let (s, co) = t.sin_cos();
                    let (nb, nc) = (*b * co + *c * s, *c * co - *b * s);
                    *b = nb;
                    *c = nc;
                };
                let rot_local_z = |a: &mut Vec3, b: &mut Vec3, t: f32| {
                    let (s, co) = t.sin_cos();
                    let (na, nb) = (*a * co + *b * s, *b * co - *a * s);
                    *a = na;
                    *b = nb;
                };
                let ry = |v: Vec3, t: f32| {
                    let (s, co) = t.sin_cos();
                    Vec3::new(co, 0.0, -s) * v.x + Vec3::Y * v.y + Vec3::new(s, 0.0, co) * v.z
                };
                rot_local_x(&mut fb, &mut fc, cr.x);
                rot_local_z(&mut fa, &mut fb, cr.z);
                fc = ry(fc, cr.y);
                let _ = (fa, fb);
                a = ry(a, cr.y);
                b = ry(b, cr.y);
                c = ry(c, cr.y);
                rot_local_z(&mut a, &mut b, -cr.z);
                rot_local_x(&mut b, &mut c, -cr.x);
                d = car - fc * dist;
            }
            // Rot and Tran in the camera's own frame (EulerXYZ, F-28753)
            d += tr.x * a + tr.y * b + tr.z * c;
            let (sx, cx) = rt.x.sin_cos();
            let (sy, cy) = rt.y.sin_cos();
            let (sz, cz) = rt.z.sin_cos();
            let s0 = Vec3::new(cz * cy, sz * cy, -sy);
            let s1 = Vec3::new(cz * sy * sx - sz * cx, sz * sy * sx + cz * cx, cy * sx);
            let s2 = Vec3::new(cz * sy * cx + sz * sx, sz * sy * cx - cz * sx, cy * cx);
            let (na, nb, nc) = (s0.x * a + s0.y * b + s0.z * c, s1.x * a + s1.y * b + s1.z * c, s2.x * a + s2.y * b + s2.z * c);
            a = na;
            b = nb;
            c = nc;
        }
        *pos = d;
        *look = d - c * reach;
        *up = b;
    }
}

/// The original's dead zone with rescale (F-93983).
fn dead_zone(v: f32, d: f32) -> f32 {
    if v > d {
        (v - d) / (1.0 - d)
    } else if v < -d {
        (v + d) / (1.0 - d)
    } else {
        0.0
    }
}

/// The driving camera's look-around (mcPlayerCamera F-36289, once a game frame; rule in
/// `docs/reference/cameras/lookaround_rule.md`): the stick's CAMERA_LEFT / CAMERA_RIGHT set a
/// yaw goal (a place, not a rate: full stick = PanLeft / PanRight in the chase views, the
/// cockpit's LimitLeft / LimitRight, 45 degrees on the bumper), CAMERA_UP raises a chase
/// camera by CamRiseUp or pitches and narrows the cockpit's; the camera's yaw, pitch, rise
/// and lens offset ease to their goals by LookaroundLerpFactor a call.
#[derive(Default, Clone, Copy, Debug)]
pub struct LookAround {
    /// the camera's +452 yaw, +456 pitch, +460 rise (m), +224 lens offset (degrees)
    pub yaw: f32,
    pub pitch: f32,
    pub rise: f32,
    pub fov: f32,
    /// mcPlayerCamera +108 / +116: the held yaw and pitch (they run back to 0)
    held_yaw: f32,
    held_pitch: f32,
}

impl LookAround {
    pub fn reset(&mut self) {
        *self = LookAround::default();
    }

    /// One frame. `view`: the original's view index (0 bumper, 1 cockpit, 2..4 chase);
    /// `side` = CAMERA_RIGHT - CAMERA_LEFT, `up` = CAMERA_UP (0..1); `input`: the pad reaches
    /// the camera (else every goal is 0); `held`: looking back on this very camera (bumper or
    /// cockpit): nothing is written; `free`: the left stick (x right, y up negative, raw) when
    /// the cockpit's parked free look may run - a car (not a bike, not a wheel controller)
    /// with no pedal for over LookaroundDelayTime and still as long (chassis +588).
    /// (approx.: the original's ease is per 30 Hz game frame, taken here as 1 - 0.8^(dt*30))
    pub fn frame(&mut self, fx: &CamFx, view: u8, bike: bool, side: f32, up: f32, input: bool, held: bool, free: Option<(f32, f32)>, dt: f32) {
        if held {
            return;
        }
        let (l, r) = match view {
            0 => (std::f32::consts::FRAC_PI_4, std::f32::consts::FRAC_PI_4),
            1 => (fx.limit.0, fx.limit.1),
            _ => fx.pan,
        };
        let (mut yaw_in, mut pitch_in, mut rise_in, mut fov_in) = (0.0, 0.0, 0.0, 0.0);
        if input {
            if view >= 2 || !bike {
                let s = -dead_zone(side, 0.4);
                yaw_in = if s > 0.0 { s * l } else { s * r };
            }
            match view {
                0 => {}
                1 => {
                    let v = dead_zone(up, 0.2);
                    if v > 0.0 {
                        self.held_pitch = (-fx.limit.3 * v).clamp(-fx.limit.3, fx.limit.2);
                        pitch_in = -fx.limit.3 * v;
                        fov_in = -fx.zoom_in * v;
                    }
                }
                _ => {
                    let v = dead_zone(up, 0.4);
                    if v > 0.0 {
                        rise_in = fx.rise_up * v;
                    }
                }
            }
        }
        // the cockpit parked: the left stick (dead zone 0.25) turns the head at 1.5 rad/s and
        // pitches it at 0.75; else the held angles run back to 0 at those rates (A-18051)
        match free {
            Some((x, y)) if input && view == 1 && !bike && yaw_in == 0.0 && pitch_in == 0.0 => {
                let (x, y) = (dead_zone(x, 0.25), dead_zone(y, 0.25));
                self.held_yaw = (self.held_yaw - x * dt * 1.5).clamp(-r, l);
                self.held_pitch = (self.held_pitch - y * dt * 0.75).clamp(-fx.limit.3, fx.limit.2);
            }
            _ => {
                let step = |a: f32, rate: f32| if a > 0.0 { (a - rate * dt).max(0.0) } else { (a + rate * dt).min(0.0) };
                self.held_yaw = step(self.held_yaw, 1.5);
                self.held_pitch = step(self.held_pitch, 0.75);
            }
        }
        let yaw_goal = (self.held_yaw + yaw_in).clamp(-r, l);
        let pitch_goal = self.held_pitch;
        let k = 1.0 - (1.0 - fx.look_lerp.clamp(0.0, 1.0)).powf(dt * 30.0);
        let a = wrap_pi(self.yaw);
        self.yaw = a + (yaw_goal - a) * k;
        self.pitch += (pitch_goal - self.pitch) * k;
        self.rise += (rise_in - self.rise) * k;
        self.fov += (fov_in - self.fov) * k;
    }
}

/// The event camera's events (camTrackCS records at +800 + 112 e, F-87722).
pub const EVENT_NAMES: [&str; 18] = [
    "SSTurbo", "Nitrous", "Tuck", "Wheelie", "Nosie", "TwoWheelSide", "TwoWheelUp", "Burnout", "Air", "Turn", "Reverse", "Hydraulics", "AirRide", "Idle", "Finish", "Results", "Over", "Damageout",
];
pub const EV_BURNOUT: usize = 7;
pub const EV_IDLE: usize = 13;

/// The Burnout view begins (F-24075, A-10955..A-35765): the handbrake (input +20)
/// is held, the burnout charge (chassis +580) is full and the car turns faster than
/// BurnoutTurnThreshold (|angular velocity y|, phys +292) - donuts and 360s on the handbrake.
pub fn burnout_starts(fx: &CamFx, handbrake: f32, charge: f32, yaw_rate: f32) -> bool {
    handbrake > 0.0 && yaw_rate.abs() > fx.burnout_turn && charge >= 1.0
}

/// It ends (A-28109..A-73572) when the handbrake is let go, the charge falls under one
/// half or the turn under half the threshold.
pub fn burnout_ends(fx: &CamFx, handbrake: f32, charge: f32, yaw_rate: f32) -> bool {
    handbrake == 0.0 || charge < 0.5 || yaw_rate.abs() < fx.burnout_turn * 0.5
}

// ---- the event camera's track view (camTrackCS, F-92343; rule in
// work/cameras2/camtrack_rule.md), as the Burnout event runs it ----

/// The fields of one block of a `.camtrackcs` file (`None` = the base block).
fn track_block(text: &str, name: Option<&str>) -> HashMap<String, Vec<f32>> {
    let mut out = HashMap::new();
    let mut depth = 0;
    let mut inside = name.is_none();
    for l in text.lines() {
        let mut it = l.split_whitespace();
        let Some(k) = it.next() else { continue };
        if l.contains('{') {
            depth += 1;
            if depth == 2 {
                inside = name == Some(k);
            }
            continue;
        }
        if k == "}" {
            if depth == 2 {
                inside = name.is_none();
            }
            depth -= 1;
            continue;
        }
        if inside && (depth == 1) == name.is_none() {
            let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
            if !v.is_empty() {
                out.insert(k.to_string(), v);
            }
        }
    }
    out
}

/// What the track camera reads of a car's event camera file for one event: the base block's
/// approach and lens fields, the event's own record (F-68099 -> F-26269).
#[derive(Clone, Debug)]
pub struct TrackTune {
    pub blend: f32,
    pub ret: f32,
    pub track: Vec3,
    pub offset: Vec3,
    pub dist: (f32, f32),
    pub dist_scale: f32,
    pub fov: (f32, f32),
    pub app_rot: f32,
    pub app_x_rot: f32,
    pub app_app: f32,
    pub app_rot_min: f32,
    pub look_at: f32,
    pub ss_speed: (f32, f32),
    pub ss_filter: f32,
    /// the place's approach rates (F-91254): the event's Min/MaxAppXZPos, Min/MaxAppYPos,
    /// RevAppXZPos over the speed range MinSpeed..MaxSpeed (10..25 m/s but Hydraulics' 2..10);
    /// the base block's AppInc / AppDec (how fast the XZ rate moves) and AppPosMin
    pub app_xz: (f32, f32),
    pub app_y: (f32, f32),
    pub rev_app_xz: f32,
    pub app_speed: (f32, f32),
    pub app_inc: f32,
    pub app_dec: f32,
    pub app_pos_min: f32,
    /// the base block's MaxAppXZPos: the XZ rate's value when the car's file is loaded
    /// (F-67439 at the load; the constructor's 7 when the key is missing)
    pub base_app_xz: f32,
    /// the hill angle (F-66356): HillMin, HillMax, HillLerp of the base block; the
    /// event's ApproachOn (off: no hill, the car's own frame)
    pub hill: (f32, f32, f32),
    pub approach_on: bool,
    /// the car's turbo / nitrous lens terms (F-92343: the event camera's class; not the wipeout
    /// or ability camera, plain vehTrackCS)
    pub car_lens: bool,
    /// the collision (F-91096): CollideType (1 / 3 in the files), CollideMinDist, the
    /// camera's near plane (CameraNear, base +172)
    pub collide_type: i32,
    pub collide_min: f32,
    pub near: f32,
}

impl TrackTune {
    pub fn load(data_dir: &str, car: &str, event: &str) -> Option<TrackTune> {
        // (the cut GT-R has no camera files: the R8's, catalog::camera_car)
        let car = crate::catalog::camera_car(car);
        // (a mod car: its own file, then its manifest's camera donor - modfs.rs)
        let text = crate::modfs::car_camera_text(data_dir, car, "_cinematic.camtrackcs", crate::modfs::Donor::Cameras)?;
        let (b, e) = (track_block(&text, None), track_block(&text, Some(event)));
        if e.is_empty() {
            return None;
        }
        let f = |m: &HashMap<String, Vec<f32>>, k: &str, d: f32| m.get(k).and_then(|v| v.first()).copied().unwrap_or(d);
        let v = |m: &HashMap<String, Vec<f32>>, k: &str| m.get(k).filter(|v| v.len() >= 3).map_or(Vec3::ZERO, |v| Vec3::new(v[0], v[1], v[2]));
        let x_rot = f(&e, "AppXRot", 0.0);
        Some(TrackTune {
            blend: f(&e, "BlendTime", f(&b, "BlendTime", 0.7)),
            ret: f(&e, "ReturnTime", 0.0),
            track: v(&e, "TrackTo"),
            offset: v(&e, "Offset"),
            dist: (f(&e, "DistMin", 0.0), f(&e, "DistMax", 0.0)),
            dist_scale: f(&e, "DistScale", 1.0),
            fov: (f(&e, "MinFOV", 50.0), f(&e, "MaxFOV", 50.0)),
            app_rot: f(&b, "AppRot", 60.0),
            app_x_rot: if x_rot != 0.0 { x_rot } else { f(&b, "AppRot", 60.0) },
            app_app: f(&b, "AppApp", 0.7),
            app_rot_min: f(&b, "AppRotMin", 0.01),
            look_at: f(&b, "LookAt", 1.0),
            ss_speed: (f(&b, "SSSpeedMin", 0.0), f(&b, "SSSpeedMax", 50.0)),
            ss_filter: f(&b, "SSFilter", 0.1),
            app_xz: (f(&e, "MinAppXZPos", 0.0), f(&e, "MaxAppXZPos", 0.0)),
            app_y: (f(&e, "MinAppYPos", 0.0), f(&e, "MaxAppYPos", 0.0)),
            rev_app_xz: f(&e, "RevAppXZPos", 0.0),
            app_speed: if event == "Hydraulics" { (2.0, 10.0) } else { (10.0, 25.0) },
            app_inc: f(&b, "AppInc", 3.0),
            app_dec: f(&b, "AppDec", 8.0),
            app_pos_min: f(&b, "AppPosMin", 0.25),
            base_app_xz: f(&b, "MaxAppXZPos", 7.0),
            hill: (f(&b, "HillMin", 0.0), f(&b, "HillMax", 0.0), f(&b, "HillLerp", 0.0)),
            approach_on: f(&e, "ApproachOn", 1.0) != 0.0,
            car_lens: true,
            collide_type: f(&b, "CollideType", 1.0) as i32,
            collide_min: f(&b, "CollideMinDist", 2.0),
            near: { let n = f(&b, "CameraNear", 0.2); if n > 0.0 { n } else { 0.2 } },
        })
    }
}

impl TrackTune {
    /// The wipeout camera (mcPlayerCamera +72, vehTrackCS): its `<class>_wipeout.camtrackcs` is not
    /// on the disc, so these are the constructors' values and F-89557's (crash_camera_rule.md
    /// 1.2): TrackTo 0.75 x the tune's ModelOffset, lens 40, held 6..10 m from the point, the place
    /// nearly still (XZ 0..0.1 m/s, Y 1..2 m/s over 5..10 m/s), turning at AppRot 60 / AppXRot 30;
    /// the Offset is set at each start.
    pub fn wipeout(model_offset: Vec3) -> TrackTune {
        TrackTune {
            blend: 1.2,
            ret: 0.0,
            track: model_offset * 0.75,
            offset: Vec3::new(0.0, 1.0, 7.0),
            dist: (6.0, 10.0),
            dist_scale: 1.0,
            fov: (40.0, 40.0),
            app_rot: 60.0,
            app_x_rot: 30.0,
            app_app: 0.7,
            app_rot_min: 0.01,
            look_at: 1.0,
            ss_speed: (0.0, 50.0),
            ss_filter: 0.1,
            app_xz: (0.0, 0.1),
            app_y: (1.0, 2.0),
            rev_app_xz: 0.1,
            app_speed: (5.0, 10.0),
            app_inc: 5.0,
            app_dec: 10.0,
            app_pos_min: 0.25,
            base_app_xz: 5.0,
            hill: (-0.8, 1.25, 0.1),
            approach_on: true,
            car_lens: false,
            collide_type: 1,
            collide_min: 0.5,
            near: 0.2,
        }
    }

    /// The ability camera (mcPlayerCamera +68, F-42437): a plain vehTrackCS with no file - the
    /// constructors' values (F-59764 over camAppCS / vehBaseCS; r5_rules 5.1): TrackTo (0, 0.8, 0),
    /// Offset (0, 1.9, 7.7), lens 60, 1..10 m from the point, AppRot 60 / AppXRot 10, XZ 2..12 and
    /// Y 5 m/s over 5..35 m/s, RevAppXZPos 20, AppInc 5 / AppDec 10, the hill, CollideType 1 with
    /// no minimum, near 0.2; no collision at all when the car's ability was Agro at its load.
    pub fn ability(agro: bool) -> TrackTune {
        TrackTune {
            blend: 1.2,
            ret: 0.0,
            track: Vec3::new(0.0, 0.8, 0.0),
            offset: Vec3::new(0.0, 1.9, 7.7),
            dist: (1.0, 10.0),
            dist_scale: 1.0,
            fov: (60.0, 60.0),
            app_rot: 60.0,
            app_x_rot: 10.0,
            app_app: 0.7,
            app_rot_min: 0.01,
            look_at: 1.0,
            ss_speed: (0.0, 50.0),
            ss_filter: 0.1,
            app_xz: (2.0, 12.0),
            app_y: (5.0, 5.0),
            rev_app_xz: 20.0,
            app_speed: (5.0, 35.0),
            app_inc: 5.0,
            app_dec: 10.0,
            app_pos_min: 0.25,
            base_app_xz: 0.0,
            hill: (-0.8, 1.25, 0.1),
            approach_on: true,
            car_lens: false,
            collide_type: if agro { 0 } else { 1 },
            collide_min: 0.0,
            near: 0.2,
        }
    }
}

/// The ability camera's place in the director (F-38892, r5_rules 5.2): with the ability running in a
/// chase view and neither the reverse camera nor a blend under way it is pushed over InTime (the event
/// camera's event cleared); when the ability stops it hands back to the view's camera over OutTime.
/// The blends are the director's cosine orbit mix.
#[derive(Default)]
pub struct AbilityCam {
    cam: TrackCam,
    /// 0 off, 1 blending in, 2 on, 3 blending out
    phase: u8,
    left: f32,
    time: f32,
}

impl AbilityCam {
    /// The ability camera is current or the target of a blend (it holds the camera buttons and the
    /// event chooser).
    pub fn active(&self) -> bool {
        self.phase == 1 || self.phase == 2
    }
    pub fn reset(&mut self) {
        self.phase = 0;
    }
    /// One frame. `push`: the ability runs and the pushing gates hold; `running`: it runs at all;
    /// (in, out): InTime / OutTime; `drive` the view's own output. Returns the camera to show and
    /// whether it was pushed this frame.
    pub fn frame(&mut self, push: bool, running: bool, times: (f32, f32), agro: bool, car: &TrackCar, drive: BoomOut, dt: f32, test: SphereTest, aspect: f32) -> (BoomOut, bool) {
        let mut pushed = false;
        if push && !self.active() {
            let t = TrackTune::ability(agro);
            // (a push while handing back turns the blend round)
            let w = if self.phase == 3 { self.left / self.time.max(1e-4) } else { 0.0 };
            if self.phase == 0 {
                self.cam.set(&t, car, 99, 0.0);
            }
            self.phase = 1;
            self.time = times.0;
            self.left = times.0 * (1.0 - w);
            pushed = true;
        } else if !running && self.active() {
            let w = if self.phase == 1 { 1.0 - self.left / self.time.max(1e-4) } else { 1.0 };
            self.phase = 3;
            self.time = times.1;
            self.left = times.1 * w;
        }
        if self.phase == 0 {
            return (drive, pushed);
        }
        let t = TrackTune::ability(agro);
        let out = self.cam.update(&t, car, dt, dt, test, aspect);
        self.left -= dt;
        let w = match self.phase {
            1 if self.left <= 0.0 => {
                self.phase = 2;
                1.0
            }
            1 => 1.0 - self.left / self.time.max(1e-4),
            2 => 1.0,
            _ if self.left <= 0.0 => {
                self.phase = 0;
                return (drive, pushed);
            }
            _ => self.left / self.time.max(1e-4),
        };
        let w = (1.0 - (std::f32::consts::PI * w.clamp(0.0, 1.0)).cos()) * 0.5;
        (if w >= 1.0 { out } else { orbit_mix(&drive, &out, car.pos, w) }, pushed)
    }
}

/// The wipeout camera (mcPlayerCamera +72, vehTrackCS; crash_camera_rule.md 1 and 2): taken by
/// F-37740 while the car is wiped out, damaged out or held by the police, and left with a
/// cut when that is over.
#[derive(Default)]
pub struct CrashCam {
    cam: TrackCam,
    tune: Option<TrackTune>,
    /// 0 off, 1 blending in from the drive view, 2 current
    phase: u8,
    left: f32,
    time: f32,
    /// the place's XZ rate has its load value (5, vt+16) until the first wipeout of a car
    used: bool,
}

/// Where F-37740 puts the wipeout camera and how it comes in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrashStart {
    /// the Offset (+448): (0, 1, 7) behind, or (+-6, 0, 0) to the side the car slides toward
    pub offset: Vec3,
    /// the director's orbit blend (mode 3), 0 = a cut (with the transition mask)
    pub blend: f32,
    /// ReverseOn -1 (an exploding car)
    pub exploding: bool,
}

impl CrashStart {
    /// F-37740's choice: BUMPER / POV or an exploding car cut (with the TRANSITIONMOVIE mask
    /// 0.01, 0.1, 0.01); a bust or a car past 84 degrees over blends 0.5 s to 7 m behind and 1 m
    /// up; else 0.25 s to 6 m on the side the car slides toward (its right axis . velocity).
    pub fn choose(view: u8, exploding: bool, police: bool, up_y: f32, right: Vec3, vel: Vec3) -> CrashStart {
        let behind = Vec3::new(0.0, 1.0, 6.0 + 1.0);
        if view < 2 || exploding {
            CrashStart { offset: behind, blend: 0.0, exploding }
        } else if police || up_y <= 0.1 {
            CrashStart { offset: behind, blend: 0.5, exploding }
        } else {
            let side = if right.dot(vel) > 0.0 { 1.0 } else { -1.0 };
            CrashStart { offset: Vec3::new(side * 6.0, 0.0, 0.0), blend: 0.25, exploding }
        }
    }
}

impl CrashCam {
    pub fn active(&self) -> bool {
        self.phase != 0
    }
    /// The drive view comes back (a cut, F-53309), or the Damageout view has taken over.
    pub fn stop(&mut self) {
        self.phase = 0;
    }
    /// A new car: the place's XZ rate starts again at the load's 5.
    pub fn new_car(&mut self) {
        self.used = false;
    }
    pub fn start(&mut self, model_offset: Vec3, car: &TrackCar, how: CrashStart) {
        let mut t = TrackTune::wipeout(model_offset);
        t.offset = how.offset;
        if !self.used {
            self.cam.app_xz = 5.0;
            self.used = true;
        }
        // (reset: snap, the side 0 - no mirror)
        self.cam.set(&t, car, 99, 0.0);
        self.cam.rev = how.exploding;
        self.tune = Some(t);
        self.phase = if how.blend > 0.0 { 1 } else { 2 };
        self.time = how.blend;
        self.left = how.blend;
    }
    /// One frame: `dt` the unscaled frame time (+184 = 1), `dt_game` the game's; `drive` the
    /// drive view's output (the blend's source).
    pub fn frame(&mut self, car: &TrackCar, drive: BoomOut, dt: f32, dt_game: f32, test: SphereTest, aspect: f32) -> BoomOut {
        let Some(t) = self.tune.clone() else { return drive };
        if self.phase == 0 {
            return drive;
        }
        let out = self.cam.update(&t, car, dt, dt_game, test, aspect);
        if self.phase == 1 {
            self.left -= dt;
            if self.left <= 0.0 {
                self.phase = 2;
            } else {
                let w = 1.0 - self.left / self.time.max(1e-4);
                let w = (1.0 - (std::f32::consts::PI * w.clamp(0.0, 1.0)).cos()) * 0.5;
                return orbit_mix(&drive, &out, car.pos, w);
            }
        }
        out
    }
}

/// A time warper of the player (mcTimeWarper, crash_camera_rule.md 5): `tune/vehicle/<name>.timewarper`
/// = SlowMotionDuration, SlowMotionTimeWarp, RiseScaleTimeWarp, FallScaleTimeWarp. Started, it eases
/// (sin) from the scale of the moment to the warp at Rise a second, holds until Duration after the
/// start, then eases back to 1 at Fall a second; on real time.
#[derive(Clone, Copy, Debug)]
pub struct TimeWarper {
    pub tune: [f32; 4],
    /// 0 easing in, 1 easing out, 2 idle, 3 holding
    state: u8,
    from: f32,
    to: f32,
    pub cur: f32,
    t: f32,
    since: f32,
    rate: f32,
}

impl TimeWarper {
    pub fn new(tune: [f32; 4]) -> TimeWarper {
        TimeWarper { tune, state: 2, from: 1.0, to: 1.0, cur: 1.0, t: 0.0, since: 0.0, rate: 0.0 }
    }
    /// The tune file's four values (`m_rSlowMotionDuration` ..), over the defaults given.
    pub fn load(data_dir: &str, name: &str, tune: [f32; 4]) -> TimeWarper {
        let mut tune = tune;
        let h = mcla_formats::hash::joaat(&format!("{name}.timewarper"));
        if let Ok(text) = std::fs::read_to_string(Path::new(data_dir).join("tune/vehicle").join(format!("0x{h:08x}"))) {
            for l in text.lines() {
                let mut it = l.split_whitespace();
                let (Some(k), Some(v)) = (it.next(), it.next().and_then(|x| x.parse::<f32>().ok())) else { continue };
                match k {
                    "m_rSlowMotionDuration" => tune[0] = v,
                    "m_rSlowMotionTimeWarp" => tune[1] = v,
                    "m_rRiseScaleTimeWarp" => tune[2] = v,
                    "m_rFallScaleTimeWarp" => tune[3] = v,
                    _ => {}
                }
            }
        }
        TimeWarper::new(tune)
    }
    /// F-85638: `now` is the game's time scale of the moment; one update follows at once.
    pub fn start(&mut self, now: f32, dt: f32) {
        self.since = 0.0;
        self.state = 0;
        self.from = now;
        self.rate = self.tune[2];
        self.t = if self.rate > 0.0 { 0.0 } else { 1.0 };
        self.to = self.tune[1];
        self.cur = self.tune[1];
        self.update(dt);
    }
    /// F-79714: back to 1 from the scale of the moment `now` (also when idle: the original returns
    /// only for a disabled warper, and the port's are never disabled).
    pub fn release(&mut self, now: f32) {
        self.to = 1.0;
        self.state = 1;
        self.from = now;
        self.cur = now;
        self.rate = self.tune[3];
        self.t = if self.rate > 0.0 { 0.0 } else { 1.0 };
    }
    pub fn reset(&mut self) {
        *self = TimeWarper::new(self.tune);
    }
    /// The ramp (F-73069): in the two easing states t += rate dt (fused), the value `to` from
    /// t = 1, `from` up to t = 0, between them from + (to - from) sin(t pi/2) (the guest's sin, fused).
    fn ramp(&mut self, dt: f32) {
        if self.state > 1 {
            return;
        }
        self.t = self.rate.mul_add(dt, self.t);
        self.cur = if self.t >= 1.0 {
            self.to
        } else {
            let k = if self.t > 0.0 { crate::physics::exact::guest_sin((self.t * std::f32::consts::FRAC_PI_2) as f64) as f32 } else { 0.0 };
            (self.to - self.from).mul_add(k, self.from)
        };
    }
    /// F-25863, once a frame on the unscaled frame time.
    pub fn update(&mut self, dt: f32) {
        self.ramp(dt);
        self.since += dt;
        if self.tune[0] > 0.0 && self.since > self.tune[0] && self.state == 3 {
            let now = self.cur;
            self.release(now);
        }
        if self.t >= 1.0 {
            match self.state {
                0 => self.state = 3,
                1 => {
                    self.state = 2;
                    self.since = 0.0;
                }
                _ => {}
            }
            self.t = 0.0;
        }
    }
}

/// What the track camera needs of the car.
#[derive(Clone, Copy, Debug)]
pub struct TrackCar {
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    /// chassis +224 (m/s), and whether the gearbox is in reverse (the lens then takes 0)
    pub speed: f32,
    pub reverse_gear: bool,
    /// the steering (chassis +132; the port's pad steer, -1 left .. 1 right)
    pub steer: f32,
    /// the front axle's z in the car (the first wheel of the chassis' held list, wheel +392)
    pub front_z: f32,
    /// the ground's slope along the car: the mean over the wheels on the ground of the y of
    /// the car's back axis laid into each wheel's ground plane (wheel +352 / +356); None with
    /// no wheel down
    pub slope: Option<f32>,
    /// the chassis' air time (vt+52): over 0.5 s and 10 m/s the place is set behind the motion
    pub air_time: f32,
    /// mcCarAero +4: 1, or AeroDampFactor while the car is wiped out and moving (it scales the
    /// place's XZ approach rate)
    pub aero: f32,
    /// chassis +100: the tune's ModelOffset.y - the track point's height in the car's frame is
    /// capped there, the rest of TrackTo.y goes along the world's up (F-10185)
    pub model_offset_y: f32,
    /// the lens terms of F-92343: mcCarSSTurbo +20 / +24 (degrees, distance scale; 0 / 1 at
    /// rest) and mcCarNitro +24 (the shot's widening)
    pub sst: (f32, f32),
    pub nitro_view: f32,
    /// the wheels on the ground and the chassis' wheel count (F-70292's 0.51 test) and the
    /// body's angular momentum in the world (body +320)
    pub grounded: usize,
    pub wheels: usize,
    pub ang_mom: Vec3,
}

impl TrackCar {
    /// The slope under a car's wheels (see `slope`).
    pub fn slope_of(rot: Quat, wheels: &[crate::physics::car::WheelState]) -> Option<f32> {
        let back = rot * Vec3::Z;
        let ys: Vec<f32> = wheels
            .iter()
            .filter(|w| w.grounded)
            .map(|w| {
                let n = w.normal.normalize_or(Vec3::Y);
                (back - n * back.dot(n)).normalize_or(back).y
            })
            .collect();
        (!ys.is_empty()).then(|| ys.iter().sum::<f32>() / ys.len() as f32)
    }
}

/// camTrackCS' approach step (F-54792): `cur` goes towards `tgt` by s * rate * dt, s the
/// distance (quadratic within `min`) through the AppApp lag `st`; no overshoot. The lag is
/// per call of the original's camera update (once a 30 Hz game frame).
fn approach(cur: f32, tgt: f32, min: f32, rate_dt: f32, app: f32, st: &mut f32, frames: f32) -> f32 {
    let d = (tgt - cur).abs();
    let mut s = if min != 0.0 && d <= min { d / min * d } else { d };
    // (the original: st = fma(s - st, AppApp, st) once per call, the fused add of the asm; a
    // different frame length takes the lag to that step)
    let lag = if frames == 1.0 { app } else { 1.0 - (1.0 - app.clamp(0.0, 1.0)).powf(frames) };
    *st = (s - *st).mul_add(lag, *st);
    if (*st < 0.0 && s > 0.0) || (*st > 0.0 && s < 0.0) {
        *st = 0.0;
        s = 0.0;
    } else {
        s = *st;
    }
    // (cur = fma(s, rate dt, cur) towards the target, held there when it would pass it)
    if cur < tgt {
        s.mul_add(rate_dt, cur).min(tgt)
    } else if cur > tgt {
        (-s).mul_add(rate_dt, cur).max(tgt)
    } else {
        cur
    }
}

fn wrap_pi(a: f32) -> f32 {
    let mut a = a;
    while a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    while a <= -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

/// yaw (about y, 0 = looking along -z) and pitch of a direction
fn yaw_pitch(d: Vec3) -> (f32, f32) {
    let d = d.normalize_or(Vec3::NEG_Z);
    ((-d.x).atan2(-d.z), d.y.clamp(-1.0, 1.0).asin())
}

fn dir_of(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos())
}

/// The track camera with one event view on (F-26269 then F-92343 each frame).
#[derive(Default)]
struct TrackCam {
    snap: bool,
    track: Vec3,
    offset: Vec3,
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    st_yaw: f32,
    st_pitch: f32,
    speed: f32,
    /// the place's XZ approach rate (+276; not touched by the reset) and the per-axis lag (+416)
    app_xz: f32,
    st_pos: [f32; 3],
    /// the hill angle (+616; not touched by the reset either)
    hill: f32,
    /// the collision's kept distance (+672) and state bits (+668); the reset zeroes both
    dist: f32,
    bits: u32,
    /// byte +156: the shown camera is the pulled-in one this frame
    corrected: bool,
    /// ReverseOn -1 (+208, the wipeout camera of an exploding car): byte +656 set, the hill's
    /// sign turned
    rev: bool,
    /// F-70292's grounded filter: +580 / +584 (the time off / on the ground) and +588 = 0
    /// (stored turned round: the constructor's 588 = 1 is `false` here); none of them is
    /// touched by the event's reset
    t_off: f32,
    t_on: f32,
    off_ground: bool,
    /// the look object's yaw (+576 -> +20): turned only by Hydraulics' CAM_PAN at 2 rad/s;
    /// zeroed at the event's entry and exit
    look_yaw: f32,
}

impl TrackCam {
    /// The event is set (F-26269): Burnout's TrackTo.z = the front axle; Offset and
    /// TrackTo mirrored when the event's side (`EventDirector::side`) is above zero; the next
    /// frame snaps (reset F-88984: the filtered speed starts at the car's).
    fn set(&mut self, t: &TrackTune, car: &TrackCar, ev: usize, side: f32) {
        let mut track = if ev == EV_BURNOUT { Vec3::new(t.track.x, t.track.y, car.front_z) } else { t.track };
        let mut offset = t.offset;
        if side > 0.0 {
            track.x = -track.x;
            offset.x = -offset.x;
        }
        let (app_xz, hill, t_off, t_on, off_ground) = (self.app_xz, self.hill, self.t_off, self.t_on, self.off_ground);
        *self = TrackCam { snap: true, track, offset, speed: if car.reverse_gear { 0.0 } else { car.speed }, app_xz, hill, t_off, t_on, off_ground, ..Default::default() };
    }

    /// Where the camera wants to stand (F-10185): the Offset in the car's level heading
    /// (behind the motion after half a second in the air above 10 m/s), its y along the world's up.
    fn place(&self, car: &TrackCar, at: Vec3) -> Vec3 {
        let m = Mat3::from_quat(car.rot);
        let back = if car.air_time > 0.5 && car.speed > 10.0 && car.vel.length_squared() > 1e-4 { -car.vel.normalize() } else { m.z_axis };
        let x = Vec3::Y.cross(back).normalize_or(m.x_axis);
        let zh = x.cross(Vec3::Y);
        at + x * self.offset.x + zh * self.offset.z + Vec3::Y * self.offset.y
    }

    /// `dt_game`: the game's step (G+8, slowed by a time warp) - the XZ rate's own approach
    /// runs on it; everything else on `dt` (the camera's clock, +184).
    fn update(&mut self, t: &TrackTune, car: &TrackCar, dt: f32, dt_game: f32, test: SphereTest, aspect: f32) -> BoomOut {
        // (per call of the original: once a 30 Hz game frame)
        let frames = (dt * 30.0).max(0.0);
        // the lens: the filtered speed over SSSpeedMin..Max, eased (1 - cos(pi x)) / 2
        let spd = if car.reverse_gear { 0.0 } else { car.speed };
        self.speed += (spd - self.speed) * (1.0 - (1.0 - t.ss_filter.clamp(0.0, 1.0)).powf(frames));
        let k = if t.ss_speed.0 < t.ss_speed.1 {
            let x = ((self.speed - t.ss_speed.0) / (t.ss_speed.1 - t.ss_speed.0)).clamp(0.0, 1.0);
            (1.0 - (std::f32::consts::PI * x).cos()) * 0.5
        } else {
            0.0
        };
        // (F-92343: the car's slipstream turbo adds its lens degrees and scales the distance,
        // the nitrous shot adds its widening - whatever the event; the event camera's class only)
        let (lf, ld) = if t.car_lens { (car.sst.0 + car.nitro_view, car.sst.1) } else { (0.0, 1.0) };
        let fov = t.fov.0 + (t.fov.1 - t.fov.0) * k + lf;
        let dsc = (1.0 + (t.dist_scale - 1.0) * k) * ld;
        // the grounded filter (F-70292, game time): more than 0.51 of the wheels down for over
        // 0.1 s is on the ground, fewer for over 0.1 s off it; off it and spinning with |L| over
        // 1500 the place's XZ rate goes for MinAppXZPos (+592)
        if car.grounded as i32 > (car.wheels as f32 * 0.51) as i32 {
            self.t_on += dt_game;
            if self.t_on > 0.1 {
                self.t_off = 0.0;
                self.off_ground = false;
            }
        } else {
            self.t_off += dt_game;
            if self.t_off > 0.1 {
                self.t_on = 0.0;
                self.off_ground = true;
            }
        }
        let tumble = self.off_ground && car.ang_mom.length_squared() > 2.25e6;
        // the point it tracks (F-10185): TrackTo through the whole car matrix, its height capped
        // at the tune's ModelOffset.y and the rest added along the world's up (+ VertOffset, 0
        // after every event reset)
        let m = Mat3::from_quat(car.rot);
        let ty = if car.model_offset_y > self.track.y { self.track.y } else { car.model_offset_y };
        let mut at = car.pos + m * Vec3::new(self.track.x, ty, self.track.z);
        at.y += self.track.y - ty;
        // the hill angle (F-66356): the slope under the wheels (its sign turned for a
        // camera ahead of the car), eased per frame by HillLerp into asin(s * HillMax) up or
        // asin(s * HillMin) down; none without ApproachOn. (HillLerp per 30 Hz
        // frame: the game halves it only at a 60 Hz refresh (-framelock=1), never in play)
        let (h_min, h_max, h_lerp) = t.hill;
        if !t.approach_on || h_lerp <= 0.0 || h_max <= h_min {
            self.hill = 0.0;
        } else {
            let s = car.slope.unwrap_or(0.0) * if (self.offset.z < 0.0) != self.rev { -1.0 } else { 1.0 };
            let goal = if s < 0.0 { -(s * h_min).min(0.9).asin() } else { (s * h_max).min(0.9).asin() };
            let l = 1.0 - (1.0 - h_lerp.min(1.0)).powf(frames);
            self.hill += (goal - self.hill) * l;
        }
        // the place, turned about the point by the hill angle round the camera's level right
        // axis (F-10185: rows * R(A, -pitch))
        // (ApproachOn 0, byte +572: the Offset in the car's whole frame, its y along the world's up)
        let want = if t.approach_on { self.place(car, at) } else { at + m.x_axis * self.offset.x + m.z_axis * self.offset.z + Vec3::Y * self.offset.y };
        let rel = want - at;
        let a = Vec3::Y.cross(rel.normalize_or(Vec3::Z)).normalize_or(Vec3::X);
        let want = if self.hill != 0.0 { at + Quat::from_axis_angle(a, -self.hill) * rel } else { want };
        // the look yaw turns the place about the point round the world's up (F-67990); the
        // deflection runs only while the look object is all zero
        let want = if self.look_yaw != 0.0 { at + Quat::from_rotation_y(self.look_yaw) * (want - at) } else { want };
        let want = if self.look_yaw == 0.0 { deflect(at, want, t, test) } else { want };
        // the place's approach rates (F-91254): over the speed range, the XZ one moving
        // towards its goal at AppInc / AppDec a second, in reverse gear RevAppXZPos
        let s = car.speed;
        let u = if t.app_speed.1 != t.app_speed.0 && s > t.app_speed.0 { ((s - t.app_speed.0) / (t.app_speed.1 - t.app_speed.0)).min(1.0) } else { 0.0 };
        let mut goal = if t.app_xz.0 > 0.0 {
            // (off the ground and tumbling: MinAppXZPos)
            if tumble { t.app_xz.0 } else { t.app_xz.1 + (t.app_xz.0 - t.app_xz.1) * u }
        } else {
            t.app_xz.1
        };
        if self.look_yaw != 0.0 {
            goal = t.app_xz.1;
        }
        if car.aero < 1.0 {
            goal *= car.aero;
        }
        if car.reverse_gear && !self.rev {
            goal = t.rev_app_xz;
        }
        let step = if goal < self.app_xz { t.app_dec } else { t.app_inc } * dt_game;
        self.app_xz += (goal - self.app_xz).clamp(-step, step);
        let app_y = t.app_y.0 + (t.app_y.1 - t.app_y.0) * u;
        if self.snap || !t.approach_on {
            // (F-48560: with ApproachOn 0 the output is the computed matrix every frame - no
            // place approach, no distance clamp, no turn; it looks at the point, world up)
            self.pos = want;
            let (y, p) = yaw_pitch(at - self.pos);
            self.yaw = y;
            self.pitch = p;
            self.snap = false;
        } else {
            // the place follows (F-48560; Burnout's App*Pos are 0: it stays), then the
            // distance is held in [DistMin, DistMax] x the lens' scale (F-59594)
            for i in 0..3 {
                let rate = if i == 1 { app_y } else { self.app_xz };
                self.pos[i] = approach(self.pos[i], want[i], t.app_pos_min, rate * dt, t.app_app, &mut self.st_pos[i], frames);
            }
            let (lo, hi) = (t.dist.0 * dsc, t.dist.1 * dsc);
            let d = self.pos - at;
            let len = d.length();
            if hi != 0.0 && hi >= lo && len > 1e-4 {
                if len > hi {
                    self.pos = at + d / len * hi;
                } else if len < lo {
                    self.pos = at + d / len * lo;
                }
            }
            // and it turns to watch the point: yaw at AppRot, pitch at AppXRot (LookAt 1)
            let (ty, tp) = yaw_pitch(at - self.pos);
            let ty = self.yaw + wrap_pi(ty - self.yaw);
            self.yaw = wrap_pi(approach(self.yaw, ty, t.app_rot_min, t.app_rot * dt, t.app_app, &mut self.st_yaw, frames));
            self.pitch = approach(self.pitch, tp, t.app_rot_min, t.app_x_rot * dt, t.app_app, &mut self.st_pitch, frames);
        }
        let reach = (at - self.pos).length().max(1.0);
        let look = self.pos + dir_of(self.yaw, self.pitch) * reach;
        // the shown camera pulled in from walls (F-91096); the look keeps its direction
        let pos = self.pull_in(t, at, dt, test, fov, aspect);
        BoomOut { pos, look: look + (pos - self.pos), up: Vec3::Y, fov }
    }

    /// CollideType 1 / 3 (F-91096, TYPE13): a ball of the near plane's corner + 0.1 from
    /// the track point to the camera; on a touch the kept distance (+672) goes in 30 % a
    /// frame to the touch (never under CollideMinDist), when clear it goes back out at
    /// MinAppXZPos m/s while the state bits last (as coded: with MinAppXZPos 0 it snaps back).
    /// The camera stands on the line from the point at that distance. (approx.: type 3's four
    /// class sweeps, which only add state bits, are left out)
    fn pull_in(&mut self, t: &TrackTune, at: Vec3, dt: f32, test: SphereTest, fov: f32, aspect: f32) -> Vec3 {
        let p = self.pull_in_at(t, at, dt, test, fov, aspect);
        self.corrected = p != self.pos;
        p
    }

    fn pull_in_at(&mut self, t: &TrackTune, at: Vec3, dt: f32, test: SphereTest, fov: f32, aspect: f32) -> Vec3 {
        if t.collide_type != 1 && t.collide_type != 3 {
            return self.pos;
        }
        let old = self.bits;
        self.bits &= !0x3C01;
        let active = old != 0;
        let c = self.pos;
        let l = (c - at).length();
        if l < 1e-4 {
            return c;
        }
        let u = (c - at) / l;
        let ty = (fov.to_radians() * 0.5).tan();
        let r = t.near * (ty * ty + (ty * aspect) * (ty * aspect)).sqrt() + 0.1;
        if self.dist == 0.0 {
            self.dist = l;
        }
        let mut f = None;
        let mut n = Vec3::ZERO;
        if let Some((_, hn)) = test.contacts(at, r, true, 1).first().copied().filter(|(_, hn)| hn.dot(u) < -1e-5) {
            f = Some(0.0);
            n = hn;
        } else if let Some((ft, hn)) = test.sweep(at, c, r, true).filter(|(_, hn)| hn.dot(u) < -1e-5) {
            f = Some(ft);
            n = hn;
        }
        if f.is_some() {
            self.bits |= 1;
            let s = Vec3::new(-u.z, 0.0, u.x);
            let v = u.cross(s);
            self.bits &= !0x18;
            let a = n.dot(s);
            if a < -0.001 {
                self.bits = (self.bits | 0x8) & !0x10;
            } else if a > 0.001 {
                self.bits = (self.bits | 0x10) & !0x8;
            }
            let b = n.dot(v);
            if b > 0.001 {
                self.bits = (self.bits | 0x2) & !0x4;
            } else if b < -0.001 {
                self.bits = (self.bits | 0x4) & !0x2;
            }
        }
        self.bits &= !0x20;
        let rate = t.app_xz.0;
        if let Some(f) = f {
            let mut d = f * l;
            if t.collide_min > 0.0 && d < t.collide_min {
                d = t.collide_min;
                self.bits |= 0x20;
            }
            if rate > 0.0 && d > self.dist {
                self.dist = (self.dist + rate * dt).min(d);
            } else {
                self.dist += (d - self.dist) * (1.0 - 0.7f32.powf(dt * 30.0));
            }
            return at + u * self.dist;
        }
        if active {
            let shown = if rate > 0.0 {
                // (F-90102: linear at MinAppXZPos m/s, shown until it is there)
                self.dist += (l - self.dist).clamp(-rate * dt, rate * dt);
                (self.dist - l).abs() > 1e-6
            } else {
                self.dist += (l - self.dist) * (1.0 - 0.7f32.powf(dt * 30.0));
                (self.dist - l).abs() <= 1e-4
            };
            if shown {
                return at + u * self.dist;
            }
            self.bits = 0;
            return c;
        }
        self.dist = l;
        self.bits = 0;
        c
    }
}

/// What the cameras' probes see (`physics::camprobe::CamLevel` in the game): a ball's
/// contacts at a point, up to `max` by the original's result list, (depth, normal); a ball swept
/// a -> b, directed, its first touch (t, normal). `cars`: the other cars too, else the city only.
pub trait CamWorld {
    fn contacts(&self, c: Vec3, r: f32, cars: bool, max: usize) -> Vec<(f32, Vec3)>;
    fn sweep(&self, a: Vec3, b: Vec3, r: f32, cars: bool) -> Option<(f32, Vec3)>;
}

pub type SphereTest<'a> = &'a dyn CamWorld;

impl CamWorld for crate::physics::camprobe::CamLevel<'_> {
    fn contacts(&self, c: Vec3, r: f32, cars: bool, max: usize) -> Vec<(f32, Vec3)> {
        crate::physics::camprobe::CamLevel::contacts(self, c, r, cars, max).into_iter().map(|k| (k.depth, k.normal)).collect()
    }
    fn sweep(&self, a: Vec3, b: Vec3, r: f32, cars: bool) -> Option<(f32, Vec3)> {
        crate::physics::camprobe::CamLevel::sweep(self, a, b, r, cars)
    }
}

/// A plain ball test as a world (the tests): its one contact; the sweep stepped and bisected.
pub struct Balls<F>(pub F);

impl<F: Fn(Vec3, f32) -> Option<(f32, Vec3)>> CamWorld for Balls<F> {
    fn contacts(&self, c: Vec3, r: f32, _cars: bool, max: usize) -> Vec<(f32, Vec3)> {
        (self.0)(c, r).into_iter().take(max).collect()
    }
    fn sweep(&self, a: Vec3, b: Vec3, r: f32, _cars: bool) -> Option<(f32, Vec3)> {
        step_sweep(&self.0, a, b, r)
    }
}

/// Nothing to touch.
pub const NO_WORLD: Balls<fn(Vec3, f32) -> Option<(f32, Vec3)>> = Balls(no_ball);

fn no_ball(_: Vec3, _: f32) -> Option<(f32, Vec3)> {
    None
}

/// A ball swept from a to b on a plain ball test (the tests' worlds): the fraction of the way at
/// the first touch and the normal there; a surface facing along the sweep does not count. Steps
/// of half the radius and a bisection.
fn step_sweep(test: &dyn Fn(Vec3, f32) -> Option<(f32, Vec3)>, a: Vec3, b: Vec3, r: f32) -> Option<(f32, Vec3)> {
    let d = b - a;
    let len = d.length();
    if len < 1e-4 {
        return None;
    }
    let n = ((len / (r * 0.5).max(0.02)).ceil() as usize).clamp(1, 256);
    let mut free = 0.0f32;
    for i in 1..=n {
        let s = i as f32 / n as f32;
        if test(a + d * s, r).is_some() {
            let (mut lo, mut hi) = (free, s);
            for _ in 0..10 {
                let m = (lo + hi) * 0.5;
                if test(a + d * m, r).is_some() {
                    hi = m;
                } else {
                    lo = m;
                }
            }
            let (_, nrm) = test(a + d * hi, r)?;
            return (nrm.dot(d) <= 0.0).then_some((lo, nrm));
        }
        free = s;
    }
    None
}

/// The event camera's target turned away from what blocks it (F-10185, A-24782): with
/// CollideHeight (0.5 for an ApproachOn event, else 0) a ball at the track point, else swept
/// from it to the place; on a touch the place turns about the point by yaw 2 atan(n.A) (at
/// most 45 degrees) and pitch 0.5 atan(n.y) (scaled by 1 - t for the sweep), and never stands
/// below the point. Nothing is kept between frames.
fn deflect(at: Vec3, want: Vec3, t: &TrackTune, test: SphereTest) -> Vec3 {
    let h = if t.approach_on { 0.5 } else { 0.0 };
    if h <= 0.0 {
        return want;
    }
    let z = (want - at).normalize_or(Vec3::Z);
    let a = Vec3::new(z.z, 0.0, -z.x);
    let (yaw, pitch) = if let Some((_, n)) = test.contacts(at, h, false, 1).first().copied() {
        (2.0 * n.dot(a).atan(), 0.5 * n.y.atan())
    } else if let Some((f, n)) = test.sweep(at, want, h, false) {
        (2.0 * (n.dot(a) * (1.0 - f)).atan(), 0.5 * (n.y * (1.0 - f)).atan())
    } else {
        return want;
    };
    let yaw = yaw.clamp(-std::f32::consts::FRAC_PI_4, std::f32::consts::FRAC_PI_4);
    let mut rel = want - at;
    if pitch != 0.0 {
        rel = Quat::from_axis_angle(a.normalize_or(Vec3::X), -pitch) * rel;
    }
    if yaw != 0.0 {
        rel = Quat::from_rotation_y(yaw) * rel;
    }
    let mut p = at + rel;
    p.y = p.y.max(at.y);
    p
}

/// The camera director's orbit blend (F-18583): the focus point (each camera's line of
/// sight out to its distance from the car), the view's angles and the distance go from one
/// camera to the other by w.
pub fn orbit_mix(a: &BoomOut, b: &BoomOut, pivot: Vec3, w: f32) -> BoomOut {
    let (da, db) = ((a.pos - pivot).length(), (b.pos - pivot).length());
    let (ua, ub) = ((a.look - a.pos).normalize_or(Vec3::NEG_Z), (b.look - b.pos).normalize_or(Vec3::NEG_Z));
    let (fa, fb) = (a.pos + ua * da, b.pos + ub * db);
    let (ya, pa) = yaw_pitch(ua);
    let (yb, pb) = yaw_pitch(ub);
    let yaw = ya + wrap_pi(yb - ya) * w;
    let pitch = pa + (pb - pa) * w;
    let d = da + (db - da) * w;
    let focus = fa.lerp(fb, w);
    let dir = dir_of(yaw, pitch);
    BoomOut { pos: focus - dir * d, look: focus, up: a.up.lerp(b.up, w).normalize_or(Vec3::Y), fov: a.fov + (b.fov - a.fov) * w }
}

/// What the event camera's chooser (F-24075) reads of the car, the pad and the camera
/// manager in a frame (rule in docs/reference/cameras/event_chooser_rule.md).
#[derive(Clone, Copy, Debug, Default)]
pub struct EventIn {
    /// the start section's gate besides the event camera's own state: the CINEMATIC CAMERA
    /// option, no director blend, no pause / photo camera; when it falls away a running event
    /// ends (look back, a wreck, a view change - approx.: the original's look back refuses
    /// Hydraulics / AirRide)
    pub allowed: bool,
    /// the drive view (mgr+80: 0 bumper, 1 cockpit, 2/3/4 STATIC/CLASSIC/ACTION)
    pub view: u8,
    pub bike: bool,
    /// gyro +16: 1 wheelie, 99 its end, -1 stoppie, -99 its end
    pub stunt: i16,
    /// gyro +18: a car's two-wheel side, a bike's turn intent (+1 / -1 / 0)
    pub g18: i16,
    /// mcCarSSTurbo +40 (running) and its lens ramp complete (F-58729)
    pub sst_on: bool,
    pub sst_full: bool,
    /// mcCarNitro +32 (a shot burning) and its lens ramp complete (F-94357)
    pub nitro_on: bool,
    pub nitro_full: bool,
    /// a bike's tuck: mcCarAero +40, the low-drag flag (stick Y < -0.5 and under 0.5 s in the air)
    pub tuck: bool,
    /// mcPlayerInput +16 brake, +20 handbrake, +32 stick Y, +36 stick X; chassis +144 (the
    /// handbrake as the sim has it)
    pub brake: f32,
    pub handbrake: f32,
    pub hb_sim: f32,
    pub stick_y: f32,
    pub stick_x: f32,
    /// the chassis' wheel 0 (front) / wheel 1 (rear) touch the ground (wheel +529)
    pub front_down: bool,
    pub rear_down: bool,
    /// chassis +224 (m/s)
    pub speed: f32,
    /// the burnout charge (chassis +580), the body's yaw rate (body +292)
    pub charge: f32,
    pub yaw_rate: f32,
    /// mcCarHydraulics +16 (mode on); CAM_PAN_LEFT / RIGHT pressed; CAM_UP / CAM_DOWN pressed
    pub hyd_on: bool,
    pub pan_pressed: bool,
    /// CAM_PAN_LEFT held -1, CAM_PAN_RIGHT held +1 (left first), else 0
    pub pan_side: f32,
    pub cam_pressed: bool,
    /// the gearbox in reverse (transmission +20 == 0)
    pub reverse_gear: bool,
    /// the chassis' air time (vt+52 = chassis +584)
    pub air_time: f32,
    /// the pad block's idle time (+112)
    pub idle: f32,
    /// the wipeout camera is in use (current, blending in, or the Damageout view's blend source):
    /// the chooser does nothing else; `damage_out`: a destroyed car (not a bike) whose damage-out
    /// timer has run out - Damageout (17) starts; `destroyed`: chassis +592 bit 0x4 (its end test)
    pub crash: bool,
    pub damage_out: bool,
    pub destroyed: bool,
}

/// A segment probe of the world (F-20855, one result): something lies between the points.
pub type RayTest<'a> = &'a dyn Fn(Vec3, Vec3) -> bool;

/// The event camera (camTrackCS at mgr+84) as mcPlayerCamera's chooser runs it: each frame the
/// first event whose start holds is set and blended in over the record's BlendTime with the
/// director's cosine orbit blend (F-94591); its end test sends it back to the drive view
/// over the record's ReturnTime (0: its BlendTime) or a time the test passes.
#[derive(Default)]
pub struct EventDirector {
    cam: TrackCam,
    /// 0 off, 1 blending in, 2 on, 3 blending out
    phase: u8,
    left: f32,
    time: f32,
    quit: bool,
    /// the event on (cam +784)
    ev: usize,
    /// the enable bytes (cam +2816 + e) at 0: such an event may not be set
    off_bytes: [bool; 18],
    /// the game's generator for Idle's side (approx.: the port's own, not 0x82860880's draws)
    rng: u32,
    /// each car's event records and the camFX tune, read once
    tunes: HashMap<(String, usize), Option<TrackTune>>,
    fx: Option<CamFx>,
    /// the car whose event camera file was last loaded (F-67439 sets the XZ rate then)
    loaded: Option<String>,
}

pub const EV_SSTURBO: usize = 0;
pub const EV_NITROUS: usize = 1;
pub const EV_TUCK: usize = 2;
pub const EV_WHEELIE: usize = 3;
pub const EV_NOSIE: usize = 4;
pub const EV_TWO_SIDE: usize = 5;
pub const EV_TWO_UP: usize = 6;
pub const EV_AIR: usize = 8;
pub const EV_TURN: usize = 9;
pub const EV_REVERSE: usize = 10;
pub const EV_HYDRAULICS: usize = 11;
pub const EV_DAMAGEOUT: usize = 17;

impl EventDirector {
    /// The event camera is running (or blending in): the camera buttons act on it.
    pub fn running(&self) -> bool {
        self.phase == 1 || self.phase == 2
    }
    /// The event running (cam +784), if any.
    pub fn event(&self) -> Option<usize> {
        self.running().then_some(self.ev)
    }
    /// The event view is wholly in (no blend in or out).
    pub fn shown(&self) -> bool {
        self.phase == 2
    }
    /// Blending back to the drive view.
    pub fn leaving(&self) -> bool {
        self.phase == 3
    }
    /// F-94591(0, -1, -1): the running event goes back over the record's time.
    pub fn end(&mut self, data_dir: &str, car_name: &str) {
        if self.running() {
            if let Some(t) = self.tune(data_dir, car_name, self.ev) {
                self.off(&t, -1.0);
            }
        }
    }
    /// F-94591(0, -1, b): the running (or leaving) event goes back over `b`; the event is cleared
    /// either way (the gas station's close asks 0.7, F-81587).
    pub fn off_with(&mut self, data_dir: &str, car_name: &str, b: f32) {
        if self.running() || self.leaving() {
            if let Some(t) = self.tune(data_dir, car_name, self.ev) {
                self.off(&t, b);
            }
        }
    }
    /// CAM_UP leaves it (F-67519: F-94591(0, -1, 0.2)).
    pub fn leave(&mut self) {
        self.quit = true;
    }
    pub fn reset(&mut self) {
        self.phase = 0;
        self.quit = false;
    }
    /// The Idle view is on (the HUD is put away while it is).
    pub fn idle(&self) -> bool {
        self.phase != 0 && self.ev == EV_IDLE
    }

    fn tune(&mut self, data_dir: &str, car_name: &str, ev: usize) -> Option<TrackTune> {
        // (a car without its own file takes the Golf's, as the chase views do)
        let name = EVENT_NAMES[ev];
        self.tunes
            .entry((car_name.to_string(), ev))
            .or_insert_with(|| TrackTune::load(data_dir, car_name, name).or_else(|| TrackTune::load(data_dir, "vp_vw_golf_83", name)))
            .clone()
    }

    fn blend_of(&mut self, data_dir: &str, car_name: &str, ev: usize) -> f32 {
        self.tune(data_dir, car_name, ev).map_or(0.0, |t| t.blend)
    }

    /// The chooser's start section (F-24075, part 2): the first event whose test holds.
    fn pick(&mut self, data_dir: &str, car_name: &str, fx: &CamFx, i: &EventIn, car: &TrackCar, test: SphereTest, ray: RayTest) -> Option<usize> {
        let fast = i.speed > 10.0;
        if i.sst_on && i.sst_full && i.view == 4 && self.blend_of(data_dir, car_name, EV_SSTURBO) > 0.0 && i.g18 == 0 {
            return Some(EV_SSTURBO);
        }
        if i.nitro_on && i.nitro_full && i.view == 4 && self.blend_of(data_dir, car_name, EV_NITROUS) > 0.0 && i.g18 == 0 {
            return Some(EV_NITROUS);
        }
        if i.bike && i.tuck && i.brake == 0.0 && fast {
            return Some(EV_TUCK);
        }
        if i.stunt > 0 && !i.front_down && fast {
            return Some(EV_WHEELIE);
        }
        if i.bike && i.stunt < 0 && !i.rear_down && fast {
            return Some(EV_NOSIE);
        }
        if burnout_starts(fx, i.handbrake, i.charge, i.yaw_rate) {
            return Some(EV_BURNOUT);
        }
        if i.hyd_on && i.pan_pressed {
            return Some(EV_HYDRAULICS);
        }
        // (12 AirRide: the air ride's on/off is not in the port's sim)
        if !i.bike && i.g18 != 0 {
            return Some(if i.view == 4 { EV_TWO_SIDE } else { EV_TWO_UP });
        }
        if i.bike && i.g18 != 0 && i.stick_x.abs() > 0.75 && fast {
            return Some(EV_TURN);
        }
        if i.air_time > 0.1 && !i.sst_on && !i.nitro_on {
            // a 2 m ball at the car clear, and the way 2 s ahead along the velocity, falling
            // 5 m more, from half a metre up the car (nothing after Air is tried this frame)
            if !test.contacts(car.pos, 2.0, false, 1).is_empty() {
                return None;
            }
            let a = car.pos + car.rot * Vec3::Y * 0.5;
            let b = a + (car.vel - Vec3::new(0.0, 2.5, 0.0)) * 2.0;
            return (!ray(a, b)).then_some(EV_AIR);
        }
        if i.bike && i.reverse_gear && i.hb_sim != 0.0 && fast {
            return Some(EV_REVERSE);
        }
        (fx.idle_delay > 0.0 && i.idle >= fx.idle_delay).then_some(EV_IDLE)
    }

    /// The side an event is set to (F-26269): its Offset and TrackTo are mirrored when it
    /// is above zero.
    fn side(&mut self, ev: usize, i: &EventIn, car: &TrackCar) -> f32 {
        match ev {
            EV_SSTURBO | EV_TWO_UP | 17 => 0.0,
            EV_NITROUS => 1.0,
            EV_TWO_SIDE => {
                if i.g18 > 0 {
                    -1.0
                } else {
                    1.0
                }
            }
            EV_BURNOUT | 12 => car.steer,
            EV_IDLE => {
                self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                if self.rng >> 31 != 0 {
                    1.0
                } else {
                    -1.0
                }
            }
            16 => -car.steer,
            _ => i.yaw_rate,
        }
    }

    /// The running event's end test (part 4): Some(blend back; < 0 = the record's).
    fn ends(&self, fx: &CamFx, i: &EventIn, car: &TrackCar, ray: RayTest) -> Option<f32> {
        let end = |c: bool, b: f32| c.then_some(b);
        match self.ev {
            EV_SSTURBO => end(!i.sst_on || i.g18 != 0, -1.0),
            EV_NITROUS => end(!i.nitro_on || i.g18 != 0, -1.0),
            EV_TUCK => end(!i.tuck || i.brake > 0.0 || i.stunt != 0, -1.0),
            EV_WHEELIE => end(i.stunt <= 0, -1.0),
            EV_NOSIE => end(i.stunt >= 0 && i.rear_down, -1.0),
            EV_TWO_SIDE | EV_TWO_UP => {
                let tx = self.cam.track.x;
                end(i.g18 == 0 || (tx != 0.0 && i.g18 as f32 != tx.signum()), -1.0)
            }
            EV_BURNOUT => end(burnout_ends(fx, i.handbrake, i.charge, i.yaw_rate), -1.0),
            EV_AIR => {
                // 1 s of the velocity, 1 m drop: clear keeps it; else after half a second in the
                // air it ends, under it keeps until the landing
                let a = car.pos + car.rot * Vec3::Y * 0.5;
                let b = a + car.vel - Vec3::Y;
                end(ray(a, b) && (i.air_time > 0.5 || i.air_time <= 0.0), -1.0)
            }
            EV_TURN => end(i.g18 == 0 || i.stick_x.abs() < 0.75 || i.stick_y.abs() > 0.75 || i.charge > 0.5 || i.tuck, -1.0),
            EV_REVERSE => end(i.hb_sim < 0.01, -1.0),
            EV_HYDRAULICS => end(!i.hyd_on || i.cam_pressed, -1.0),
            // (the pad's buttons zero the idle time - main.rs)
            EV_IDLE => end(i.idle == 0.0, 0.7),
            EV_DAMAGEOUT => end(!i.destroyed, -1.0),
            _ => None,
        }
    }

    /// Turns the event off (F-94591(0, -1, b)): back to the drive view over b, else the
    /// record's ReturnTime, else its BlendTime.
    fn off(&mut self, t: &TrackTune, b: f32) {
        let back = if b >= 0.0 { b } else if t.ret > 0.0 { t.ret } else { t.blend };
        // (leaving while still blending in turns the blend round: w stays where it is)
        let w = if self.phase == 1 { 1.0 - self.left / self.time.max(1e-4) } else { 1.0 };
        // (the event's exit zeroes the look object, F-94591)
        self.cam.look_yaw = 0.0;
        self.phase = 3;
        self.time = back;
        self.left = back * w;
        self.quit = false;
    }

    /// One frame (after the drive view's own). `chase`: the drive view's output this frame;
    /// `test` the world's ball test, `ray` its segment probe.
    pub fn frame(&mut self, data_dir: &str, car_name: &str, i: &EventIn, car: &TrackCar, chase: BoomOut, dt: f32, test: SphereTest, ray: RayTest, aspect: f32) -> BoomOut {
        let fx = self.fx.get_or_insert_with(|| CamFx::load(data_dir)).clone();
        let fx = &fx;
        // the car's file loaded (F-67439): the place's XZ rate starts at the base block's
        // MaxAppXZPos and then keeps its value across events until the next car
        if self.loaded.as_deref() != Some(car_name) {
            self.loaded = Some(car_name.to_string());
            let base = (0..EVENT_NAMES.len()).find_map(|e| self.tune(data_dir, car_name, e)).map_or(7.0, |t| t.base_app_xz);
            self.cam.app_xz = base;
        }
        // Hydraulics' pan (F-38892): CAM_PAN_LEFT / RIGHT turn the look yaw at 2 rad/s while the
        // event camera has the view; it never decays
        if self.running() && self.ev == EV_HYDRAULICS {
            self.cam.look_yaw += dt * i.pan_side * 2.0;
        }
        // part 1: while the wipeout camera is in use the only start is Damageout, whatever the
        // option and the view, and nothing ends
        if i.crash {
            if i.damage_out && self.phase == 0 {
                if let Some(t) = self.tune(data_dir, car_name, EV_DAMAGEOUT).filter(|t| t.blend > 0.0) {
                    self.off_bytes = [false; 18];
                    self.ev = EV_DAMAGEOUT;
                    self.cam.set(&t, car, EV_DAMAGEOUT, 0.0);
                    self.phase = 1;
                    self.time = t.blend;
                    self.left = t.blend;
                }
            }
            if self.phase == 0 {
                return chase;
            }
            let Some(t) = self.tune(data_dir, car_name, self.ev) else { return chase };
            return self.show(&t, car, chase, dt, test, aspect);
        }
        // part 2: an event starts only while the camera is wholly out of the director
        if self.phase == 0 {
            self.quit = false;
            if i.allowed && i.view >= 2 {
                if let Some(ev) = self.pick(data_dir, car_name, fx, i, car, test, ray).filter(|&e| !self.off_bytes[e]) {
                    if let Some(t) = self.tune(data_dir, car_name, ev).filter(|t| t.blend > 0.0) {
                        // (a successful on re-arms every event, vt+36)
                        self.off_bytes = [false; 18];
                        let side = self.side(ev, i, car);
                        self.ev = ev;
                        self.cam.set(&t, car, ev, side);
                        self.phase = 1;
                        self.time = t.blend;
                        self.left = t.blend;
                    }
                }
            }
        }
        if self.phase != 0 {
            let Some(t) = self.tune(data_dir, car_name, self.ev) else {
                self.phase = 0;
                return chase;
            };
            if self.running() {
                // part 3, the pre-emption: in ACTION a wheelie ends the nitrous view; a slipstream
                // turbo or a nitrous shot ends any other event
                let pre = if self.ev == EV_NITROUS && i.stunt != 0 {
                    Some(0.5)
                } else if i.sst_on && i.g18 == 0 && i.view == 4 && self.ev != EV_SSTURBO && self.blend_of(data_dir, car_name, EV_SSTURBO) > 0.0 {
                    Some(-1.0)
                } else if i.nitro_on && i.g18 == 0 && i.view == 4 && self.ev != EV_NITROUS && self.blend_of(data_dir, car_name, EV_NITROUS) > 0.0 {
                    Some(-1.0)
                } else {
                    None
                };
                // (CAM_UP's leave asks for 0.2 s; the start gate falling away for the record's)
                let b = if self.quit {
                    Some(0.2)
                } else if !i.allowed {
                    Some(-1.0)
                } else {
                    pre.or_else(|| self.ends(fx, i, car, ray))
                };
                if let Some(b) = b {
                    self.off(&t, b);
                }
                // Nitrous: the event camera pulled in from a wall ends it (0.2 s; at once while
                // still blending in) and keeps it off until the shot ends
                if self.ev == EV_NITROUS && self.cam.corrected {
                    if self.running() {
                        let b = if self.phase == 1 { 0.0 } else { 0.2 };
                        self.off(&t, b);
                    }
                    self.off_bytes[EV_NITROUS] = true;
                }
            }
            // part 5: the Nitrous byte comes back when the shot ends
            if !i.nitro_on {
                self.off_bytes[EV_NITROUS] = false;
            }
            if self.phase != 0 {
                return self.show(&t, car, chase, dt, test, aspect);
            }
        }
        if !i.nitro_on {
            self.off_bytes[EV_NITROUS] = false;
        }
        chase
    }

    fn show(&mut self, t: &TrackTune, car: &TrackCar, chase: BoomOut, dt: f32, test: SphereTest, aspect: f32) -> BoomOut {
        let ev = self.cam.update(t, car, dt, dt, test, aspect);
        self.left -= dt;
        let w = match self.phase {
            1 if self.left <= 0.0 => {
                self.phase = 2;
                1.0
            }
            1 => 1.0 - self.left / self.time.max(1e-4),
            2 => 1.0,
            _ if self.left <= 0.0 => {
                self.phase = 0;
                return chase;
            }
            _ => self.left / self.time.max(1e-4),
        };
        let w = (1.0 - (std::f32::consts::PI * w.clamp(0.0, 1.0)).cos()) * 0.5;
        if w >= 1.0 {
            ev
        } else {
            orbit_mix(&chase, &ev, car.pos, w)
        }
    }
}

/// One view of the event camera.
#[derive(Clone, Copy, Debug)]
pub struct EventView {
    pub track: Vec3,
    pub offset: Vec3,
    pub blend: f32,
    pub ret: f32,
    pub fov: (f32, f32),
}

/// All the views of a car's event camera file: the base view and the named ones.
pub struct EventViews {
    pub base: EventView,
    pub named: HashMap<String, EventView>,
}

impl EventViews {
    pub fn load(data_dir: &str, car: &str) -> Option<EventViews> {
        // (the cut GT-R has no camera files: the R8's, catalog::camera_car)
        let car = crate::catalog::camera_car(car);
        // (a mod car: its own file, then its manifest's camera donor - modfs.rs)
        let text = crate::modfs::car_camera_text(data_dir, car, "_cinematic.camtrackcs", crate::modfs::Donor::Cameras)?;
        let blank = EventView { track: Vec3::new(0.0, 1.0, 0.0), offset: Vec3::new(0.0, 0.5, 6.0), blend: 0.7, ret: 0.7, fov: (50.0, 50.0) };
        let mut base = blank;
        let mut named = HashMap::new();
        let mut cur: Option<(String, EventView)> = None;
        let mut depth = 0;
        for l in text.lines() {
            let mut it = l.split_whitespace();
            let Some(k) = it.next() else { continue };
            if l.contains('{') {
                depth += 1;
                if depth == 2 {
                    cur = Some((k.to_string(), EventView { fov: (0.0, 0.0), ..blank }));
                }
                continue;
            }
            if k == "}" {
                if depth == 2 {
                    if let Some((n, mut v)) = cur.take() {
                        if v.fov.0 <= 0.0 {
                            v.fov = base.fov;
                        }
                        named.insert(n, v);
                    }
                }
                depth -= 1;
                continue;
            }
            let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
            let view = match cur.as_mut() {
                Some((_, v)) => v,
                None => &mut base,
            };
            match (k, v.len()) {
                ("TrackTo", 3) => view.track = Vec3::new(v[0], v[1], v[2]),
                ("Offset", 3) => view.offset = Vec3::new(v[0], v[1], v[2]),
                ("BlendTime", 1) => view.blend = v[0],
                ("ReturnTime", 1) => view.ret = v[0],
                ("MinFOV", 1) | ("SSFovMin", 1) => view.fov.0 = v[0],
                ("MaxFOV", 1) | ("SSFovMax", 1) => view.fov.1 = v[0],
                _ => {}
            }
        }
        Some(EventViews { base, named })
    }
}

/// The cinematic view: the event camera's base view, cutting to the named views while their
/// event lasts (nitrous, a wheelie, a burnout, air time, reversing).
#[derive(Default)]
pub struct EventCam {
    started: bool,
    track: Vec3,
    offset: Vec3,
    fov: f32,
    yaw: f32,
    event: String,
}

impl EventCam {
    pub fn reset(&mut self) {
        self.started = false;
    }

    /// `event`: the name of the view that applies now ("" = the base view); `speed_share`: 0..1
    /// of the car's top speed (opens the lens between a view's two angles).
    pub fn update(&mut self, views: &EventViews, event: &str, pos: Vec3, forward: Vec3, speed_share: f32, dt: f32) -> BoomOut {
        let target = views.named.get(event).copied().unwrap_or(views.base);
        let fov_t = target.fov.0 + (target.fov.1 - target.fov.0) * speed_share.clamp(0.0, 1.0);
        let yaw_t = (-forward.x).atan2(-forward.z);
        if !self.started {
            self.started = true;
            self.track = target.track;
            self.offset = target.offset;
            self.fov = fov_t;
            self.yaw = yaw_t;
            self.event = event.to_string();
        }
        // towards a named view in its blend time, back to the base in the time the view left names
        let time = if views.named.contains_key(event) { target.blend } else { views.named.get(&self.event).map_or(views.base.blend, |v| if v.ret > 0.0 { v.ret } else { v.blend }) };
        if views.named.contains_key(event) {
            self.event = event.to_string();
        }
        let k = 1.0 - (-dt * 3.0 / time.max(0.05)).exp();
        self.track = self.track.lerp(target.track, k);
        self.offset = self.offset.lerp(target.offset, k);
        self.fov += (fov_t - self.fov) * k;
        // the view turns with the car at 60 degrees a second at most (AppRot)
        let mut d = yaw_t - self.yaw;
        if d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        if d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        let max = 60f32.to_radians() * dt + d.abs() * (1.0 - (-dt * 4.0).exp());
        self.yaw += d.clamp(-max, max);
        let rot = Quat::from_rotation_y(self.yaw);
        let look = pos + rot * self.track;
        BoomOut { pos: look + rot * self.offset, look, up: Vec3::Y, fov: self.fov }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines of `<name>` rows of the original's reference run (`work/triage-cameras/verify/b1.py`
    /// runs the recompiled functions on `ref/b1_cases.txt`; `MCLA_CAMERA_REF_DIR` = its `ref` folder).
    fn reference(file: &str, name: &str) -> Option<Vec<Vec<u32>>> {
        let dir = std::env::var("MCLA_CAMERA_REF_DIR").ok()?;
        let text = std::fs::read_to_string(Path::new(&dir).join(file)).ok()?;
        Some(
            text.lines()
                .filter(|l| l.split(' ').next() == Some(name))
                .map(|l| l.split(' ').skip(1).map(|h| u32::from_str_radix(h, 16).unwrap()).collect())
                .collect(),
        )
    }

    /// F-73069 (the time warper's ramp) against the original's code, 8,000 cases of state, t, rate,
    /// ends and frame time (`b2.py`).
    #[test]
    fn time_warper_ramp_is_the_originals() {
        let Some(rows) = reference("b1_ref.txt", "tw") else { return };
        assert!(rows.len() >= 8000);
        for r in &rows {
            let f = |i: usize| f32::from_bits(r[i]);
            let mut w = TimeWarper::new([0.0; 4]);
            w.state = r[0] as u8;
            (w.t, w.rate, w.from, w.to, w.cur) = (f(1), f(2), f(3), f(4), 10.0);
            w.ramp(f(5));
            assert_eq!((w.t.to_bits(), w.cur.to_bits()), (r[6], r[7]), "case {r:x?}");
        }
    }

    /// F-93983 (the dead zone with rescale) against the original's own code, 4000 cases.
    #[test]
    fn dead_zone_is_the_originals() {
        let Some(rows) = reference("b1_ref.txt", "dz") else { return };
        assert!(rows.len() >= 4000);
        for r in &rows {
            let got = dead_zone(f32::from_bits(r[0]), f32::from_bits(r[1]));
            assert_eq!(got.to_bits(), r[2], "v {:x} d {:x}", r[0], r[1]);
        }
    }

    /// The camTrackCS approach step at the original's own step (one game frame) against the
    /// original's code: 10,000 cases of position, lag state and rates (cap argument 0, lag on).
    #[test]
    fn approach_is_the_originals_at_one_frame() {
        let Some(rows) = reference("b1_ref.txt", "ap") else { return };
        let mut n = 0;
        for r in rows.iter().filter(|r| r[7] == 1) {
            let f = |i: usize| f32::from_bits(r[i]);
            let mut st = f(6);
            let cur = approach(f(0), f(1), f(2), f(4), f(5), &mut st, 1.0);
            assert_eq!((cur.to_bits(), st.to_bits()), (r[8], r[9]), "case {r:x?}");
            n += 1;
        }
        assert!(n >= 7000);
    }

    /// CAM_UP from COCKPIT runs BUMPER, ACTION, CLASSIC, STATIC, COCKPIT (recorded in the
    /// recomp, work/cameras2/cap s_p1..s_p6); CAM_DOWN the other way.
    #[test]
    fn camera_button_order() {
        let mut v = PORT_OF_VIEW[1];
        let mut seen = vec![];
        for _ in 0..5 {
            v = next_view(v, true);
            seen.push(VIEW_OF_PORT[v as usize]);
        }
        assert_eq!(seen, vec![0, 4, 3, 2, 1]);
        for _ in 0..5 {
            let w = next_view(v, false);
            assert_eq!(next_view(w, true), v);
            v = w;
        }
        // the test hook's event camera (5) counts as Boom 3
        assert_eq!(VIEW_OF_PORT[next_view(5, true) as usize], 3);
    }

    #[test]
    fn cuts_and_blends() {
        // (recorded: COCKPIT->BUMPER, BUMPER->ACTION, STATIC->COCKPIT flash; ACTION->CLASSIC,
        // CLASSIC->STATIC do not)
        let p = |v: usize| PORT_OF_VIEW[v];
        assert!(is_cut(p(1), p(0)) && is_cut(p(0), p(4)) && is_cut(p(2), p(1)));
        assert!(!is_cut(p(4), p(3)) && !is_cut(p(3), p(2)));
    }

    /// The view change's mask: black on the frame of the cut (86.7 % after the first movie
    /// frame), off 13.33 points a frame, gone on the 8th frame (recorded: about 7-8 frames).
    #[test]
    fn cut_mask_timing() {
        let mut m = CutMask::default();
        m.start(0.01, 0.25, 0.01);
        let a: Vec<f32> = (0..9).map(|_| m.update(1.0 / 30.0)).collect();
        assert!((a[0] - 0.8667).abs() < 1e-3, "{a:?}");
        assert!(a[6] > 0.0 && a[7] == 0.0, "{a:?}");
        let mut m = CutMask::default();
        m.start(0.01, 0.1, 0.01);
        let a: Vec<f32> = (0..4).map(|_| m.update(1.0 / 30.0)).collect();
        // (3.333 / 0.1 leaves 0.01 % on the third frame, as the movie's own sums do)
        assert!((a[0] - 0.6667).abs() < 1e-3 && a[2] < 1e-3 && a[3] == 0.0, "{a:?}");
    }

    /// R4 (r4_event_rules.md): ApproachOn 0 copies the computed place in the car's whole frame
    /// every frame; the slipstream turbo / nitrous lens terms; TrackTo.y above ModelOffset.y along
    /// the world's up; MinAppXZPos while off the ground and tumbling; MaxAppXZPos with a look yaw.
    #[test]
    fn event_r4_rules() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let Some(t) = TrackTune::load(&data, "vp_aud_r8_08", "SSTurbo") else { return };
        assert!(!t.approach_on && t.base_app_xz == 12.0, "{t:?}");
        let none: SphereTest = &NO_WORLD;
        let car0 = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 30.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (10.0, 1.2), nitro_view: 5.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO };
        let mut c = TrackCam::default();
        c.set(&t, &car0, EV_SSTURBO, 0.0);
        let o = c.update(&t, &car0, 1.0 / 30.0, 1.0 / 30.0, none, 16.0 / 9.0);
        let k = { let x = ((c.speed - t.ss_speed.0) / (t.ss_speed.1 - t.ss_speed.0)).clamp(0.0, 1.0); (1.0 - (std::f32::consts::PI * x).cos()) * 0.5 };
        assert!((o.fov - (t.fov.0 + (t.fov.1 - t.fov.0) * k + 15.0)).abs() < 1e-3, "{}", o.fov);
        // a rolled, moved car: the camera is where the frame puts it at once (no approach)
        let car = TrackCar { pos: Vec3::new(5.0, 1.0, -20.0), rot: Quat::from_rotation_y(0.7) * Quat::from_rotation_z(0.4), ..car0 };
        let o = c.update(&t, &car, 1.0 / 30.0, 1.0 / 30.0, none, 16.0 / 9.0);
        let m = Mat3::from_quat(car.rot);
        let ty = t.track.y.min(0.708);
        let at = car.pos + m * Vec3::new(t.track.x, ty, t.track.z) + Vec3::Y * (t.track.y - ty);
        let want = at + m.x_axis * t.offset.x + m.z_axis * t.offset.z + Vec3::Y * t.offset.y;
        assert!(o.pos.distance(want) < 1e-4, "{:?} {:?}", o.pos, want);
        assert!((o.look - o.pos).normalize().dot((at - want).normalize()) > 0.9999);
        // the tumble: MinAppXZPos once off the ground over 0.1 s and |L|^2 over 2.25e6
        let mut t2 = t.clone();
        (t2.app_xz, t2.approach_on) = ((2.0, 9.0), true);
        let mut c = TrackCam { app_xz: 9.0, ..Default::default() };
        c.set(&t2, &car0, EV_WHEELIE, 0.0);
        let air = TrackCar { grounded: 0, ang_mom: Vec3::new(0.0, 0.0, 1600.0), speed: 0.0, ..car0 };
        // (three 1/30 s steps sum just over 0.1 in f32: off the ground on the third)
        for _ in 0..2 {
            c.update(&t2, &air, 1.0 / 30.0, 1.0 / 30.0, none, 16.0 / 9.0);
        }
        assert!(c.app_xz == 9.0 && !c.off_ground, "{}", c.app_xz);
        c.update(&t2, &air, 1.0 / 30.0, 1.0 / 30.0, none, 16.0 / 9.0);
        assert!(c.off_ground && (c.app_xz - (9.0 - 8.0 / 30.0)).abs() < 1e-4, "{}", c.app_xz);
        // spinning slower: no tumble, back up at AppInc
        c.update(&t2, &TrackCar { ang_mom: Vec3::new(0.0, 0.0, 1400.0), ..air }, 1.0 / 30.0, 1.0 / 30.0, none, 16.0 / 9.0);
        assert!((c.app_xz - (9.0 - 8.0 / 30.0 + 3.0 / 30.0)).abs() < 1e-4, "{}", c.app_xz);
        // a look yaw takes MaxAppXZPos even while tumbling
        c.look_yaw = 0.3;
        c.update(&t2, &air, 1.0 / 30.0, 1.0 / 30.0, none, 16.0 / 9.0);
        assert!((c.app_xz - (9.0 - 8.0 / 30.0 + 6.0 / 30.0)).abs() < 1e-4, "{}", c.app_xz);
    }

    /// The look-back warper (F-49780's defaults: no tune file on the disc): started every frame
    /// from 1 it is at 0.96 after the first; released every frame on the ground it eases back at
    /// sin(Fall dt pi/2) a frame; a release of an idle warper re-seeds from the scale given.
    #[test]
    fn lookback_warper() {
        let mut w = TimeWarper::new([0.0, 0.239, 2.009, 3.009]);
        let dt = 1.0 / 60.0;
        w.start(1.0, dt);
        assert!((w.cur - (1.0 - 0.761 * (2.009f32 * dt * std::f32::consts::FRAC_PI_2).sin())).abs() < 1e-5, "{}", w.cur);
        let mut s = w.cur;
        for _ in 0..60 {
            w.update(dt);
            s = w.cur;
            w.start(s, dt);
        }
        assert!((w.cur - 0.239).abs() < 1e-3, "{}", w.cur);
        let k = (3.009f32 * dt * std::f32::consts::FRAC_PI_2).sin();
        w.release(s);
        w.update(dt);
        assert!((w.cur - (s + (1.0 - s) * k)).abs() < 1e-5, "{} {}", w.cur, s);
        let mut idle = TimeWarper::new([0.0, 0.239, 2.009, 3.009]);
        idle.release(0.239);
        assert_eq!(idle.cur, 0.239);
    }

    /// The Burnout view on the R8's file: in on a handbrake spin above 1.5 rad/s with the
    /// charge full; first frame at TrackTo + Offset in the car's level heading; then the place
    /// stays (all App*Pos 0) and only the aim follows; lens 25 at a standstill; out when the
    /// spin falls under 0.75 rad/s.
    #[test]
    fn burnout_view() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let Some(t) = TrackTune::load(&data, "vp_aud_r8_08", "Burnout") else { return };
        assert_eq!((t.offset, t.track, t.blend, t.ret), (Vec3::new(2.5, 1.5, 13.0), Vec3::new(0.0, 1.0, 0.0), 0.7, 0.0));
        assert_eq!((t.fov, t.app_rot, t.app_x_rot), ((25.0, 60.0), 60.0, 10.0));
        let fx = CamFx::load(&data);
        assert_eq!((fx.burnout_turn, fx.idle_delay), (1.5, 60.0));
        assert!(!burnout_starts(&fx, 1.0, 1.0, 1.4) && !burnout_starts(&fx, 1.0, 0.9, 2.0) && burnout_starts(&fx, 1.0, 1.0, -2.0));
        assert!(!burnout_ends(&fx, 1.0, 0.6, 0.8) && burnout_ends(&fx, 1.0, 0.6, 0.7) && burnout_ends(&fx, 0.0, 1.0, 2.0));
        let mut d = EventDirector::default();
        let chase = BoomOut { pos: Vec3::new(0.0, 2.0, 9.0), look: Vec3::ZERO, up: Vec3::Y, fov: 25.0 };
        let mut car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO };
        let o = d.frame(&data, "vp_aud_r8_08", &ein(1.0, 1.0, 2.0, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        assert!(d.running());
        // (blending in: between the chase view and the event view)
        assert!(o.pos.distance(chase.pos) < 1.0, "{:?}", o.pos);
        // (the XZ rate starts at the file's base MaxAppXZPos 12 - F-67439 at the load - and falls
        // to Burnout's 0 at AppDec 8 a second: for the first 1.5 s the place still follows)
        assert!((d.cam.app_xz - (12.0 - 8.0 / 60.0)).abs() < 1e-4, "{}", d.cam.app_xz);
        let mut last = Vec3::ZERO;
        let mut held = None;
        for i in 0..150 {
            car.rot = Quat::from_rotation_y(i.min(59) as f32 * 2.0 / 60.0);
            let o = d.frame(&data, "vp_aud_r8_08", &ein(1.0, 1.0, 2.0, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
            if i >= 50 {
                assert!((o.fov - 25.0).abs() < 1e-3);
            }
            if i == 100 {
                // rate 0: the place stays; it has left the first frame's (2.5, 1 + 1.5, -1.3 + 13)
                assert!(d.cam.app_xz == 0.0 && o.pos.distance(Vec3::new(2.5, 2.5, 11.7)) > 0.5, "{:?}", o.pos);
                held = Some(o.pos);
            }
            if i > 100 {
                assert!(o.pos.distance(held.unwrap()) < 1e-4, "{:?}", o.pos);
                last = o.look;
            }
        }
        let p = held.unwrap();
        assert!((last - p).normalize().dot((Vec3::new(0.0, 1.0, 0.0) + Quat::from_rotation_y(59.0 * 2.0 / 60.0) * Vec3::new(0.0, 0.0, -1.3) - p).normalize()) > 0.99);
        d.frame(&data, "vp_aud_r8_08", &ein(1.0, 1.0, 0.7, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        assert!(!d.running());
    }

    /// The chooser's inputs of a car standing in a chase view: the handbrake, the burnout
    /// charge, the yaw rate and the idle time.
    fn ein(handbrake: f32, charge: f32, yaw_rate: f32, idle: f32) -> EventIn {
        EventIn { allowed: true, view: 3, front_down: true, rear_down: true, handbrake, hb_sim: handbrake, charge, yaw_rate, idle, ..Default::default() }
    }

    /// The chooser's order and gates (F-24075): a wheelie over 10 m/s with the front wheel
    /// up; a car on two wheels is TwoWheelSide in ACTION, TwoWheelUp in the other chase views;
    /// Air only with nothing in reach of the probes; in ACTION a nitrous shot ends any other
    /// event; nothing in the bumper or cockpit.
    #[test]
    fn chooser_events() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        if TrackTune::load(&data, "vp_aud_r8_08", "Wheelie").is_none() {
            return;
        }
        let chase = BoomOut { pos: Vec3::new(0.0, 2.0, 9.0), look: Vec3::ZERO, up: Vec3::Y, fov: 40.0 };
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::new(0.0, 0.0, -15.0), speed: 15.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO };
        let dt = 1.0 / 60.0;
        let run = |i: EventIn, ray: bool| {
            let mut d = EventDirector::default();
            d.frame(&data, "vp_aud_r8_08", &i, &car, BoomOut { ..chase }, dt, &NO_WORLD, &|_, _| ray, 16.0 / 9.0);
            d.event()
        };
        let base = EventIn { speed: 15.0, ..ein(0.0, 0.0, 0.0, 0.0) };
        assert_eq!(run(EventIn { stunt: 1, front_down: false, ..base }, false), Some(EV_WHEELIE));
        assert_eq!(run(EventIn { stunt: 1, front_down: false, speed: 9.0, ..base }, false), None);
        assert_eq!(run(EventIn { stunt: 1, front_down: false, view: 1, ..base }, false), None);
        assert_eq!(run(EventIn { g18: 1, view: 4, ..base }, false), Some(EV_TWO_SIDE));
        assert_eq!(run(EventIn { g18: -1, ..base }, false), Some(EV_TWO_UP));
        assert_eq!(run(EventIn { air_time: 0.2, ..base }, false), Some(EV_AIR));
        assert_eq!(run(EventIn { air_time: 0.2, ..base }, true), None);
        let mut d = EventDirector::default();
        let w = EventIn { stunt: 1, front_down: false, view: 4, ..base };
        d.frame(&data, "vp_aud_r8_08", &w, &car, BoomOut { ..chase }, dt, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        assert_eq!(d.event(), Some(EV_WHEELIE));
        d.frame(&data, "vp_aud_r8_08", &EventIn { nitro_on: true, ..w }, &car, BoomOut { ..chase }, dt, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        assert_eq!(d.event(), None);
    }

    /// The crash cameras (crash_camera_rule.md): F-37740's choice of start; the wipeout camera
    /// on the R8 snaps 6 m to the side the car slides toward at T = 0.75 x ModelOffset, lens 40,
    /// comes in over 0.25 s and keeps 6..10 m from T while the car slides away; the wipeout warper
    /// is at 0.1 after 0.2 s, held to 1.25 s, back to 1 by 1.35 s.
    #[test]
    fn crash_cameras() {
        let right = Vec3::X;
        let slide = Vec3::new(3.0, 0.0, -20.0);
        assert_eq!(CrashStart::choose(3, false, false, 1.0, right, slide), CrashStart { offset: Vec3::new(6.0, 0.0, 0.0), blend: 0.25, exploding: false });
        assert_eq!(CrashStart::choose(3, false, false, 1.0, right, -slide).offset, Vec3::new(-6.0, 0.0, 0.0));
        assert_eq!(CrashStart::choose(3, false, false, 0.05, right, slide), CrashStart { offset: Vec3::new(0.0, 1.0, 7.0), blend: 0.5, exploding: false });
        assert_eq!(CrashStart::choose(1, false, false, 1.0, right, slide).blend, 0.0);
        assert_eq!(CrashStart::choose(4, true, false, 1.0, right, slide).blend, 0.0);
        let mo = Vec3::new(0.0, 0.708, 0.0);
        let mut car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: slide, speed: slide.length(), reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 0.3, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO };
        let drive = BoomOut { pos: Vec3::new(0.0, 2.0, 6.0), look: Vec3::ZERO, up: Vec3::Y, fov: 60.0 };
        let mut c = CrashCam::default();
        c.start(mo, &car, CrashStart::choose(3, false, false, 1.0, right, slide));
        let dt = 1.0 / 60.0;
        let o = c.frame(&car, BoomOut { ..drive }, dt, dt, &NO_WORLD, 16.0 / 9.0);
        assert!(o.pos.distance(drive.pos) < 1.0, "blending in: {:?}", o.pos);
        let mut o = o;
        for _ in 0..15 {
            o = c.frame(&car, BoomOut { ..drive }, dt, dt, &NO_WORLD, 16.0 / 9.0);
        }
        let t = Vec3::new(0.0, 0.531, 0.0);
        assert!(o.pos.distance(t + Vec3::new(6.0, 0.0, 0.0)) < 1e-3 && o.fov == 40.0, "{:?}", o.pos);
        for _ in 0..120 {
            car.pos += car.vel * dt;
            o = c.frame(&car, BoomOut { ..drive }, dt, dt, &NO_WORLD, 16.0 / 9.0);
        }
        let d = o.pos.distance(car.pos + t);
        assert!((6.0 - 1e-3..=10.0 + 1e-3).contains(&d), "{d}");
        let mut w = TimeWarper::new([1.25, 0.1, 5.0, 10.0]);
        w.start(1.0, 0.0);
        let mut at = |w: &mut TimeWarper, secs: f32| {
            for _ in 0..(secs * 1000.0).round() as usize {
                w.update(0.001);
            }
            w.cur
        };
        assert!((at(&mut w, 0.1) - (1.0 - 0.9 * (0.5f32 * std::f32::consts::FRAC_PI_2).sin())).abs() < 0.01);
        assert_eq!(at(&mut w, 0.15), 0.1);
        assert_eq!(at(&mut w, 0.99), 0.1);
        assert!(at(&mut w, 0.02) > 0.1);
        assert_eq!(at(&mut w, 0.1), 1.0);
    }

    /// The look-around with the file's camFX: full stick left in a chase view goes to +1.25 rad
    /// (0.2 of the way a 30 Hz frame), right in the cockpit to -0.9, the bumper 45 degrees; let
    /// go it comes back; CAMERA_UP lifts a chase camera 0.75 m; inside the dead zone nothing.
    #[test]
    fn look_around() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let fx = CamFx::load(&data);
        if fx.idle_delay != 60.0 {
            return;
        }
        assert_eq!((fx.pan, fx.limit, fx.rise_up, fx.zoom_in, fx.look_lerp), ((1.25, 1.25), (0.75, 0.9, 0.1, 0.1), 0.75, 15.0, 0.2));
        let dt = 1.0 / 30.0;
        let mut l = LookAround::default();
        l.frame(&fx, 3, false, -1.0, 0.0, true, false, None, dt);
        assert!((l.yaw - 0.25).abs() < 1e-4, "{}", l.yaw);
        for _ in 0..60 {
            l.frame(&fx, 3, false, -1.0, 1.0, true, false, None, dt);
        }
        assert!((l.yaw - 1.25).abs() < 1e-3 && (l.rise - 0.75).abs() < 1e-3);
        for _ in 0..60 {
            l.frame(&fx, 3, false, 0.0, 0.0, true, false, None, dt);
        }
        assert!(l.yaw.abs() < 1e-3 && l.rise.abs() < 1e-3);
        let mut c = LookAround::default();
        for _ in 0..60 {
            c.frame(&fx, 1, false, 1.0, 0.0, true, false, None, dt);
        }
        assert!((c.yaw + 0.9).abs() < 1e-3);
        let mut b = LookAround::default();
        for _ in 0..60 {
            b.frame(&fx, 0, false, 0.3, 0.0, true, false, None, dt);
        }
        assert_eq!(b.yaw, 0.0);
    }

    /// The event camera's collision: a wall 5 m ahead of the car (its face towards the car)
    /// between the Idle view's track point and its place - the target turns away and the shown
    /// camera is pulled in to the near side of the wall, looking the same way.
    #[test]
    fn event_collision() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        if TrackTune::load(&data, "vp_aud_r8_08", "Idle").is_none() {
            return;
        }
        let wall = |c: Vec3, r: f32| (c.z - r < -5.0).then(|| (-5.0 - (c.z - r), Vec3::Z));
        let mut d = EventDirector::default();
        let chase = BoomOut { pos: Vec3::new(0.0, 2.0, 9.0), look: Vec3::ZERO, up: Vec3::Y, fov: 40.0 };
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO };
        let mut o = BoomOut { ..chase };
        let mut free = BoomOut { ..chase };
        let mut e = EventDirector::default();
        for i in 0..240 {
            o = d.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &Balls(wall), &|_, _| false, 16.0 / 9.0);
            free = e.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        }
        assert!(free.pos.z < -8.0, "{:?}", free.pos);
        assert!(o.pos.z > -5.0 && o.pos.distance(Vec3::new(0.0, 0.895, 0.55)) >= 2.0 - 1e-3, "{:?}", o.pos);
        let (dir, dir_free) = ((o.look - o.pos).normalize(), (free.look - free.pos).normalize());
        assert!(dir.dot(dir_free) < 0.9999, "the target turned away");
    }

    /// The boom collision: a wall 5 m behind the probe point stops the camera r off it; a clear
    /// line leaves it; a probe point inside the wall is pushed out.
    #[test]
    fn boom_collision() {
        let wall = |c: Vec3, r: f32| (c.z + r > 5.0).then(|| (c.z + r - 5.0, Vec3::NEG_Z));
        let wall = &Balls(wall);
        let m = boom_collide(Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.0, 2.0, 8.0), 0.27, false, wall).unwrap();
        let cam = Vec3::new(0.0, 2.0, 8.0) + m;
        assert!((cam.z - 4.73).abs() < 0.01 && cam.y < 2.0, "{cam:?}");
        assert!(boom_collide(Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.0, 2.0, 4.0), 0.27, false, wall).is_none());
        let m = boom_collide(Vec3::new(0.0, 1.2, 4.9), Vec3::new(0.0, 2.0, 8.0), 0.27, false, wall).unwrap();
        assert!(((Vec3::new(0.0, 2.0, 8.0) + m).z - 4.73).abs() < 0.01);
    }

    /// The same on the level's own probes (camprobe.rs): a wall of two triangles facing the
    /// car at z = 5, and another car's hull. The sweep stops the ball r off the wall exactly; a
    /// probe point in a corner takes both pushes summed (F-87913); a car behind is hit, the
    /// camera's own is not in the level.
    #[test]
    fn boom_collision_on_the_level() {
        use crate::physics::{camprobe::{CamLevel, CamObj}, collision::{SectorMesh, WorldCollider}};
        let (a, b, c, d) = (Vec3::new(-10.0, -5.0, 5.0), Vec3::new(10.0, -5.0, 5.0), Vec3::new(10.0, 10.0, 5.0), Vec3::new(-10.0, 10.0, 5.0));
        // (facing -z: (V1 - V0) x (V2 - V0))
        let mut city = WorldCollider::default();
        city.sectors.insert(0, SectorMesh::build(vec![[a, d, c], [a, c, b]]));
        let level = CamLevel::city(&city);
        let w: SphereTest = &level;
        let p = Vec3::new(0.0, 1.2, 0.0);
        let m = boom_collide(p, Vec3::new(0.0, 2.0, 8.0), 0.27, false, w).unwrap();
        let cam = Vec3::new(0.0, 2.0, 8.0) + m;
        assert!((cam.z - 4.73).abs() < 1e-5 && (cam.y - (1.2 + 0.8 * 4.73 / 8.0)).abs() < 1e-5, "{cam:?}");
        // the probe point 0.1 from the wall: one contact, the camera at p + n depth
        let p = Vec3::new(0.0, 1.2, 4.9);
        let cam = Vec3::new(0.0, 2.0, 8.0) + boom_collide(p, Vec3::new(0.0, 2.0, 8.0), 0.27, false, w).unwrap();
        assert!((cam - Vec3::new(0.0, 1.2, 4.73)).length() < 1e-5, "{cam:?}");
        // a second wall at x = 1 facing -x: two contacts, both pushes
        let mut city2 = WorldCollider::default();
        let (e, f, g, h) = (Vec3::new(1.0, -5.0, -10.0), Vec3::new(1.0, -5.0, 10.0), Vec3::new(1.0, 10.0, 10.0), Vec3::new(1.0, 10.0, -10.0));
        city2.sectors.insert(0, SectorMesh::build(vec![[a, d, c], [a, c, b], [e, f, g], [e, g, h]]));
        let level2 = CamLevel::city(&city2);
        let p = Vec3::new(0.9, 1.2, 4.9);
        let cam = Vec3::new(0.0, 2.0, 8.0) + boom_collide(p, Vec3::new(0.0, 2.0, 8.0), 0.27, false, &level2).unwrap();
        // (the z wall's contact first, then the x wall's part not along it - not a plain sum)
        assert!((cam - Vec3::new(0.782542, 1.153296, 4.709098)).length() < 1e-4, "{cam:?}");
        // a car 6 m behind in open ground: the cars' sweep meets its hull
        let hull = crate::physics::hull::HullShape::new(
            vec![
                Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 0.0, -2.0), Vec3::new(1.0, 1.4, -2.0), Vec3::new(-1.0, 1.4, -2.0),
                Vec3::new(-1.0, 0.0, 2.0), Vec3::new(1.0, 0.0, 2.0), Vec3::new(1.0, 1.4, 2.0), Vec3::new(-1.0, 1.4, 2.0),
            ],
            &[[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [3, 6, 2], [3, 7, 6], [0, 4, 7], [0, 7, 3], [1, 2, 6], [1, 6, 5]],
        );
        let open = WorldCollider::default();
        let mut level3 = CamLevel::city(&open);
        level3.cars.push(CamObj::of_hull(&hull, Vec3::new(0.0, 0.0, 9.0), Quat::IDENTITY));
        let p = Vec3::new(0.0, 1.0, 0.0);
        let cam = Vec3::new(0.0, 1.0, 10.0) + boom_collide(p, Vec3::new(0.0, 1.0, 10.0), 0.27, false, &level3).unwrap();
        assert!((cam.z - (7.0 - 0.27)).abs() < 1e-4, "{cam:?}");
    }

    /// The Idle view on the R8's file: nothing before camFX IdleDelayTime (60 s); then in over
    /// 1.5 s to TrackTo + Offset ((0, 0.895, 0.55) + (+-2.525, 0, -9): ahead of the car, to one
    /// side), lens 25; a standing car's camera does not move; out at a touch.
    #[test]
    fn idle_view() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let Some(t) = TrackTune::load(&data, "vp_aud_r8_08", "Idle") else { return };
        assert_eq!((t.offset, t.blend, t.dist, t.app_xz), (Vec3::new(-2.525, 0.0, -9.0), 1.5, (5.0, 10.0), (4.0, 9.0)));
        let mut d = EventDirector::default();
        let chase = BoomOut { pos: Vec3::new(0.0, 2.0, 9.0), look: Vec3::ZERO, up: Vec3::Y, fov: 40.0 };
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO };
        let o = d.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 59.9), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        assert!(!d.idle() && o.pos == chase.pos);
        let mut held = Vec::new();
        for i in 0..240 {
            let o = d.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
            assert!(d.idle());
            if i >= 100 {
                held.push(o);
            }
        }
        let p = held[0].pos;
        assert!((p.x.abs() - 2.525).abs() < 1e-3 && (p.y - 0.895).abs() < 1e-3 && (p.z - (0.55 - 9.0)).abs() < 1e-3, "{p:?}");
        assert!(held.iter().all(|o| o.pos.distance(p) < 1e-4 && (o.fov - 25.0).abs() < 1e-3));
        d.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        assert!(!d.running());
        // on a hill (the car's back axis 0.2 up the slope: facing downhill) the Idle camera's
        // place, ahead of the car, turns down with the ground by asin(0.2 * 0.8) about the
        // point; the view's AppYPos are 0, so only the level part is followed: it comes in to
        // |rel| cos(hill) and keeps its height
        let car = TrackCar { slope: Some(0.2), ..car };
        let mut d = EventDirector::default();
        let mut o = BoomOut { ..chase };
        for i in 0..600 {
            o = d.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        }
        let at = Vec3::new(0.0, 0.895, 0.55);
        let rel = Vec3::new(2.525, 0.0, 9.0).length();
        let level = Vec3::new(o.pos.x - at.x, 0.0, o.pos.z - at.z).length();
        assert!((level - rel * (0.2f32 * 0.8).asin().cos()).abs() < 0.01 && (o.pos.y - at.y).abs() < 0.2, "{:?} {}", o.pos, level);
    }

    /// The cockpit camera of a car accelerating to 55 m/s (GitHub #9): it stays at the dashboard -
    /// the boom shortened by |Offset - Center| x (1 - SSDistScale) = 0.19 m, SpeedZOffsetZ 0.24 m
    /// forward, GsZOffsetZ back under power - and stands the same at 30, 60 and 144 frames a second
    /// (the port shortened by the boom's whole 2.74 m and let the homing lag four times the
    /// original's: 1.1 m forward at 30 Hz, out through the windscreen).
    #[test]
    fn cockpit_under_power() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let Some(set) = BoomSet::load(&data, "vp_aud_r8_08") else { return };
        let t = &set.views[3];
        for hz in [30.0f32, 60.0, 144.0] {
            let dt = 1.0 / hz;
            let mut cam = BoomCam::default();
            let (mut z, mut v) = (0.0f32, 0.0f32);
            let mut seen = Vec::new();
            let mut at = Vec3::ZERO;
            for i in 0..(12.0 * hz) as usize {
                v = (v + 6.0 * dt).min(55.0);
                z -= v * dt;
                let car = BoomTarget { pos: Vec3::new(0.0, 0.0, z), rot: Quat::IDENTITY, vel: Vec3::new(0.0, 0.0, -v), ang_vel: Vec3::ZERO, grounded: 4, gear: 3, rev: false, nitro: false, nitro_fov: (0.0, 0.5, 1.0), ability: (0.0, 1.0), sst: (0.0, 1.0), pan: (0.0, 0.0), rise: 0.0, fixed_mount: true, model_offset: Vec3::ZERO };
                let o = cam.update(t, &car, dt);
                at = o.pos - car.pos;
                if i % (hz as usize * 2) == 0 {
                    seen.push(format!("v {:.0}: {:.3?}", v, at));
                }
            }
            eprintln!("{hz} Hz: {}", seen.join(" | "));
            assert!((at - Vec3::new(-0.35, 1.038, -0.083)).length() < 0.005, "{hz} Hz: {at:?}");
        }
    }

    /// Photo mode's free camera reads PHOTO_CAM_* (DECISIONS.md D35): every one of the disc's
    /// control styles binds them where the prompt bar shows them - ROTATE the right stick, ZOOM
    /// the triggers, PAN the left stick.
    #[test]
    fn photo_pad_bindings() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let maps = crate::mc4::control_maps(&data);
        if maps.is_empty() {
            return;
        }
        let s = crate::mc4::control_slots(&maps, None, false);
        let get = |a: &str| s.get(a).copied().unwrap_or((0, 0));
        // (axis keys 22..25 are the left stick's half axes, 26..29 the right stick's; button keys 0 / 1
        // the left / right trigger - `mc4::PadState::byte`)
        for (i, a) in ["MOVE_UP", "MOVE_DN", "MOVE_LT", "MOVE_RT"].iter().enumerate() {
            assert_eq!(get(&format!("PHOTO_CAM_{a}")), (9, 26 + i as u32), "{a}");
        }
        for (i, a) in ["PAN_UP", "PAN_DN", "PAN_LT", "PAN_RT"].iter().enumerate() {
            assert_eq!(get(&format!("PHOTO_CAM_{a}")), (9, 22 + i as u32), "{a}");
        }
        assert_eq!((get("PHOTO_CAM_ZOOM_IN"), get("PHOTO_CAM_ZOOM_OUT")), ((8, 1), (8, 0)));
    }
}

