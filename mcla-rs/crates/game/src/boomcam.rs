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
    /// +456, set by mcPlayerCamera; 0 for the other views)
    pub pan: (f32, f32),
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
    gz: f32,
    nitro: f32,
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

    pub fn update(&mut self, t: &BoomTune, car: &BoomTarget, dt: f32) -> BoomOut {
        let dt = dt.clamp(1.0 / 600.0, 0.1);
        // the filters are per frame at 60 Hz
        let frames = dt * 60.0;
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
            let (kv, ka) = (per_frame(t.s("VelocityFilter")), per_frame(t.s("AccelFilter")));
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
        // and turns its boom about the car: pitch about x, then yaw about y (0x82320EFC..0x82320F80)
        if pan_yaw != 0.0 || pan_pitch != 0.0 {
            let r = Mat3::from_rotation_y(pan_yaw) * Mat3::from_rotation_x(pan_pitch);
            off = r * off;
            foc_target = r * foc_target;
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
            // and nothing moves faster than the approach rate
            let rate = t.s("ApproachRate");
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
}

impl Default for CamFx {
    fn default() -> CamFx {
        CamFx { idle_delay: 60.0, burnout_turn: 1.5 }
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
                _ => {}
            }
        }
        fx
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
        })
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
}

/// camTrackCS' approach step (sub_823223E8): `cur` goes towards `tgt` by s * rate * dt, s the
/// distance (quadratic within `min`) through the AppApp lag `st`; no overshoot. The lag is
/// per call of the original's frame (taken at 60 Hz).
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
}

impl TrackCam {
    /// The event is set (sub_82316148): Burnout's TrackTo.z = the front axle and its side the
    /// steering; Idle's side a random bit; Offset and TrackTo mirrored when the side is above
    /// zero; the next frame snaps (reset sub_8231AC50: the filtered speed starts at the car's).
    fn set(&mut self, t: &TrackTune, car: &TrackCar, ev: usize, coin: bool) {
        let (mut track, side) = match ev {
            EV_IDLE => (t.track, if coin { 1.0 } else { -1.0 }),
            _ => (Vec3::new(t.track.x, t.track.y, car.front_z), car.steer),
        };
        let mut offset = t.offset;
        if side > 0.0 {
            track.x = -track.x;
            offset.x = -offset.x;
        }
        let app_xz = self.app_xz;
        *self = TrackCam { snap: true, track, offset, speed: if car.reverse_gear { 0.0 } else { car.speed }, app_xz, ..Default::default() };
    }

    /// Where the camera wants to stand (sub_8231C4F8): the Offset in the car's level heading
    /// (behind the motion above 10 m/s), its y along the world's up.
    fn place(&self, car: &TrackCar, at: Vec3) -> Vec3 {
        let m = Mat3::from_quat(car.rot);
        let h = Vec3::new(car.vel.x, 0.0, car.vel.z);
        let back = if car.speed > 10.0 && h.length_squared() > 1e-4 { -car.vel.normalize() } else { m.z_axis };
        let x = Vec3::Y.cross(back).normalize_or(m.x_axis);
        let zh = x.cross(Vec3::Y);
        at + x * self.offset.x + zh * self.offset.z + Vec3::Y * self.offset.y
    }

    fn update(&mut self, t: &TrackTune, car: &TrackCar, dt: f32) -> BoomOut {
        let frames = (dt * 60.0).max(0.0);
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
        let want = self.place(car, at);
        // the place's approach rates (sub_8231B368): over the speed range, the XZ one moving
        // towards its goal at AppInc / AppDec a second, in reverse gear RevAppXZPos
        let s = car.speed;
        let u = if t.app_speed.1 != t.app_speed.0 && s > t.app_speed.0 { ((s - t.app_speed.0) / (t.app_speed.1 - t.app_speed.0)).min(1.0) } else { 0.0 };
        let goal = if car.reverse_gear {
            t.rev_app_xz
        } else if t.app_xz.0 > 0.0 {
            t.app_xz.1 + (t.app_xz.0 - t.app_xz.1) * u
        } else {
            t.app_xz.1
        };
        let step = if goal < self.app_xz { t.app_dec } else { t.app_inc } * dt;
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
        BoomOut { pos: self.pos, look: self.pos + dir_of(self.yaw, self.pitch) * reach, up: Vec3::Y, fov }
    }
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

/// The burnout camera: the event camera's Burnout view, chosen by sub_822B3BF0 while the
/// CINEMATIC CAMERA option is on and a chase view is in use, blended in over the event's
/// BlendTime and back to the chase view over its ReturnTime (0: the BlendTime) with the
/// director's cosine orbit blend (sub_822AFCD0).
#[derive(Default)]
pub struct EventDirector {
    cam: TrackCam,
    /// 0 off, 1 blending in, 2 on, 3 blending out
    phase: u8,
    left: f32,
    time: f32,
    quit: bool,
    /// the event on (EV_BURNOUT or EV_IDLE)
    ev: usize,
    /// the game's generator for Idle's side (approx.: the port's own, not 0x82860880's draws)
    rng: u32,
    /// each car's Burnout and Idle records and the camFX tune, read once
    tunes: HashMap<(String, usize), Option<TrackTune>>,
    fx: Option<CamFx>,
}

impl EventDirector {
    /// The event camera is running (or blending in): the camera buttons act on it.
    pub fn running(&self) -> bool {
        self.phase == 1 || self.phase == 2
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

    /// One frame. `allowed`: the chooser's gate (option on, a chase view, nothing else on the
    /// camera); `idle`: seconds the player has left the car alone (0 at any input); `chase`:
    /// the chase view's output this frame.
    pub fn frame(&mut self, data_dir: &str, car_name: &str, allowed: bool, handbrake: f32, charge: f32, yaw_rate: f32, idle: f32, car: &TrackCar, chase: BoomOut, dt: f32) -> BoomOut {
        let fx = self.fx.get_or_insert_with(|| CamFx::load(data_dir)).clone();
        let fx = &fx;
        if self.phase == 0 {
            self.quit = false;
            // (the chooser's order, sub_822B3BF0: Burnout before Idle)
            let pick = if burnout_starts(fx, handbrake, charge, yaw_rate) {
                Some(EV_BURNOUT)
            } else if idle >= fx.idle_delay {
                Some(EV_IDLE)
            } else {
                None
            };
            if let (true, Some(ev)) = (allowed, pick) {
                if let Some(t) = self.tune(data_dir, car_name, ev).filter(|t| t.blend > 0.0) {
                    self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    self.ev = ev;
                    self.cam.set(&t, car, ev, self.rng >> 31 != 0);
                    self.phase = 1;
                    self.time = t.blend;
                    self.left = t.blend;
                }
            }
            if self.phase == 0 {
                return chase;
            }
        }
        let Some(t) = self.tune(data_dir, car_name, self.ev) else {
            self.phase = 0;
            return chase;
        };
        let t = &t;
        match self.phase {
            1 | 2 => {
                let ends = match self.ev {
                    EV_IDLE => idle <= 0.0,
                    _ => burnout_ends(fx, handbrake, charge, yaw_rate),
                };
                if self.quit || !allowed || ends {
                    // (CAM_UP's leave asks for 0.2 s)
                    let back = if self.quit { 0.2 } else if t.ret > 0.0 { t.ret } else { t.blend };
                    // (leaving while still blending in turns the blend round: w stays where it is)
                    let w = if self.phase == 1 { 1.0 - self.left / self.time.max(1e-4) } else { 1.0 };
                    self.phase = 3;
                    self.time = back;
                    self.left = back * w;
                    self.quit = false;
                }
            }
            _ => {}
        }
        if self.phase == 0 {
            return chase;
        }
        let ev = self.cam.update(t, car, dt);
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

/// One named view of a car's event camera file (`<car>_cinematic.camtrackcs`: Nitrous,
/// Wheelie, Finish, Damageout ...): the point on the car it looks at, where it stands
/// (both in the car's frame) and the seconds it takes to get there.
pub fn event_view(data_dir: &str, car: &str, view: &str) -> Option<(Vec3, Vec3, f32)> {
    let h = mcla_formats::hash::joaat(&format!("{car}_cinematic.camtrackcs"));
    let text = std::fs::read_to_string(Path::new(data_dir).join("tune/camera").join(format!("0x{h:08x}"))).ok()?;
    let mut inside = false;
    let (mut track, mut offset, mut blend) = (None, None, 0.7);
    for l in text.lines() {
        let mut it = l.split_whitespace();
        let Some(k) = it.next() else { continue };
        if !inside {
            inside = k == view && l.contains('{');
            continue;
        }
        if k == "}" {
            break;
        }
        let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
        match (k, v.len()) {
            ("TrackTo", 3) => track = Some(Vec3::new(v[0], v[1], v[2])),
            ("Offset", 3) => offset = Some(Vec3::new(v[0], v[1], v[2])),
            ("BlendTime", 1) => blend = v[0],
            _ => {}
        }
    }
    Some((track?, offset?, blend))
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
        let mut car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3 };
        let o = d.frame(&data, "vp_aud_r8_08", true, 1.0, 1.0, 2.0, 0.0, &car, BoomOut { ..chase }, 1.0 / 60.0);
        assert!(d.running());
        // (blending in: between the chase view and the event view)
        assert!(o.pos.distance(chase.pos) < 1.0, "{:?}", o.pos);
        let mut last = Vec3::ZERO;
        for i in 0..60 {
            car.rot = Quat::from_rotation_y(i as f32 * 2.0 / 60.0);
            let o = d.frame(&data, "vp_aud_r8_08", true, 1.0, 1.0, 2.0, 0.0, &car, BoomOut { ..chase }, 1.0 / 60.0);
            if i >= 50 {
                // fully on: the place it took on the first frame, (2.5, 1 + 1.5, -1.3 + 13)
                assert!(o.pos.distance(Vec3::new(2.5, 2.5, 11.7)) < 1e-3, "{:?}", o.pos);
                assert!((o.fov - 25.0).abs() < 1e-3);
                last = o.look;
            }
        }
        assert!((last - Vec3::new(2.5, 2.5, 11.7)).normalize().dot((Vec3::new(0.0, 1.0, 0.0) + Quat::from_rotation_y(2.0) * Vec3::new(0.0, 0.0, -1.3) - Vec3::new(2.5, 2.5, 11.7)).normalize()) > 0.99);
        d.frame(&data, "vp_aud_r8_08", true, 1.0, 1.0, 0.7, 0.0, &car, BoomOut { ..chase }, 1.0 / 60.0);
        assert!(!d.running());
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
        let car = TrackCar { pos: Vec3::ZERO, rot: Quat::IDENTITY, vel: Vec3::ZERO, speed: 0.0, reverse_gear: false, steer: 0.0, front_z: -1.3 };
        let o = d.frame(&data, "vp_aud_r8_08", true, 0.0, 0.0, 0.0, 59.9, &car, BoomOut { ..chase }, 1.0 / 60.0);
        assert!(!d.idle() && o.pos == chase.pos);
        let mut held = Vec::new();
        for i in 0..240 {
            let o = d.frame(&data, "vp_aud_r8_08", true, 0.0, 0.0, 0.0, 60.0 + i as f32 / 60.0, &car, BoomOut { ..chase }, 1.0 / 60.0);
            assert!(d.idle());
            if i >= 100 {
                held.push(o);
            }
        }
        let p = held[0].pos;
        assert!((p.x.abs() - 2.525).abs() < 1e-3 && (p.y - 0.895).abs() < 1e-3 && (p.z - (0.55 - 9.0)).abs() < 1e-3, "{p:?}");
        assert!(held.iter().all(|o| o.pos.distance(p) < 1e-4 && (o.fov - 25.0).abs() < 1e-3));
        d.frame(&data, "vp_aud_r8_08", true, 0.0, 0.0, 0.0, 0.0, &car, BoomOut { ..chase }, 1.0 / 60.0);
        assert!(!d.running());
    }
}
