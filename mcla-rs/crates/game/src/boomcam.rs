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
        let get = |suffix: &str| {
            let h = mcla_formats::hash::joaat(&format!("{car}_{suffix}.camboomcs"));
            let p = Path::new(data_dir).join("tune/camera").join(format!("0x{h:08x}"));
            std::fs::read_to_string(p).ok().and_then(|t| BoomTune::parse(&t))
        };
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
    /// look along the Rev* boom (the car is backing up, or the look-behind camera)
    pub rev: bool,
    pub nitro: bool,
    /// the car's nitrous lens: degrees added, seconds to open, seconds to close (mcCarNitroTune)
    pub nitro_fov: (f32, f32, f32),
    /// a running special ability's change to the lens (degrees) and to the distance (scale)
    pub ability: (f32, f32),
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

    /// Where the camera's collision probe starts (sub_8231E700): the file's Collide point in
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

/// The boom camera's collision (camBoomCS CollideType 2, sub_8231FD60 -> sub_8231E700 for the
/// chase and reverse views, sub_8231EC50 for POV / bumper; rule in
/// `docs/reference/cameras/boom_collision_rule.md`). A ball the size of the near plane's corner
/// (near * sqrt(tanX^2 + tanY^2), + 0.01; POV / bumper twice the corner) goes from the probe
/// point `p` to the camera `c`; on a touch the camera stands where the ball stopped, held its
/// radius off the surface, and the focus moves with it (the look keeps its direction). If the
/// probe point is already inside something the camera goes to the ball's place pushed out of
/// it. Every frame, no smoothing, no minimum distance; the boom's own state is not changed.
/// `test(centre, r)` is the world's sphere test (deepest contact: depth, normal). Returns the
/// camera's move, or None when it is clear. (approx.: the original keeps up to ten contacts at
/// P and sums their pushes - here the deepest one, repeated; it also hits other cars - here the
/// city only.)
pub fn boom_collide(p: Vec3, c: Vec3, r: f32, inside_car: bool, test: impl Fn(Vec3, f32) -> Option<(f32, Vec3)>) -> Option<Vec3> {
    if !inside_car {
        if test(p, r).is_some() {
            let mut q = p;
            for _ in 0..4 {
                match test(q, r) {
                    Some((pen, n)) => q += n * (pen + 1e-3),
                    None => break,
                }
            }
            return Some(q - c);
        }
    }
    let d = c - p;
    let len = d.length();
    if len < 1e-4 {
        return None;
    }
    let n = ((len / (r * 0.5).max(0.02)).ceil() as usize).clamp(1, 256);
    let mut free = 0.0f32;
    for i in 1..=n {
        let s = i as f32 / n as f32;
        if test(p + d * s, r).is_some() {
            // the touch, between the last free step and this one
            let (mut lo, mut hi) = (free, s);
            for _ in 0..10 {
                let m = (lo + hi) * 0.5;
                if test(p + d * m, r).is_some() {
                    hi = m;
                } else {
                    lo = m;
                }
            }
            return Some(p + d * lo - c);
        }
        free = s;
    }
    None
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
            // 0x82320420)
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
        let fov = t.s("SSFovMin") + (t.s("SSFovMax") - t.s("SSFovMin")) * k + car.nitro_fov.0 * self.nitro + car.ability.0;
        let dist_scale = (1.0 + (t.s("SSDistScale") - 1.0) * k) * car.ability.1;

        // ---- where the camera and its focus want to be ----
        let (off0, foc0) = if car.rev { (t.v3("RevOffset"), t.v3("RevFocus")) } else { (t.v3("Offset"), t.v3("Focus")) };
        let (off0, foc0) = (off0 - t.v3("Center"), foc0 - t.v3("Center"));
        let boom = off0 - foc0;
        let mut off = off0 - boom.normalize_or_zero() * boom.length() * (1.0 - dist_scale);
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
        // fore-aft terms by 1 - min(yaw^2, 1), the pitch the height terms - 0x82320D8C..0x82320E10)
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
        // and turns its boom: pitch about x, then yaw about y (0x82320ED0..0x82320F80), about the
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
            let homing = t.v3("HomingTime");
            if homing != Vec3::ZERO {
                let len = off.length();
                let last = to_frame * (self.world - origin);
                let h = Vec3::new(per_frame(homing.x), per_frame(homing.y), per_frame(homing.z));
                let homed = last + (off - last) * h;
                off = homed.normalize_or(off.normalize_or(Vec3::Z)) * len;
            }
            // and nothing moves faster than the approach rate (+656: 25 m/s while the camera is
            // turned, else the file's ApproachRate, moving there at 50 per second; none when the
            // file has none - 0x823210A0)
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

// ---- the player's choice of view (mcPlayerCamera, sub_822B1648 / sub_822B35E8) ----
//
// The original keeps its views in a list: 0 Bumper (+44), 1 POV (+48), 2 Boom 1 (+52), 3 Boom 2,
// 4 Boom 3 (+60); index at +80. The port's `cam_mode` keeps its own order (0 boom1, 1 boom2,
// 2 boom3, 3 POV, 4 bumper; 5 = the event camera, only by the test hook).

/// The port's view for each of the original's list places.
pub const PORT_OF_VIEW: [u8; 5] = [4, 3, 0, 1, 2];
/// The original's list place of each port view (5, the event camera, counts as Boom 3).
pub const VIEW_OF_PORT: [usize; 6] = [2, 3, 4, 1, 0, 4];
/// The names the HUD shows (`_global.cameras`, sub_8223D300): string table keys of the list
/// places 0..4, then the overhead view of the race editor (`Cheat_RE_OverHeadCam`).
pub const VIEW_KEYS: [&str; 6] = ["Bumper", "POV", "Boom 1", "Boom 2", "Boom 3", "Cheat_RE_OverHeadCam"];

/// The camera buttons outside the race editor (sub_822B35E8, 0x822B38E0..0x822B3A84):
/// CAM_UP takes the list's previous place, CAM_DOWN the next; both wrap within the five.
/// The event camera is never a place of the list.
pub fn next_view(port: u8, cam_up: bool) -> u8 {
    let v = VIEW_OF_PORT[(port as usize).min(5)];
    PORT_OF_VIEW[if cam_up { (v + 4) % 5 } else { (v + 1) % 5 }]
}

/// A change between two views is a cut (transition type 4) when either is Bumper or POV,
/// else a blend (type 3) over the new view's own BlendTime (sub_822B35E8 -> sub_822B0F10).
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

/// The blend from one view to the next (the camera manager's MCCAMERATRANSITION, sub_8271F438):
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

/// The black mask of a cut (TRANSITIONMOVIE as sub_82207EF0 starts it: mask_color 0,
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
        // (only when no transition is running: sub_82207EF0 reads StartOfTransitionin and
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
/// round (sub_822B23D0: vt+80(1) on the camera in use); else backing up takes the Rev* boom.
pub fn tune_for(set: &BoomSet, view: u8, back: bool, backing: bool) -> (&BoomTune, bool) {
    let i = (view as usize).min(set.views.len() - 1);
    let chase = VIEW_OF_PORT[(view as usize).min(5)] >= 2;
    match (back, chase) {
        (true, true) => (set.reverse.as_ref().unwrap_or(&set.views[0]), true),
        (true, false) => (&set.views[i], true),
        _ => (&set.views[i], backing),
    }
}

/// What the player's camera does when the view or the look back changes (mcPlayerCamera:
/// sub_822B35E8 the buttons, sub_822B23D0 the look back, sub_822B4C98 its turning).
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
    pub fn frame(&mut self, set: &BoomSet, view: u8, back: bool, backing: bool, cinematic: bool, moving: bool, cam: &mut BoomCam, dt: f32) {
        let Some((was_view, was_back)) = self.last.replace((view, back)) else { return };
        let chase = VIEW_OF_PORT[(view as usize).min(5)] >= 2;
        if back != was_back {
            // (both ways a cut with the mask's quick fade: 0.01, 0.1, 0.01)
            self.blend.stop();
            self.mask.start(0.01, 0.1, 0.01);
            cam.reset();
            self.pan = (0.0, 0.0);
            if back && chase && cinematic && moving {
                // the look back of the cinematic camera stands off to one side: 22.5 .. 67.5
                // degrees either way (0x822B24F4..0x822B25D4)
                let sign = if self.rand23() * 2.384_185_8e-7 - 1.0 >= 0.0 { 1.0 } else { -1.0 };
                self.pan.0 = sign * (0.392_699 + self.rand23() * 9.362_68e-8);
            }
        } else if view != was_view && !back {
            if is_cut(was_view, view) {
                self.blend.stop();
                self.mask.start(0.01, 0.25, 0.01);
            } else {
                let (old, _) = tune_for(set, was_view, false, backing);
                let (new, _) = tune_for(set, view, false, backing);
                self.blend.start(old.clone(), cam.clone(), new.blend_time(), new.blend_goal());
            }
            cam.reset();
        }
        // turning the look-back camera with CAM_PAN_*: 2 rad/s round, 1.25 rad/s up, the
        // pitch kept in 0 .. 1.2566 (0x822B5048..0x822B5140)
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
/// sub_82314838 into the player camera's +24 object): the times and thresholds the event
/// camera's chooser reads.
#[derive(Clone, Debug)]
pub struct CamFx {
    /// seconds without input before the Idle view (+36)
    pub idle_delay: f32,
    /// yaw rate (rad/s) that makes a burnout the Burnout view (+44)
    pub burnout_turn: f32,
    /// the look-around (sub_822B2AB0): the chase views' yaw at full stick left / right (+60,
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
        // (the constructor's defaults, sub_82314C30)
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

/// The camera shake (camFX blocks, sub_82315000 applied to the camera in use each frame).
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

    /// Every block off (sub_82314790: the phases are kept).
    pub fn reset(&mut self) {
        for b in &mut self.blocks {
            b.timer = 0.0;
            b.state = 0;
        }
    }

    /// A one-shot on the Shake block (sub_822AFA88 -> sub_823147F0): `fixed_mount` (bumper /
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

    /// A continuous block's strength (sub_823147C0).
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

    /// Shakes the camera (eye `pos`, `look`, `up`) about the car at `car` (sub_82315000).
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
            // the oscillator (sub_82314E58): phase += freq dt; (amp sin phase + uniform(-r, r)) m
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
            // Rot and Tran in the camera's own frame (EulerXYZ, sub_821B4B00)
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

/// The original's dead zone with rescale (sub_8260C5C8).
fn dead_zone(v: f32, d: f32) -> f32 {
    if v > d {
        (v - d) / (1.0 - d)
    } else if v < -d {
        (v + d) / (1.0 - d)
    } else {
        0.0
    }
}

/// The driving camera's look-around (mcPlayerCamera sub_822B2AB0, once a game frame; rule in
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
        // pitches it at 0.75; else the held angles run back to 0 at those rates (0x822B2EE8)
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

/// The event camera's events (camTrackCS records at +800 + 112 e, sub_82315E08).
pub const EVENT_NAMES: [&str; 18] = [
    "SSTurbo", "Nitrous", "Tuck", "Wheelie", "Nosie", "TwoWheelSide", "TwoWheelUp", "Burnout", "Air", "Turn", "Reverse", "Hydraulics", "AirRide", "Idle", "Finish", "Results", "Over", "Damageout",
];
pub const EV_BURNOUT: usize = 7;
pub const EV_IDLE: usize = 13;

/// The Burnout view begins (sub_822B3BF0, 0x822B4038..0x822B4080): the handbrake (input +20)
/// is held, the burnout charge (chassis +580) is full and the car turns faster than
/// BurnoutTurnThreshold (|angular velocity y|, phys +292) - donuts and 360s on the handbrake.
pub fn burnout_starts(fx: &CamFx, handbrake: f32, charge: f32, yaw_rate: f32) -> bool {
    handbrake > 0.0 && yaw_rate.abs() > fx.burnout_turn && charge >= 1.0
}

/// It ends (0x822B47F8..0x822B4840) when the handbrake is let go, the charge falls under one
/// half or the turn under half the threshold.
pub fn burnout_ends(fx: &CamFx, handbrake: f32, charge: f32, yaw_rate: f32) -> bool {
    handbrake == 0.0 || charge < 0.5 || yaw_rate.abs() < fx.burnout_turn * 0.5
}

// ---- the event camera's track view (camTrackCS, sub_8231CF48; rule in
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
/// approach and lens fields, the event's own record (sub_823154F0 -> sub_82316148).
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
    /// the place's approach rates (sub_8231B368): the event's Min/MaxAppXZPos, Min/MaxAppYPos,
    /// RevAppXZPos over the speed range MinSpeed..MaxSpeed (10..25 m/s but Hydraulics' 2..10);
    /// the base block's AppInc / AppDec (how fast the XZ rate moves) and AppPosMin
    pub app_xz: (f32, f32),
    pub app_y: (f32, f32),
    pub rev_app_xz: f32,
    pub app_speed: (f32, f32),
    pub app_inc: f32,
    pub app_dec: f32,
    pub app_pos_min: f32,
    /// the hill angle (sub_8231A468): HillMin, HillMax, HillLerp of the base block; the
    /// event's ApproachOn (off: no hill, the car's own frame)
    pub hill: (f32, f32, f32),
    pub approach_on: bool,
    /// the collision (sub_8231B740): CollideType (1 / 3 in the files), CollideMinDist, the
    /// camera's near plane (CameraNear, base +172)
    pub collide_type: i32,
    pub collide_min: f32,
    pub near: f32,
}

impl TrackTune {
    pub fn load(data_dir: &str, car: &str, event: &str) -> Option<TrackTune> {
        let h = mcla_formats::hash::joaat(&format!("{car}_cinematic.camtrackcs"));
        let text = std::fs::read_to_string(Path::new(data_dir).join("tune/camera").join(format!("0x{h:08x}"))).ok()?;
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
            hill: (f(&b, "HillMin", 0.0), f(&b, "HillMax", 0.0), f(&b, "HillLerp", 0.0)),
            approach_on: f(&e, "ApproachOn", 1.0) != 0.0,
            collide_type: f(&b, "CollideType", 1.0) as i32,
            collide_min: f(&b, "CollideMinDist", 2.0),
            near: { let n = f(&b, "CameraNear", 0.2); if n > 0.0 { n } else { 0.2 } },
        })
    }
}

impl TrackTune {
    /// The wipeout camera (mcPlayerCamera +72, vehTrackCS): its `<class>_wipeout.camtrackcs` is not
    /// on the disc, so these are the constructors' values and sub_822B1648's (crash_camera_rule.md
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
            hill: (-0.8, 1.25, 0.1),
            approach_on: true,
            collide_type: 1,
            collide_min: 0.5,
            near: 0.2,
        }
    }
}

/// The wipeout camera (mcPlayerCamera +72, vehTrackCS; crash_camera_rule.md 1 and 2): taken by
/// sub_822AFF10 while the car is wiped out, damaged out or held by the police, and left with a
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

/// Where sub_822AFF10 puts the wipeout camera and how it comes in.
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
    /// sub_822AFF10's choice: BUMPER / POV or an exploding car cut (with the TRANSITIONMOVIE mask
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
    /// The drive view comes back (a cut, sub_8271E930), or the Damageout view has taken over.
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
    /// sub_822EB400: `now` is the game's time scale of the moment; one update follows at once.
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
    /// sub_822EB078: back to 1 from where it is.
    pub fn release(&mut self, now: f32) {
        if self.state == 2 {
            return;
        }
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
    /// sub_822EB3A8, once a frame on the unscaled frame time.
    pub fn update(&mut self, dt: f32) {
        if self.state <= 1 {
            self.t += self.rate * dt;
            self.cur = if self.t >= 1.0 {
                self.to
            } else if self.t <= 0.0 {
                self.from
            } else {
                self.from + (self.to - self.from) * (self.t * std::f32::consts::FRAC_PI_2).sin()
            };
        }
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

/// camTrackCS' approach step (sub_823223E8): `cur` goes towards `tgt` by s * rate * dt, s the
/// distance (quadratic within `min`) through the AppApp lag `st`; no overshoot. The lag is
/// per call of the original's camera update (once a 30 Hz game frame).
fn approach(cur: f32, tgt: f32, min: f32, rate_dt: f32, app: f32, st: &mut f32, frames: f32) -> f32 {
    let d = (tgt - cur).abs();
    let mut s = if min != 0.0 && d <= min { d * d / min } else { d };
    let lag = 1.0 - (1.0 - app.clamp(0.0, 1.0)).powf(frames);
    *st += (s - *st) * lag;
    if (*st < 0.0) != (s < 0.0) && *st != 0.0 && s != 0.0 {
        *st = 0.0;
        s = 0.0;
    } else {
        s = *st;
    }
    let step = (s * rate_dt).min(d);
    cur + step * (tgt - cur).signum()
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

/// The track camera with one event view on (sub_82316148 then sub_8231CF48 each frame).
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
}

impl TrackCam {
    /// The event is set (sub_82316148): Burnout's TrackTo.z = the front axle; Offset and
    /// TrackTo mirrored when the event's side (`EventDirector::side`) is above zero; the next
    /// frame snaps (reset sub_8231AC50: the filtered speed starts at the car's).
    fn set(&mut self, t: &TrackTune, car: &TrackCar, ev: usize, side: f32) {
        let mut track = if ev == EV_BURNOUT { Vec3::new(t.track.x, t.track.y, car.front_z) } else { t.track };
        let mut offset = t.offset;
        if side > 0.0 {
            track.x = -track.x;
            offset.x = -offset.x;
        }
        let (app_xz, hill) = (self.app_xz, self.hill);
        *self = TrackCam { snap: true, track, offset, speed: if car.reverse_gear { 0.0 } else { car.speed }, app_xz, hill, ..Default::default() };
    }

    /// Where the camera wants to stand (sub_8231C4F8): the Offset in the car's level heading
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
        let fov = t.fov.0 + (t.fov.1 - t.fov.0) * k;
        let dsc = 1.0 + (t.dist_scale - 1.0) * k;
        // the point it tracks: TrackTo through the whole car matrix
        let m = Mat3::from_quat(car.rot);
        let at = car.pos + m * self.track;
        // the hill angle (sub_8231A468): the slope under the wheels (its sign turned for a
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
        // axis (sub_8231C4F8: rows * R(A, -pitch))
        let want = self.place(car, at);
        let rel = want - at;
        let a = Vec3::Y.cross(rel.normalize_or(Vec3::Z)).normalize_or(Vec3::X);
        let want = if self.hill != 0.0 { at + Quat::from_axis_angle(a, -self.hill) * rel } else { want };
        let want = deflect(at, want, t, test);
        // the place's approach rates (sub_8231B368): over the speed range, the XZ one moving
        // towards its goal at AppInc / AppDec a second, in reverse gear RevAppXZPos
        let s = car.speed;
        let u = if t.app_speed.1 != t.app_speed.0 && s > t.app_speed.0 { ((s - t.app_speed.0) / (t.app_speed.1 - t.app_speed.0)).min(1.0) } else { 0.0 };
        let mut goal = if t.app_xz.0 > 0.0 { t.app_xz.1 + (t.app_xz.0 - t.app_xz.1) * u } else { t.app_xz.1 };
        if car.aero < 1.0 {
            goal *= car.aero;
        }
        if car.reverse_gear && !self.rev {
            goal = t.rev_app_xz;
        }
        let step = if goal < self.app_xz { t.app_dec } else { t.app_inc } * dt_game;
        self.app_xz += (goal - self.app_xz).clamp(-step, step);
        let app_y = t.app_y.0 + (t.app_y.1 - t.app_y.0) * u;
        if self.snap {
            self.pos = want;
            let (y, p) = yaw_pitch(at - self.pos);
            self.yaw = y;
            self.pitch = p;
            self.snap = false;
        } else {
            // the place follows (sub_823228F0; Burnout's App*Pos are 0: it stays), then the
            // distance is held in [DistMin, DistMax] x the lens' scale (sub_82321EE8)
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
        // the shown camera pulled in from walls (sub_8231B740); the look keeps its direction
        let pos = self.pull_in(t, at, dt, test, fov, aspect);
        BoomOut { pos, look: look + (pos - self.pos), up: Vec3::Y, fov }
    }

    /// CollideType 1 / 3 (sub_8231B740, TYPE13): a ball of the near plane's corner + 0.1 from
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
        if let Some((_, hn)) = test(at, r).filter(|(_, hn)| hn.dot(u) < -1e-5) {
            f = Some(0.0);
            n = hn;
        } else if let Some((ft, hn)) = sweep(test, at, c, r).filter(|(_, hn)| hn.dot(u) < -1e-5) {
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
                // (sub_821CE490: linear at MinAppXZPos m/s, shown until it is there)
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

/// The world's sphere test the cameras use: a ball's deepest contact (depth, normal).
pub type SphereTest<'a> = &'a dyn Fn(Vec3, f32) -> Option<(f32, Vec3)>;

/// A ball swept from  to  (sub_82574C38, directed): the fraction of the way at the first
/// touch and the normal there; a surface facing along the sweep does not count. (approx.:
/// steps of half the radius and a bisection, on the port's sphere test)
pub fn sweep(test: SphereTest, a: Vec3, b: Vec3, r: f32) -> Option<(f32, Vec3)> {
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

/// The event camera's target turned away from what blocks it (sub_8231C4F8, 0x8231CB0C): with
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
    let (yaw, pitch) = if let Some((_, n)) = test(at, h) {
        (2.0 * n.dot(a).atan(), 0.5 * n.y.atan())
    } else if let Some((f, n)) = sweep(test, at, want, h) {
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

/// The camera director's orbit blend (sub_8271F438): the focus point (each camera's line of
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

/// What the event camera's chooser (sub_822B3BF0) reads of the car, the pad and the camera
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
    /// mcCarSSTurbo +40 (running) and its lens ramp complete (sub_82345DD0)
    pub sst_on: bool,
    pub sst_full: bool,
    /// mcCarNitro +32 (a shot burning) and its lens ramp complete (sub_82345638)
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

/// A segment probe of the world (sub_82574418, one result): something lies between the points.
pub type RayTest<'a> = &'a dyn Fn(Vec3, Vec3) -> bool;

/// The event camera (camTrackCS at mgr+84) as mcPlayerCamera's chooser runs it: each frame the
/// first event whose start holds is set and blended in over the record's BlendTime with the
/// director's cosine orbit blend (sub_822AFCD0); its end test sends it back to the drive view
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
    /// sub_822AFCD0(0, -1, -1): the running event goes back over the record's time.
    pub fn end(&mut self, data_dir: &str, car_name: &str) {
        if self.running() {
            if let Some(t) = self.tune(data_dir, car_name, self.ev) {
                self.off(&t, -1.0);
            }
        }
    }
    /// CAM_UP leaves it (sub_822B35E8: sub_822AFCD0(0, -1, 0.2)).
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

    /// The chooser's start section (sub_822B3BF0, part 2): the first event whose test holds.
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
            if test(car.pos, 2.0).is_some() {
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

    /// The side an event is set to (sub_82316148): its Offset and TrackTo are mirrored when it
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

    /// Turns the event off (sub_822AFCD0(0, -1, b)): back to the drive view over b, else the
    /// record's ReturnTime, else its BlendTime.
    fn off(&mut self, t: &TrackTune, b: f32) {
        let back = if b >= 0.0 { b } else if t.ret > 0.0 { t.ret } else { t.blend };
        // (leaving while still blending in turns the blend round: w stays where it is)
        let w = if self.phase == 1 { 1.0 - self.left / self.time.max(1e-4) } else { 1.0 };
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
        let h = mcla_formats::hash::joaat(&format!("{car}_cinematic.camtrackcs"));
        let text = std::fs::read_to_string(Path::new(data_dir).join("tune/camera").join(format!("0x{h:08x}"))).ok()?;
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
        let mut car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0 };
        let o = d.frame(&data, "vp_aud_r8_08", &ein(1.0, 1.0, 2.0, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &|_, _| None, &|_, _| false, 16.0 / 9.0);
        assert!(d.running());
        // (blending in: between the chase view and the event view)
        assert!(o.pos.distance(chase.pos) < 1.0, "{:?}", o.pos);
        let mut last = Vec3::ZERO;
        for i in 0..60 {
            car.rot = Quat::from_rotation_y(i as f32 * 2.0 / 60.0);
            let o = d.frame(&data, "vp_aud_r8_08", &ein(1.0, 1.0, 2.0, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &|_, _| None, &|_, _| false, 16.0 / 9.0);
            if i >= 50 {
                // fully on: the place it took on the first frame, (2.5, 1 + 1.5, -1.3 + 13)
                assert!(o.pos.distance(Vec3::new(2.5, 2.5, 11.7)) < 1e-3, "{:?}", o.pos);
                assert!((o.fov - 25.0).abs() < 1e-3);
                last = o.look;
            }
        }
        assert!((last - Vec3::new(2.5, 2.5, 11.7)).normalize().dot((Vec3::new(0.0, 1.0, 0.0) + Quat::from_rotation_y(2.0) * Vec3::new(0.0, 0.0, -1.3) - Vec3::new(2.5, 2.5, 11.7)).normalize()) > 0.99);
        d.frame(&data, "vp_aud_r8_08", &ein(1.0, 1.0, 0.7, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &|_, _| None, &|_, _| false, 16.0 / 9.0);
        assert!(!d.running());
    }

    /// The chooser's inputs of a car standing in a chase view: the handbrake, the burnout
    /// charge, the yaw rate and the idle time.
    fn ein(handbrake: f32, charge: f32, yaw_rate: f32, idle: f32) -> EventIn {
        EventIn { allowed: true, view: 3, front_down: true, rear_down: true, handbrake, hb_sim: handbrake, charge, yaw_rate, idle, ..Default::default() }
    }

    /// The chooser's order and gates (sub_822B3BF0): a wheelie over 10 m/s with the front wheel
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
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::new(0.0, 0.0, -15.0), speed: 15.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0 };
        let dt = 1.0 / 60.0;
        let run = |i: EventIn, ray: bool| {
            let mut d = EventDirector::default();
            d.frame(&data, "vp_aud_r8_08", &i, &car, BoomOut { ..chase }, dt, &|_, _| None, &|_, _| ray, 16.0 / 9.0);
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
        d.frame(&data, "vp_aud_r8_08", &w, &car, BoomOut { ..chase }, dt, &|_, _| None, &|_, _| false, 16.0 / 9.0);
        assert_eq!(d.event(), Some(EV_WHEELIE));
        d.frame(&data, "vp_aud_r8_08", &EventIn { nitro_on: true, ..w }, &car, BoomOut { ..chase }, dt, &|_, _| None, &|_, _| false, 16.0 / 9.0);
        assert_eq!(d.event(), None);
    }

    /// The crash cameras (crash_camera_rule.md): sub_822AFF10's choice of start; the wipeout camera
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
        let mut car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: slide, speed: slide.length(), reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 0.3 };
        let drive = BoomOut { pos: Vec3::new(0.0, 2.0, 6.0), look: Vec3::ZERO, up: Vec3::Y, fov: 60.0 };
        let mut c = CrashCam::default();
        c.start(mo, &car, CrashStart::choose(3, false, false, 1.0, right, slide));
        let dt = 1.0 / 60.0;
        let o = c.frame(&car, BoomOut { ..drive }, dt, dt, &|_, _| None, 16.0 / 9.0);
        assert!(o.pos.distance(drive.pos) < 1.0, "blending in: {:?}", o.pos);
        let mut o = o;
        for _ in 0..15 {
            o = c.frame(&car, BoomOut { ..drive }, dt, dt, &|_, _| None, 16.0 / 9.0);
        }
        let t = Vec3::new(0.0, 0.531, 0.0);
        assert!(o.pos.distance(t + Vec3::new(6.0, 0.0, 0.0)) < 1e-3 && o.fov == 40.0, "{:?}", o.pos);
        for _ in 0..120 {
            car.pos += car.vel * dt;
            o = c.frame(&car, BoomOut { ..drive }, dt, dt, &|_, _| None, 16.0 / 9.0);
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
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0 };
        let mut o = BoomOut { ..chase };
        let mut free = BoomOut { ..chase };
        let mut e = EventDirector::default();
        for i in 0..240 {
            o = d.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &wall, &|_, _| false, 16.0 / 9.0);
            free = e.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &|_, _| None, &|_, _| false, 16.0 / 9.0);
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
        let m = boom_collide(Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.0, 2.0, 8.0), 0.27, false, wall).unwrap();
        let cam = Vec3::new(0.0, 2.0, 8.0) + m;
        assert!((cam.z - 4.73).abs() < 0.01 && cam.y < 2.0, "{cam:?}");
        assert!(boom_collide(Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.0, 2.0, 4.0), 0.27, false, wall).is_none());
        let m = boom_collide(Vec3::new(0.0, 1.2, 4.9), Vec3::new(0.0, 2.0, 8.0), 0.27, false, wall).unwrap();
        assert!(((Vec3::new(0.0, 2.0, 8.0) + m).z - 4.73).abs() < 0.01);
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
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0 };
        let o = d.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 59.9), &car, BoomOut { ..chase }, 1.0 / 60.0, &|_, _| None, &|_, _| false, 16.0 / 9.0);
        assert!(!d.idle() && o.pos == chase.pos);
        let mut held = Vec::new();
        for i in 0..240 {
            let o = d.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &|_, _| None, &|_, _| false, 16.0 / 9.0);
            assert!(d.idle());
            if i >= 100 {
                held.push(o);
            }
        }
        let p = held[0].pos;
        assert!((p.x.abs() - 2.525).abs() < 1e-3 && (p.y - 0.895).abs() < 1e-3 && (p.z - (0.55 - 9.0)).abs() < 1e-3, "{p:?}");
        assert!(held.iter().all(|o| o.pos.distance(p) < 1e-4 && (o.fov - 25.0).abs() < 1e-3));
        d.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &|_, _| None, &|_, _| false, 16.0 / 9.0);
        assert!(!d.running());
        // on a hill (the car's back axis 0.2 up the slope: facing downhill) the Idle camera's
        // place, ahead of the car, turns down with the ground by asin(0.2 * 0.8) about the
        // point; the view's AppYPos are 0, so only the level part is followed: it comes in to
        // |rel| cos(hill) and keeps its height
        let car = TrackCar { slope: Some(0.2), ..car };
        let mut d = EventDirector::default();
        let mut o = BoomOut { ..chase };
        for i in 0..600 {
            o = d.frame(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &|_, _| None, &|_, _| false, 16.0 / 9.0);
        }
        let at = Vec3::new(0.0, 0.895, 0.55);
        let rel = Vec3::new(2.525, 0.0, 9.0).length();
        let level = Vec3::new(o.pos.x - at.x, 0.0, o.pos.z - at.z).length();
        assert!((level - rel * (0.2f32 * 0.8).asin().cos()).abs() < 0.01 && (o.pos.y - at.y).abs() < 0.2, "{:?} {}", o.pos, level);
    }
}
