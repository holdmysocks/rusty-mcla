//! The game's driving cameras (`camBoomCS`, tune/camera/<car>_boom1..3, _POV, _bumper,
//! _reverse): a boom from a focus point on the car to the camera, in a frame that follows
//! the car's heading and the ground with a lag. The boom is bent by how the car moves:
//! its speed, turn rate and accelerations, each normalised by a range from the file, move
//! the camera and the point it looks at, and a gear change kicks it. The lens widens and
//! the boom shortens with speed.
use glam::{Mat3, Quat, Vec3};
use std::collections::HashMap;
use std::path::Path;

/// camDynamicBoomTune's constructor values (F-13319) of the keys its loader registers (F-77242),
/// where they are not 0: 15 shipped camera files leave out DirectionFilter, FishtailFactor and
/// HillPitchDn, 2 the gear-shift terms. (The shift curve is 5 keys at 0, .25, .5, .75 and 1.)
const BOOM_DEFAULTS: &[(&str, &[f32])] = &[
    ("Offset", &[0.0, 2.0, 5.0]),
    ("Focus", &[0.0, 1.25, -1.0]),
    ("Collide", &[0.0, 1.0, 0.0]),
    ("RevOffset", &[0.0, 2.0, -5.0]),
    ("RevFocus", &[0.0, 1.25, 1.0]),
    ("ApproachRate", &[20.0]),
    ("CollideType", &[2.0]),
    ("OrientationFactor", &[0.5]),
    ("OrientationFilter", &[0.1]),
    ("DirectionFilter", &[0.4]),
    ("VelocityFilter", &[0.8]),
    ("AccelFilter", &[0.15]),
    ("FishtailFactor", &[1.0]),
    ("HillPitchDn", &[2.5]),
    ("LinVelRange", &[30.0, 10.0, 50.0, 50.0]),
    ("LinAccRange", &[25.0, 10.0, 20.0, 20.0]),
    ("AngVelRange", &[2.0, 2.0, 2.0]),
    ("AngAccRange", &[2.0, 4.0, 10.0]),
    ("SSFovMin", &[35.0]),
    ("SSFovMax", &[50.0]),
    ("SSDistScale", &[0.7]),
    ("ShiftDuration", &[1.0]),
    ("ShiftMagnitude", &[0.5]),
    ("ShiftTime1", &[0.25]),
    ("ShiftTime2", &[0.5]),
    ("ShiftTime3", &[0.75]),
];

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
        // a key the file leaves out keeps the tune's constructor value (F-13319; every other
        // registered key is 0 there)
        let mut f: HashMap<String, Vec<f32>> = BOOM_DEFAULTS.iter().map(|(k, v)| (k.to_string(), v.to_vec())).collect();
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
}

/// The near plane of every view the camera director shows (the drive views, the look back, the event,
/// wipeout and ability cameras). The director's lens (F-98562, every frame) takes the current camera's
/// CameraNear / CameraFar (+172 / +176) only while its byte +92 is clear. The manager's setup (F-82384)
/// sets the main viewport's near 0.2 (the image constant 0x3E4CCCCD) and far 6000, then +92 = 1 with +88 / +84 = those two.
/// F-38892 only ever writes 1 there; the director's constructor F-52855 is the one place that clears it.
/// So a file's CameraNear (0.1 in the POV files, 0.2 in the rest) never reaches the lens: 0.2
/// everywhere (read live: viewport +704 = 0.2, +708 = 6000). GitHub #43: the port drew the cockpit with
/// its own 0.3, which cut the roof lining once speed moved the eye back under it.
pub const DIRECTOR_NEAR: f32 = f32::from_bits(0x3E4C_CCCD);

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

#[derive(Clone, Copy)]
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
    /// a gear change up is under way (+608): the kick fires when it is over
    shift_armed: bool,
    /// the lens of the last step (+168)
    lens: f32,
    /// `step` drives this camera (not `update`)
    stepped: bool,
    /// compare with recordings of the static recomp, which rescales the two filter factors by the frame time
    /// (its frame delta, the time word that rescale reads)
    recomp_timing: Option<f32>,
    /// the snap byte (+664): set where the original cuts to this camera; the next `step` takes its filters at 1
    /// and skips the approach rate, then clears it (it keeps the camera's state, unlike the port's own restart)
    snap: bool,
    /// `step_held`: a camera that has not been running is stepped from where it was left, however far (the port's
    /// 80 m restart stands for the game's teleport snap, which such a camera never gets)
    held: bool,
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
        // (the car's own terms - the slipstream turbo's lens and distance scale, the nitrous lens - only on a
        // chase view not looking back: F-44852 skips them on a fixed mount (+668) and while vt+56 (+208) is set;
        // GitHub #17: the cockpit took the turbo's 0.8 and 15 degrees and went up into the roof)
        let (car_fov, car_dist) = if car.fixed_mount || car.rev { (0.0, 1.0) } else { (car.nitro_fov.0 * self.nitro + car.sst.0, car.sst.1) };
        let fov = t.s("SSFovMin") + (t.s("SSFovMax") - t.s("SSFovMin")) * k + car_fov + ab_fov;
        let dist_scale = (1.0 + (t.s("SSDistScale") - 1.0) * k) * car_dist * ab_dist;

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

/// What the chase camera's frame (F-90220) reads from the car besides its matrix: the wheels in the chassis's
/// table order (contact point +368 and contact normal +336 of each wheel whose contact byte +529 is set), the
/// wheel count, the chassis speed (+224) and its state word (+592).
#[derive(Clone, Copy, Default, Debug)]
pub struct BoomGround {
    pub wheels: [Option<(Vec3, Vec3)>; 4],
    pub count: usize,
    pub speed: f32,
    pub flags: u32,
    /// the gearbox (chassis +20): its gear (+20, the original's numbering: 2 = first) and its shift under way (+24:
    /// 1 up, -1 down, 0 none)
    pub gear: i32,
    pub shifting: i32,
    /// the chassis's air time (+584, F-75234): a fixed mount's frame snaps by min(air - 0.5, 1) above 0.5 s
    pub air: f32,
}

/// the world's up as the frame takes it (a global vector; (0, 1, 0))
const BOOM_UP: Vec3 = Vec3::Y;

/// normalize as the vector unit does it: the estimate of 1 / sqrt and one Newton step; a zero vector stays zero
fn vnorm(v: Vec3) -> Vec3 {
    use crate::physics::exact::{dot3, rsqrt};
    let l2 = dot3(v, v);
    if l2 == 0.0 {
        Vec3::ZERO
    } else {
        v * rsqrt(l2)
    }
}

/// clamp(v / range, -1, 1) by the two fsel pairs; 0 when the range is not above 0
fn boom_range(v: f32, range: f32) -> f32 {
    if range > 0.0 {
        let q = v / range;
        let q = if q - -1.0 >= 0.0 { q } else { -1.0 };
        if q - 1.0 >= 0.0 { 1.0 } else { q }
    } else {
        0.0
    }
}

/// F-17888: sin(x pi/2) for 0 < x < 1, else x
fn boom_ease(x: f32) -> f32 {
    if x > 0.0 && x < 1.0 {
        crate::physics::exact::guest_sin((x * std::f32::consts::FRAC_PI_2) as f64) as f32
    } else {
        x
    }
}

/// +1, 0 or -1 (the fsel pair after F-17888)
fn boom_sign(x: f32) -> f32 {
    if x >= 0.0 {
        if -x >= 0.0 { 0.0 } else { 1.0 }
    } else {
        -1.0
    }
}

/// F-43547: about x, y' = y cos - z sin, z' = z cos + y sin (the guest's sin / cos, fused sums)
fn boom_rot_x(v: Vec3, a: f32) -> Vec3 {
    let s = crate::physics::exact::guest_sin(a as f64) as f32;
    let c = crate::physics::exact::guest_cos(a as f64) as f32;
    Vec3::new(v.x, v.y.mul_add(c, -(v.z * s)), v.z.mul_add(c, v.y * s))
}

/// F-57591: about y, x' = x cos + z sin, z' = z cos - x sin (fused)
fn boom_rot_y(v: Vec3, a: f32) -> Vec3 {
    use crate::physics::exact::{fma, fms};
    let s = crate::physics::exact::guest_sin(a as f64) as f32;
    let c = crate::physics::exact::guest_cos(a as f64) as f32;
    Vec3::new(fma(v.x, c, v.z * s), v.y, fms(v.z, c, v.x * s))
}

/// toward goal by `step` a call, stopping there (the tumble +400, F-90220)
fn boom_toward(cur: f32, goal: f32, step: f32) -> f32 {
    if cur < goal {
        (cur + step).min(goal)
    } else if cur > goal {
        (cur - step).max(goal)
    } else {
        cur
    }
}

/// The shift kick's curve (camBoomCS tune +448): a rage spline of five keys (time, value) - (0, 0), (ShiftTime1,
/// ShiftValue1) .. (ShiftTime3, ShiftValue3), (1, 0) - with its segments prepared as Catmull-Rom cubics, the end
/// tangents from mirrored nodes; each segment's value at local time u (F-39947: (u^3, u^2, u, 1) on the prepared row,
/// summed in the vector unit's pair order).
fn shift_curve(t: &BoomTune) -> [impl Fn(f32) -> f32; 4] {
    let n = [0.0, t.s("ShiftValue1"), t.s("ShiftValue2"), t.s("ShiftValue3"), 0.0];
    let node = |i: i32| -> f32 {
        if i < 0 {
            (n[0] - n[1]) * 2.0 + n[1]
        } else if i > 4 {
            (n[4] - n[3]) * 2.0 + n[3]
        } else {
            n[i as usize]
        }
    };
    let row = |s: i32| {
        let (p0, p1, p2, p3) = (node(s - 1), node(s), node(s + 1), node(s + 2));
        const M: [[f32; 4]; 4] = [[-0.5, 1.5, -1.5, 0.5], [1.0, -2.5, 2.0, -0.5], [-0.5, 0.0, 0.5, 0.0], [0.0, 1.0, 0.0, 0.0]];
        let mut c = [0.0f32; 4];
        for (r, m) in c.iter_mut().zip(M) {
            *r = p3 * m[3] + (p2 * m[2] + (p1 * m[1] + p0 * m[0]));
        }
        move |u: f32| {
            let (sq, cube) = (u * u, u * u * u);
            (c[3] + c[2] * u) + (c[1] * sq + c[0] * cube)
        }
    };
    [row(0), row(1), row(2), row(3)]
}

impl BoomCam {
    /// `step` on a camera that was not running (the replay shot test's candidate, F-51014 -> slot 10): from the state
    /// it was left in, without the port's 80 m restart; a camera never run yet starts with the reset.
    pub fn step_held(&mut self, t: &BoomTune, car: &BoomTarget, g: &BoomGround, dt: f32, fps: f32) -> BoomOut {
        self.held = true;
        let o = self.step(t, car, g, dt, fps);
        self.held = false;
        o
    }

    /// The camera's reset as the original's camDynamicBoomCS slot 9 (F-83404), run when the camera is made current:
    /// the base reset (F-74452: the ability's lens offset 0), the approach rate from ApproachRate, the snap byte, the
    /// boom's offset and focus (RevOffset / RevFocus while looking back) less Center, the roll and the tumble 0, the
    /// frame's up and direction from the car's own rows and then the frame (F-90220) at a snap share of 1, the place
    /// = the car's matrix x (offset, 1), the filters cleared and the filtered velocity and turn rate started at the car's
    /// in the frame (F-17419), the shift kick off, the lens SSFovMin.
    pub fn reset_as_original(&mut self, t: &BoomTune, car: &BoomTarget, g: &BoomGround, dt: f32) {
        use crate::physics::exact::dot3;
        let car_m = Mat3::from_quat(car.rot);
        let (r0, r1, r2) = (car_m.x_axis, car_m.y_axis, car_m.z_axis);
        self.started = true;
        self.stepped = true;
        self.last_gear = car.gear;
        self.nitro = 0.0;
        self.ab_fov = 0.0;
        self.rate = t.s("ApproachRate");
        self.snap = true;
        let c = t.v3("Center");
        let (o, f) = if car.rev { (t.v3("RevOffset"), t.v3("RevFocus")) } else { (t.v3("Offset"), t.v3("Focus")) };
        self.off = o - c;
        self.foc = f - c;
        self.roll = 0.0;
        self.tumble = 0.0;
        self.up = r1;
        self.dir = r2;
        let (fx, fz) = self.frame(t, car, g, 1.0, dt);
        // (vmsum4fp128 of (offset, 1) with the matrix's columns: (place + z c) + (y b + x a))
        let o = self.off;
        let k = |i: usize| (car.pos[i] + o.z * r2[i]) + (o.y * r1[i] + o.x * r0[i]);
        self.world = Vec3::new(k(0), k(1), k(2));
        // (F-17419: the frame's rows the frame left at +256 - x, up, z - dotted with the matrix object's velocity and spin)
        let fu = self.up;
        self.vel = Vec3::new(dot3(fx, car.vel), dot3(fu, car.vel), dot3(fz, car.vel));
        self.ang = Vec3::new(dot3(fx, car.ang_vel), dot3(fu, car.ang_vel), dot3(fz, car.ang_vel));
        self.acc = Vec3::ZERO;
        self.ang_acc = Vec3::ZERO;
        self.gz = 0.0;
        self.shift = 0.0;
        self.shift_val = 0.0;
        self.shift_armed = false;
        self.lens = t.s("SSFovMin");
    }

    /// The frame (F-90220): the up and the heading toward the grounded wheels' polygon and the car's back row, by the
    /// snap share `k`; returns the frame's x and z.
    fn frame(&mut self, t: &BoomTune, car: &BoomTarget, g: &BoomGround, k: f32, dt: f32) -> (Vec3, Vec3) {
        use crate::physics::exact::{cross, fma};
        let car_m = Mat3::from_quat(car.rot);
        let s0 = fma(1.0 - t.s("OrientationFactor"), k, t.s("OrientationFactor"));
        let s1 = fma(1.0 - t.s("OrientationFilter"), k, t.s("OrientationFilter"));
        let s2 = fma(1.0 - t.s("DirectionFilter"), k, t.s("DirectionFilter"));
        // the grounded wheels whose normal is within 0.7 of up, in the table's order 2, 3, 1, 0 (four) or 1, 0 (two)
        let order: &[usize] = if g.count >= 4 { &[2, 3, 1, 0] } else { &[1, 0] };
        let pts: Vec<Vec3> = order
            .iter()
            .filter_map(|&i| g.wheels.get(i).copied().flatten())
            .filter(|&(_, n)| fma(n.x, BOOM_UP.x, fma(n.y, BOOM_UP.y, n.z * BOOM_UP.z)) > 0.7)
            .map(|(p, _)| p)
            .collect();
        let c = pts.len();
        let mut sum = BOOM_UP;
        if c > 2 {
            for i in 1..=c {
                let a = pts[i - 1];
                let n = cross(pts[i % c] - a, pts[(c - 2 + i) % c] - a);
                sum = vnorm(n) + sum;
            }
        }
        // (a fixed mount, +668: the up target is the followed matrix's own up row, the direction its back row as it
        // is - no tumble, no heading filter, +384 left alone)
        let up_target = if car.fixed_mount { (car_m.y_axis - BOOM_UP) * s0 + BOOM_UP } else { (vnorm(sum) - BOOM_UP) * s0 + BOOM_UP };
        if car.fixed_mount {
            self.up = vnorm((up_target - self.up) * s1 + self.up);
        }
        let vh = Vec3::new(-car.vel.x, 0.0, -car.vel.z);
        let speed_h = fma(car.vel.x, car.vel.x, car.vel.z * car.vel.z).sqrt();
        let along = vnorm(car_m.z_axis + vh);
        let up_y = car_m.y_axis.y;
        let goal = if g.flags & 0x10 != 0 {
            Some(1.0)
        } else if speed_h < 5.0 || up_y > 0.7 {
            Some(0.0)
        } else if speed_h > 15.0 || up_y < 0.3 {
            Some(1.0)
        } else {
            None
        };
        let frame_dir = if car.fixed_mount {
            car_m.z_axis
        } else {
            if let Some(goal) = goal {
                self.tumble = boom_toward(self.tumble, goal, dt);
            }
            let heading = vnorm((along - car_m.z_axis) * self.tumble + car_m.z_axis);
            // (while the state word's 0x10 is set the heading is followed only from 10 m/s)
            if g.flags & 0x10 == 0 || g.speed >= 10.0 {
                self.dir = vnorm((heading - self.dir) * s2 + self.dir);
            }
            self.up = vnorm((up_target - self.up) * s1 + self.up);
            self.dir
        };
        let x = vnorm(cross(self.up, frame_dir));
        let z = vnorm(cross(x, self.up));
        (x, z)
    }

    /// One call of the original's chase-camera update (camDynamicBoomCS, F-14014), as the original runs it once a
    /// game frame: the lens (F-44852) from last call's filtered velocity, the frame (F-90220), the filters, the
    /// normalised sets, the boom, the hill pitch, the look-around turn, the homing and the approach rate, for a chase
    /// view (+668 clear) - every factor per call, as the original's. `dt` is the frame's time (F-76427), `fps` the
    /// game's frame-rate word that halves both filters below 60.
    pub fn step(&mut self, t: &BoomTune, car: &BoomTarget, g: &BoomGround, dt: f32, fps: f32) -> BoomOut {
        use crate::physics::exact::{dot3, fma, fnms, guest_asin, guest_cos};
        // (the port's: a car moved over 80 m - a teleport - starts the camera afresh, as `update` does; the original
        // sets its snap byte +664 where the game moves the car)
        // (the port's own restart: a fresh camera, or a car moved over 80 m - a teleport; the original's snap byte
        // +664 snaps the filters without clearing the state)
        // (compared with a recording - `recomp_timing` - the recorded snap byte stands for the teleport, so the port's
        // 80 m rule is off there)
        let far = self.recomp_timing.is_none() && !self.held && (self.world - car.pos).length_squared() > 80.0 * 80.0;
        let restart = !self.started || far;
        if restart {
            // (a fresh or moved camera: the original's slot 9, F-83404, which every camera gets when it is made current;
            // it sets the snap byte this call takes)
            self.reset_as_original(t, car, g, dt);
        }
        let snap = std::mem::take(&mut self.snap);
        // (the frame's snap share: 1 on a snap; on a fixed mount (+668) the chassis's air time over 0.5 s, at most 1 -
        // F-75234 through the chassis's vt+52 - so the bumper / cockpit frame follows the car in a long jump)
        let k = if snap {
            1.0
        } else if car.fixed_mount && g.air > 0.5 {
            let a = g.air - 0.5;
            let a = if a >= 0.0 { a } else { 0.0 };
            if a - 1.0 >= 0.0 { 1.0 } else { a }
        } else {
            0.0
        };
        let car_m = Mat3::from_quat(car.rot);

        // ---- the lens, F-44852: from last call's filtered velocity (+464, +472) ----
        let q = fma(self.vel.x, self.vel.x, self.vel.z * self.vel.z).sqrt() / t.range("LinVelRange", 2);
        let q = if q >= 0.0 { q } else { 0.0 };
        let ss = if q - 1.0 >= 0.0 { 1.0 } else { q };
        let cosv = if ss <= 0.0 {
            1.0
        } else if ss >= 1.0 {
            -1.0
        } else {
            guest_cos((ss * std::f32::consts::PI) as f64) as f32
        };
        let kk = (1.0 - cosv) * 0.5;
        let mut dist = fma(t.s("SSDistScale") - 1.0, kk, 1.0);
        let mut fov = fma(t.s("SSFovMax") - t.s("SSFovMin"), kk, t.s("SSFovMin"));
        // (the slipstream turbo and the nitrous only on a chase view not looking back: F-44852 skips them on a fixed
        // mount, +668, and while vt+56, +208, is set)
        if !car.rev && !car.fixed_mount {
            fov = car.sst.0 + fov;
            dist = car.sst.1 * dist;
            // (approx.: the nitrous object's +24 lens is the port's ramp)
            self.nitro = if car.nitro { (self.nitro + dt / car.nitro_fov.1.max(0.05)).min(1.0) } else { (self.nitro - dt / car.nitro_fov.2.max(0.05)).max(0.0) };
            if self.nitro > 0.0 {
                fov = car.nitro_fov.0 * self.nitro + fov;
            }
        }
        // (then every view: the ability's lens +224 added, its distance +228 multiplied; F-38892 writes them to the
        // bumper and cockpit only, the lens held at its lowest since the view was set - the chase views get (0, 1))
        let ab_fov = if car.fixed_mount {
            self.ab_fov = self.ab_fov.min(car.ability.0);
            self.ab_fov
        } else {
            car.ability.0
        };
        fov = ab_fov + fov;
        dist = car.ability.1 * dist;
        let dist_scale = dist;

        // ---- the frame, F-90220 ----
        let (x, z) = self.frame(t, car, g, k, dt);
        let ctr = t.v3("Center");
        let col = |i: usize| Vec3::new(car_m.x_axis[i], car_m.y_axis[i], car_m.z_axis[i]);
        let origin = car.pos + Vec3::new(dot3(ctr, col(0)), dot3(ctr, col(1)), dot3(ctr, col(2)));

        // ---- the filters: velocity and turn rate (VelocityFilter), their changes (AccelFilter) ----
        // (the frame rate rounded, fctiwz of fps + 0.5, below 60)
        let half = if ((fps + 0.5) as i32) < 60 { 0.5 } else { 1.0 };
        let (mut kv, mut ka) = (t.s("VelocityFilter") * half, t.s("AccelFilter") * half);
        if let Some(rdt) = self.recomp_timing {
            // (the static recomp's own frame-time fix, not the original's: its hooks at these two factors make them
            // 1 - (1 - f)^(30 dt) - the recomp's RescaleCameraFactor, which halves a factor the game left whole (at a
            // rounded 60 frames a second or more) first; only to compare with its recordings)
            let per = |f: f32| {
                if f > 0.0 && f < 1.0 && rdt > 0.0 {
                    let f = if half == 1.0 { 0.5 * f as f64 } else { f as f64 };
                    (1.0 - (1.0 - f).powf(rdt as f64 * 30.0)) as f32
                } else {
                    f
                }
            };
            (kv, ka) = (per(kv), per(ka));
        }
        let up_now = self.up;
        let to_frame = |v: Vec3| Vec3::new(dot3(x, v), dot3(up_now, v), dot3(z, v));
        let inv_dt = 1.0 / dt;
        let vel = to_frame(car.vel) * kv + self.vel * (1.0 - kv);
        let mut acc = (vel - self.vel) * inv_dt * ka + self.acc * (1.0 - ka);
        let mut ang = to_frame(car.ang_vel) * kv + self.ang * (1.0 - kv);
        let mut ang_acc = (ang - self.ang) * inv_dt * ka + self.ang_acc * (1.0 - ka);
        // (below SpeedThreshold the changes and the turn are scaled by speed / threshold)
        let speed = fma(vel.z, vel.z, fma(vel.x, vel.x, vel.y * vel.y)).sqrt();
        let thr = t.s("SpeedThreshold");
        if speed < thr {
            let s = (1.0 / thr) * speed;
            acc *= s;
            ang *= s;
            ang_acc *= s;
        }
        self.vel = vel;
        self.acc = acc;
        self.ang = ang;
        self.ang_acc = ang_acc;

        // ---- the normalised sets (+528 speed, +544 turn, +560 acceleration, +576 turn acceleration) ----
        let sy = boom_range(vel.y, t.range("LinVelRange", 1));
        let sp = Vec3::new(
            boom_range(vel.x, t.range("LinVelRange", 0)),
            boom_ease(sy.abs()) * boom_sign(sy),
            -boom_range(vel.z, if vel.z > 0.0 { t.range("LinVelRange", 3) } else { t.range("LinVelRange", 2) }),
        );
        let tn = Vec3::new(boom_range(ang.x, t.range("AngVelRange", 0)), -boom_range(ang.y, t.range("AngVelRange", 1)), -boom_range(ang.z, t.range("AngVelRange", 2)));
        let gy = boom_range(acc.y, t.range("LinAccRange", 1));
        let mut gs = Vec3::new(
            boom_range(acc.x, t.range("LinAccRange", 0)),
            boom_ease(gy.abs()) * boom_sign(gy),
            -boom_range(acc.z, if acc.z > 0.0 { t.range("LinAccRange", 3) } else { t.range("LinAccRange", 2) }),
        );
        let asx = Vec3::new(
            boom_range(ang_acc.x, t.range("AngAccRange", 0)),
            -boom_range(ang_acc.y, t.range("AngAccRange", 1)),
            -boom_range(ang_acc.z, t.range("AngAccRange", 2)),
        );
        if gs.z < 0.0 {
            gs.z = fnms(tn.y.abs(), t.s("FishtailFactor"), 1.0) * gs.z;
        }
        self.gz = gs.z;

        // ---- the gear change kick (F-66782) ----
        // (armed while the gearbox shifts up - vehTransmission +24 = 1 - above first (+20 > 2, the original's
        // numbering); fired the frame the shift is over, if no kick runs: +596 = 1, its size +604 = this frame's
        // gz, at least 0.01)
        if g.shifting > 0 && g.gear > 2 {
            self.shift_armed = true;
        }
        if self.shift_armed && g.shifting == 0 && self.shift == 0.0 {
            self.shift = 1.0;
            self.shift_size = if self.gz >= 0.01 { self.gz } else { 0.01 };
            self.shift_armed = false;
        }
        if self.shift > 0.0 {
            // the tune's curve (tune +448): five keys at times 0, ShiftTime1..3, 1; the segment and its local time,
            // the value its prepared Catmull-Rom (F-39947, the y of the vector it returns)
            let p = 1.0 - self.shift;
            let (t1, t2, t3) = (t.s("ShiftTime1"), t.s("ShiftTime2"), t.s("ShiftTime3"));
            let (seg, u) = if p < t1 {
                (0, p / t1)
            } else if p < t2 {
                (1, (p - t1) / (t2 - t1))
            } else if p < t3 {
                (2, (p - t2) / (t3 - t2))
            } else {
                (3, (p - t3) / (1.0 - t3))
            };
            let val = shift_curve(t)[seg](u);
            self.shift_val = t.s("ShiftMagnitude") * self.shift_size * val;
            self.shift = fnms(1.0 / t.s("ShiftDuration"), dt, self.shift);
            if self.shift < 0.0 {
                self.shift = 0.0;
            }
        }

        // ---- the boom: Offset and Focus less Center, shortened by the distance scale ----
        let (off_k, foc_k) = if car.rev { ("RevOffset", "RevFocus") } else { ("Offset", "Focus") };
        let off0 = t.v3(off_k) - ctr;
        let mut foc0 = t.v3(foc_k) - ctr;
        // (a chase view - vt+60, the fixed-mount byte +668, clear - takes the focus's height times the target's
        // [+28] +36, clamped to 0..1, before the boom's direction is formed: the car's up axis's y - equal to the
        // followed matrix's on every recorded frame - so a car on its side looks at its middle)
        if !car.fixed_mount {
            let h = car_m.y_axis.y;
            let h = if h >= 0.0 { h } else { 0.0 };
            foc0.y = (if h - 1.0 >= 0.0 { 1.0 } else { h }) * foc0.y;
        }
        let len0 = fma(off0.y, off0.y, fma(off0.z, off0.z, off0.x * off0.x)).sqrt();
        let off = vnorm(off0 - foc0) * -(len0 * (1.0 - dist_scale)) + off0;
        let szz = sp.z.abs();
        let (mut zoff, mut yoff, mut yfoc, mut zfoc) = (szz * t.s("SpeedZOffsetZ"), szz * t.s("SpeedZOffsetY"), szz * t.s("SpeedZFocusY"), 0.0f32);
        if gs.z >= 0.0 {
            zoff = fma(t.s("GsZOffsetZ"), gs.z, zoff);
            yoff = fma(t.s("GsZOffsetY"), gs.z, yoff);
            yfoc = fma(t.s("GsZFocusY"), gs.z, yfoc);
        } else {
            zoff = -(t.s("GsWOffsetZ") * gs.z) + zoff;
            yoff = -(t.s("GsWOffsetY") * gs.z) + yoff;
            yfoc = -(t.s("GsWFocusY") * gs.z) + yfoc;
            zfoc = -(t.s("GsWFocusZ") * gs.z);
        }
        let (sx, ty, tz, gx, asy, asz) = (sp.x, tn.y, tn.z, gs.x, asx.y, asx.z);
        zfoc = fma(sx.abs(), t.s("SpeedXFocusZ"), zfoc);
        zoff = fma(sx.abs(), t.s("SpeedXOffsetZ"), zoff);
        let mut roll = fma(t.s("SpeedXRollZ"), sx, t.s("TurnYRollZ") * ty);
        let mut xfoc = fma(t.s("TurnYFocusX"), ty, t.s("SpeedXFocusX") * sx);
        let mut xoff = fma(t.s("TurnYOffsetX"), ty, t.s("SpeedXOffsetX") * sx);
        zfoc = fma(ty.abs(), t.s("TurnYFocusZ"), zfoc);
        yoff = fma(sx.abs(), t.s("SpeedXOffsetY"), yoff);
        roll = fma(t.s("TurnZRollZ"), tz, roll);
        xfoc = fma(t.s("GsXFocusX"), gx, xfoc);
        zoff = fma(ty.abs(), t.s("TurnYOffsetZ"), zoff);
        xoff = fma(t.s("GsXOffsetX"), gx, xoff);
        yfoc = fma(sx.abs(), t.s("SpeedXFocusY"), yfoc);
        zfoc = fma(gx.abs(), t.s("GsXFocusZ"), zfoc);
        yoff = fma(ty.abs(), t.s("TurnYOffsetY"), yoff);
        roll = fma(t.s("GsXRollZ"), gx, roll);
        xfoc = fma(t.s("AsYFocusX"), asy, xfoc);
        zoff = fma(gx.abs(), t.s("GsXOffsetZ"), zoff);
        xoff = fma(t.s("AsYOffsetX"), asy, xoff);
        yfoc = fma(ty.abs(), t.s("TurnYFocusY"), yfoc);
        zfoc = fma(asy.abs(), t.s("AsYFocusZ"), zfoc);
        zoff = fma(asy.abs(), t.s("AsYOffsetZ"), zoff);
        roll = fma(t.s("AsZRollZ"), asz, roll);
        // (the original adds AsYRollZ to the focus's z, not to the roll)
        zfoc = fma(t.s("AsYRollZ"), asy, zfoc);
        yoff = fma(sp.y.abs(), t.s("SpeedYOffsetY"), yoff);
        yfoc = fma(sp.y.abs(), t.s("SpeedYFocusY"), yfoc);
        yoff = fma(t.s("TurnXOffsetY"), tn.x, yoff);
        yfoc = fma(t.s("TurnXFocusY"), tn.x, yfoc);
        yoff = fma(t.s("GsYOffsetY"), gs.y, yoff);
        yfoc = fma(t.s("GsYFocusY"), gs.y, yfoc);
        yoff = fma(t.s("AsYOffsetY"), asx.x, yoff);
        yfoc = fma(t.s("AsYFocusY"), asx.x, yfoc);
        // (the kick only while +596 is above 0 after this frame's step; +600 keeps its last value)
        if self.shift > 0.0 {
            zoff = fma(t.s("ShiftZOffsetZ"), self.shift_val, zoff);
            yoff = fma(t.s("ShiftZOffsetY"), self.shift_val, yoff);
            yfoc = fma(t.s("ShiftZFocusY"), self.shift_val, yfoc);
        }
        if z.y < 0.0 {
            // (times 1 - the snap)
            let (ay, az) = (t.s("AngleXOffsetY") * (1.0 - k), t.s("AngleXOffsetZ") * (1.0 - k));
            zoff = fma(az, z.y, zoff);
            yoff = -(ay * z.y) + yoff;
        }
        let (pan_yaw, pan_pitch) = car.pan;
        if pan_yaw != 0.0 {
            let m = 1.0 - (pan_yaw * pan_yaw).clamp(0.0, 1.0);
            xfoc *= m;
            zfoc *= m;
            xoff *= m;
            zoff *= m;
        }
        if pan_pitch != 0.0 {
            let m = 1.0 - (pan_pitch * pan_pitch).clamp(0.0, 1.0);
            yfoc *= m;
            yoff *= m;
        }
        let mut off = off + Vec3::new(xoff, yoff, zoff);
        let mut foc = foc0 + Vec3::new(xfoc, yfoc, zfoc);
        // the hill pitch: asin of the frame's nose (F-80613), the boom turned about x (F-43547); a chase view only
        // (+668 skips it)
        if !car.fixed_mount {
            let (up_k, dn_k) = if car.rev { (-t.s("HillPitchDn"), -t.s("HillPitchUp")) } else { (t.s("HillPitchUp"), t.s("HillPitchDn")) };
            let a = guest_asin(z.y as f64) as f32;
            let swing = if a < 0.0 { a.abs() * up_k } else { a.abs() * dn_k };
            off = boom_rot_x(off, -swing);
        }
        off.y += car.rise;
        foc.y += car.rise;
        // the look-around turns camera and focus about the car tune's ModelOffset (pitch, then yaw); a fixed mount
        // turns them about its own last offset (+416)
        if pan_yaw != 0.0 || pan_pitch != 0.0 {
            let pivot = if car.fixed_mount { self.off } else { car.model_offset };
            off = boom_rot_y(boom_rot_x(off - pivot, pan_pitch), pan_yaw) + pivot;
            foc = boom_rot_y(boom_rot_x(foc - pivot, pan_pitch), pan_yaw) + pivot;
        }

        // ---- the homing: last call's place seen from the new frame goes HomingTime of the way ----
        let len = fma(off.y, off.y, fma(off.z, off.z, off.x * off.x)).sqrt();
        let homing = t.v3("HomingTime");
        // (not gated by the snap; after a reset it homes from the place F-83404 put at +64)
        if homing != Vec3::ZERO {
            let d = self.world - origin;
            let last = Vec3::new(dot3(x, d), dot3(self.up, d), dot3(z, d));
            off = (off - last) * homing + last;
        }
        off = vnorm(off) * len;

        // ---- the approach rate (+656: 25 while turned, else ApproachRate, moving there at 50 a second, F-90102) ----
        let file_rate = t.s("ApproachRate");
        if file_rate > 0.0 {
            let goal = if pan_yaw != 0.0 || pan_pitch != 0.0 { 25.0 } else { file_rate };
            self.rate = if self.rate < goal {
                fma(50.0, dt, self.rate).min(goal)
            } else if self.rate > goal {
                fnms(50.0, dt, self.rate).max(goal)
            } else {
                self.rate
            };
        } else {
            self.rate = 0.0;
        }
        if self.rate > 0.0 && !snap {
            let step = self.rate * dt;
            let mut share = 1.0f32;
            for (a, b) in [(off.x, self.off.x), (off.y, self.off.y), (off.z, self.off.z), (foc.x, self.foc.x), (foc.y, self.foc.y), (foc.z, self.foc.z)] {
                let d = (a - b).abs();
                if d > step {
                    share = share.min(step / d);
                }
            }
            off = Vec3::new(fma(off.x - self.off.x, share, self.off.x), fma(off.y - self.off.y, share, self.off.y), fma(off.z - self.off.z, share, self.off.z));
            foc = Vec3::new(fma(foc.x - self.foc.x, share, self.foc.x), fma(foc.y - self.foc.y, share, self.foc.y), fma(foc.z - self.foc.z, share, self.foc.z));
            roll = fma(roll - self.roll, share, self.roll);
        }
        self.off = off;
        self.foc = foc;
        self.roll = roll;
        self.lens = fov;
        let frame = Mat3::from_cols(x, self.up, z);
        let pos = origin + frame * off;
        self.world = pos;
        self.draw(t, car)
    }

    /// The view `step` left, on the car as it is drawn: the boom's frame, camera, focus and roll of the last game
    /// frame on the drawn car's place (origin = place + car x Center). At a game frame's own instant it is the
    /// original's output; between frames the boom keeps its last state while the car moves on.
    pub fn draw(&self, t: &BoomTune, car: &BoomTarget) -> BoomOut {
        use crate::physics::exact::{cross, dot3};
        let car_m = Mat3::from_quat(car.rot);
        let ctr = t.v3("Center");
        let col = |i: usize| Vec3::new(car_m.x_axis[i], car_m.y_axis[i], car_m.z_axis[i]);
        let origin = car.pos + Vec3::new(dot3(ctr, col(0)), dot3(ctr, col(1)), dot3(ctr, col(2)));
        // (a fixed mount's frame takes the followed car's back axis itself: here the car as drawn)
        let x = vnorm(cross(self.up, if car.fixed_mount { car_m.z_axis } else { self.dir }));
        let z = vnorm(cross(x, self.up));
        let frame = Mat3::from_cols(x, self.up, z);
        let pos = origin + frame * self.off;
        let look = origin + frame * self.foc;
        let view = (look - pos).normalize_or(-z);
        let up = Quat::from_axis_angle(view, self.roll) * self.up;
        BoomOut { pos, look, up, fov: self.lens }
    }

    /// Whether `step` has run since the last reset.
    pub fn stepped(&self) -> bool {
        self.started && self.stepped
    }
}

impl BoomGround {
    /// The frame's ground from the car's sim state: the wheels in the chassis table's order (the port's FL, FR, RL,
    /// RR is that order; a bike's two are the port's 0 and 2), each in contact with its contact point and normal;
    /// the speed the chassis came into the tick with (+224, |velocity|); the crash word; the gearbox's gear in the
    /// original's numbering (port + 1) and its shift under way.
    pub fn from_state(st: &crate::physics::car::CarState, bike: bool, flags: u32) -> BoomGround {
        let mut g = BoomGround { count: if bike { 2 } else { 4 }, speed: st.vel.length(), flags, ..Default::default() };
        let slots: &[usize] = if bike { &[0, 2] } else { &[0, 1, 2, 3] };
        for (k, &i) in slots.iter().enumerate() {
            let w = &st.wheels[i];
            if w.grounded {
                g.wheels[k] = Some((w.contact, w.normal));
            }
        }
        if let Some(v) = st.veh.as_ref() {
            g.gear = v.gear + 1;
            g.shifting = v.gear_change.direction as i32;
            g.air = v.air584();
        } else {
            g.gear = st.gear + 1;
        }
        g
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

/// A camera the director can show: one of the player's views (port view), the event camera
/// (mcPlayerCamera +84), the wipeout camera (+72) or the ability camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CamId {
    View(u8),
    Event,
    Wipeout,
    Ability,
}

impl Default for CamId {
    fn default() -> CamId {
        CamId::View(0)
    }
}

/// The director (camViewCS): one current camera (+48) or its blend camera (camTransitionCS,
/// `DirBlend`) between a source (+192) and a target (+196) with one queued camera (+200), and
/// the last push's time (+20). Every camera of the player goes through it - the views, the
/// event, wipeout and ability cameras -, so a push that meets another camera's blend is queued
/// (F-43758) and cut to at the blend's end (F-70013). While it blends both cameras keep running
/// and the blend's matrix is mixed from theirs each frame (F-18583), as an orbit about the
/// followed car; the weight runs 0 -> the source's BlendGoal over the push's time, eased by the
/// transition type (3).
#[derive(Default)]
pub struct Director {
    /// the source view's camera when the blend runs from one view to another (the target view's
    /// camera is the caller's own)
    from: Option<(BoomTune, BoomCam)>,
    pub blend: DirBlend,
    /// the director's time (+20): the last push's
    pub time: f32,
    /// the director shows its blend camera
    on: bool,
    pub src: CamId,
    pub dst: CamId,
    pub queued: Option<CamId>,
    /// the current camera when not blending; None: the drive view
    cur: Option<CamId>,
    /// the drive view: the view whose camera `Switch` runs (set each frame by `Switch::frame`)
    pub view: u8,
    /// set when the blend ended with a view queued (F-70013): the view the director cut to
    finished: Option<u8>,
}

/// What a push did (F-98424).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    Nothing,
    Queued,
    Turned,
    Begun,
    Cut,
}

impl Director {
    /// The director shows its blend camera.
    pub fn active(&self) -> bool {
        self.on
    }
    /// It blends from one view to another.
    pub fn views_blending(&self) -> bool {
        self.on && matches!((self.src, self.dst), (CamId::View(_), CamId::View(_)))
    }
    /// The current camera (when not blending).
    pub fn current(&self) -> CamId {
        self.cur.unwrap_or(CamId::View(self.view))
    }
    pub fn is_current(&self, id: CamId) -> bool {
        !self.on && self.current() == id
    }
    /// `id` is the current camera or the blend's source or target.
    pub fn involves(&self, id: CamId) -> bool {
        if self.on {
            self.src == id || self.dst == id
        } else {
            self.current() == id
        }
    }
    /// `id` is the current camera or the blend's target (F-38782 nonzero).
    pub fn current_or_target(&self, id: CamId) -> bool {
        if self.on {
            self.dst == id
        } else {
            self.current() == id
        }
    }
    /// The view of the blend (its target's, else its source's).
    pub fn view_in_blend(&self) -> Option<u8> {
        match (self.on, self.dst, self.src) {
            (true, CamId::View(v), _) | (true, _, CamId::View(v)) => Some(v),
            _ => None,
        }
    }
    /// The begin (F-18878) and start (F-41709): weight 0, the goal the SOURCE camera's
    /// BlendGoal, the push's time.
    fn begin(&mut self, src: CamId, dst: CamId, time: f32, goal: f32) {
        self.on = true;
        (self.src, self.dst) = (src, dst);
        self.blend = DirBlend { weight: 0.0, goal, left: time, ..Default::default() };
        self.time = time;
        self.queued = None;
        self.from = None;
    }
    /// A cut to the drive view (setView's type-4 push F-41662 -> F-53309): no blend, nothing queued.
    pub fn stop(&mut self) {
        self.cut(CamId::View(self.view));
    }
    /// A cut (F-53309): `id` current with no blend.
    pub fn cut(&mut self, id: CamId) {
        self.on = false;
        self.from = None;
        self.queued = None;
        self.cur = match id {
            CamId::View(_) => None,
            x => Some(x),
        };
    }
    /// The push F-98424 of `id` over `time` (a blend type; `src_goal` the BlendGoal of the camera
    /// a begun blend starts from). On the blend camera it stores its time as the director's (+20)
    /// and with a time above 0 (F-43758): nothing when the QUEUED camera is the target; when the
    /// queued camera is the source the blend turns round (F-29802, nothing queued after); else
    /// `id` is queued. Not blending: a time above 0 begins a blend from the current camera, else
    /// (or `id` already current) a cut.
    pub fn push(&mut self, id: CamId, time: f32, src_goal: f32) -> Pushed {
        if self.on {
            self.time = time;
            if !(time > 0.0) || self.queued == Some(self.dst) {
                return Pushed::Nothing;
            }
            if self.queued == Some(self.src) {
                self.turn_round(src_goal);
                self.queued = None;
                return Pushed::Turned;
            }
            self.queued = Some(id);
            return Pushed::Queued;
        }
        let cur = self.current();
        if time > 0.0 && cur != id {
            self.begin(cur, id, time, src_goal);
            Pushed::Begun
        } else {
            self.cut(id);
            Pushed::Cut
        }
    }
    /// The turn-round F-29802: goal = the old source's BlendGoal, left = the director's time x
    /// weight, weight = 1 - weight, source and target swapped (the queue kept).
    pub fn turn_round(&mut self, src_goal: f32) {
        self.blend.goal = src_goal;
        self.blend.left = self.time * self.blend.weight;
        self.blend.weight = 1.0 - self.blend.weight;
        std::mem::swap(&mut self.src, &mut self.dst);
    }
    /// A view push (setView, F-41662 -> F-98424) while the director is on its blend camera: as
    /// `push`; a turn-round between two views swaps the two views' cameras. `cam` is the running
    /// (target view's) camera, `dst_tune` its view's tune.
    pub fn push_view(&mut self, view: u8, time: f32, dst_tune: &BoomTune, cam: &mut BoomCam) -> Pushed {
        let goal = self.from.as_ref().map_or(1.0, |f| f.0.blend_goal());
        let r = self.push(CamId::View(view), time, goal);
        if r == Pushed::Turned {
            if let Some((_, src_cam)) = self.from.take() {
                let old_dst = std::mem::replace(cam, src_cam);
                self.from = Some((dst_tune.clone(), old_dst));
            }
        }
        r
    }
    /// A blend from the view `src` (its tune and camera) to the view `dst` over `time`.
    pub fn begin_views(&mut self, src: u8, dst: u8, tune: BoomTune, cam: BoomCam, time: f32) {
        if time > 0.0 {
            let goal = tune.blend_goal();
            self.begin(CamId::View(src), CamId::View(dst), time, goal);
            self.from = Some((tune, cam));
        } else {
            self.stop();
        }
    }
    /// The camera of `view` as the director runs it, for setView's reset (vt+36): the source's;
    /// the target's is the caller's own.
    pub fn source_cam(&mut self, view: u8) -> Option<&mut BoomCam> {
        if self.on && self.src == CamId::View(view) {
            self.from.as_mut().map(|f| &mut f.1)
        } else {
            None
        }
    }
    /// The source view's tune and camera when two views blend (stepped by the caller as the
    /// target's is).
    pub fn source_view(&mut self) -> Option<(&BoomTune, &mut BoomCam)> {
        let on = self.on;
        self.from.as_mut().filter(|_| on).map(|f| (&f.0, &mut f.1))
    }
    /// F-37740's head: the director on its blend camera finished at once (F-70013). Returns the
    /// camera that is current now.
    pub fn finish_now(&mut self) -> Option<CamId> {
        self.on.then(|| self.finish())
    }
    /// The finish (F-70013): nothing queued, or the target queued, the target is current; a
    /// queued camera other than the target is cut to (type 4 makes the start F-41709 a cut).
    fn finish(&mut self) -> CamId {
        self.on = false;
        self.from = None;
        let to = match self.queued.take() {
            Some(q) if q != self.dst => q,
            _ => self.dst,
        };
        if let (CamId::View(v), true) = (to, to != self.dst) {
            self.finished = Some(v);
        }
        self.cur = match to {
            CamId::View(_) => None,
            x => Some(x),
        };
        to
    }
    /// The director's output this frame: `drive` the drive view's camera's, `others` the event /
    /// wipeout / ability cameras' that run; while blending the source view's camera (two views)
    /// runs here and the blend steps (F-18583, type 3) about the followed car `car`.
    pub fn apply(&mut self, car: &BoomTarget, drive: BoomOut, others: &[(CamId, BoomOut)], dt: f32) -> BoomOut {
        let find = |id: CamId| match id {
            CamId::View(_) => Some(drive),
            x => others.iter().find(|o| o.0 == x).map(|o| o.1),
        };
        if !self.on {
            return find(self.current()).unwrap_or(drive);
        }
        let src = match (&mut self.from, self.src) {
            (Some((tune, cam)), CamId::View(_)) => {
                if cam.stepped() {
                    cam.draw(tune, car)
                } else {
                    cam.update(tune, car, dt)
                }
            }
            (_, id) => find(id).unwrap_or(drive),
        };
        let dst = find(self.dst).unwrap_or(drive);
        let (a, b) = (dir_cam_of(&src, car.pos), dir_cam_of(&dst, car.pos));
        if !self.blend.step(&a, &b, 3, self.time, dt) {
            let to = self.finish();
            return find(to).unwrap_or(dst);
        }
        let m = &self.blend;
        BoomOut { pos: m.pos, look: m.pos - m.rows[2] * (dst.look - dst.pos).length().max(1.0), up: m.rows[1], fov: m.lens[0] }
    }
}

/// A port camera's output as the director's blend camera reads it (the matrix from the eye, aim
/// and up: x right, y up, z back), following the car at `target`.
fn dir_cam_of(o: &BoomOut, target: Vec3) -> DirCam {
    let z = (o.pos - o.look).normalize_or(Vec3::Z);
    let x = o.up.cross(z).normalize_or(Vec3::X);
    DirCam { rows: [x, z.cross(x), z], pos: o.pos, target: Some(target), lens: [o.fov, 0.0, 0.0, 0.0], blend_goal: 1.0 }
}

/// One camera as the director's blend camera reads it: its matrix (the corrected one when
/// it has one), the place of the object it follows (+144's matrix, +48), its lens (+168 FOV,
/// +172 near, +176 far, +180) and its BlendGoal (+164).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirCam {
    pub rows: [Vec3; 3],
    pub pos: Vec3,
    pub target: Option<Vec3>,
    pub lens: [f32; 4],
    pub blend_goal: f32,
}

/// The director's blend camera (camTransitionCS): its matrix, lens, weight (+204), goal
/// (+208) and time left (+212).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DirBlend {
    pub rows: [Vec3; 3],
    pub pos: Vec3,
    pub lens: [f32; 4],
    pub weight: f32,
    pub goal: f32,
    pub left: f32,
}

impl DirBlend {
    /// Start (F-41709): weight 0, the goal the SOURCE camera's BlendGoal, the push's time;
    /// the source's matrix.
    pub fn start(from: &DirCam, t: f32) -> DirBlend {
        DirBlend { rows: from.rows, pos: from.pos, lens: from.lens, weight: 0.0, goal: from.blend_goal, left: t }
    }

    /// The step (F-18583): `mode` is the director's transition type (+16), `t` its time (+20).
    /// False when the blend is over (time up with goal 1: F-70013 hands over to the target).
    pub fn step(&mut self, from: &DirCam, to: &DirCam, mode: u16, t: f32, dt: f32) -> bool {
        use crate::physics::exact::{fma, fnms};
        self.left -= dt;
        if !(self.left > 0.0) && self.goal == 1.0 {
            return false;
        }
        if self.weight < self.goal {
            let s = if !(self.left > 0.0) {
                0.0
            } else if self.left < t {
                self.left / t
            } else {
                1.0
            };
            let w = fnms(self.goal, s, 1.0);
            self.weight = match mode {
                1 => ease_cos_quarter(w),
                2 => ease_sin_quarter(w),
                3 => (1.0 - cos_pi(w)) * 0.5,
                _ => w,
            };
            // (the lens only while the weight is short of the goal)
            for i in 0..4 {
                self.lens[i] = fma(to.lens[i] - from.lens[i], self.weight, from.lens[i]);
            }
        }
        let w = self.weight;
        // the pivot: the followed objects' places, mixed
        let pivot = match (from.target, to.target) {
            (Some(a), Some(b)) => (b - a) * w + a,
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => Vec3::ZERO,
        };
        let dist = |c: &DirCam| {
            let d = c.pos - pivot;
            fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)).sqrt()
        };
        let (da, db) = (dist(from), dist(to));
        let fa = from.pos - from.rows[2] * da;
        let fb = to.pos - to.rows[2] * db;
        let ea = euler_zxy(&from.rows);
        let mut eb = euler_zxy(&to.rows);
        const PI: f32 = std::f32::consts::PI;
        for k in 0..3 {
            let d = ea[k] - eb[k];
            if d > PI {
                eb[k] += 2.0 * PI;
            } else if d < -PI {
                eb[k] -= 2.0 * PI;
            }
        }
        let x = -fma(-eb[0] - -ea[0], w, -ea[0]);
        let y = fma(eb[1] - ea[1], w, ea[1]);
        let z = fma(eb[2] - ea[2], w, ea[2]);
        let d = fma(db - da, w, da);
        self.rows = rows_of_zxy([x, y, z]);
        self.pos = self.rows[2] * d + ((fb - fa) * w + fa);
        true
    }
}

/// F-86323: 1 - cos(x pi / 2) in 0..1.
fn ease_cos_quarter(x: f32) -> f32 {
    use crate::physics::exact::guest_cos;
    if x <= 0.0 {
        0.0
    } else if x >= 1.0 {
        1.0
    } else {
        1.0 - guest_cos((x * std::f32::consts::FRAC_PI_2) as f64) as f32
    }
}

/// F-17888: sin(x pi / 2) in 0..1.
fn ease_sin_quarter(x: f32) -> f32 {
    use crate::physics::exact::guest_sin;
    if x <= 0.0 {
        0.0
    } else if x >= 1.0 {
        1.0
    } else {
        guest_sin((x * std::f32::consts::FRAC_PI_2) as f64) as f32
    }
}

/// F-39265: cos(x pi) in 0..1 (1 below, -1 above).
fn cos_pi(x: f32) -> f32 {
    use crate::physics::exact::guest_cos;
    if x <= 0.0 {
        1.0
    } else if x >= 1.0 {
        -1.0
    } else {
        guest_cos((x * std::f32::consts::PI) as f64) as f32
    }
}

/// F-49999: the rows made orthonormal: row 2 kept and normalised, row 0 = row 1 x row 2
/// normalised, row 1 = row 2 x row 0 (the vector unit's products, each rounded).
fn orthonormal(r: &[Vec3; 3]) -> [Vec3; 3] {
    use crate::physics::exact::dot3;
    let unit = |v: Vec3| {
        let l2 = dot3(v, v);
        let e = 1.0 / l2.sqrt();
        // (an estimate of 0 - a length of infinity - is replaced by 0)
        let e = if e == 0.0 { 0.0 } else { e };
        v * ((1.0 - l2 * (e * e)) * (0.5 * e) + e)
    };
    let cross = |a: Vec3, b: Vec3| Vec3::new(a.z * b.y - a.y * b.z, a.x * b.z - a.z * b.x, a.y * b.x - a.x * b.y);
    let r2 = unit(r[2]);
    let r0 = unit(cross(r2, r[1]));
    let r1 = cross(r0, r2);
    [r0, r1, r2]
}

/// F-33934 for the order "zxy": the matrix orthonormalised (F-49999), then x = asin(-m21),
/// y = atan2(m20, m22), z = atan2(m01, m11).
pub fn euler_zxy(r: &[Vec3; 3]) -> [f32; 3] {
    use crate::physics::exact::{guest_asin, guest_atan2};
    let m = orthonormal(r);
    let z = guest_atan2(m[0].y as f64, m[1].y as f64) as f32;
    let y = guest_atan2(m[2].x as f64, m[2].z as f64) as f32;
    let x = guest_asin((-m[2].y) as f64) as f32;
    [x, y, z]
}

/// F-62659: the rows of the angles (order zxy); an angle of 0 takes sin 0, cos 1.
pub fn rows_of_zxy(e: [f32; 3]) -> [Vec3; 3] {
    use crate::physics::exact::{fma, fms, guest_cos, guest_sin};
    let sc = |a: f32| if a == 0.0 { (0.0, 1.0) } else { (guest_sin(a as f64) as f32, guest_cos(a as f64) as f32) };
    let ((sx, cx), (sy, cy), (sz, cz)) = (sc(e[0]), sc(e[1]), sc(e[2]));
    let (a, b, c, d) = (sz * sy, cz * cy, sz * cy, cz * sy);
    [
        Vec3::new(fma(a, sx, b), sz * cx, fms(c, sx, d)),
        Vec3::new(fms(d, sx, c), cz * cx, fma(b, sx, a)),
        Vec3::new(sy * cx, -sx, cy * cx),
    ]
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
    /// the director (every camera of the player goes through it)
    pub blend: Director,
    pub mask: CutMask,
    last: Option<(u8, bool)>,
    /// the look-back camera's turn: yaw, pitch (+452, +456)
    pub pan: (f32, f32),
    /// CAM_PAN_LEFT/RIGHT (-1 / +1) and CAM_PAN_UP/DOWN (+1 / -1) held, from the pad
    pub pan_in: (f32, f32),
    seed: u32,
    /// the look back went off this frame (F-39925: the look-back slow motion is released)
    pub look_off: bool,
    /// the view the director shows when it is not the player's chosen one (a change pushed
    /// while blending only queues the new view, or turns the blend round)
    shown: Option<u8>,
}

impl Switch {
    /// Forget the last view (the next frame starts without a blend or a mask).
    pub fn reset(&mut self) {
        self.last = None;
        self.blend.stop();
        self.shown = None;
        self.pan = (0.0, 0.0);
    }

    /// The view whose camera runs (the blend's target, or the director's camera after it), for
    /// the chosen view `view`.
    pub fn shown(&self, view: u8) -> u8 {
        self.shown.unwrap_or(view)
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
        // (a blend that ended with a camera queued: the director cut to it - F-70013)
        if let Some(q) = self.blend.finished.take() {
            self.shown = Some(q);
            cam.reset();
        }
        self.blend.view = self.shown(view);
        let Some((was_view, was_back)) = self.last.replace((view, back)) else {
            self.shown = None;
            self.blend.view = view;
            return;
        };
        let chase = VIEW_OF_PORT[(view as usize).min(5)] >= 2;
        self.look_off = was_back && !back;
        if back != was_back {
            // (both ways a cut with the mask's quick fade: 0.01, 0.1, 0.01)
            self.blend.stop();
            self.shown = None;
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
                self.shown = None;
                self.mask.start(0.01, 0.25, 0.01);
                cam.reset();
            } else if self.blend.active() {
                // setView (F-41662) resets the new view's camera (vt+36) - the running target's or
                // source's when it is one of them -, then pushes it (F-98424): the director is on
                // the blend camera, so the push queues it or turns the blend round (F-43758)
                let shown = self.shown(was_view);
                if view == shown {
                    cam.reset();
                } else if let Some(c) = self.blend.source_cam(view) {
                    c.reset();
                }
                let (dst, _) = tune_for(set, shown, false);
                let (new, _) = tune_for(set, view, false);
                self.blend.push_view(view, new.blend_time(), dst, cam);
                self.shown = Some(self.blend.view_in_blend().unwrap_or(shown));
            } else if self.shown(was_view) == view {
                // (the director's own camera pushed again - the old view stayed on after a drop or a
                // turn-round: F-98424 makes it current with no blend, type 4)
                self.blend.stop();
                self.shown = None;
                cam.reset();
            } else {
                let from = self.shown(was_view);
                let (old, _) = tune_for(set, from, false);
                let (new, _) = tune_for(set, view, false);
                self.blend.begin_views(from, view, old.clone(), cam.clone(), new.blend_time());
                self.shown = None;
                cam.reset();
            }
        }
        // (the director shows the chosen view again: nothing to tell apart - after the change test,
        // which needs the shown view of the frame before)
        if self.shown == Some(view) && !self.blend.active() {
            self.shown = None;
        }
        self.blend.view = self.shown(view);
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

/// 23 bits of the game's multiply-with-carry generator (state lo, carry), as a number below 1.
fn rand23(rng: &mut (u32, u32)) -> f32 {
    let x = rng.0 as u64 * 0x5CDC_FAA7 + rng.1 as u64;
    *rng = (x as u32, (x >> 32) as u32);
    (rng.0 & 0x7F_FFFF) as f32 * (1.0 / 8_388_608.0)
}

/// One part (CarRot, Rot or Tran) of a shake block, F-17992: the phase advances by frequency x dt, then per axis
/// (amplitude sin(phase) + (-random + 2 random u)) m with u the next number of the game's generator (x, then y, then z)
/// and sin the game's own sine (the phase and the sine are floats). `part` = [amplitude, frequency, random, phase].
fn oscillate(part: &mut [Vec3; 4], dt: f32, m: f32, rng: &mut (u32, u32)) -> Vec3 {
    let [amp, freq, rnd, ph] = *part;
    let ph = ph + freq * dt;
    part[3] = ph;
    let u = Vec3::new(rand23(rng), rand23(rng), rand23(rng));
    let sin = |x: f32| crate::physics::exact::guest_sin(x as f64) as f32;
    (amp * Vec3::new(sin(ph.x), sin(ph.y), sin(ph.z)) + (-rnd + rnd * 2.0 * u)) * m
}

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
                out[k] = oscillate(&mut self.blocks[i].parts[k], dt, m, &mut self.rng);
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
            let [s0, s1, s2] = euler_xyz(rt);
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

/// The rows of the rotation from the angles x, y, z (F-28753).
pub fn euler_xyz(r: Vec3) -> [Vec3; 3] {
    crate::physics::artic_build::euler_rows(r.to_array()).map(Vec3::from_array)
}

/// The original's dead zone with rescale (F-93983).
pub(crate) fn dead_zone(v: f32, d: f32) -> f32 {
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
        Some(Self::from_text(&text, event))
    }

    /// One event of a `<car>_cinematic.camtrackcs` text (its base block and the event's own block).
    pub fn from_text(text: &str, event: &str) -> TrackTune {
        // The 18 event records are always there (F-87722 builds them; F-91063, the record's constructor,
        // gives each its values); F-68099 registers the 16 keys of an event block, so a key or a whole block
        // the file leaves out keeps the constructor's value: BlendTime 0.7, ReturnTime 0, TrackTo (0, 1, 0.5),
        // Offset (0, 0, 2), ApproachOn 1, AppXRot 10, RevAppXZPos 10, Min/MaxAppXZPos 4 / 9, Min/MaxAppYPos
        // 10 / 10, Min/MaxFOV 40 / 60, DistScale 0.8, DistMin / DistMax 2 / 5 (dumped from the original's code
        // run offline, docs/reference/cameras/verify_cameras_offline.md). On the disc 8 cars' files have no
        // Damageout block and the police Caprice's none for TwoWheelSide / TwoWheelUp: those run on these values.
        let (b, e) = (track_block(text, None), track_block(text, Some(event)));
        let f = |m: &HashMap<String, Vec<f32>>, k: &str, d: f32| m.get(k).and_then(|v| v.first()).copied().unwrap_or(d);
        let v = |m: &HashMap<String, Vec<f32>>, k: &str, d: Vec3| m.get(k).filter(|v| v.len() >= 3).map_or(d, |v| Vec3::new(v[0], v[1], v[2]));
        let x_rot = f(&e, "AppXRot", 10.0);
        TrackTune {
            blend: f(&e, "BlendTime", 0.7),
            ret: f(&e, "ReturnTime", 0.0),
            track: v(&e, "TrackTo", Vec3::new(0.0, 1.0, 0.5)),
            offset: v(&e, "Offset", Vec3::new(0.0, 0.0, 2.0)),
            dist: (f(&e, "DistMin", 2.0), f(&e, "DistMax", 5.0)),
            dist_scale: f(&e, "DistScale", 0.8),
            fov: (f(&e, "MinFOV", 40.0), f(&e, "MaxFOV", 60.0)),
            app_rot: f(&b, "AppRot", 60.0),
            app_x_rot: if x_rot != 0.0 { x_rot } else { f(&b, "AppRot", 60.0) },
            app_app: f(&b, "AppApp", 0.7),
            app_rot_min: f(&b, "AppRotMin", 0.01),
            look_at: f(&b, "LookAt", 1.0),
            ss_speed: (f(&b, "SSSpeedMin", 15.0), f(&b, "SSSpeedMax", 55.0)),
            ss_filter: f(&b, "SSFilter", 0.1),
            app_xz: (f(&e, "MinAppXZPos", 4.0), f(&e, "MaxAppXZPos", 9.0)),
            app_y: (f(&e, "MinAppYPos", 10.0), f(&e, "MaxAppYPos", 10.0)),
            rev_app_xz: f(&e, "RevAppXZPos", 10.0),
            app_speed: if event == "Hydraulics" { (2.0, 10.0) } else { (10.0, 25.0) },
            app_inc: f(&b, "AppInc", 3.0),
            app_dec: f(&b, "AppDec", 8.0),
            app_pos_min: f(&b, "AppPosMin", 0.25),
            // (a base key the file leaves out keeps camTrackCS's constructor value, F-13135 run
            // offline; every shipped file sets them all; the near plane is the load's 0.2, F-89557)
            base_app_xz: f(&b, "MaxAppXZPos", 9.5),
            hill: (f(&b, "HillMin", -0.8), f(&b, "HillMax", 1.25), f(&b, "HillLerp", 0.15)),
            approach_on: f(&e, "ApproachOn", 1.0) != 0.0,
            car_lens: true,
            collide_type: f(&b, "CollideType", 1.0) as i32,
            collide_min: f(&b, "CollideMinDist", 2.0),
            near: { let n = f(&b, "CameraNear", 0.2); if n > 0.0 { n } else { 0.2 } },
        }
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
/// The blends are the director's blend camera (`DirBlend`, type 3, goal 1). F-38892 pushes only while
/// the director is not on its blend camera, and hands back only once the ability camera is current
/// (the view's camera is the blend's source while it blends in: F-38782 nonzero), so a stop while
/// blending in lets the blend in finish and then goes back over the whole OutTime.
#[derive(Default)]
pub struct AbilityCam {
    cam: TrackCam,
}

impl AbilityCam {
    /// The ability camera is current or the target of a blend (it holds the camera buttons and the
    /// event chooser).
    pub fn active(&self, dir: &Director) -> bool {
        dir.current_or_target(CamId::Ability)
    }
    /// One frame, before the director's step. `push`: the ability runs and the pushing gates hold;
    /// `running`: it runs at all; (in, out): InTime / OutTime. Returns the ability camera's output
    /// when the director runs it, and whether it was pushed this frame.
    pub fn frame(&mut self, dir: &mut Director, push: bool, running: bool, times: (f32, f32), agro: bool, car: &TrackCar, dt: f32, test: SphereTest, aspect: f32) -> (Option<BoomOut>, bool) {
        let mut pushed = false;
        // (F-98424 type 3: a time above 0 begins a blend, else a cut)
        if push && !dir.involves(CamId::Ability) {
            let t = TrackTune::ability(agro);
            self.cam.set(&t, car, 99, 0.0);
            dir.push(CamId::Ability, times.0, 1.0);
            pushed = true;
        } else if !running && dir.is_current(CamId::Ability) {
            dir.push(CamId::View(dir.view), times.1, 1.0);
        }
        if !dir.involves(CamId::Ability) {
            return (None, pushed);
        }
        let t = TrackTune::ability(agro);
        (Some(self.cam.update(&t, car, dt, dt, test, aspect)), pushed)
    }
}

/// The wipeout camera (mcPlayerCamera +72, vehTrackCS; crash_camera_rule.md 1 and 2): taken by
/// F-37740 while the car is wiped out, damaged out or held by the police, and left with a
/// cut when that is over.
#[derive(Default)]
pub struct CrashCam {
    cam: TrackCam,
    tune: Option<TrackTune>,
    /// set up by F-37740 for this wipeout (until the drive view comes back)
    started: bool,
    /// the place's XZ rate has its load value (5, vt+16) until the first wipeout of a car
    used: bool,
    /// F-37740 pointed the camera's matrix (+144) at the rider's copy (racer +704): a bike that
    /// is not exploding nor in water (F-12574), with a rider; else the body's matrix (chassis +304)
    rider: bool,
    /// the copy at racer +704, refreshed by F-17180 at the start and by F-32386 every frame while
    /// the wipeout timer runs (the ragdoll's matrix, else the character's, F-31677)
    rider_mat: Option<(Mat3, Vec3)>,
}

/// Where F-37740 puts the wipeout camera and how it comes in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrashStart {
    /// the Offset (+448): (0, 1, 7) behind, or (+-6, 0, 0) to the side the car slides toward
    pub offset: Vec3,
    /// the director's blend (type 3), 0 = a cut (with the transition mask)
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
    /// Set up for this wipeout and run by the director (current, or a blend's source or target).
    pub fn active(&self, dir: &Director) -> bool {
        self.started && dir.involves(CamId::Wipeout)
    }
    /// Set up for this wipeout (it may still wait in the director's queue).
    pub fn started(&self) -> bool {
        self.started
    }
    fn with_rider(&self, car: &TrackCar) -> TrackCar {
        match (self.rider, self.rider_mat) {
            (true, Some(m)) => TrackCar { follow: Some(m), ..*car },
            _ => *car,
        }
    }
    /// The drive view comes back (a cut, F-53309), or the Damageout view has taken over.
    pub fn stop(&mut self) {
        self.started = false;
    }
    /// A new car: the place's XZ rate starts again at the load's 5.
    pub fn new_car(&mut self) {
        self.used = false;
    }
    /// Whether the camera follows the rider's matrix (F-37740's bike branch).
    pub fn follows_rider(&self, dir: &Director) -> bool {
        self.active(dir) && self.rider
    }
    /// The rider's matrix copy (racer +704) refreshed (F-17180).
    pub fn set_rider(&mut self, m: (Mat3, Vec3)) {
        self.rider_mat = Some(m);
    }
    /// `rider`: F-37740's bike branch holds and F-17180 has a matrix (refreshed into racer +704
    /// now); None: the body's matrix. The caller pushes the camera to the director (type 3 over
    /// `how.blend`).
    pub fn start(&mut self, model_offset: Vec3, car: &TrackCar, how: CrashStart, rider: Option<(Mat3, Vec3)>) {
        self.rider = rider.is_some();
        if rider.is_some() {
            self.rider_mat = rider;
        }
        let car = &self.with_rider(car);
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
        self.started = true;
    }
    /// One frame, before the director's step: `dt` the unscaled frame time (+184 = 1), `dt_game`
    /// the game's. The wipeout camera's output when the director runs it.
    pub fn frame(&mut self, dir: &Director, car: &TrackCar, dt: f32, dt_game: f32, test: SphereTest, aspect: f32) -> Option<BoomOut> {
        if !self.active(dir) {
            return None;
        }
        let t = self.tune.clone()?;
        Some(self.cam.update(&t, &self.with_rider(car), dt, dt_game, test, aspect))
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
    /// the matrix the camera follows (+144) when it is not the car's own (rows as columns, then the
    /// position): the thrown rider's copy, racer +704 (F-17180), for the wipeout camera of a bike
    pub follow: Option<(Mat3, Vec3)>,
    /// a motorcycle with chassis +592 & 0x14 (wiped out / damaged out): F-10185 sets the place
    /// behind unit_xz(M.Z - the body's velocity)
    pub bike_wiped: bool,
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

    /// The point it tracks (F-10185): TrackTo through the followed matrix (the car's, or the
    /// thrown rider's), its height capped at the tune's ModelOffset.y and the rest added along the
    /// world's up, + VertOffset (`vert`, 0 after every event reset).
    fn track_point(&self, car: &TrackCar, vert: f32) -> Vec3 {
        let (m, origin) = car.follow.unwrap_or_else(|| (Mat3::from_quat(car.rot), car.pos));
        let ty = if car.model_offset_y > self.track.y { self.track.y } else { car.model_offset_y };
        let mut at = origin + m * Vec3::new(self.track.x, ty, self.track.z);
        at.y += self.track.y - ty + vert;
        at
    }

    /// Where the camera wants to stand (F-10185): the Offset in the car's level heading
    /// (behind the motion after half a second in the air above 10 m/s), its y along the world's up.
    fn place(&self, car: &TrackCar, at: Vec3) -> Vec3 {
        let m = car.follow.map_or_else(|| Mat3::from_quat(car.rot), |f| f.0);
        // (F-10185: a wiped-out bike first - M.Z less the body's velocity, laid flat; then the
        // air rule; else M.Z)
        let back = if car.bike_wiped {
            let d = m.z_axis - car.vel;
            Vec3::new(d.x, 0.0, d.z).normalize_or(m.z_axis)
        } else if car.air_time > 0.5 && car.speed > 10.0 && car.vel.length_squared() > 1e-4 {
            -car.vel.normalize()
        } else {
            m.z_axis
        };
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
        let m = car.follow.map_or_else(|| Mat3::from_quat(car.rot), |f| f.0);
        let at = self.track_point(car, 0.0);
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
    /// mcCarAirRide (chassis +672): fitted (+12) and on (+13); `air_held`: any of MUSIC_DOWN,
    /// MUSIC_UP, MUSIC_PAUSE, MAP, CAM_UP, CAM_DOWN, LOOKBACK held or CAMERA_LEFT / RIGHT / UP
    /// above 0.2 (the AirRide end test's buttons)
    pub air_on: bool,
    pub air_held: bool,
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

/// The AirRide end test's buttons (F-24075): MUSIC_DOWN, MUSIC_UP, MUSIC_PAUSE, MAP, CAM_UP,
/// CAM_DOWN, LOOKBACK held, or CAMERA_LEFT / RIGHT / UP above 0.2 - the Idle view's set
/// without HYDRAULICS.
pub fn air_ride_buttons(p: &crate::mc4::PadState, slots: &crate::mc4::ControlSlots) -> bool {
    ["MUSIC_DOWN", "MUSIC_UP", "MUSIC_PAUSE", "MAP", "CAM_UP", "CAM_DOWN", "LOOKBACK"].iter().any(|a| crate::mc4::pad_down(p, slots, a))
        || ["CAMERA_LEFT", "CAMERA_RIGHT", "CAMERA_UP"].iter().any(|a| crate::mc4::pad_analog(p, slots, a) > 0.2)
}

/// A segment probe of the world (F-20855, one result): something lies between the points.
pub type RayTest<'a> = &'a dyn Fn(Vec3, Vec3) -> bool;

/// The event camera (camTrackCS at mgr+84) as mcPlayerCamera's chooser runs it: each frame the
/// first event whose start holds is set and pushed to the director over the record's BlendTime
/// (F-94591, type 3: the director's blend camera, `DirBlend`); its end test pushes the drive
/// view back over the record's ReturnTime (0: its BlendTime) or a time the test passes.
#[derive(Default)]
pub struct EventDirector {
    cam: TrackCam,
    /// the event set (cam +784 holds it): it was pushed to the director and not turned off since
    on: bool,
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
    /// (tests: the director the wrappers below run it on)
    #[cfg(test)]
    tdir: Director,
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
pub const EV_AIR_RIDE: usize = 12;
pub const EV_DAMAGEOUT: usize = 17;

impl EventDirector {
    /// The event is set (F-38782 nonzero while it is the director's camera or the blend's target):
    /// the camera buttons act on it.
    pub fn running(&self) -> bool {
        self.on
    }
    /// The event running (cam +784), if any.
    pub fn event(&self) -> Option<usize> {
        self.running().then_some(self.ev)
    }
    /// The event view is wholly in (the director's current camera, no blend).
    pub fn shown(&self, dir: &Director) -> bool {
        self.on && dir.is_current(CamId::Event)
    }
    /// Turned off, the director still blending from (or through) the event camera.
    pub fn leaving(&self, dir: &Director) -> bool {
        !self.on && dir.involves(CamId::Event)
    }
    /// F-94591(0, -1, -1): the running event goes back over the record's time.
    pub fn end(&mut self, dir: &mut Director, data_dir: &str, car_name: &str) {
        if self.running() {
            if let Some(t) = self.tune(data_dir, car_name, self.ev) {
                self.off(dir, &t, -1.0);
            }
        }
    }
    /// F-94591(0, -1, b): the running (or leaving) event goes back over `b`; the event is cleared
    /// either way (the gas station's close asks 0.7, F-81587).
    pub fn off_with(&mut self, dir: &mut Director, data_dir: &str, car_name: &str, b: f32) {
        if self.running() || self.leaving(dir) {
            if let Some(t) = self.tune(data_dir, car_name, self.ev) {
                self.off(dir, &t, b);
            }
        }
    }
    /// CAM_UP leaves it (F-67519: F-94591(0, -1, 0.2)), before the view's own push.
    pub fn leave(&mut self, dir: &mut Director, data_dir: &str, car_name: &str) {
        if self.running() {
            if let Some(t) = self.tune(data_dir, car_name, self.ev) {
                self.off(dir, &t, 0.2);
            }
        }
    }
    /// The director cut away from the event camera (setView's type-4 cut F-41662 -> F-53309 over
    /// the blend camera): the event view is gone at once.
    pub fn cut_out(&mut self) {
        self.cam.look_yaw = 0.0;
        self.on = false;
        self.quit = false;
    }
    pub fn reset(&mut self) {
        self.on = false;
        self.quit = false;
    }
    /// The Idle view is on (the HUD is put away while it is).
    pub fn idle(&self, dir: &Director) -> bool {
        (self.on || dir.involves(CamId::Event)) && self.ev == EV_IDLE
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
        if i.air_on {
            return Some(EV_AIR_RIDE);
        }
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
            // (the buttons' second test is in `frame`: it also turns the event's byte off)
            EV_AIR_RIDE => end(!i.air_on, -1.0),
            // (the pad's buttons zero the idle time - main.rs)
            EV_IDLE => end(i.idle == 0.0, 0.7),
            EV_DAMAGEOUT => end(!i.destroyed, -1.0),
            _ => None,
        }
    }

    /// Turns the event off (F-94591(0, -1, b)): back to the drive view over b, else the
    /// record's ReturnTime, else its BlendTime.
    /// The push of the drive view's camera is type 4 (a cut) at or below 0.001 s, else 3. While
    /// the director shows the blend camera F-29802 first turns it round (goal = the old
    /// source's BlendGoal, left = the director's time x weight, weight = 1 - weight, source and
    /// target swapped); the push then (F-98424 -> F-43758, a blend type only) stores its time as
    /// the director's and queues the drive view's camera (the finish cuts to it unless it is the
    /// target). Without a blend a cut ends the event view at once, a blend starts one from the
    /// event camera (F-18878, F-41709).
    fn off(&mut self, dir: &mut Director, t: &TrackTune, b: f32) {
        let back = if b >= 0.0 { b } else if t.ret > 0.0 { t.ret } else { t.blend };
        let blend = back > f32::from_bits(0x3a83_126f);
        // (the event's exit zeroes the look object, F-94591)
        self.cam.look_yaw = 0.0;
        self.quit = false;
        self.on = false;
        let view = CamId::View(dir.view);
        // (every camera's BlendGoal is 1: the constructors' +164 and all 719 shipped keys)
        if dir.active() {
            dir.turn_round(1.0);
            if blend {
                dir.push(view, back, 1.0);
            }
        } else if blend {
            dir.push(view, back, 1.0);
        } else {
            dir.cut(view);
        }
    }

    /// The event camera pushed over `time` (F-98424 type 3 from the director's camera; the begin
    /// F-18878 and start F-41709): weight 0, goal 1, the director's time - or queued behind a
    /// blend the director runs (F-43758).
    fn begin_in(&mut self, dir: &mut Director, time: f32) {
        self.on = true;
        dir.push(CamId::Event, time, 1.0);
    }

    /// One frame (after the drive view's own), before the director's step: the event's start and
    /// end tests and its pushes; the event camera's output when the director runs it. `test` the
    /// world's ball test, `ray` its segment probe.
    pub fn frame(&mut self, dir: &mut Director, data_dir: &str, car_name: &str, i: &EventIn, car: &TrackCar, dt: f32, test: SphereTest, ray: RayTest, aspect: f32) -> Option<BoomOut> {
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
        let out_of_director = |s: &Self, dir: &Director| !s.on && !dir.involves(CamId::Event);
        // part 1: while the wipeout camera is in use the only start is Damageout, whatever the
        // option and the view, and nothing ends
        if i.crash {
            if i.damage_out && out_of_director(self, dir) {
                if let Some(t) = self.tune(data_dir, car_name, EV_DAMAGEOUT).filter(|t| t.blend > 0.0) {
                    self.off_bytes = [false; 18];
                    self.ev = EV_DAMAGEOUT;
                    self.cam.set(&t, car, EV_DAMAGEOUT, 0.0);
                    self.begin_in(dir, t.blend);
                }
            }
            let t = self.tune(data_dir, car_name, self.ev)?;
            return self.show(dir, &t, car, dt, test, aspect);
        }
        // part 2: an event starts only while the camera is wholly out of the director
        if out_of_director(self, dir) {
            self.quit = false;
            if i.allowed && i.view >= 2 {
                if let Some(ev) = self.pick(data_dir, car_name, fx, i, car, test, ray).filter(|&e| !self.off_bytes[e]) {
                    if let Some(t) = self.tune(data_dir, car_name, ev).filter(|t| t.blend > 0.0) {
                        // (a successful on re-arms every event, vt+36)
                        self.off_bytes = [false; 18];
                        let side = self.side(ev, i, car);
                        self.ev = ev;
                        self.cam.set(&t, car, ev, side);
                        self.begin_in(dir, t.blend);
                    }
                }
            }
        }
        let mut out = None;
        if !out_of_director(self, dir) {
            let Some(t) = self.tune(data_dir, car_name, self.ev) else {
                self.on = false;
                return None;
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
                    self.off(dir, &t, b);
                }
                // Nitrous: the event camera pulled in from a wall ends it (0.2 s; 0 while still
                // blending in: a cut push, so only F-29802's turn-round) and keeps it off until
                // the shot ends
                if self.ev == EV_NITROUS && self.cam.corrected {
                    if self.running() {
                        let b = if dir.active() && dir.dst == CamId::Event { 0.0 } else { 0.2 };
                        self.off(dir, &t, b);
                    }
                    self.off_bytes[EV_NITROUS] = true;
                }
                // AirRide: whether or not the air ride went off, a held camera / music button
                // ends it and keeps it off until the air ride is switched off (F-24075)
                if self.ev == EV_AIR_RIDE && i.air_held {
                    if self.running() {
                        self.off(dir, &t, -1.0);
                    }
                    self.off_bytes[EV_AIR_RIDE] = true;
                }
            }
            out = self.show(dir, &t, car, dt, test, aspect);
        }
        // part 5: the AirRide byte comes back when the air ride is off, the Nitrous byte when the
        // shot ends
        if !i.air_on {
            self.off_bytes[EV_AIR_RIDE] = false;
        }
        if !i.nitro_on {
            self.off_bytes[EV_NITROUS] = false;
        }
        out
    }

    /// The event camera's update when the director runs it (current, or a blend's source or target).
    fn show(&mut self, dir: &Director, t: &TrackTune, car: &TrackCar, dt: f32, test: SphereTest, aspect: f32) -> Option<BoomOut> {
        dir.involves(CamId::Event).then(|| self.cam.update(t, car, dt, dt, test, aspect))
    }
}

#[cfg(test)]
pub(crate) fn target_at(pos: Vec3) -> BoomTarget {
    BoomTarget { pos, rot: Quat::IDENTITY, vel: Vec3::ZERO, ang_vel: Vec3::ZERO, grounded: 4, gear: 1, rev: false, nitro: false, nitro_fov: (0.0, 0.5, 1.0), ability: (0.0, 1.0), sst: (0.0, 1.0), pan: (0.0, 0.0), rise: 0.0, fixed_mount: false, model_offset: Vec3::ZERO }
}

#[cfg(test)]
impl EventDirector {
    /// One frame on its own director, `chase` the drive view's output, then the director's step.
    fn frame_t(&mut self, data_dir: &str, car_name: &str, i: &EventIn, car: &TrackCar, chase: BoomOut, dt: f32, test: SphereTest, ray: RayTest, aspect: f32) -> BoomOut {
        let mut dir = std::mem::take(&mut self.tdir);
        let o = self.frame(&mut dir, data_dir, car_name, i, car, dt, test, ray, aspect);
        let others: Vec<(CamId, BoomOut)> = o.map(|o| (CamId::Event, o)).into_iter().collect();
        let out = dir.apply(&target_at(car.pos), chase, &others, dt);
        self.tdir = dir;
        out
    }
    fn shown_t(&self) -> bool {
        self.shown(&self.tdir)
    }
    fn leaving_t(&self) -> bool {
        self.leaving(&self.tdir)
    }
    fn idle_t(&self) -> bool {
        self.idle(&self.tdir)
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
    /// camTrackCS's constructor F-13135 run offline (work/cameras2/d34/basector.py)
    /// at the base keys the port reads: a file whose base block sets none of them gives these words.
    #[test]
    fn track_base_defaults_match_the_original() {
        let t = super::TrackTune::from_text("type: a\ncamTrackCS {\n}\n", "SSTurbo");
        let got = [t.app_rot, t.app_app, t.app_rot_min, t.look_at, t.app_inc, t.app_dec, t.app_pos_min, t.base_app_xz, t.hill.0, t.hill.1, t.hill.2, t.ss_speed.0, t.ss_speed.1, t.ss_filter];
        let want = [0x42700000u32, 0x3f333333, 0x3c23d70a, 0x3f800000, 0x40400000, 0x41000000, 0x3e800000, 0x41180000, 0xbf4ccccd, 0x3fa00000, 0x3e19999a, 0x41700000, 0x425c0000, 0x3dcccccd];
        assert_eq!(got.map(f32::to_bits), want);
        assert_eq!(t.collide_type, 1);
    }

    /// camDynamicBoomTune's constructor F-13319 and its loader's field table F-77242 run offline
    /// (work/cameras2/d34/boomtune.py): per registered key (82) its type and constructor words. A
    /// file that sets nothing gives the port the same value of every key.
    #[test]
    fn boom_tune_defaults_match_the_original() {
        let t = super::BoomTune::parse("camBoomCS\n").unwrap();
        let mut n = 0;
        for line in include_str!("testdata/boom_tune_defaults_ref.txt").lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let (key, ty) = (w[0], w[1]);
            for (i, h) in w[2..].iter().enumerate() {
                let bits = u32::from_str_radix(h, 16).unwrap();
                let want = if ty == "5" { bits as i32 as f32 } else { f32::from_bits(bits) };
                assert_eq!(t.range(key, i).to_bits(), want.to_bits(), "{key}[{i}]");
            }
            n += 1;
        }
        assert_eq!(n, 82);
    }

    /// F-28753 run offline on 4,128 angle triples (work/cameras2/euler/euler_ref.py; the bits as decimal words): every stored word equal.
    #[test]
    fn euler_xyz_matches_the_original() {
        let text = include_str!("testdata/euler_xyz_ref.txt");
        let h = |s: &str| s.parse::<u32>().unwrap();
        let mut n = 0;
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let r = euler_xyz(Vec3::new(f32::from_bits(h(w[0])), f32::from_bits(h(w[1])), f32::from_bits(h(w[2]))));
            let got: Vec<u32> = r.iter().flat_map(|v| [v.x, v.y, v.z]).map(f32::to_bits).collect();
            let want: Vec<u32> = w[4..13].iter().map(|s| h(s)).collect();
            assert_eq!(got, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 4128);
    }

    use super::*;

    /// The event record's values where a file leaves a key or the whole block out: the original's record
    /// constructor F-91063 and the key table of F-68099, both run offline (work/cameras2/close/ev_fields.py;
    /// words as decimal). A block-less event and an empty block give the constructor's words bit for bit.
    #[test]
    fn missing_event_keys_take_the_record_constructor() {
        let text = "camTrackCS {
  AppRot 60
  Tuck {
  }
}
";
        for ev in ["Damageout", "Tuck"] {
            let t = TrackTune::from_text(text, ev);
            let mut n = 0;
            for line in include_str!("testdata/cam_event_record_ref.txt").lines().filter(|l| !l.starts_with('#')) {
                let w: Vec<&str> = line.split_whitespace().collect();
                let want: Vec<u32> = w[3..].iter().map(|s| s.parse().unwrap()).collect();
                let got: Vec<u32> = match w[0] {
                    "BlendTime" => vec![t.blend.to_bits()],
                    "ReturnTime" => vec![t.ret.to_bits()],
                    "TrackTo" => [t.track.x, t.track.y, t.track.z].map(f32::to_bits).to_vec(),
                    "Offset" => [t.offset.x, t.offset.y, t.offset.z].map(f32::to_bits).to_vec(),
                    "ApproachOn" => vec![t.approach_on as u32],
                    "DistMin" => vec![t.dist.0.to_bits()],
                    "DistMax" => vec![t.dist.1.to_bits()],
                    "AppXRot" => vec![t.app_x_rot.to_bits()],
                    "RevAppXZPos" => vec![t.rev_app_xz.to_bits()],
                    "MinAppXZPos" => vec![t.app_xz.0.to_bits()],
                    "MaxAppXZPos" => vec![t.app_xz.1.to_bits()],
                    "MinAppYPos" => vec![t.app_y.0.to_bits()],
                    "MaxAppYPos" => vec![t.app_y.1.to_bits()],
                    "MinFOV" => vec![t.fov.0.to_bits()],
                    "MaxFOV" => vec![t.fov.1.to_bits()],
                    "DistScale" => vec![t.dist_scale.to_bits()],
                    k => panic!("key {k} not mapped"),
                };
                assert_eq!(got, want, "{ev} {line}");
                n += 1;
            }
            assert_eq!(n, 16);
        }
    }

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
        let car0 = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 30.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (10.0, 1.2), nitro_view: 5.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO, follow: None, bike_wiped: false };
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
        let mut car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO, follow: None, bike_wiped: false };
        let o = d.frame_t(&data, "vp_aud_r8_08", &ein(1.0, 1.0, 2.0, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
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
            let o = d.frame_t(&data, "vp_aud_r8_08", &ein(1.0, 1.0, 2.0, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
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
        d.frame_t(&data, "vp_aud_r8_08", &ein(1.0, 1.0, 0.7, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
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
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::new(0.0, 0.0, -15.0), speed: 15.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO, follow: None, bike_wiped: false };
        let dt = 1.0 / 60.0;
        let run = |i: EventIn, ray: bool| {
            let mut d = EventDirector::default();
            d.frame_t(&data, "vp_aud_r8_08", &i, &car, BoomOut { ..chase }, dt, &NO_WORLD, &|_, _| ray, 16.0 / 9.0);
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
        d.frame_t(&data, "vp_aud_r8_08", &w, &car, BoomOut { ..chase }, dt, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        assert_eq!(d.event(), Some(EV_WHEELIE));
        d.frame_t(&data, "vp_aud_r8_08", &EventIn { nitro_on: true, ..w }, &car, BoomOut { ..chase }, dt, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        assert_eq!(d.event(), None);
    }

    /// The ability camera on the director (F-38892): a stop while it blends in lets the blend in
    /// finish (the view's camera is the blend's source, so no hand-back push), then hands back over
    /// the whole OutTime.
    #[test]
    fn ability_hands_back_after_its_blend_in() {
        let chase = BoomOut { pos: Vec3::new(0.0, 2.0, 9.0), look: Vec3::ZERO, up: Vec3::Y, fov: 40.0 };
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO, follow: None, bike_wiped: false };
        let dt = 0.1;
        let (mut a, mut dir) = (AbilityCam::default(), Director::default());
        let f = |a: &mut AbilityCam, dir: &mut Director, push: bool, running: bool| {
            let (o, pushed) = a.frame(dir, push, running, (0.5, 0.3), false, &car, dt, &NO_WORLD, 16.0 / 9.0);
            let others: Vec<(CamId, BoomOut)> = o.map(|o| (CamId::Ability, o)).into_iter().collect();
            dir.apply(&target_at(car.pos), chase, &others, dt);
            pushed
        };
        assert!(f(&mut a, &mut dir, true, true));
        assert!(dir.active() && dir.dst == CamId::Ability && dir.time == 0.5);
        // stopped at once: still blending in for the rest of the 0.5 s
        let mut n = 0;
        while dir.active() {
            f(&mut a, &mut dir, false, false);
            n += 1;
        }
        assert!((4..=5).contains(&n) && dir.is_current(CamId::Ability), "{n}");
        f(&mut a, &mut dir, false, false);
        assert!(dir.active() && dir.src == CamId::Ability && dir.time == 0.3);
    }

    /// The event's off on the director (F-94591, F-29802, F-98424 -> F-43758): while blending in a
    /// cut (0 s) only turns the blend round (left = the director's time x weight, weight = 1 - w,
    /// the director's time kept); an off while it blends out turns it round again and queues
    /// the drive view (the finish cuts to it); shown, a blend starts from the event camera and a
    /// time at or below 0.001 s cuts.
    #[test]
    fn event_off_on_the_director() {
        let t = TrackTune::from_text("type: a
camTrackCS {
}
", "SSTurbo");
        let (mut d, mut dir) = (EventDirector::default(), Director::default());
        let ev = CamId::Event;
        let queued_view = |dir: &Director| matches!(dir.queued, Some(CamId::View(_)));
        d.begin_in(&mut dir, 1.0);
        dir.blend.weight = 0.25;
        d.off(&mut dir, &t, 0.0);
        assert!(d.leaving(&dir));
        assert_eq!((dir.src == ev, queued_view(&dir), dir.blend.left, dir.blend.weight, dir.time), (true, false, 0.25, 0.75, 1.0));
        d.off(&mut dir, &t, 0.5);
        assert!(d.leaving(&dir));
        assert_eq!((dir.src == ev, queued_view(&dir), dir.blend.left, dir.blend.weight, dir.time), (false, true, 0.75, 0.25, 0.5));
        let (mut d, mut dir) = (EventDirector { on: true, ..Default::default() }, Director::default());
        dir.cut(ev);
        d.off(&mut dir, &t, 0.4);
        assert!(d.leaving(&dir));
        assert_eq!((dir.src == ev, dir.blend.left, dir.blend.weight, dir.time), (true, 0.4, 0.0, 0.4));
        let (mut d, mut dir) = (EventDirector { on: true, ..Default::default() }, Director::default());
        dir.cut(ev);
        d.off(&mut dir, &t, 0.001);
        assert!(!d.running() && !dir.involves(ev));
    }

    /// The director's blends recorded in the running original (work/recomp-trace cam_trace, the
    /// R8 100 % save idling in the city; tools/cam_rec_cases.py): the Idle camera on (F-94591 ->
    /// F-98424 type 3 over its BlendTime 1.5), off after it is shown, off 1.2 s and 0.2 s into the
    /// blend in (F-29802 turns it round, the push's 0.7 s becomes the director's time), and the bust's
    /// wipeout camera (F-37740, 0.5 s). The port's begin / off / CrashCam::start with the recorded frame
    /// times give the recorded weight and time left at every step, bit for bit, and end where it
    /// ended (on the event camera, or back on the drive view).
    #[test]
    fn event_blends_match_the_recording() {
        let t = TrackTune::from_text("type: a
camTrackCS {
}
", "Idle");
        let (o1, o2) = (BoomOut { pos: Vec3::new(0.0, 2.0, 9.0), look: Vec3::ZERO, up: Vec3::Y, fov: 40.0 }, BoomOut { pos: Vec3::new(3.0, 1.0, 4.0), look: Vec3::ZERO, up: Vec3::Y, fov: 50.0 });
        let (a, b) = (dir_cam_of(&o1, Vec3::ZERO), dir_cam_of(&o2, Vec3::ZERO));
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO, follow: None, bike_wiped: false };
        let h = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let text = include_str!("testdata/event_blend_rec.txt");
        let (mut cases, mut steps) = (0, 0);
        let mut lines = text.lines().filter(|l| !l.starts_with('#')).peekable();
        while let Some(head) = lines.next() {
            let wipe = head.ends_with("wipe");
            let mut d = EventDirector::default();
            let mut c = CrashCam::default();
            let mut dir = Director::default();
            let mut more = true;
            while let Some(l) = lines.next_if(|l| !l.starts_with("case")) {
                let w: Vec<&str> = l.split(' ').collect();
                match w[0] {
                    "push" if wipe => {
                        let how = CrashStart { offset: Vec3::new(0.0, 1.0, 7.0), blend: 0.5, exploding: false };
                        assert_eq!(how.blend, h(w[1]), "{head}");
                        c.start(Vec3::ZERO, &car, how, None);
                        dir.push(CamId::Wipeout, how.blend, 1.0);
                    }
                    "push" => d.begin_in(&mut dir, h(w[1])),
                    "off" => {
                        assert!(more, "{head}: an off after the finish");
                        d.off(&mut dir, &t, h(w[1]));
                    }
                    "s" => {
                        assert!(more, "{head}: a step after the finish");
                        let bl = &mut dir.blend;
                        assert_eq!((bl.weight.to_bits(), bl.left.to_bits()), (h(w[2]).to_bits(), h(w[3]).to_bits()), "{head} {l}");
                        more = bl.step(&a, &b, 3, dir.time, h(w[1]));
                        steps += 1;
                    }
                    "fin" => {
                        assert!(!more, "{head}: no finish");
                        if !wipe {
                            // (the director's finish, F-70013)
                            dir.finish();
                            more = true;
                        }
                    }
                    _ => unreachable!(),
                }
            }
            // events end back on the drive view (every case has its off), the wipeout camera on itself
            assert!(wipe || (!d.running() && dir.is_current(CamId::View(0))), "{head}");
            cases += 1;
        }
        assert_eq!((cases, steps), (5, 181));
    }

    /// A bust while the Idle view is shown, as the running original did it (recording c0, steps 226 / 227 -
    /// docs/reference/cameras/recorded_blends_evidence.md): the wipeout start's off begins the event's
    /// blend out over the record's 1.5 s, the wipeout camera's push (0.5 s) is queued and stores its time,
    /// so the step keeps the weight at 0 (the event camera's own view); the next frame's start finishes the
    /// blend at once and cuts to the queued wipeout camera.
    #[test]
    fn bust_while_the_event_is_shown_matches_the_recording() {
        let mut t = TrackTune::from_text("type: a
camTrackCS {
}
", "Idle");
        (t.blend, t.ret) = (1.5, 0.0);
        let o = BoomOut { pos: Vec3::new(0.0, 2.0, 9.0), look: Vec3::ZERO, up: Vec3::Y, fov: 40.0 };
        let a = dir_cam_of(&o, Vec3::ZERO);
        let h = |x: u32| f32::from_bits(x);
        let (mut d, mut dir) = (EventDirector::default(), Director::default());
        d.ev = EV_IDLE;
        d.on = true;
        dir.cut(CamId::Event);
        dir.time = 1.5;
        // frame 226: the head finds the event camera current (no finish), the off, the queued push
        assert_eq!(dir.finish_now(), None);
        d.off(&mut dir, &t, -1.0);
        assert!(d.leaving(&dir));
        assert_eq!(dir.push(CamId::Wipeout, 0.5, 1.0), Pushed::Queued);
        assert_eq!(dir.queued, Some(CamId::Wipeout));
        // the step of frame 227 at its entry: w 0, left 1.5 (the recorded 0x3FC00000); T 0.5 (0x3F000000)
        assert_eq!((dir.blend.weight.to_bits(), dir.blend.left.to_bits(), dir.time.to_bits()), (0, 0x3FC0_0000, 0x3F00_0000));
        assert!(dir.blend.step(&a, &a, 3, dir.time, h(0x3D0E_8FE9)));
        // the finish's record: w 0, left 0x3FBB8B81
        assert_eq!((dir.blend.weight.to_bits(), dir.blend.left.to_bits()), (0, 0x3FBB_8B81));
        // frame 227's head: the finish cuts to the queued wipeout camera; the event view is gone
        assert_eq!(dir.finish_now(), Some(CamId::Wipeout));
        assert!(!d.running() && !d.leaving(&dir) && dir.queued.is_none() && dir.is_current(CamId::Wipeout));
    }

    /// AirRide (F-24075): it starts while the air ride is on (after Hydraulics, before the two-wheel
    /// views), ends over the record's 0.7 s when it goes off; a held camera / music button ends it
    /// and keeps it off while the air ride stays on; switched off, it may come again.
    #[test]
    fn air_ride_view() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let Some(t) = TrackTune::load(&data, "vp_aud_r8_08", "AirRide") else { return };
        assert_eq!((t.blend, t.ret), (0.7, 0.0));
        let chase = BoomOut { pos: Vec3::new(0.0, 2.0, 9.0), look: Vec3::ZERO, up: Vec3::Y, fov: 40.0 };
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO, follow: None, bike_wiped: false };
        let dt = 1.0 / 60.0;
        let mut d = EventDirector::default();
        let step = |d: &mut EventDirector, i: EventIn| {
            d.frame_t(&data, "vp_aud_r8_08", &i, &car, BoomOut { ..chase }, dt, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        };
        let on = EventIn { air_on: true, ..ein(0.0, 0.0, 0.0, 0.0) };
        // (Hydraulics comes first, two wheels after)
        step(&mut d, EventIn { hyd_on: true, pan_pressed: true, g18: 1, ..on });
        assert_eq!(d.event(), Some(EV_HYDRAULICS));
        let mut d = EventDirector::default();
        step(&mut d, EventIn { g18: 1, ..on });
        assert_eq!(d.event(), Some(EV_AIR_RIDE));
        assert_eq!(d.tdir.time, 0.7);
        // (not in the bumper / cockpit views)
        let mut e = EventDirector::default();
        step(&mut e, EventIn { view: 1, ..on });
        assert_eq!(e.event(), None);
        // the air ride off: back over the BlendTime (ReturnTime 0)
        for _ in 0..60 {
            step(&mut d, on);
        }
        assert!(d.shown_t());
        step(&mut d, EventIn { air_on: false, ..on });
        assert!(d.leaving_t() && d.tdir.time == 0.7);
        // a held button: off, and not again while the air ride stays on
        let mut d = EventDirector::default();
        for _ in 0..60 {
            step(&mut d, on);
        }
        step(&mut d, EventIn { air_held: true, ..on });
        assert!(d.leaving_t());
        for _ in 0..60 {
            step(&mut d, on);
        }
        assert_eq!((d.tdir.involves(CamId::Event), d.event()), (false, None));
        // switched off once, it starts again
        step(&mut d, EventIn { air_on: false, ..on });
        step(&mut d, on);
        assert_eq!(d.event(), Some(EV_AIR_RIDE));
    }

    /// The crash cameras (crash_camera_rule.md): F-37740's choice of start; the wipeout camera
    /// on the R8 snaps 6 m to the side the car slides toward at T = 0.75 x ModelOffset, lens 40,
    /// comes in over 0.25 s and keeps 6..10 m from T while the car slides away; the wipeout warper
    /// is at 0.1 after 0.2 s, held to 1.25 s, back to 1 by 1.35 s.
    /// The wipeout camera of a thrown rider against the original (recording rf.0, ZX-14 wheelie into a wall,
    /// 36 frames from the start to the stand-up): the matrix it follows is the rider's copy (F-37740 / F-17180),
    /// the track point (+320) and the place before the hill (+688) of F-10185 with the bike's rule
    /// back = unit_xz(M.Z - body velocity), per frame from the recorded inputs.
    #[test]
    fn bike_wipeout_camera_follows_the_rider() {
        let h = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        let mut worst = (0.0f32, 0.0f32);
        for line in include_str!("testdata/bike_wipeout_cam_rf0.txt").lines().filter(|l| !l.starts_with('#')) {
            let w: Vec<&str> = line.split_whitespace().collect();
            let v = |i: usize| Vec3::new(h(w[i]), h(w[i + 1]), h(w[i + 2]));
            let m = Mat3::from_cols(v(1), v(4), v(7));
            let (pos, vel, mo_y, track, vert, offset) = (v(10), v(13), h(w[16]), v(17), h(w[20]), v(21));
            let flags = u32::from_str_radix(w[24], 16).unwrap();
            let (t_rec, p_rec) = (v(25), v(28));
            let car = TrackCar {
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                vel,
                speed: 0.0,
                reverse_gear: false,
                steer: 0.0,
                front_z: 0.0,
                slope: None,
                air_time: 0.0,
                aero: 1.0,
                model_offset_y: mo_y,
                sst: (0.0, 1.0),
                nitro_view: 0.0,
                grounded: 0,
                wheels: 2,
                ang_mom: Vec3::ZERO,
                follow: Some((m, pos)),
                bike_wiped: flags & 0x14 != 0,
            };
            let c = TrackCam { track, offset, ..Default::default() };
            let t = c.track_point(&car, vert);
            let p = c.place(&car, t);
            worst = (worst.0.max((t - t_rec).abs().max_element()), worst.1.max((p - p_rec).abs().max_element()));
            n += 1;
        }
        assert_eq!(n, 36);
        // (world coordinates near 800 m: one f32 step is 6e-5 m)
        assert!(worst.0 < 2e-4 && worst.1 < 2e-4, "track point / place off by {worst:?}");
    }

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
        let mut car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: slide, speed: slide.length(), reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 0.3, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO, follow: None, bike_wiped: false };
        let drive = BoomOut { pos: Vec3::new(0.0, 2.0, 6.0), look: Vec3::ZERO, up: Vec3::Y, fov: 60.0 };
        let (mut c, mut dir) = (CrashCam::default(), Director::default());
        let how = CrashStart::choose(3, false, false, 1.0, right, slide);
        c.start(mo, &car, how, None);
        dir.push(CamId::Wipeout, how.blend, 1.0);
        let dt = 1.0 / 60.0;
        let frame = |c: &mut CrashCam, dir: &mut Director, car: &TrackCar| {
            let others: Vec<(CamId, BoomOut)> = c.frame(dir, car, dt, dt, &NO_WORLD, 16.0 / 9.0).map(|o| (CamId::Wipeout, o)).into_iter().collect();
            dir.apply(&target_at(car.pos), drive, &others, dt)
        };
        let o = frame(&mut c, &mut dir, &car);
        assert!(o.pos.distance(drive.pos) < 1.0, "blending in: {:?}", o.pos);
        let mut o = o;
        for _ in 0..15 {
            o = frame(&mut c, &mut dir, &car);
        }
        let t = Vec3::new(0.0, 0.531, 0.0);
        assert!(o.pos.distance(t + Vec3::new(6.0, 0.0, 0.0)) < 1e-3 && o.fov == 40.0, "{:?}", o.pos);
        for _ in 0..120 {
            car.pos += car.vel * dt;
            o = frame(&mut c, &mut dir, &car);
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
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO, follow: None, bike_wiped: false };
        let mut o = BoomOut { ..chase };
        let mut free = BoomOut { ..chase };
        let mut e = EventDirector::default();
        for i in 0..240 {
            o = d.frame_t(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &Balls(wall), &|_, _| false, 16.0 / 9.0);
            free = e.frame_t(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
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
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3, slope: Some(0.0), air_time: 0.0, aero: 1.0, model_offset_y: 0.708, sst: (0.0, 1.0), nitro_view: 0.0, grounded: 4, wheels: 4, ang_mom: Vec3::ZERO, follow: None, bike_wiped: false };
        let o = d.frame_t(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 59.9), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        assert!(!d.idle_t() && o.pos == chase.pos);
        let mut held = Vec::new();
        for i in 0..240 {
            let o = d.frame_t(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
            assert!(d.idle_t());
            if i >= 100 {
                held.push(o);
            }
        }
        let p = held[0].pos;
        assert!((p.x.abs() - 2.525).abs() < 1e-3 && (p.y - 0.895).abs() < 1e-3 && (p.z - (0.55 - 9.0)).abs() < 1e-3, "{p:?}");
        assert!(held.iter().all(|o| o.pos.distance(p) < 1e-4 && (o.fov - 25.0).abs() < 1e-3));
        d.frame_t(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 0.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
        assert!(!d.running());
        // on a hill (the car's back axis 0.2 up the slope: facing downhill) the Idle camera's
        // place, ahead of the car, turns down with the ground by asin(0.2 * 0.8) about the
        // point; the view's AppYPos are 0, so only the level part is followed: it comes in to
        // |rel| cos(hill) and keeps its height
        let car = TrackCar { slope: Some(0.2), ..car };
        let mut d = EventDirector::default();
        let mut o = BoomOut { ..chase };
        for i in 0..600 {
            o = d.frame_t(&data, "vp_aud_r8_08", &ein(0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0), &car, BoomOut { ..chase }, 1.0 / 60.0, &NO_WORLD, &|_, _| false, 16.0 / 9.0);
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
        // (GitHub #17: the slipstream turbo burning and the nitrous change nothing on a fixed mount - F-44852)
        let run = |sst: (f32, f32), nitro: bool| {
            let mut cam = BoomCam::default();
            let (mut z, mut v, mut o) = (0.0f32, 0.0f32, None);
            for _ in 0..360 {
                v = (v + 6.0 / 30.0).min(55.0);
                z -= v / 30.0;
                let car = BoomTarget { pos: Vec3::new(0.0, 0.0, z), rot: Quat::IDENTITY, vel: Vec3::new(0.0, 0.0, -v), ang_vel: Vec3::ZERO, grounded: 4, gear: 3, rev: false, nitro, nitro_fov: (10.0, 0.5, 1.0), ability: (0.0, 1.0), sst, pan: (0.0, 0.0), rise: 0.0, fixed_mount: true, model_offset: Vec3::ZERO };
                let out = cam.update(t, &car, 1.0 / 30.0);
                o = Some((out.pos - car.pos, out.fov));
            }
            o.unwrap()
        };
        let (plain, boosted) = (run((0.0, 1.0), false), run((15.0, 0.8), true));
        assert!((plain.0 - boosted.0).length() < 1e-5 && plain.1 == boosted.1, "{plain:?} {boosted:?}");
    }

    /// The chase camera's update (camDynamicBoomCS F-14014 with its lens F-44852, frame F-90220 and shift kick F-66782)
    /// against the running original: 1,153 updates of the R8's boom2 view recorded from the static recomp
    /// (cam_trace, driving with turns, braking and gear changes; testdata/boom_step_rec.txt, words of
    /// work/cameras2/rec/boom_testdata.py). Each update starts from the recorded camera and is compared with the
    /// recorded result: the frame, the filters, the offsets, the homing, the approach rate, the kick and the lens.
    /// (The recomp's own frame-time fix of the two filter factors is applied, as it was in the recording.)
    #[test]
    fn boom_step_matches_the_original() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let Some(set) = BoomSet::load(&data, "vp_aud_r8_08") else { return };
        let (n, kicks) = boom_rec_check(&set.views[1], include_str!("testdata/boom_step_rec.txt"), false, false);
        assert_eq!(n, 1153);
        assert!(kicks > 20, "{kicks} frames of a shift kick");
    }

    /// The bumper and cockpit (+668, the fixed mount) through the same update (F-14014): the lens without the
    /// slipstream / nitrous terms (F-44852), the frame's snap share from the chassis's air time over 0.5 s, the focus
    /// height not scaled, no hill pitch. 1,016 recorded calls of the R8's POV and 1,065 of its bumper (recording c13:
    /// throttle, steering, braking, coasting), each run from the recorded entry state.
    #[test]
    fn fixed_mount_step_matches_the_original() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let Some(set) = BoomSet::load(&data, "vp_aud_r8_08") else { return };
        let (n, _) = boom_rec_check(&set.views[3], include_str!("testdata/boom_step_pov_rec.txt"), true, false);
        assert_eq!(n, 1016);
        let (n, _) = boom_rec_check(&set.views[4], include_str!("testdata/boom_step_bumper_rec.txt"), true, false);
        assert_eq!(n, 1065);
    }

    /// The look-back branch (+208, vt+56): the chase views' reverse camera runs every frame with +208 set (RevOffset /
    /// RevFocus, the hill pitch swapped and negated, no turbo / nitrous lens). 1,153 recorded calls of the R8's
    /// `_reverse` camera beside the boom2 of recording c11.
    #[test]
    fn reverse_step_matches_the_original() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let Some(set) = BoomSet::load(&data, "vp_aud_r8_08") else { return };
        let (n, _) = boom_rec_check(set.reverse.as_ref().unwrap(), include_str!("testdata/boom_step_reverse_rec.txt"), true, true);
        assert_eq!(n, 1153);
    }

    /// The reset F-83404 (camDynamicBoomCS slot 9, with its frame F-90220 at the snap share 1 and the velocity start F-17419)
    /// against the original's code run offline (tools/boom_reset_reference.py, 4,000 cases: tunes, car matrices, wheels
    /// grounded or not, two or four or none, velocities, spins, the state word, the look back and the fixed mount): every
    /// field the port's camera keeps, bit for bit.
    #[test]
    fn reset_matches_the_original() {
        let data = include_str!("testdata/boom_reset_ref.txt");
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let (mut n, mut worst) = (0, 0.0f32);
        for line in data.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split_whitespace().collect();
            let f = |k: usize| f32::from_bits(h(w[k]));
            let v3 = |k: usize| Vec3::new(f(k), f(k + 1), f(k + 2));
            let (rev, fixed, dt) = (w[1] == "1", w[2] == "1", f(3));
            let tu = 4;
            let names = ["Center", "Offset", "Focus", "RevOffset", "RevFocus"];
            let mut text = String::from("camBoomCS
");
            for (q, nm) in names.iter().enumerate() {
                let v = v3(tu + 3 * q);
                text += &format!("{nm} {} {} {}
", v.x, v.y, v.z);
            }
            for (k, nm) in ["ApproachRate", "OrientationFactor", "OrientationFilter", "DirectionFilter", "SSFovMin"].iter().enumerate() {
                text += &format!("{nm} {}
", f(tu + 15 + k));
            }
            let t = BoomTune::parse(&text).unwrap();
            let m0 = tu + 20;
            let rot = Quat::from_mat3(&Mat3::from_cols(v3(m0), v3(m0 + 3), v3(m0 + 6)));
            let car = BoomTarget {
                pos: v3(m0 + 9), rot, vel: v3(m0 + 12), ang_vel: v3(m0 + 15), grounded: 0, gear: 0, rev, nitro: false, nitro_fov: (0.0, 0.5, 1.0),
                ability: (0.0, 1.0), sst: (0.0, 1.0), pan: (0.0, 0.0), rise: 0.0, fixed_mount: fixed, model_offset: Vec3::ZERO,
            };
            let speed = f(m0 + 18);
            let state: u32 = w[m0 + 19].parse().unwrap();
            let count: usize = w[m0 + 20].parse().unwrap();
            let mut g = BoomGround { count, speed, flags: state, ..Default::default() };
            for i in 0..count {
                let b = m0 + 21 + 10 * i;
                if w[b] == "1" {
                    g.wheels[i] = Some((v3(b + 1), v3(b + 4)));
                }
            }
            let mut cam = BoomCam { tumble: 0.5, vel: Vec3::splat(10.0), off: Vec3::splat(5.0), lens: 32.0, ab_fov: 10.0, rate: 20.0, ..Default::default() };
            cam.reset_as_original(&t, &car, &g, dt);
            let o: Vec<u32> = b.split_whitespace().map(h).collect();
            let fo = |k: usize| f32::from_bits(o[k]);
            let vo = |k: usize| Vec3::new(fo(k), fo(k + 1), fo(k + 2));
            // (the car's matrix reaches the port as a quaternion (BoomTarget::rot), so what is made from its rows - the frame,
            // the place, the velocity in the frame - is compared as the recorded update tests do, within 1e-5 of its size;
            // the rest bit for bit)
            let close = [("up", cam.up, vo(0)), ("dir", cam.dir, vo(3)), ("vel", cam.vel, vo(7)), ("ang", cam.ang, vo(13)), ("world", cam.world, vo(26))];
            for (nm, got, want) in close {
                let e = (got - want).abs().max_element() / want.abs().max_element().max(1.0);
                worst = worst.max(e);
                assert!(e < 1e-5, "{nm}: {got} against {want} ({e:e}): {line}");
            }
            let bits = |v: Vec3| v.to_array().map(f32::to_bits).to_vec();
            let exact: [(&str, Vec<u32>, Vec<u32>); 10] = [
                ("tumble", vec![cam.tumble.to_bits()], vec![o[6]]),
                ("acc", bits(cam.acc), bits(vo(10))),
                ("ang_acc", bits(cam.ang_acc), bits(vo(16))),
                ("off", bits(cam.off), bits(vo(19))),
                ("foc", bits(cam.foc), bits(vo(22))),
                ("roll", vec![cam.roll.to_bits()], vec![o[25]]),
                ("rate", vec![cam.rate.to_bits()], vec![o[29]]),
                ("gz / shift / value", vec![cam.gz.to_bits(), cam.shift.to_bits(), cam.shift_val.to_bits()], vec![o[30], o[31], o[32]]),
                ("armed / lens / snap", vec![cam.shift_armed as u32, cam.lens.to_bits(), cam.snap as u32], vec![o[33], o[34], o[35]]),
                ("ability lens", vec![cam.ab_fov.to_bits()], vec![o[36]]),
            ];
            for (nm, got, want) in exact {
                assert_eq!(got, want, "{nm}: {line}");
            }
            n += 1;
        }
        eprintln!("{n} resets, frame / place / velocity worst {worst:e} of their size");
        assert!(n >= 3900, "{n}");
    }

    /// Runs `BoomCam::step` on every recorded call of `text` (boom_testdata.py lines; `tail`: fixed_testdata.py lines,
    /// with the chassis air time and the snap byte +664 as the last two words) from its entry state, on a fixed mount
    /// (`fixed`) or with the look-back flag (`rev`), and asserts every field within 1e-5 of its size; returns the
    /// calls and those with a shift kick running.
    fn boom_rec_check(t: &BoomTune, text: &str, tail: bool, rev: bool) -> (usize, usize) {
        let fixed = tail && !rev;
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let names = ["up", "dir", "tumble", "vel", "acc", "ang", "ang_acc", "off", "foc", "roll", "world", "rate", "gz", "shift", "lens"];
        let mut worst = vec![0.0f32; names.len()];
        let (mut n, mut kicks) = (0, 0);
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let r: Vec<u32> = line.split_whitespace().map(h).collect();
            let f = |k: usize| f32::from_bits(r[k]);
            let v3 = |k: usize| Vec3::new(f(k), f(k + 1), f(k + 2));
            let (dt, k8, fps) = (f(0), f(1), f(2));
            let nw = r[30] as usize;
            let mut g = BoomGround { count: nw, speed: f(3), flags: r[4], gear: r[5] as i32, shifting: r[6] as i32, ..Default::default() };
            if tail {
                g.air = f(r.len() - 2);
            }
            for k in 0..nw.min(4) {
                let b = 31 + 7 * k;
                if r[b] != 0 {
                    g.wheels[k] = Some((v3(b + 1), v3(b + 4)));
                }
            }
            let ci = 31 + 7 * nw;
            let co = ci + 35;
            // (entry and exit: up 0, dir 3, tumble 6, vel 7, acc 10, ang 13, ang_acc 16, off 19, foc 22, roll 25,
            // world 26, rate 29, gz 30, shift 31 / 32 / 33, armed 34; then the lens)
            let mut cam = BoomCam {
                started: true, up: v3(ci), dir: v3(ci + 3), tumble: f(ci + 6), vel: v3(ci + 7), acc: v3(ci + 10), ang: v3(ci + 13),
                ang_acc: v3(ci + 16), off: v3(ci + 19), foc: v3(ci + 22), roll: f(ci + 25), world: v3(ci + 26), rate: f(ci + 29),
                gz: f(ci + 30), shift: f(ci + 31), shift_val: f(ci + 32), shift_size: f(ci + 33), shift_armed: r[ci + 34] != 0,
                recomp_timing: Some(k8), snap: tail && r[r.len() - 1] != 0, ..Default::default()
            };
            let rot = Quat::from_mat3(&Mat3::from_cols(v3(8), v3(12), v3(16))).normalize();
            let car = BoomTarget {
                pos: v3(20), rot, vel: v3(24), ang_vel: v3(27), grounded: 0, gear: 0, rev, nitro: false, nitro_fov: (0.0, 0.5, 1.0),
                ability: (0.0, 1.0), sst: (0.0, 1.0), pan: (0.0, 0.0), rise: 0.0, fixed_mount: fixed, model_offset: Vec3::ZERO,
            };
            // (the followed matrix: T+144 x, +160 y, +176 z, +192 the place)
            let o = cam.step(t, &car, &g, dt, fps);
            let got = [cam.up, cam.dir, Vec3::splat(cam.tumble), cam.vel, cam.acc, cam.ang, cam.ang_acc, cam.off, cam.foc, Vec3::splat(cam.roll), o.pos, Vec3::splat(cam.rate), Vec3::splat(cam.gz), Vec3::new(cam.shift, cam.shift_val, cam.shift_size), Vec3::splat(o.fov)];
            let want = [v3(co), v3(co + 3), Vec3::splat(f(co + 6)), v3(co + 7), v3(co + 10), v3(co + 13), v3(co + 16), v3(co + 19), v3(co + 22), Vec3::splat(f(co + 25)), v3(co + 26), Vec3::splat(f(co + 29)), Vec3::splat(f(co + 30)), Vec3::new(f(co + 31), f(co + 32), f(co + 33)), Vec3::splat(f(co + 35))];
            for k in 0..names.len() {
                let e = (got[k] - want[k]).abs().max_element() / want[k].abs().max_element().max(1.0);
                worst[k] = worst[k].max(e);
            }
            if f(co + 31) > 0.0 {
                kicks += 1;
            }
            n += 1;
        }
        for k in 0..names.len() {
            eprintln!("{:8} worst {:e}", names[k], worst[k]);
        }
        for k in 0..names.len() {
            assert!(worst[k] < 1e-5, "{}: off by {:e} of its size", names[k], worst[k]);
        }
        (n, kicks)
    }

    /// GitHub #43: the director's lens near (F-82384 -> director +88 with +92 set, read by F-98562) is the
    /// main viewport's 0.2 (0x3E4CCCCD), not a file's CameraNear: the POV files say 0.1 and
    /// the rest 0.2, and none of them reaches the lens. The port used 0.3, which cut the roof lining.
    #[test]
    fn director_near_is_the_viewports() {
        assert_eq!(DIRECTOR_NEAR.to_bits(), 0x3E4C_CCCD);
        assert!(DIRECTOR_NEAR < crate::camera::Camera::default().near);
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let mut seen = 0;
        for car in ["vp_nsn_240sx_98", "vp_vw_golf_83", "vp_aud_r8_08"] {
            let Some(set) = BoomSet::load(&data, car) else { continue };
            seen += 1;
            assert_eq!(set.views.len(), 5, "{car}");
            for (i, v) in set.views.iter().enumerate() {
                let file = v.s("CameraNear");
                assert_eq!(file, if i == 3 { 0.1 } else { 0.2 }, "{car} view {i}: the file's CameraNear");
            }
        }
        eprintln!("{seen} cars' camera files read");
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

/// F-85638 / F-79714 / F-25863 / F-22575 / F-30992 / F-49780: the camera time warper against the original's own code run offline
/// (tools/camera_warper_reference.py -> testdata/camera_warper_ref.txt: 300 random tunes with 80 random start / release / update
/// operations each, the state compared after every operation; the constructor; the tune constructor's defaults).
#[cfg(test)]
mod original_warper_tests {
    use super::*;

    #[test]
    fn time_warper_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let f = |x: &str| f32::from_bits(h(x));
        let mut seqs = 0;
        for line in include_str!("testdata/camera_warper_ref.txt").lines() {
            if let Some(r) = line.strip_prefix("DEFAULTS -> ") {
                // the tune constructor: duration, slow-motion scale, rise rate, fall rate (the lookback warper's tune in main.rs)
                let v: Vec<u32> = r.split(' ').map(h).collect();
                assert_eq!(v, [0.0f32, 0.239, 2.009, 3.009].map(f32::to_bits));
            } else if let Some(r) = line.strip_prefix("CTOR -> ") {
                // state 2 (idle), normal scale and from / to / cur 1, since 0; t (+24) is left alone and the rate (+32) is 1 in the original -
                // neither can be seen while idle (the ramp returns for state > 1) and both are set by start / release
                let v: Vec<u32> = r.split(' ').map(h).collect();
                let w = TimeWarper::new([0.0; 4]);
                assert_eq!((v[0], v[2], v[3], v[4], v[5], v[7]), (w.state as u32, 1.0f32.to_bits(), w.from.to_bits(), w.to.to_bits(), w.cur.to_bits(), w.since.to_bits()));
                assert_eq!(v[8], 1.0f32.to_bits());
            } else if let Some(r) = line.strip_prefix("SEQ ") {
                let (a, b) = r.split_once(" -> ").unwrap();
                let (head, ops) = a.split_once(" 80 ").unwrap();
                let tune: Vec<f32> = head.split(' ').map(f).collect();
                let mut w = TimeWarper::new([tune[0], tune[1], tune[2], tune[3]]);
                let mut touched = false;
                for (op, row) in ops.split(' ').zip(b.split(" | ")) {
                    let args: Vec<&str> = op[1..].split(',').collect();
                    match op.as_bytes()[0] {
                        b'S' => {
                            w.start(f(args[0]), f(args[1]));
                            touched = true;
                        }
                        b'R' => {
                            w.release(f(args[0]));
                            touched = true;
                        }
                        _ => w.update(f(args[0])),
                    }
                    let v: Vec<u32> = row.split(' ').map(h).collect();
                    let got = [w.state as u32, w.from.to_bits(), w.to.to_bits(), w.cur.to_bits(), w.t.to_bits(), w.since.to_bits()];
                    assert_eq!(got[..], [v[0], v[1], v[2], v[3], v[4], v[5]], "{line} at {op}");
                    if touched {
                        assert_eq!(w.rate.to_bits(), v[6], "rate, {line} at {op}");
                    }
                }
                seqs += 1;
            }
        }
        assert_eq!(seqs, 300);
    }
}

/// F-29203 / F-54070 / F-48972 / F-68650: the camera shake's reset, continuous strength, one-shot and the manager's trigger against the
/// original's own code run offline (tools/camera_shake_reference.py -> testdata/camera_shake_ref.txt: 400 random block states with 30
/// random operations each; the five blocks' timer / magnitude / scale / state compared after every operation).
#[cfg(test)]
mod original_shake_tests {
    use super::*;

    #[test]
    fn shake_setters_match_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let f = |x: &str| f32::from_bits(h(x));
        let mut seqs = 0;
        for line in include_str!("testdata/camera_shake_ref.txt").lines() {
            let (a, b) = line.strip_prefix("SEQ ").unwrap().split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let blocks: Vec<ShakeBlock> = (0..5)
                .map(|i| ShakeBlock { timer: f(w[4 * i]), magnitude: f(w[4 * i + 1]), scale: f(w[4 * i + 2]), state: w[4 * i + 3].parse().unwrap(), ..Default::default() })
                .collect();
            let mut cs = CamShake { blocks, rng: (1, 0) };
            for (op, row) in w[21..].iter().zip(b.split(" | ")) {
                let args: Vec<&str> = op[1..].split(',').collect();
                match op.as_bytes()[0] {
                    b'R' => cs.reset(),
                    b'S' => cs.set(args[0].parse().unwrap(), f(args[1])),
                    _ => cs.jolt(f(args[1]), f(args[2]), args[0].parse::<u32>().unwrap() < 2),
                }
                let v: Vec<&str> = row.split(' ').collect();
                for (i, bl) in cs.blocks.iter().enumerate() {
                    let got = [bl.timer.to_bits(), bl.magnitude.to_bits(), bl.scale.to_bits(), bl.state as u32];
                    let want = [h(v[4 * i]), h(v[4 * i + 1]), h(v[4 * i + 2]), v[4 * i + 3].parse::<u32>().unwrap()];
                    assert_eq!(got, want, "block {i} after {op} in {line}");
                }
            }
            seqs += 1;
        }
        assert_eq!(seqs, 400);
    }
}

/// F-17992: the shake oscillator against the original's own code run offline (tools/camera_shake_osc_reference.py ->
/// testdata/camera_shake_osc_ref.txt: 2,000 random phases (up to 1e5 rad), frequencies, amplitudes, random ranges, multipliers
/// and generator states). The port used the host's sine before; the numbers need the game's own sine.
#[cfg(test)]
mod original_osc_tests {
    use super::*;

    #[test]
    fn shake_oscillator_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let f = |x: &str| f32::from_bits(h(x));
        let mut n = 0;
        for line in include_str!("testdata/camera_shake_osc_ref.txt").lines() {
            let (a, b) = line.strip_prefix("OSC ").unwrap().split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let v3 = |i: usize| Vec3::new(f(w[i]), f(w[i + 1]), f(w[i + 2]));
            // [amplitude, frequency, random, phase]
            let mut part = [v3(10), v3(7), v3(13), v3(4)];
            let mut rng = (h(w[0]), h(w[1]));
            let out = oscillate(&mut part, f(w[2]), f(w[3]), &mut rng);
            let o: Vec<u32> = b.split(' ').map(h).collect();
            let got = [out.x.to_bits(), out.y.to_bits(), out.z.to_bits(), part[3].x.to_bits(), part[3].y.to_bits(), part[3].z.to_bits(), rng.0, rng.1];
            assert_eq!(got[..], o[..], "{line}");
            n += 1;
        }
        assert_eq!(n, 2000);
    }
}


/// The director's blend step (F-18583, `DirBlend::step`) against the original's own code run offline
/// (tools/director_blend_reference.py -> testdata/director_blend_ref.txt, 5,000 random blends).
#[cfg(test)]
mod director_blend_tests {
    use super::*;

    fn views() -> BoomSet {
        let t = |z: f32, bt: f32| BoomTune::parse(&format!("camBoomCS\nBlendTime {bt}\nBlendGoal 1\nOffset 0 2 {z}\nFocus 0 1 0\n")).unwrap();
        BoomSet { views: vec![t(5.0, 1.0), t(6.0, 1.0), t(7.0, 0.5), t(0.0, 1.0), t(0.0, 1.0)], reverse: None }
    }

    /// One frame as main.rs runs it: the switch, the shown view's camera, the blend.
    fn run(sw: &mut Switch, set: &BoomSet, cam: &mut BoomCam, view: u8, dt: f32) {
        let car = BoomTarget { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, ang_vel: Vec3::ZERO, grounded: 4, gear: 1, rev: false, nitro: false, nitro_fov: (0.0, 0.5, 1.0), ability: (0.0, 1.0), sst: (0.0, 1.0), pan: (0.0, 0.0), rise: 0.0, fixed_mount: false, model_offset: Vec3::ZERO };
        sw.frame(set, view, false, false, false, cam, dt);
        let (tune, _) = tune_for(set, sw.shown(view), false);
        let out = cam.update(tune, &car, dt);
        sw.blend.apply(&car, out, &[], dt);
    }

    /// The director's queue and finish (F-98424 -> F-43758, F-70013): a view change while the views
    /// blend only queues the new view (the blend keeps its target, its time becomes the push's); at
    /// the blend's end the director cuts to the queued view.
    #[test]
    fn a_change_while_blending_is_queued_then_cut_to() {
        let set = views();
        let (mut sw, mut cam) = (Switch::default(), BoomCam::default());
        let dt = 1.0 / 30.0;
        for _ in 0..10 {
            run(&mut sw, &set, &mut cam, 0, dt);
        }
        run(&mut sw, &set, &mut cam, 1, dt);
        assert!(sw.blend.active() && sw.shown(1) == 1);
        for _ in 0..6 {
            run(&mut sw, &set, &mut cam, 1, dt);
        }
        run(&mut sw, &set, &mut cam, 2, dt);
        assert_eq!((sw.shown(2), sw.blend.queued, sw.blend.time), (1, Some(CamId::View(2)), 0.5));
        let mut n = 0;
        while sw.blend.active() {
            run(&mut sw, &set, &mut cam, 2, dt);
            assert_eq!(sw.shown(2), 1);
            n += 1;
            assert!(n < 60);
        }
        run(&mut sw, &set, &mut cam, 2, dt);
        assert!(!sw.blend.active() && sw.shown(2) == 2 && sw.shown.is_none());
    }

    /// The turn-round (F-43758): back to the source queues it; the next change finds the source
    /// queued and turns the blend round (weight 1 - w, time left = the push's time x w, goal the old
    /// source's), whatever view was chosen - the director then ends on the old source.
    #[test]
    fn a_queued_source_turns_the_blend_round() {
        let set = views();
        let (mut sw, mut cam) = (Switch::default(), BoomCam::default());
        let dt = 1.0 / 30.0;
        for _ in 0..10 {
            run(&mut sw, &set, &mut cam, 0, dt);
        }
        run(&mut sw, &set, &mut cam, 1, dt);
        for _ in 0..8 {
            run(&mut sw, &set, &mut cam, 1, dt);
        }
        run(&mut sw, &set, &mut cam, 0, dt);
        assert_eq!((sw.shown(0), sw.blend.queued), (1, Some(CamId::View(0))));
        let w = sw.blend.blend.weight;
        assert!(w > 0.0 && w < 1.0);
        run(&mut sw, &set, &mut cam, 1, dt);
        assert_eq!((sw.blend.src, sw.blend.dst, sw.blend.queued, sw.shown(1)), (CamId::View(1), CamId::View(0), None, 0));
        let w2 = sw.blend.blend.weight;
        assert!(w2 > 1.0 - w - 0.2 && w2 < 1.0);
        let mut n = 0;
        while sw.blend.active() {
            run(&mut sw, &set, &mut cam, 1, dt);
            n += 1;
            assert!(n < 60);
        }
        run(&mut sw, &set, &mut cam, 1, dt);
        assert_eq!(sw.shown(1), 0);
    }

    /// CAM_UP over the event camera (F-67519): the event's off (F-94591, 0.2 s) goes first, then the
    /// view's push (F-41662 -> F-98424) on the one director. With the event shown the off begins the
    /// blend out and the new view is queued behind it (F-43758), cut to at its end (F-70013); while
    /// the event still blends in, the off turns the blend round (F-29802) and queues the old view -
    /// the target -, so the new view's push is dropped and the old view stays on.
    #[test]
    fn a_view_change_over_the_event_camera() {
        let set = views();
        let t = TrackTune::from_text("type: a\ncamTrackCS {\n}\n", "Idle");
        let ev_out = BoomOut { pos: Vec3::new(3.0, 1.0, 4.0), look: Vec3::ZERO, up: Vec3::Y, fov: 50.0 };
        let car = target_at(Vec3::ZERO);
        let dt = 1.0 / 30.0;
        let frame = |sw: &mut Switch, cam: &mut BoomCam, view: u8, ev: bool| {
            sw.frame(&set, view, false, false, false, cam, dt);
            let (tune, _) = tune_for(&set, sw.shown(view), false);
            let out = cam.update(tune, &car, dt);
            let others = if ev { vec![(CamId::Event, ev_out)] } else { vec![] };
            sw.blend.apply(&car, out, &others, dt);
        };
        let (v1, v2) = (CamId::View(1), CamId::View(2));
        // shown: queued, then cut to
        let (mut sw, mut cam, mut d) = (Switch::default(), BoomCam::default(), EventDirector::default());
        for _ in 0..5 {
            frame(&mut sw, &mut cam, 1, false);
        }
        d.begin_in(&mut sw.blend, 0.5);
        let mut n = 0;
        while sw.blend.active() {
            frame(&mut sw, &mut cam, 1, true);
            n += 1;
            assert!(n < 60);
        }
        assert!(d.shown(&sw.blend));
        d.off(&mut sw.blend, &t, 0.2);
        frame(&mut sw, &mut cam, 2, true);
        assert_eq!((sw.blend.src, sw.blend.dst, sw.blend.queued, sw.shown(2)), (CamId::Event, v1, Some(v2), 1));
        while sw.blend.active() {
            frame(&mut sw, &mut cam, 2, true);
            assert_eq!(sw.shown(2), 1);
        }
        frame(&mut sw, &mut cam, 2, false);
        assert!(sw.shown(2) == 2 && sw.blend.is_current(v2) && !d.leaving(&sw.blend));
        // blending in: dropped
        let (mut sw, mut cam, mut d) = (Switch::default(), BoomCam::default(), EventDirector::default());
        for _ in 0..5 {
            frame(&mut sw, &mut cam, 1, false);
        }
        d.begin_in(&mut sw.blend, 0.5);
        for _ in 0..3 {
            frame(&mut sw, &mut cam, 1, true);
        }
        d.off(&mut sw.blend, &t, 0.2);
        frame(&mut sw, &mut cam, 2, true);
        assert_eq!((sw.blend.src, sw.blend.dst, sw.blend.queued, sw.shown(2)), (CamId::Event, v1, Some(v1), 1));
        while sw.blend.active() {
            frame(&mut sw, &mut cam, 2, true);
        }
        frame(&mut sw, &mut cam, 2, false);
        assert!(sw.shown(2) == 1 && sw.blend.is_current(v1));
    }

    /// One director for every camera: the wipeout camera pushed (F-37740, 0.25 s) while the ability
    /// camera blends in (F-38892, 0.5 s) is queued (F-43758, its time the director's) and cut to at
    /// the blend's end (F-70013) - not blended in from the drive view behind the ability's blend.
    #[test]
    fn a_push_during_another_cameras_blend_is_queued_then_cut_to() {
        let car = target_at(Vec3::ZERO);
        let drive = BoomOut { pos: Vec3::new(0.0, 2.0, 9.0), look: Vec3::ZERO, up: Vec3::Y, fov: 40.0 };
        let ab = BoomOut { pos: Vec3::new(3.0, 1.0, 4.0), look: Vec3::ZERO, up: Vec3::Y, fov: 50.0 };
        let wipe = BoomOut { pos: Vec3::new(-6.0, 0.5, 0.0), look: Vec3::ZERO, up: Vec3::Y, fov: 40.0 };
        let others = [(CamId::Ability, ab), (CamId::Wipeout, wipe)];
        let dt = 1.0 / 30.0;
        let mut dir = Director::default();
        assert_eq!(dir.push(CamId::Ability, 0.5, 1.0), Pushed::Begun);
        for _ in 0..5 {
            dir.apply(&car, drive, &others, dt);
        }
        let w = dir.blend.weight;
        assert!(w > 0.0 && w < 1.0);
        assert_eq!(dir.push(CamId::Wipeout, 0.25, 1.0), Pushed::Queued);
        assert_eq!((dir.src, dir.dst, dir.queued, dir.time), (CamId::View(0), CamId::Ability, Some(CamId::Wipeout), 0.25));
        let mut last = drive;
        let mut n = 0;
        while dir.active() {
            last = dir.apply(&car, drive, &others, dt);
            n += 1;
            assert!(n < 60);
        }
        assert!(dir.is_current(CamId::Wipeout) && dir.queued.is_none());
        assert_eq!(last.pos, wipe.pos);
        assert_eq!(dir.apply(&car, drive, &others, dt).pos, wipe.pos);
    }

    /// The director's push F-98424 with its queue F-43758, begin F-18878, cut F-53309 and finish
    /// F-70013 run offline on the original's code (work/cameras2/dirq/dirq_ref.py: 1,000 runs of
    /// view changes, blend progress and finishes over three chase views; BlendTime 1 / 0.7 / 0.25,
    /// BlendGoal 1 / 0.8 / 0.9): after every step the director's camera, and while it blends the
    /// source, target, queued camera, weight, goal, time left and push time to the bit.
    #[test]
    fn director_queue_matches_the_original() {
        let t = |bt: &str, g: &str| BoomTune::parse(&format!("camBoomCS
BlendTime {bt}
BlendGoal {g}
Offset 0 2 5
Focus 0 1 0
")).unwrap();
        let set = BoomSet { views: vec![t("1", "1"), t("0.7", "0.8"), t("0.25", "0.9"), t("0", "1"), t("0", "1")], reverse: None };
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let (mut sw, mut cam, mut view) = (Switch::default(), BoomCam::default(), 0u8);
        let mut steps = 0;
        for (n, line) in include_str!("testdata/director_queue_ref.txt").lines().enumerate() {
            let (op, want) = line.split_once('|').unwrap();
            let op: Vec<&str> = op.split_whitespace().collect();
            match op[0] {
                "E" => continue,
                "S" => {
                    (sw, cam) = (Switch::default(), BoomCam::default());
                    view = op[1].parse::<u8>().unwrap() - 1;
                    sw.frame(&set, view, false, false, false, &mut cam, 1.0 / 60.0);
                }
                "P" => {
                    view = op[1].parse::<u8>().unwrap() - 1;
                    assert_eq!(set.views[view as usize].blend_time().to_bits(), h(op[2]));
                    sw.frame(&set, view, false, false, false, &mut cam, 1.0 / 60.0);
                }
                "W" => sw.blend.blend.weight = f32::from_bits(h(op[1])),
                _ => {
                    if sw.blend.active() {
                        sw.blend.finish();
                        sw.frame(&set, view, false, false, false, &mut cam, 1.0 / 60.0);
                    }
                }
            }
            let w: Vec<&str> = want.split_whitespace().collect();
            let num = |v: Option<CamId>| match v {
                Some(CamId::View(v)) => v as u32 + 1,
                _ => 0,
            };
            if w[0] == "9" {
                assert!(sw.blend.active(), "line {}: {line}", n + 1);
                let b = &sw.blend;
                let got = (num(Some(b.src)), num(Some(b.dst)), num(b.queued), b.blend.weight.to_bits(), b.blend.goal.to_bits(), b.blend.left.to_bits(), b.time.to_bits());
                let exp = (w[3].parse().unwrap(), w[4].parse().unwrap(), w[5].parse().unwrap(), h(w[6]), h(w[7]), h(w[8]), h(w[2]));
                assert_eq!(got, exp, "line {}: {line}", n + 1);
            } else {
                assert!(!sw.blend.active(), "line {}: {line}", n + 1);
                assert_eq!(num(Some(CamId::View(sw.shown(view)))), w[0].parse::<u32>().unwrap(), "line {}: {line}", n + 1);
            }
            steps += 1;
        }
        assert_eq!(steps, 9132);
    }

    #[test]
    fn director_blend_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let f = |x: &str| f32::from_bits(h(x));
        let cam = |w: &[&str]| DirCam {
            rows: [0, 1, 2].map(|r| Vec3::new(f(w[5 + 3 * r]), f(w[6 + 3 * r]), f(w[7 + 3 * r]))),
            pos: Vec3::new(f(w[14]), f(w[15]), f(w[16])),
            target: (h(w[1]) != 0).then(|| Vec3::new(f(w[2]), f(w[3]), f(w[4]))),
            lens: [f(w[17]), f(w[18]), f(w[19]), f(w[20])],
            blend_goal: 1.0,
        };
        let mut n = 0;
        for line in include_str!("testdata/director_blend_ref.txt").lines() {
            let (a, b) = line.strip_prefix("DB ").unwrap().split_once(" -> ").unwrap();
            let i: Vec<&str> = a.split(' ').collect();
            let o: Vec<&str> = b.split(' ').collect();
            let (from, to) = (cam(&i[6..27]), cam(&i[27..48]));
            let mut bl = DirBlend { weight: f(i[3]), goal: f(i[4]), left: f(i[5]), ..Default::default() };
            let on = bl.step(&from, &to, h(i[0]) as u16, f(i[1]), f(i[2]));
            assert_eq!(!on, o[0] == "1", "done, {line}");
            assert_eq!((bl.weight.to_bits(), bl.left.to_bits()), (h(o[1]), h(o[2])), "weight / left, {line}");
            if on {
                for k in 0..4 {
                    assert_eq!(bl.lens[k].to_bits(), h(o[3 + k]), "lens {k}, {line}");
                }
                for r in 0..3 {
                    for k in 0..3 {
                        assert_eq!(bl.rows[r][k].to_bits(), h(o[7 + 3 * r + k]), "row {r}.{k}, {line}");
                    }
                }
                for k in 0..3 {
                    assert_eq!(bl.pos[k].to_bits(), h(o[16 + k]), "pos {k}, {line}");
                }
            }
            n += 1;
        }
        assert_eq!(n, 5000);
    }
}
