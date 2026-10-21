//! The original's car simulation (rage `veh*` classes and `mcCarSim`), ported from the
//! executable: suspension, the tyre model with its displacement springs and friction
//! curve, the drivetrain with engine, torque converter and gearbox, aerodynamics, and
//! gravity scaled by the car's `BoundGravity`. All figures come from the car's `.r50`.
//!
//! The original integrates at a fixed 60 Hz with an implicit wheel/drivetrain solve; this
//! port uses the validated implicit shaft/tyre kernels once per frame. Chassis
//! motion/collision still uses four smaller steps with approximate contact response.

use super::car::{CarInput, CarParams, CarState};
use super::collision::WorldCollider;
use glam::{Mat3, Quat, Vec3};
use mcla_formats::rage::simtune::{SimTune, WheelTune};

const RPM: f32 = 0.104_719_76; // original float at 0x8207D3E8: rpm -> rad/s
const MPH: f32 = 0.447_04;

/// What `SimTune` implies for one car: gear ratios and per-wheel constants.
#[derive(Clone, Debug)]
pub struct VehTune {
    pub t: SimTune,
    /// engine turns per wheel turn: [reverse, first, second ..]
    pub ratios: Vec<f32>,
    pub top_speed: f32,
    pub gravity: f32,
    /// a motorcycle: held upright by the lean controller, can wheelie (mcBikeGyro)
    pub bike: bool,
    /// the gearbox's shift points per gear, in rpm (vehTransmission::Init): shifting up on a
    /// full and on a light pedal, shifting down on a full and on a light pedal
    pub shift: Vec<[f32; 4]>,
    /// vehEngine +20: torque at peak power, used to synchronize engine speed.
    pub sync_torque: f32,
    /// the body's moment of inertia about its height (the box of the tuning)
    pub inertia_yaw: f32,
    /// the tyres' width as fitted (front, rear; vehWheel +424; 0 = not known, and the wheel
    /// is then a plain disc of the tuning's radius). The wheels' collision shapes are crowned
    /// tyres made from it - see `crown`
    pub tyre_width: [f32; 2],
    /// the wheels as the collision has them (front, rear)
    pub wheel_shape: [std::sync::Arc<super::hull::HullShape>; 2],
    torque_curve: Option<super::engine_curve::Curve>,
    idle_curve: Option<super::engine_curve::Curve>,
}

/// The radius of a wheel's collision shape `aside` of its middle. The original's wheel is a
/// curved bound (0x8255BB80 scales a unit one to TireWidth and TireRadius - 0.04, its margin):
/// across the tread an arc through the two shoulders about a point (sqrt 6 - 1) (r - 0.04)
/// beyond the axle, so the shoulders are at the tyre's radius and the middle stands proud of
/// it - 0.3072 on a tyre of 0.2953 x 0.245 (ten recorded wheels agree to 0.2 mm).
pub fn crown(radius: f32, width: f32, aside: f32) -> f32 {
    let s = radius - 0.04;
    let far = (6.0f32.sqrt() - 1.0) * s;
    (width * width * 0.25 + 6.0 * s * s - aside * aside).max(0.0).sqrt() - far + 0.04
}

/// Half the width of a wheel's collision shape, margin and all: its sides are caps of spheres
/// about points 5.4992 half-widths beyond the other side (the unit wheel bound's proportions).
pub fn crown_half_width(radius: f32, width: f32) -> f32 {
    let (s, h) = (radius - 0.04, width * 0.5);
    ((6.4992 * h) * (6.4992 * h) + s * s).sqrt() - 5.4992 * h + 0.04
}

impl VehTune {
    /// The car's set-up (0x82363210) moves the tuned centre of mass by how the car sits on its
    /// wheels: f = SuspensionOffset + tyre radius - the height its wheels were modelled at
    /// (each the mean of front and rear); lower than modelled (f < 0) it goes up a quarter of
    /// that, higher it comes down by all of it, never under 1 cm. The routine runs twice when a
    /// car is made: recorded heights are the tuning's plus twice the step on five vehicles
    /// (R8 0.09 -> 0.1125, S281 0.125 -> 0.1298, ZX-14 0.150 -> 0.1557, a lowered Range Rover
    /// 0.325 -> 0.3647).
    pub fn seat(&mut self, rest_height: f32) {
        let t = &mut self.t;
        let f = (t.wheel_front.suspension_offset + t.wheel_back.suspension_offset) * 0.5 + (t.wheel_front.radius + t.wheel_back.radius) * 0.5 - rest_height;
        for _ in 0..2 {
            t.s.com_y = (t.s.com_y - if f < 0.0 { 0.25 * f } else { f }).max(0.01);
        }
    }
    pub fn new(t: SimTune) -> VehTune {
        let e = &t.engine.s;
        let tr = &t.trans.s;
        let radii = match t.drivetrain_type {
            1 | 4 => [t.wheel_front.radius; 4],
            2 | 3 => [t.wheel_front.radius, t.wheel_front.radius, t.wheel_back.radius, t.wheel_back.radius],
            _ => [t.wheel_back.radius; 4],
        };
        let count = if matches!(t.drivetrain_type, 2 | 3) { 4 } else { 2 };
        let ratios = super::transmission::ratios(&t.trans, e.max_rpm * RPM, &radii[..count]);
        let top_speed = tr.high_gear_mph * MPH;
        let gravity = 9.8 * if t.s.bound_gravity > 0.1 { t.s.bound_gravity } else { 1.0 };
        let torque_curve = (t.engine.torque.len() >= 3).then(||
            super::engine_curve::Curve::new(&t.engine.torque, e.idle_rpm * RPM, e.max_rpm * RPM, false));
        let idle_curve = (t.engine.idle.len() >= 3).then(||
            super::engine_curve::Curve::new(&t.engine.idle, e.idle_rpm * RPM, e.max_rpm * RPM, true));
        let inertia_yaw = {
            let pick = |tuned: f32, size: f32| if tuned > 0.1 { tuned } else { size.max(0.8) };
            let (x, z) = (pick(t.s.inertia_x, t.s.size_x), pick(t.s.inertia_z, t.s.size_z));
            t.s.mass / 12.0 * (x * x + z * z)
        };
        let shape = |w: &mcla_formats::rage::simtune::WheelTune| std::sync::Arc::new(super::hull::wheel_shape(w.radius.max(0.1), (w.width * 0.5).clamp(0.03, 0.25)));
        let wheel_shape = [shape(&t.wheel_front), shape(&t.wheel_back)];
        // (a width a set-up gave - the tuning files' own TireWidth is a placeholder of 0.1)
        let fitted = |w: f32| if w >= 0.15 { w } else { 0.0 };
        let tyre_width = [fitted(t.wheel_front.width), fitted(t.wheel_back.width)];
        let mut vt = VehTune { t, ratios, top_speed, gravity, bike: false, shift: Vec::new(), sync_torque: 0.0, inertia_yaw, tyre_width, wheel_shape, torque_curve, idle_curve };
        (vt.shift, vt.sync_torque) = vt.shift_points();
        vt
    }

    /// What holds the car back at speed `v` on the ground, per unit of mass: the air (DragMax)
    /// and the shafts' drag. The calibration tables leave both out.
    pub fn resistance(&self, v: f32) -> f32 {
        let t = &self.t;
        let top = self.top_speed.max(10.0);
        let all = matches!(t.drivetrain_type, 2 | 3);
        let shafts = t.powertrain.ang_drag.max(0.0) / t.wheel_back.radius.max(0.1) + if all || self.bike { 0.0 } else { t.freetrain.ang_drag.max(0.0) / t.wheel_front.radius.max(0.1) };
        t.aero.drag_max.max(0.0) * (v / top) * (v / top) + shafts / t.s.mass.max(1.0)
    }

    /// For each gear the engine speed at which the next gear gives the same power (searched
    /// between the best and the highest engine speed), pulled down by the tuning's biases; the
    /// way down sits under the speed the engine falls to after that shift.
    fn shift_points(&self) -> (Vec<[f32; 4]>, f32) {
        let e = &self.t.engine.s;
        let tr = &self.t.trans.s;
        let n = self.ratios.len() - 1;
        let max_rpm = (e.max_rpm * RPM) * super::transmission::RPM_FROM_SPEED;
        let mut out = vec![[max_rpm, max_rpm, 0.0, 0.0]; n + 1];
        out[0] = [0.0; 4];
        let power = |rpm: f32| {
            let speed = rpm * RPM;
            (self.torque_at(speed) * speed) * 0.001_818_181_8
        };
        let (best, sync_torque) = if let Some(curve) = &self.torque_curve {
            let (speed, torque) = curve.peak();
            (speed * super::transmission::RPM_FROM_SPEED, torque)
        } else { ((e.opt_rpm * RPM) * super::transmission::RPM_FROM_SPEED, self.torque_at(e.opt_rpm * RPM)) };
        for g in 1..n {
            let step = self.ratios[g + 1] / self.ratios[g];
            let (mut lo, mut hi) = (best, max_rpm);
            while hi - lo > 1.0 {
                let mid = (lo + hi) * 0.5;
                if power(mid) >= power(mid * step) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let at = (lo + hi) * 0.5;
            out[g][0] = (1.0 - tr.upshift_bias) * at;
            out[g][1] = (1.0 - tr.upshift_bias_low) * at;
            out[g + 1][2] = (1.0 - tr.downshift_bias_min) * at * step;
            out[g + 1][3] = (1.0 - tr.downshift_bias_max) * at * step;
        }
        (out, sync_torque)
    }

    /// Engine torque at full throttle (the control values, a smooth curve through them).
    pub fn torque_at(&self, w: f32) -> f32 {
        let e = &self.t.engine.s;
        if let Some(curve) = &self.torque_curve {
            return curve.evaluate(w);
        }
        // no control values (the ambient cars): the engine's built-in curve around the
        // speed of best power, shaped with the golden ratio
        let (opt, max) = (e.opt_rpm * RPM, e.max_rpm * RPM);
        let (a, b) = (0.618_034, 1.618_034);
        let p = e.max_hp * 550.0;
        if w <= opt {
            (a * opt + w) * (b * opt - w) / (opt * opt * opt) * p
        } else if w <= max {
            ((max + w - 2.0 * opt) * (a * opt + w) * (b * opt - w) * (max - w) / (opt * opt * opt * (max - opt) * (max - opt)) * p).max(0.0)
        } else {
            0.0
        }
    }

    fn idle_torque_at(&self, w: f32) -> f32 {
        let e = &self.t.engine.s;
        if let Some(curve) = &self.idle_curve {
            return curve.evaluate(w);
        }
        let opt = e.opt_rpm * RPM;
        (e.idle_rpm * RPM - w).signum() * (e.max_hp * 550.0 / opt) * (w / opt)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WheelSim {
    /// suspension compression from rest (m, positive = compressed)
    pub comp: f32,
    /// how far the wheel's top leans in towards the car (radians): what its collision shape
    /// stands at between ticks
    pub camber: f32,
    /// the wheel's matrix among the car's parts as the last update left it (rows axle, up,
    /// way; place) - what its collision shape stands at until the next (0x8255CEA0)
    pub part: Option<[Vec3; 4]>,
    /// wheel byte +530: its last contact stood deeper than SuspensionLimit and went to the solver
    pub bottomed: bool,
    /// the wheel's own matrix in the world as the last update left it (rows axle, up, way;
    /// place - wheel +16..+64): steered, lifted by its compression, leaned
    pub world: Option<[Vec3; 4]>,
    /// tyre displacement (m) along the wheel's lateral and forward directions
    pub disp_lat: f32,
    pub disp_long: f32,
    /// how fast the suspension is being compressed (m/s)
    pub comp_rate: f32,
    /// how far the tyre is into sliding (0..1, 0.5 = at its best slip)
    pub slide: f32,
    pub load: f32,
    pub f_lat: f32,
    pub f_long: f32,
    pub slip_lat: f32,
    pub slip_long: f32,
    /// Last tyre update: wheel +296 and +548 in the original.
    pub velocity_long: f32,
    /// (wheel +292: the contact's speed sideways, as the last tyre update had it)
    pub velocity_lat: f32,
    /// (the contact record the last update worked with: point and normal)
    pub contact_in: [Vec3; 2],
    /// (wheel +508: the damping the wheel handed the body in the last update)
    pub damp_value: f32,
    pub tyre_torque: f32,
    /// Last assigned wheel speed, positive forward in this port.
    pub spin_rate: f32,
    /// The road's roughness under this wheel (vehWheel 0x82565B90): the height the surface adds
    /// to the suspension's compression right now (+436), how far along the current bump the car
    /// is (+460) and this bump's random share of height and length (+452, +456).
    pub bump: f32,
    pub bump_phase: f32,
    pub bump_height: f32,
    pub bump_width: f32,
    /// what the wheel stood on at its last update (+464; None in the air)
    pub ground: Option<u8>,
    /// wheel byte +529 (it had a contact record at its last update) and the record's frame as
    /// that update left it: +352 (its forwards, which points to the car's rear) and +320 (sideways)
    pub record: bool,
    pub back_then: Vec3,
    pub side_then: Vec3,
    /// wheel +320..+383: sideways, normal, forwards, point - or the wheel's own matrix (see where it is set)
    pub frame: [Vec3; 4],
    /// the contact this wheel had in the last tick: the surface's normal and n.p, and how
    /// deep the wheel's shape stood in it
    pub kept: Option<(Vec3, f32, f32)>,
}

/// mcBikeGyro's state (gyro +16..+120): stunt state (0, 1 a wheelie, 99 ending; -1 a stoppie,
/// -99 ending), the roll asked for and the last roll error, the handbrake's level, the turning
/// rate asked for and its last error, velocity and acceleration low-passed, the lean's share
/// and the sideways pull low-passed, the stunt's boost time, pitch asked for and last pitch
/// error, the burnout's turning rate.
#[derive(Clone, Copy, Debug, Default)]
pub struct BikeGyro {
    pub state: i16,
    /// gyro +18: with the button held, which way the turn asks (1, -1), else 0
    pub intent: i16,
    pub roll: f32,
    pub roll_err: f32,
    pub level: f32,
    pub yaw_target: f32,
    pub yaw_err: f32,
    pub vel: Vec3,
    pub acc: Vec3,
    pub share: f32,
    pub lat: f32,
    pub boost: f32,
    pub pitch: f32,
    pub pitch_err: f32,
    pub burn: f32,
    /// gyro +40: the wobble's clock, -1 while it is off
    pub wobble: f32,
}

/// mcBikeGyro's T[+84] (0x827D7554): the main loop's global time scale, never under 0.1 - set by
/// the game each frame (f32 bits; 1.0 by default).
pub static TIME_SCALE84: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// mcCarSim's rev limiter (chassis +736, +740, +744): the throttle it allows, how long the engine has
/// stood past RevLimitMax, and whether it is cutting.
#[derive(Clone, Copy, Debug, Default)]
pub struct RevLimit {
    pub cut: f32,
    pub timer: f32,
    pub on: bool,
    /// (replays: what the last update left in chassis +136, and the pad's own throttle)
    pub left: f32,
    pub pad: f32,
}

/// What mcCarSim::Update's burnout rules (0x8232F7C0..0x8232FD7C) decided this tick, for the
/// parts of the tick that follow it.
#[derive(Clone, Copy, Debug, Default)]
pub struct BurnTick {
    /// the rules ran in the original's place this tick (cars; MCLA_DBG_Y6: the port's older block)
    pub exact: bool,
    /// chassis +578 as the tick came in (what re-arms the engine's boost, 0x8232F170)
    pub state_in: u8,
    /// transmission +33: held in its gear
    pub held: bool,
    /// charging: the drive layout as the rules found it - the wheels it does not turn are braked
    pub brake_layout: Option<u8>,
    /// charging: the centre of mass goes to (x, 0, this)
    pub cg: Option<f32>,
    /// the centre of mass may go back along the car this tick (0x8232F62C)
    pub cg_return: bool,
    /// launching: the push's size before the mass (0x8232FC64), and the drive layout it finds
    pub launch: Option<f32>,
    pub launch_layout: u8,
}

/// Hydraulics (mcCarHydraulics): each wheel's ram is told to go up, to the middle or down and
/// moves there at the tuned rates; the wheel buttons hop a corner (a tap) or hold it up (a long
/// press, until the next one).
#[derive(Clone, Copy, Debug, Default)]
pub struct Hydraulics {
    pub on: bool,
    buttons: u8,
    /// what a button asked for (1 up, 2 down), waiting for the ram to stand still
    asked: [u8; 4],
    queued: [u8; 4],
    /// the ram's order: 1 up, 0 middle, -1 down
    pub order: [i8; 4],
    /// the ram's length against the stock ride height (m)
    pub height: [f32; 4],
    press_time: [f32; 4],
    settle: [f32; 4],
    moving: [bool; 4],
    latched: [bool; 4],
    /// for the sounds: rams that started up / down / came to rest, and the switch (cleared by the listener)
    pub ev_rise: u8,
    pub ev_drop: u8,
    pub ev_end: u8,
    pub ev_switch: i8,
}

impl Hydraulics {
    fn step(&mut self, t: &mcla_formats::rage::simtune::HydraulicsTune, input: u8, dt: f32) {
        use super::car::{HYD_FITTED, HYD_ON};
        let fitted = input & HYD_FITTED != 0;
        let want_on = fitted && input & HYD_ON != 0;
        if want_on && !self.on {
            // switched on: every ram goes up
            let h = self.height;
            *self = Hydraulics { on: true, height: h, order: [1; 4], moving: [true; 4], ev_switch: 1, ..Default::default() };
        } else if !want_on && self.on {
            self.on = false;
            self.order = [0; 4];
            self.moving = [true; 4];
            self.ev_switch = -1;
        }
        let any_moving = self.moving.iter().any(|m| *m);
        if !self.on && !any_moving {
            self.buttons = 0;
            return;
        }
        // the wheel buttons
        if self.on {
            for i in 0..4 {
                let (now, was) = (input >> i & 1 != 0, self.buttons >> i & 1 != 0);
                if now && !was {
                    if self.latched[i] {
                        self.asked[i] = 2;
                        self.latched[i] = false;
                    } else {
                        if self.asked[i] != 1 {
                            self.press_time[i] = t.latch_time;
                        }
                        self.asked[i] = 1;
                        self.latched[i] = true;
                    }
                } else if !now && was {
                    // let go late: the corner stays up until the next press
                    if self.press_time[i] < 0.0 {
                        self.latched[i] = true;
                        self.asked[i] = 0;
                    } else {
                        self.asked[i] = 2;
                        self.latched[i] = false;
                    }
                }
            }
            self.buttons = input & 0x0f;
        }
        // orders change only while the ram stands still
        for i in 0..4 {
            match self.order[i] {
                -1 if !self.moving[i] && self.queued[i] == 1 => {
                    self.order[i] = 1;
                    self.moving[i] = true;
                    self.queued[i] = 0;
                    self.ev_rise |= 1 << i;
                }
                0 if !self.moving[i] => {
                    self.order[i] = -1;
                    self.moving[i] = true;
                }
                1 if !self.moving[i] && self.queued[i] == 2 => {
                    self.order[i] = -1;
                    self.moving[i] = true;
                    self.queued[i] = 0;
                    self.ev_drop |= 1 << i;
                }
                _ => {}
            }
        }
        for i in 0..4 {
            if self.asked[i] != 0 && !self.moving[i] {
                self.queued[i] = self.asked[i];
                self.asked[i] = 0;
                self.moving[i] = true;
            }
            if self.latched[i] {
                self.press_time[i] -= dt;
            } else {
                self.press_time[i] = 0.0;
            }
        }
        // the rams
        for i in 0..4 {
            let (rise, drop) = if i < 2 { (t.front_rise, t.front_drop) } else { (t.back_rise, t.back_drop) };
            let h = &mut self.height[i];
            let was_moving = self.moving[i];
            let mut towards = |h: &mut f32, target: f32, rate: f32| {
                *h = if *h < target { (*h + rate * dt).min(target) } else { (*h - rate * dt).max(target) };
            };
            let (target, rate) = match self.order[i] {
                1 => (rise, t.rate_rise),
                -1 => (-drop, t.rate_drop),
                _ => (0.0, if *h < 0.0 { t.rate_rise } else { t.rate_drop }),
            };
            if (*h - target).abs() > 1e-6 {
                towards(h, target, rate.max(0.05));
                self.moving[i] = true;
                self.settle[i] = 0.0;
            } else if self.order[i] != 0 && self.settle[i] < 0.2 {
                // (at its stop the ram counts as moving for a fifth of a second more)
                self.moving[i] = true;
                self.settle[i] += dt;
            } else {
                self.moving[i] = false;
                if was_moving {
                    self.ev_end |= 1 << i;
                }
            }
        }
    }
}

/// Engine-side gear-change latch (vehEngine +44/+48, transmission +24).
#[derive(Clone, Copy, Debug, Default)]
pub struct GearChange {
    /// +1 upshift, -1 downshift, zero when the transmission can shift again.
    pub direction: i8,
    pub remaining: f32,
    pub active: bool,
}

impl GearChange {
    fn start(&mut self, old: i32, new: i32, gcl: f32) {
        if old != new {
            self.direction = if new > old { 1 } else { -1 };
            self.remaining = gcl.max(0.0);
            self.active = true;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn update(&mut self, throttle: f32, gear: i32, forward_gears: usize, ratio: f32,
        engine_w: f32, shaft_w: f32, tune: &mcla_formats::rage::simtune::EngineScalars,
        match_rpm: f32, dt: f32) -> f32 {
        if !self.active {
            return throttle;
        }
        let mut pedal = throttle;
        // sub_82559A70: the shift into first/reverse does not alter the pedal.
        if self.remaining > 0.0 {
            if self.direction > 0 && gear > 1 {
                let blend = ((tune.gcl - self.remaining) * 10.0).clamp(0.0, 1.0);
                pedal = blend.mul_add(tune.upshift_throttle - pedal, pedal);
            } else if self.direction < 0 && gear > 1 {
                pedal = pedal.max(tune.downshift_throttle);
            }
            self.remaining = (self.remaining - dt).max(0.0);
        }
        if self.remaining <= 0.0 {
            self.active = false;
            self.direction = 0;
        } else if forward_gears > 1 && match_rpm > 0.0 {
            // Exact constant at 0x8207D3E8 in the original executable.
            let tolerance = ratio * match_rpm * 0.104_719_76;
            let target = (ratio * shaft_w).abs() + tolerance;
            let matched = (self.direction > 0 && gear > 1 && engine_w <= target)
                || (self.direction < 0 && gear >= 1 && engine_w >= target);
            if matched {
                // Original clears the timer now and the transmission latch next update.
                self.remaining = 0.0;
            }
        }
        pedal
    }
}

/// Surface grip of each wheel forced by `physics::replay` (bits of an f32; 0 = the ground's own).
pub static REPLAY_SURFACE: [std::sync::atomic::AtomicU32; 4] = [const { std::sync::atomic::AtomicU32::new(0) }; 4];
/// The roughness height under each wheel forced by `physics::replay` (bits of an f32), used
/// while `REPLAY_BUMP_ON` is set: the original draws its bumps from the game's shared random
/// numbers, which a replay cannot repeat.
pub static REPLAY_BUMP: [std::sync::atomic::AtomicU32; 4] = [const { std::sync::atomic::AtomicU32::new(0) }; 4];
pub static REPLAY_BUMP_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub static REPLAY_STEER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub static REPLAY_AI: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// experiment: the motorcycles' turn and lean left out
pub static REPLAY_NO_BIKE_GYRO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// R2, a replay's: the damage object's frame update ahead of the tick - 0 none, else the thrown rider's term + 1
pub static REPLAY_RIDER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// R3, a replay's: the tick's island holds a jointed body (the thrown rider) - its solves sweep eight times
pub static REPLAY_JOINTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub static REPLAY_OTHERS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static REPLAY_OTHERS_E: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static REPLAY_ORDER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static REPLAY_RIDER_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// which way a replay's gearbox goes: 1 forward, -1 reverse, 0 the port decides
pub static REPLAY_WAY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// RB3, a replay of a recording that logged the pad: the pad's frame that follows this tick's update has
/// the nitrous button down (the fire rule itself is the port's, `nitro_can_fire`).
pub static REPLAY_NITRO_PAD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// a replay's: the gearbox's way as the tick's physics step finds it (the pad code has run in between)
pub static REPLAY_WAY_AFTER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// 0x82007FA8 (mcCarSim::Update 0x8232EFB4)
const BIKE_BURN_STEER: f32 = 0.9;
/// Z3: motorcycles go through the cars' exact code (wheel rows about the fork's axis, suspension,
/// tyres, sums, integrator, collision) and mcBikeGyro in the original's place and operations.
/// MCLA_DBG_Z3: the port's older, approximate bike path (its own suspension arithmetic, the gyro
/// ahead of the wheels, four substeps).
pub fn bike_old(vt: &VehTune) -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    vt.bike && *V.get_or_init(|| std::env::var_os("MCLA_DBG_Z3").is_some())
}

/// 0x82333C2C: the gear is not reverse (neutral counts) as the contact callbacks find it
fn gear_not_reverse(gear: i32, way_after: i32) -> bool {
    if dbg_flag("MCLA_DBG_Y2") { return gear > 0; }
    match REPLAY_WAY_AFTER.load(std::sync::atomic::Ordering::Relaxed) { 0 => if way_after != 0 { way_after > 0 } else { gear >= 0 }, w => w > 0 }
}

/// The body as the original keeps it, to the bit: its matrix rows, the model's origin and the
/// body's own place, its momenta (the velocities are derived from them), the count of moves
/// since its rows were last made square; and the car's pose and velocities these go with - if
/// anything else has moved the car since, they are made afresh from those.
#[derive(Clone, Copy, Debug)]
pub struct BodyExact {
    pub rows: [Vec3; 3],
    pub origin: Vec3,
    pub at: Vec3,
    pub p: Vec3,
    pub l: Vec3,
    pub count: u32,
    pub snap: (Vec3, Quat, Vec3, Vec3),
    /// the matrix the tick's collision saw - rows and origin as they stood before the push
    /// solve moved the body out of what it stood in. The wheels' contacts are of that pose.
    pub seen: [Vec3; 4],
}

#[derive(Clone, Debug)]
pub struct VehState {
    /// engine speed (rad/s)
    pub engine_w: f32,
    /// -1 reverse, 0 neutral, 1.. forward gears
    pub gear: i32,
    pub shift_timer: f32,
    pub gear_change: GearChange,
    /// Engine pedal after the gear-change envelope for this simulation frame.
    pub engine_pedal: f32,
    /// Torque evaluated by engine Update, before automatic transmission Update.
    pub engine_torque: f32,
    /// the turning acceleration about the car's length the engine gives the body this tick (0x82559F80)
    pub engine_roll: f32,
    pub engine_auxiliary: f32,
    /// Original transmission +32; independent of shaft engine binding.
    pub trans_inhibited: bool,
    /// how hard the pedal has been pressed lately (0..1), for shifting up and for shifting down
    pub shift_mod: [f32; 2],
    /// the pedal last frame (for the kick-down)
    pub pedal_was: f32,
    pub hyd: Hydraulics,
    /// spin of the powered and of the free wheels (rad/s, positive rolling forward)
    pub omega: [f32; 2],
    /// Differential: left wheels turn at omega * diff, right at omega / diff.
    pub diff: [f32; 2],
    pub wheels: [WheelSim; 4],
    pub boost_timer: f32,
    /// smoothed steering (-1..1)
    pub steer: f32,
    pub nitro_timer: f32,
    /// the turn gyro's last error (for its damping term) and its force this frame (N, to the right at the front axle)
    pub turn_err: f32,
    pub turn_force: f32,
    /// airborne: roll angle last frame, and the levelling / steering accelerations
    pub air_roll: f32,
    /// two-wheel driving (mcCarGyro's lean): the side that is up (1 right, -1 left, 0 off), the
    /// roll it is heading for, the error of the last frame, frames until it may start again,
    /// and the roll acceleration it asks for
    pub two: i8,
    pub two_target: f32,
    two_err: f32,
    two_trim: f32,
    two_wait: u8,
    two_acc: f32,
    pub air_acc: [f32; 3],
    /// how long the car has stood still with no pedal (chassis +588), and the speed it came
    /// into its last tick with (chassis +224)
    pub prev_up: Option<Vec3>,
    /// (a replay's: the body's matrix to the bit - rows and place - for the coming tick)
    pub body_exact: Option<[Vec3; 4]>,
    /// a replay's: the pose the recorded tick's collision saw (`BodyExact::seen`), where the forced state's
    /// body was pushed after it
    pub seen_exact: Option<[Vec3; 4]>,
    /// (the body's matrix as a tick left it, rows and place, where it is kept to the bit)
    pub body_now: Option<[Vec3; 4]>,
    /// (a replay's: each wheel's place of rest, pivot arm and offset as the recorded car has
    /// them - wheel +384, +416, +432)
    pub rest_exact: Option<[[f32; 5]; 4]>,
    /// (a replay's: where the body the original integrates is, to the bit, for the coming tick)
    pub body_at_exact: Option<Vec3>,
    /// (the force and torque sums as the tick had them before the gyro and after it; and a
    /// replay's: the recorded sums after the gyro, to stand in for this port's)
    pub sums_gyro: [Vec3; 4],
    pub gyro_sums_exact: Option<[Vec3; 2]>,
    /// (a replay's: the centre of mass in the car's frame as the recorded car has it, chassis +112)
    pub com_exact: Option<Vec3>,
    /// the body carried from tick to tick in the original's own numbers
    pub body: Option<BodyExact>,
    /// (a replay's: the recorded body's momenta and its count of moves, body +304, +320, +664)
    pub momentum_exact: Option<(Vec3, Vec3, u32)>,
    /// the body's contacts' multipliers from the last tick (the solver's warm start)
    pub hull_kept: Vec<super::hull::Kept>,
    /// the hull's contacts with the city as the original keeps them from tick to tick
    pub manifolds: Vec<super::hull::Manifold>,
    /// stage P3: the rider is off (chassis +592 bit 0x10): the damage object's timer (+44) and the ticks since
    pub wipe: Option<(f32, u32)>,
    /// R4: the direction saved as the rider came off (chassis +608), and whether the car was set down
    /// afresh in the tick before - its pairs are made again only in the step after (no wheel contact)
    pub wipe_dir: Vec3,
    pub set_down: bool,
    /// stage P: the props and pieces the car has had to do with
    pub loose: super::loose::LooseState,
    /// the body's "last" matrix for the collision (collider +96): where the last tick left it,
    /// unless that tick moved it out of something deep (bytes +659, +660)
    pub hull_last: Option<[Vec3; 4]>,
    /// the ball the city's grid picked the car's cells by at the last collide (centre, radius)
    pub cell_ball: Option<(Vec3, f32)>,
    /// this tick's burnout decisions
    pub by: BurnTick,
    pub rev: RevLimit,
    /// the body's last matrix as the last collide took it (rows and the model's origin)
    pub last_seen: Option<[Vec3; 4]>,
    /// gyro +80: the car's pitch as the gyro last saw it - asin of its length's height, or the launch's lift error
    pub lift_err: f32,
    /// body +416 between ticks: a push's turn too small to have been made
    pub push_turn_left: Vec3,
    pub deep_count: u8,
    pub last_set: bool,
    /// mcCarStuck (car +44): 0 idle, 1 armed by a contact of the body, 2 turning the car;
    /// where the car was when it was armed; and how long since
    pub stuck: u8,
    pub stuck_at: Vec3,
    pub stuck_timer: f32,
    pub still_timer: f32,
    pub last_speed: f32,
    /// the drift thrust's level (gyro +56)
    pub drift_level: f32,
    /// chassis +572: the drive shaft's speed at its first wheel's radius as the tick came in -
    /// taken before the tyres' melt may switch an all-wheel-drive car's layout
    pub shaft_speed_in: f32,
    /// the surface's grip under each wheel as it last touched (wheel +444)
    pub wheel_grip: [f32; 4],
    /// tyre melt 0..1: driven tyres spun up in the low gears lose their sliding grip (mcCarTireFric)
    pub melt: f32,
    /// Which wheels the engine turns right now (chassis +372): 0 the rear, 1 the front, 2 and 3
    /// all four, 255 = not set yet (the tuning's DrivetrainType). An all-wheel-drive car is put
    /// on one axle while its tyres are melted or a burnout runs (type 2 to the rear, 3 to the front).
    pub drive: u8,
    /// slipstream turbo running: 1 -> 0 over turboUseTime
    pub sst_level: f32,
    /// motorcycles: the lean the rider holds (rad, positive as the yaw rate times speed), the
    /// filtered sideways acceleration behind it
    pub lean: f32,
    pub lean_acc: f32,
    /// wheelie: 0 none, 1 up, 99 coming down; boost time left; the pitch aimed for; last error
    pub wheelie: u8,
    pub wheelie_boost: f32,
    pub wheelie_pitch: f32,
    pub wheelie_err: f32,
    pub wheelie_lift: f32,
    /// engine torque multiplier from the gyro's DriftTorque (more pull while steering or sliding)
    pub torque_mult: f32,
    /// the engine is connected to the wheels (it lets go near standstill off the throttle)
    pub clutch: bool,
    /// brake the car applies by itself when it rolls slowly off the throttle (0.25 rising to 1)
    pub auto_brake: f32,
    /// the state has been matched to the car's speed (a car put on the road already moving)
    pub started: bool,
    /// nitrous shot: 1 when fired, running down to 0 over NitroBoostTime
    pub nitro_level: f32,
    nitro_held: bool,
    /// mcCarNitro +32: a shot is burning
    pub nitro_burning: bool,
    nitro_records: usize,
    /// chassis +584, the air time as the chassis update keeps it (negative in the tick of a landing), and the value of the tick before
    air584: f32,
    air584_before: f32,
    nitro_rest: f32,
    /// RB3, mcCarNitro +24: the view's widening - towards NitroFov while a shot burns (in NitroFovOutTime),
    /// back to +20 (nothing) in NitroFovInTime after it; no shot fires until it is back
    pub nitro_view: f32,
    /// RB3, a replay: the pad's own pedal while the update forces the chassis's, and whether the next
    /// sample's 1 is what this update left
    nitro_pad: f32,
    nitro_left: bool,
    /// scale of the aerodynamic angular damping (less in the air, more just after landing)
    pub aero_damp: f32,
    air_time: f32,
    /// burnout: 0 none, 1 charging on the handbrake, 2 full, 3 launching; and its charge (0..1)
    pub burn_state: u8,
    pub burn: f32,
    /// yaw rate the burnout is swinging the car at
    burn_yaw: f32,
    /// how far along the road's current bump the car is (vehChassis +548)
    pub bump_phase: f32,
    /// random numbers of the bumps (the original's generator: multiply with carry, 0x5CDCFAA7)
    rng: [u32; 2],
    gauss_spare: Option<f32>,
    /// the force and torque (about the centre of mass, world axes) of the last tick, for
    /// `physics::replay` to lay beside a recording's
    pub last_force: Vec3,
    pub last_torque: Vec3,
    /// step 19f: a Pulse power-up's push waiting for the next tick (sub_82380418): the force per
    /// unit of mass and the torque per unit of the body's own inertia, world axes
    pub pulse: Option<(Vec3, Vec3)>,
    pub last_gyro: Vec3,
    /// where the centre of mass is right now (height, along the car): a burnout moves it
    pub cg: [f32; 2],
    cg_set: bool,
    /// motorcycles (mcBikeGyro): the body's velocity and acceleration, low-passed (gyro +64,
    /// +80); the share of the balancing angle the rider leans to (+96), the sideways
    /// acceleration it is taken from (+100) and the roll asked of the body (+20); and this
    /// tick's turn of the body about the tyres' line with the shift of its middle that goes
    /// with it (the collider's turn and push)
    bike_vel: Vec3,
    bike_acc: Vec3,
    lean_scale: f32,
    lean_lat: f32,
    pub lean_target: f32,
    bike_turn: Vec3,
    bike_push: Vec3,
    /// the chassis's speed factor of the steering (+568)
    sss_chassis: f32,
    /// a replay's: the pad's steering of the frame (the input object's), and what the last update left in chassis +132
    pub pad_steer: f32,
    /// each wheel's static weight (wheel +428) as the last tick had it
    pub w_then: [f32; 4],
    /// Z3: mcBikeGyro's own state, and what its update leaves for the physics step: the turning
    /// rate it set (body vfunc 43 -> +288 and the angular momentum) and the turn and push it
    /// queued (body +416, +400)
    pub bg: BikeGyro,
    pub bike_spin: Option<Vec3>,
    pub bike_pending: Option<(Vec3, Vec3)>,
    /// chassis +568 as the last update left it (the pad's code reads it)
    pub sss_chassis_exact: f32,
    /// normal play: the gear the pad's code has chosen after this tick's update (what the
    /// physics step's contact callbacks find), 0 = as the update left it
    pub way_after: i32,
    /// A2: the player's pad between frames (pedals, the next steering value, the smoothed stick)
    pub pad: super::pad::PadHeld,
    pub steer_back: f32,
    /// N-VP-1: the drivetrain's reset seen at the end of a computer driver's opening grid burst (engine to
    /// idle, wheels and shafts still, no burnout charge, the drive layout and centre of mass as tuned), done at
    /// the head of the next update
    pub grid_reset: bool,
}

impl Default for VehState {
    fn default() -> Self {
        VehState { shaft_speed_in: 0.0, wipe_dir: Vec3::ZERO, set_down: false, prev_up: None, body_exact: None, seen_exact: None, body_now: None, rest_exact: None, body_at_exact: None, sums_gyro: [Vec3::ZERO; 4], gyro_sums_exact: None, com_exact: None, body: None, momentum_exact: None, drift_level: 1.0, wheel_grip: [1.0; 4], hull_kept: Vec::new(), manifolds: Vec::new(), wipe: None, loose: Default::default(), hull_last: None, cell_ball: None, by: BurnTick { exact: false, state_in: 0, held: false, brake_layout: None, cg: None, cg_return: true, launch: None, launch_layout: 0 }, lift_err: 0.0, rev: RevLimit { cut: 0.0, timer: 0.0, on: false, left: 0.0, pad: 0.0 }, last_seen: None, push_turn_left: Vec3::ZERO, deep_count: 0, last_set: false, stuck: 0, stuck_at: Vec3::ZERO, stuck_timer: 0.0, still_timer: 0.0, last_speed: 0.0, drive: 255, engine_w: 100.0, gear: 1, shift_timer: 0.0, gear_change: GearChange::default(), engine_pedal: 0.0, engine_torque: 0.0, engine_roll: 0.0, engine_auxiliary: 0.0, trans_inhibited: false, shift_mod: [1.0; 2], pedal_was: 0.0, hyd: Hydraulics::default(), two: 0, two_target: 0.0, two_err: 0.0, two_trim: 0.0, two_wait: 0, two_acc: 0.0, omega: [0.0; 2], diff: [1.0; 2], wheels: [WheelSim::default(); 4], boost_timer: 0.0, steer: 0.0, nitro_timer: 0.0, turn_err: 0.0, turn_force: 0.0, melt: 0.0, sst_level: 0.0, lean: 0.0, lean_acc: 0.0, wheelie: 0, wheelie_boost: 0.0, wheelie_pitch: 0.0, wheelie_err: 0.0, wheelie_lift: 0.0, air_roll: 0.0, air_acc: [0.0; 3], torque_mult: 1.0, clutch: true, auto_brake: 0.0, started: false, nitro_level: 0.0, nitro_held: false, nitro_burning: false, nitro_records: 0, air584: 0.0, air584_before: 0.0, nitro_rest: 0.0, nitro_view: 0.0, nitro_pad: 0.0, nitro_left: false, aero_damp: 1.0, air_time: 0.0, burn_state: 0, burn: 0.0, burn_yaw: 0.0, bump_phase: 0.0, rng: [0x2545_F491, 0x1234_5678], gauss_spare: None, last_force: Vec3::ZERO, last_torque: Vec3::ZERO, pulse: None, last_gyro: Vec3::ZERO, cg: [0.0; 2], cg_set: false, bike_vel: Vec3::ZERO, bike_acc: Vec3::ZERO, lean_scale: 0.0, lean_lat: 0.0, lean_target: 0.0, bike_turn: Vec3::ZERO, bike_push: Vec3::ZERO, sss_chassis: 1.0, pad_steer: 0.0, w_then: [0.0; 4], bg: BikeGyro::default(), bike_spin: None, bike_pending: None, sss_chassis_exact: 1.0, way_after: 0, pad: super::pad::PadHeld { pedals: super::pad::Pedals { state: 0, throttle: 0.0, brake: 0.0, way: 1 }, steer_next: 0.0, stick_y: 0.0, stick_next: 0.0, second_tick: false, live: false, cruise: super::pad::CruiseHeld { on: false, speed: 0.0 }, button: false, handbrake: false }, steer_back: 0.0, grid_reset: false }
    }
}

impl VehState {
    /// 23 random bits (the original's generator at 0x82860890).
    fn random_bits(&mut self) -> f32 {
        let t = self.rng[0] as u64 * 0x5CDC_FAA7 + self.rng[1] as u64;
        self.rng = [t as u32, (t >> 32) as u32];
        (t as u32 & 0x7F_FFFF) as f32
    }

    /// A normally distributed number (0x821CE328: the polar method, the second value kept).
    fn random_normal(&mut self, mean: f32, deviation: f32) -> f32 {
        if let Some(v) = self.gauss_spare.take() {
            return v * deviation + mean;
        }
        loop {
            let a = self.random_bits() * (1.0 / 4_194_304.0) - 1.0;
            let b = self.random_bits() * (1.0 / 4_194_304.0) - 1.0;
            let s = a * a + b * b;
            if s < 1.0 && s != 0.0 {
                let f = (s.ln() * -2.0 / s).sqrt();
                self.gauss_spare = Some(f * a);
                return f * b * deviation + mean;
            }
        }
    }
}

/// slip of a velocity against the rolling speed, -1..1 (tapered below 1 m/s)
pub(super) fn slip(v: f32, reference: f32) -> f32 {
    if v == 0.0 {
        return 0.0;
    }
    let (av, ar) = (v.abs(), reference.abs());
    if av < ar {
        let s = v / ar;
        if ar < 1.0 {
            s * ar
        } else {
            s
        }
    } else {
        let s = v.signum();
        if av < 1.0 {
            s * av
        } else {
            s
        }
    }
}

/// Friction against slip: rises to the static value at the optimum slip, then falls
/// back to the sliding value. Also returns how far into sliding the tyre is.
pub(super) fn friction(wt: &WheelTune, sliding_fric: f32, surf: f32, s: f32) -> (f32, f32) {
    let opt = wt.optimum_slip.max(0.01);
    let mut s = s.abs();
    if sliding_fric == 0.0 && s > opt {
        s = opt;
    }
    // sub_825648C8 multiplies the cached inverse optimum-slip square before
    // multiplying slip. Reordering this expression changes tyre-force bits.
    let inverse_opt_squared = 1.0 / (opt * opt);
    let x = ((wt.static_fric * surf * opt.mul_add(2.0, -s)) * inverse_opt_squared) * s;
    if s <= opt {
        (x, 0.5 * s / opt)
    } else {
        (x.max(sliding_fric * surf), 0.5 + 0.5 * (s - opt) / (1.0 - opt).max(0.01))
    }
}

/// Move a tyre displacement by `step` unless that passes `limit` (signed); past it the
/// displacement eases to the limit by `relax`. Returns (applied step, still gripping).
pub(super) fn displace(disp: &mut f32, limit: f32, step: f32, relax: f32) -> (f32, bool) {
    let next = *disp + step;
    if step >= 0.0 {
        if limit >= next {
            *disp = next;
            (step, true)
        } else {
            *disp = limit.max(*disp - relax);
            (0.0, false)
        }
    } else if limit <= next {
        *disp = next;
        (step, true)
    } else {
        *disp = limit.min(*disp + relax);
        (0.0, false)
    }
}

pub(super) fn synchronize_engine(current: f32, target: f32, torque: f32, inertia: f32, dt: f32) -> f32 {
    // sub_82568F10 uses fmadds/fnmsubs, then selects the target directly when
    // reached. Adding a clamped delta loses precision at both operations.
    let rate = torque / inertia.max(0.01);
    if current < target {
        rate.mul_add(dt, current).min(target)
    } else if current > target {
        (-rate.mul_add(dt, -current)).max(target)
    } else {
        current
    }
}

// Original dispatchers run engine before the automatic transmission and shafts.
fn update_engine(vs: &mut VehState, vt: &VehTune, input: CarInput, pedal_released: (bool, bool), ratio: f32, dt: f32) {
    let e = &vt.t.engine.s;
    // (chassis +816, set on every car but the player's: its boost is never started - a recorded
    // police cruiser's timer stays 0 - and, +817, the automatic brake does not act on it)
    let computer = input.ai || REPLAY_AI.load(std::sync::atomic::Ordering::Relaxed);
    // mcCarSim::Update (0x8232F0D4..0x8232F1A0), ahead of the engine: while a gear above first is
    // being changed the boost is cancelled and the transmission held off (the engine's
    // sub_82558710, then transmission +32 = 1), likewise in neutral; the frame that hold ends,
    // the engine's boost starts afresh (sub_825586E8: timer = BoostDuration). And with the pedal
    // fully released the boost is re-armed every frame - so each time the driver gets back on
    // the throttle, and after every upshift, the engine pulls BoostTorque harder, fading over
    // BoostDuration. (Checked against the running original: timer 2.0 at rest, 1.983 on the
    // first frame of throttle and again after the change into second.)
    // (0x8232F098: on the grid - chassis +592 bit 0x2 - with the handbrake off and less than 0.95 of
    // burnout charge, 0x82007FB4, the transmission is held, +32 = 1, ahead of the rules below and
    // skipping them: the engine revs free against the held wheels and saws on the rev limiter. Recorded:
    // the player's throttle-only grid (a.2), the opponents' revving phases.)
    let grid_held = input.hold && vs.burn < 0.95 && !input.handbrake && !bike_old(vt);
    if grid_held {
        vs.trans_inhibited = true;
    } else if vs.gear_change.direction != 0 && vs.gear >= 2 {
        vs.boost_timer = 0.0;
        vs.trans_inhibited = true;
    } else if vs.gear == 0 {
        vs.trans_inhibited = true;
    } else if vt.bike && vs.gear < 0 && !bike_old(vt) && !dbg_flag("MCLA_DBG_A1") {
        // (0x8232F118: on two wheels the reverse gear is always held off - the engine never drives a
        // motorcycle backwards; the gyro's reverse drive pushes it)
        vs.trans_inhibited = true;
    } else {
        if vs.trans_inhibited && !computer {
            vs.boost_timer = e.boost_duration;
        }
        vs.trans_inhibited = false;
    }
    // (in reverse the brake is the pedal that drives)
    // (0x8232F170: and all through a burnout - it launches with the whole boost)
    let burn_then = if vs.by.exact { vs.by.state_in } else { vs.burn_state };
    if !computer && (burn_then == 1 || burn_then == 2 || if vs.gear < 0 { pedal_released.1 } else { pedal_released.0 }) {
        vs.boost_timer = e.boost_duration;
    }
    // (0x8232F2FC, after the rules above: the automatic brake holds the transmission - a car
    // standing still is held, which is why the first frame of throttle starts a whole boost)
    // (N-VP-1: on the grid, chassis +592 bit 0x2, the computer drivers' cars get the automatic brake and its hold
    // too - recorded: the opponents' transmission +32 = 1 and their wheels' +480 rising 45 a tick, 1389, 1434 ..,
    // in their idle and handbrake phases, c4 R8 k44.., a.1 k116..)
    if vs.auto_brake > 0.0 && (!computer || input.hold) {
        vs.trans_inhibited = true;
    }
    let pedal = if vs.gear < 0 { input.brake } else { input.throttle }.clamp(0.0, 1.0);
    // vehEngine::SetThrottle compares against the preceding envelope-adjusted pedal.
    let pedal_velocity = (pedal - vs.engine_pedal) * (1.0 / dt);
    if e.boost_throttle_threshold > 0.0 && pedal_velocity > e.boost_throttle_threshold && !computer {
        vs.boost_timer = e.boost_duration;
    }
    vs.engine_pedal = vs.gear_change.update(pedal, vs.gear, vt.ratios.len() - 1,
        ratio, vs.engine_w, vs.omega[0], e, vt.t.trans.s.gear_change_match_rpm, dt);
    let pedal = vs.engine_pedal;
    let torque = if pedal > 0.5 {
        let base = vs.torque_mult * vt.torque_at(vs.engine_w);
        let boost = if e.boost_duration > 0.0 { (vs.boost_timer / e.boost_duration) * e.boost_torque } else { 0.0 };
        ((pedal - 0.5) * 2.0) * (boost + base)
    } else {
        vt.idle_torque_at(vs.engine_w) * (-pedal.mul_add(2.0, -1.0))
    };
    let out = super::engine::update_prepared(super::engine::PreparedEngine {
        speed: vs.engine_w, torque, pedal, idle_speed: e.idle_rpm * 0.104_719_76,
        max_speed: e.max_rpm * 0.104_719_76, inertia: e.inertia, engage_rpm: if computer { e.idle_rpm } else { e.engage_rpm },
        ratio, shaft: -vs.omega[0], gear: if vs.gear < 0 { 0 } else { vs.gear + 1 },
        running: true, inhibited: vs.trans_inhibited, bound: vs.clutch,
        auxiliary: vs.engine_auxiliary, can_bind: true,
    }, dt);
    vs.engine_w = out.speed;
    vs.engine_torque = out.torque;
    // (the end of vehEngine::Update, 0x82559F80: out of gear - in neutral, or while the
    // transmission is held for a gear change - an engine turning faster than a quarter over
    // its idle speed rocks the body about its length against its own torque:
    // -(inertia * ((torque / greatest torque) * inertia)) rad/s^2. MCLA_DBG_NOENGINEROLL: none.)
    vs.engine_roll = 0.0;
    if (vs.gear == 0 || vs.trans_inhibited) && !bike_old(vt) && !dbg_flag("MCLA_DBG_NOENGINEROLL") {
        let share = if vt.sync_torque > 0.0 { (vs.engine_torque / vt.sync_torque) * e.inertia } else { 0.0 };
        if vs.engine_w > (e.idle_rpm * 0.104_719_76) * 1.25 {
            vs.engine_roll = -(e.inertia * share);
        }
    }
    vs.clutch = out.bound;
    vs.engine_auxiliary = out.auxiliary;
    if vs.boost_timer > 0.0 {
        vs.boost_timer = (vs.boost_timer - dt).max(0.0);
    }
}

/// The steering's factor for the car's speed along its length (0x8232D800, chassis +568):
/// 1 below SSSThresholdMin, SSSValue from SSSThresholdMax, eased between.
pub fn sss_factor(s: &mcla_formats::rage::simtune::SimScalars, speed: f32) -> f32 {
    if !(s.sss_threshold_max > 0.0) || speed < s.sss_threshold_min {
        return 1.0;
    }
    if speed < s.sss_threshold_max {
        let t = (speed - s.sss_threshold_min) / (s.sss_threshold_max - s.sss_threshold_min);
        super::exact::fma(super::exact::ease(t), s.sss_value - 1.0, 1.0)
    } else {
        s.sss_value
    }
}

/// mcCarTireFric::Update (0x82380BF8): the driven tyres' melt and, with it, an all-wheel-drive
/// car's switch to two driven wheels and back. `gear`: the gear the transmission's update of
/// this tick finds (the routine runs ahead of the engine and the transmission).
fn tire_fric_update(vs: &mut VehState, t: &SimTune, input: CarInput, gear: i32, dt: f32) {
    // ---- tyre melt (mcCarTireFric): in the low gears, at speed, a floored throttle on sliding
    // driven tyres melts them (MeltRate per second); they recover at RecoverRate ----
    // (before the gyro, as in the original: the gyro's share recorded in a tick, gyro +72, is
    // 1 - the melt that tick ends with)
    {
        let tf = &t.tire_fric;
        if tf.spinning_fric != 0.0 && tf.melt_rate != 0.0 && tf.skidding_threshold < 1.0 {
            // mcCarTireFric::Update (0x82380BF8). The slide is the worst of the wheels the
            // engine turns at the moment; the gear counts from first up to GearThreshold.
            let drive = vs.drive;
            let driven = |i: usize| match drive {
                0 => i >= 2,
                1 | 4 => i < 2,
                _ => true,
            };
            let slide = (0..4).filter(|i| driven(*i)).map(|i| vs.wheels[i].slide).fold(0.0f32, f32::max);
            // (the routine runs ahead of the transmission's update: the gear is the one the
            // tick came in with. MCLA_DBG_OLDMELT: the gear the tick ends in.)
            let melt_gear = gear;
            let eligible = melt_gear >= 1
                && melt_gear <= tf.gear_threshold_bits.to_bits() as i32
                && vs.burn <= 0.0
                && !(input.throttle < 0.5 && input.brake > 0.1)
                && !(input.throttle < 0.1 && input.handbrake)
                // (chassis +572: the drive shaft's speed of the tick before at its first wheel's
                // radius, not the road speed - a car spinning its wheels from rest melts them)
                && (vs.omega[0] * if drive == 0 { t.wheel_back.radius } else { t.wheel_front.radius }).abs() >= 5.0;
            let pedal = ((input.throttle - 0.5) * 2.0).clamp(0.0, 1.0);
            if eligible && pedal * slide > tf.skidding_threshold {
                vs.melt = tf.melt_rate.mul_add(dt, vs.melt).min(1.0);
            } else if vs.melt > 0.0 {
                // (not held at zero: the recovery ends a shade below it, which is what puts
                // an all-wheel-drive car back on four wheels)
                vs.melt = -(tf.recover_rate.mul_add(dt, -vs.melt));
            }
            if vs.melt > 0.0 {
                if t.drivetrain_type == 2 && vs.drive == 2 {
                    vs.drive = 0;
                } else if t.drivetrain_type == 3 && vs.drive == 3 {
                    vs.drive = 1;
                }
            } else if vs.burn <= 0.0 && vs.melt < 0.0 {
                if t.drivetrain_type == 2 && vs.drive == 0 {
                    vs.drive = 2;
                } else if t.drivetrain_type == 3 && vs.drive == 1 {
                    vs.drive = 3;
                }
            }
            // (0x8238135C: the routine ends by lifting a melt below zero back to it - after
            // the layout's switch has seen it. MCLA_DBG_OLDMELT: left below zero.)
            if vs.melt < 0.0 && !dbg_flag("MCLA_DBG_OLDMELT") {
                vs.melt = 0.0;
            }
        } else {
            vs.melt = 0.0;
        }
    }
}

/// The body's moments (body +160).
pub fn moments(p: &CarParams, t: &SimTune) -> Vec3 {
    let mass = t.s.mass;
    // (ambient cars carry no InertiaBox: their body is the box)
    let body = p.body_half * 2.0;
    let pick = |tuned: f32, size: f32| if tuned > 0.1 { tuned } else { size.max(0.8) };
    let b = Vec3::new(pick(t.s.inertia_x, body.x), pick(t.s.inertia_y, body.y), pick(t.s.inertia_z, body.z));
    // (the body's own numbers, body +160: the mass times the squares' sum, times a twelfth -
    // in that order; the three recorded cars' nine values agree to the bit)
    if dbg_flag("MCLA_DBG_OLDSUMS") {
        let m = mass / 12.0;
        Vec3::new(m * (b.y * b.y + b.z * b.z), m * (b.x * b.x + b.z * b.z), m * (b.x * b.x + b.y * b.y))
    } else {
        let k = 1.0f32 / 12.0;
        Vec3::new((mass * (b.y * b.y + b.z * b.z)) * k, (mass * (b.x * b.x + b.z * b.z)) * k, (mass * (b.x * b.x + b.y * b.y)) * k)
    }
}

/// The car as another car's step meets it in the shared level: its body (place +256, rows, momenta,
/// mass, moments) moved as a tick's pre-collide moves a body (0x8259E768: by the velocities it has),
/// the matrix its last collide had, and the forces its last update left. With its place offset.
pub fn island_body(st: &CarState, p: &CarParams, vt: &VehTune, dt: f32) -> Option<(super::loose::PBody, Vec3)> {
    use super::exact as ex;
    let vs = st.veh.as_ref()?;
    let t = &vt.t;
    let pivot_local = Vec3::new(t.s.model_offset_x, t.s.model_offset_y, t.s.model_offset_z);
    let kept = vs.body.filter(|b| b.snap == (st.pos, st.rot, st.vel, st.ang_vel));
    let rows = kept.map(|b| b.rows).unwrap_or_else(|| {
        let m = Mat3::from_quat(st.rot);
        [m.x_axis, m.y_axis, m.z_axis]
    });
    let at = kept.map(|b| b.at).unwrap_or(st.pos + Mat3::from_quat(st.rot) * pivot_local);
    let now = [rows[0], rows[1], rows[2], st.pos];
    let last = vs.hull_last.unwrap_or(now);
    let at = st.vel * dt + at;
    let turn = st.ang_vel * dt;
    let rows = if ex::dot3(turn, turn) > f32::from_bits(0x2b8c_bccc) { ex::rows_turned(&rows, turn) } else { rows };
    let mut b = super::loose::PBody::of_car(at, rows, st.vel, st.ang_vel, t.s.mass, moments(p, t), last, now, vs.last_force, vs.last_torque);
    if let Some(k) = kept {
        b.p = k.p;
        b.l = k.l;
    }
    Some((b, pivot_local))
}

/// What another car's step gave this car (the shared level's solves): the impulse and its turn into the
/// momenta, the push solve's move and turn into the pose - as the car's own body takes them (0x8259BE30,
/// 0x8259E790), so the next step keeps its exact body.
pub fn take_answer(st: &mut CarState, p: &CarParams, vt: &VehTune, jf: Vec3, jt: Vec3, push: Vec3, turn: Vec3) {
    use super::exact as ex;
    let t = &vt.t;
    let mass = t.s.mass;
    let inertia = moments(p, t);
    let inverse = Vec3::new(ex::recip(inertia.x), ex::recip(inertia.y), ex::recip(inertia.z));
    let pivot_local = Vec3::new(t.s.model_offset_x, t.s.model_offset_y, t.s.model_offset_z);
    let Some(vs) = st.veh.as_mut() else { return };
    let kept = vs.body.filter(|b| b.snap == (st.pos, st.rot, st.vel, st.ang_vel));
    let (mut rows, mut at, mut pm, mut lm, count, seen) = match kept {
        Some(b) => (b.rows, b.at, b.p, b.l, b.count, b.seen),
        None => {
            let m = Mat3::from_quat(st.rot);
            let rows = [m.x_axis, m.y_axis, m.z_axis];
            let iw = ex::world_inertia(&rows, inertia);
            let w = st.ang_vel;
            let l = Vec3::new(ex::dot3(Vec3::new(iw[0].x, iw[1].x, iw[2].x), w), ex::dot3(Vec3::new(iw[0].y, iw[1].y, iw[2].y), w), ex::dot3(Vec3::new(iw[0].z, iw[1].z, iw[2].z), w));
            (rows, st.pos + m * pivot_local, st.vel * mass, l, 0, [rows[0], rows[1], rows[2], st.pos])
        }
    };
    pm = pm + jf;
    lm = lm + jt;
    let mut v = pm * (1.0 / mass);
    if v.length_squared() > 134.1 * 134.1 {
        let k = 134.1 / v.length();
        v *= k;
        pm *= k;
    }
    let mut w = ex::spin_of(lm, &rows, inverse);
    if w.length_squared() > 12.566_371 * 12.566_371 {
        let k = 12.566_371 / w.length();
        w *= k;
        lm *= k;
    }
    at = at + push;
    if super::solver::turn_applies(turn) {
        rows = super::solver::rows_pushed(&rows, turn);
    }
    let origin = ex::origin_of(at, &rows, pivot_local);
    st.vel = v;
    st.ang_vel = w;
    st.pos = origin;
    st.rot = Quat::from_mat3(&Mat3::from_cols(rows[0], rows[1], rows[2])).normalize();
    vs.body_now = Some([rows[0], rows[1], rows[2], origin]);
    vs.body = Some(BodyExact { rows, origin, at, p: pm, l: lm, count, snap: (st.pos, st.rot, st.vel, st.ang_vel), seen });
}

pub fn step(st: &mut CarState, p: &CarParams, vt: &VehTune, input: CarInput, world: &WorldCollider, dt: f32) {
    // (a replay of a recorded computer driver gives the chassis' own pedals: no mapping)
    let replay_ai = REPLAY_AI.load(std::sync::atomic::Ordering::Relaxed);
    let input = CarInput { ai: input.ai && !replay_ai, ..input };
    let input_ai = input.ai || replay_ai;
    // (the chassis' own throttle, +136: nothing on the pedal at all)
    let pedal_released = (input.throttle <= if input.ai { 0.01 } else { 0.0 }, input.brake <= if input.ai { 0.01 } else { 0.0 });
    // a computer driver asks for a share of the pull; the engine's pedal coasts at one half
    // (on the grid, `hold`, the brain's pedals are written to the chassis as they are: a phase with the
    // throttle up shows +136 = 0 in the recordings, so the coasting share below does not apply)
    let input = if input.ai && !input.hold && !input.animated {
        let map = |x: f32| if x > 0.01 { 0.5 + 0.5 * x.min(1.0) } else { 0.42 };
        if input.brake > 0.1 && input.throttle < 0.01 { input } else { CarInput { throttle: map(input.throttle), ..input } }
    } else {
        input
    };
    let mut vs = st.veh.take().map(|b| *b).unwrap_or_default();
    // (P3, mcCarSim::Update 0x8232EDEC: a motorcycle whose rider is off - no throttle, half the brake, no handbrake)
    let wiped = vs.wipe.is_some() && vt.bike && !bike_old(vt) && !dbg_flag("MCLA_DBG_P3");
    let input = if wiped { CarInput { throttle: 0.0, brake: 0.5, handbrake: false, ..input } } else { input };
    if vs.drive == 255 {
        vs.drive = vt.t.drivetrain_type as u8;
    }
    let t = &vt.t;
    let e = &t.engine.s;
    // (N-VP-1, recorded: a.1 k114 -> k115, every opponent's first burst - in one tick the engine is at idle,
    // the wheels stand, the charge and burnout state are 0 and the centre of mass is back, where the car stands.
    // What sends it is not found; ai.rs asks for it at the burst's end, `AiDriver::grid_input`.)
    if std::mem::take(&mut vs.grid_reset) {
        vs.burn = 0.0;
        vs.burn_state = 0;
        vs.drive = t.drivetrain_type as u8;
        vs.cg_set = false;
        vs.engine_w = e.idle_rpm * 0.104_719_76;
        vs.omega = [0.0; 2];
        for w in vs.wheels.iter_mut() {
            w.spin_rate = 0.0;
        }
        vs.rev.on = false;
        vs.rev.timer = 0.0;
    }

    // ---- gearbox (automatic) ----
    let fwd_speed = st.fwd_speed();
    let n_fwd = vt.ratios.len() as i32 - 1;
    if !vs.started {
        // a car that appears already moving: wheels turning, in the gear for its speed
        vs.started = true;
        let r = t.wheel_back.radius.max(0.1);
        vs.omega = [fwd_speed / r; 2];
        for wheel in &mut vs.wheels { wheel.spin_rate = fwd_speed / r; }
        if fwd_speed > 1.0 {
            let mut g = 1;
            while g < n_fwd && fwd_speed / r * vt.ratios[g as usize] > t.engine.s.max_rpm * RPM * 0.8 {
                g += 1;
            }
            vs.gear = g;
        }
    }
    // ---- A2: the player's pad as mcPlayerInput reads it (physics/pad.rs). The pad's code runs once a
    // FRAME - every other tick, between the vehicle's update and the physics step (further down, just
    // ahead of `physics_frame`) - and what it leaves is what the next two updates are handed: chassis
    // +136 / +140 from the triggers through the auto-reverse state machine, the way the gearbox is to
    // point, the steering value P, the rider's smoothed stick. Here those are laid over the raw input
    // in this port's own pedal convention (in reverse its `brake` is the pedal that drives).
    // Not for computer drivers, replays or the older bike path. MCLA_DBG_A2: the raw triggers and the
    // port's older reverse rule, the pad every tick. ----
    let raw_input = input;
    let pad_exact = !input_ai && !input.animated && !REPLAY_STEER.load(std::sync::atomic::Ordering::Relaxed) && !bike_old(vt) && !dbg_flag("MCLA_DBG_A2") && !dbg_flag("MCLA_DBG_Z2");
    let mut input = input;
    if pad_exact {
        vs.pad.live = true;
        let held = vs.pad.pedals;
        if held.way < 0 {
            input.brake = held.throttle;
            input.throttle = held.brake;
        } else {
            input.throttle = held.throttle;
            input.brake = held.brake;
        }
        vs.pad.stick_y = vs.pad.stick_next;
        // (A6: the handbrake too is the chassis's +144 as the pad's last frame wrote it, not the button this tick -
        // sd.2 s241: written in the frame, the update before it still without. MCLA_DBG_A6: the raw button.)
        if !dbg_flag("MCLA_DBG_A6") {
            input.handbrake = vs.pad.handbrake;
        }
        if vt.bike {
            // (the game has one key for the rider's weight: the stick - smoothed, input +32 - and the
            // button together)
            input.xfer = input.xfer || raw_input.lean != 0.0;
            input.lean = vs.pad.stick_y;
        }
    } else {
        vs.pad = super::pad::PadHeld::default();
    }
    // ---- the race's grid hold (N-VP-1; recorded on the original 2026-10-18, work/physics2/rec a.0 /
    // a.2 / a.3): from the first tick on the grid with the engine running until GO the player's chassis
    // brake, +140, is written 1 once a frame whatever the pedal - the wheels' pedal torque, +480, is the
    // full one on every tick - while throttle and handbrake pass: the throttle alone revs the engine
    // against the brake in first gear (the rev limiter cutting from a half), throttle and handbrake burn
    // out as anywhere. A computer driver gets no such write: its brain brakes itself (ai.rs). ----
    // (the grid rule itself, mcCarSim::Update 0x8232EC8C..0x8232EDB8, on every car with chassis +592 bit 0x2:
    // first gear is set out of reverse or neutral (0x82561D90); throttle and handbrake together - a burnout -
    // keep the throttle, the handbrake is written 1; otherwise the brake is written 1, and with burnout charge
    // left under the handbrake the throttle 0. The steering, +132, is written 0 in every case - here it is
    // zeroed where the grid input is made, vehicles.rs. The `noburnout` switch, 0x8288E614, is 0 in a game.)
    if input.hold {
        if vs.gear <= 0 {
            vs.gear = 1;
            vs.shift_timer = 0.0;
        }
        if input.throttle > 0.0 && input.handbrake {
            input.handbrake = true;
        } else if vs.burn == 0.0 || !input.handbrake {
            input.brake = 1.0;
        } else {
            input.throttle = 0.0;
            input.brake = 1.0;
        }
    }
    // (a replay's pedals are the recording's chassis +136 - which the update itself leaves cut by the rev
    // limiter, and the pad writes only every other tick: where a sample shows what the last update
    // left there, the pad's own throttle is the one before)
    if REPLAY_STEER.load(std::sync::atomic::Ordering::Relaxed) && !bike_old(vt) && !dbg_flag("MCLA_DBG_Y6") {
        let way_now = REPLAY_WAY.load(std::sync::atomic::Ordering::Relaxed);
        let reversed = if way_now != 0 { way_now < 0 } else { vs.gear < 0 };
        let shown = if reversed { input.brake } else { input.throttle };
        if vs.rev.on && shown.to_bits() == vs.rev.left.to_bits() {
            if reversed { input.brake = vs.rev.pad; } else { input.throttle = vs.rev.pad; }
        } else {
            vs.rev.pad = shown;
        }
    }
    // (RB3, the head of mcCarSim::Update, 0x8232EF30: while a nitrous shot burns (nitro +32) the chassis's
    // throttle, +136, is set to 1 - ahead of everything that reads it: the boost's "pedal released" rule, the
    // automatic brake, the burnout, the rev limiter, the engine. Only the pad's own stomp test goes by the
    // pad's pedal (input +12). A shot fired with the throttle up, sd.0: the engine pulls from the first
    // tick. MCLA_DBG_RB3: forced further down, after the engine, as before. Not computer drivers, +818.)
    let mut pad_input = input;
    if !input_ai && !bike_old(vt) && !dbg_flag("MCLA_DBG_Y6") && !dbg_flag("MCLA_DBG_A1") && !dbg_flag("MCLA_DBG_RB3") {
        let way_now = REPLAY_WAY.load(std::sync::atomic::Ordering::Relaxed);
        let reversed = if way_now != 0 { way_now < 0 } else if pad_exact { vs.pad.pedals.way < 0 } else { vs.gear < 0 };
        if REPLAY_STEER.load(std::sync::atomic::Ordering::Relaxed) {
            // (a replay's pedal is the recording's chassis +136, which the pad writes only every other tick:
            // the tick after one this update forced shows the 1 it left - the pad's own pedal is still the
            // one before. The shot is fired in a pad's frame, so the first burning tick shows the pad's.)
            let shown = if reversed { input.brake } else { input.throttle };
            if vs.nitro_left && shown == 1.0 {
                if reversed { pad_input.brake = vs.nitro_pad; } else { pad_input.throttle = vs.nitro_pad; }
                vs.nitro_left = false;
            } else {
                vs.nitro_pad = shown;
                vs.nitro_left = vs.nitro_burning;
            }
        }
        if vs.nitro_burning {
            if reversed { input.brake = 1.0; } else { input.throttle = 1.0; }
        }
    }
    let pedal_released = (input.throttle <= if input.ai { 0.01 } else { 0.0 }, input.brake <= if input.ai { 0.01 } else { 0.0 });
    // (not on the grid, `hold`: the gearbox is held there whenever the handbrake is off (0x8232F098) and
    // every recorded racer stays in first gear through its braking phases)
    let reverse_wanted = !input.hold && input.brake > 0.1 && input.throttle < 0.1 && fwd_speed < 1.0;
    let old_gear = vs.gear;
    // (a replay goes the way its recording's gearbox did: choosing reverse is the pad code's)
    let replay_way = REPLAY_WAY.load(std::sync::atomic::Ordering::Relaxed);
    if input.animated {
        // (an mcBrainAnimated writes input +44 = max(gear, first) every time, sub_822B97E0)
        if vs.gear <= 0 {
            vs.gear = 1;
            vs.shift_timer = 0.0;
        }
    } else if replay_way != 0 {
        if (replay_way < 0) != (vs.gear < 0) {
            vs.gear = if replay_way < 0 { -1 } else { 1 };
            vs.shift_timer = 0.0;
        }
    } else if !dbg_flag("MCLA_DBG_Z2") && vs.way_after != 0 {
        // (Z2: the pad's code runs between a tick's vehicle update and its physics step - the way
        // it chose there, `way_after`, is the one that step's contact callbacks found and this
        // update drives in)
        if vs.way_after < 0 && vs.gear >= 0 {
            vs.gear = -1;
            vs.shift_timer = 0.0;
        } else if vs.way_after > 0 && vs.gear < 0 {
            vs.gear = 1;
            vs.shift_timer = 0.0;
        } else if vs.gear == 0 {
            vs.gear = 1;
        }
    } else if reverse_wanted && vs.gear >= 0 && fwd_speed < 0.5 {
        vs.gear = -1;
        vs.shift_timer = 0.0;
    } else if vs.gear < 0 && ((input.throttle > 0.1 && fwd_speed > -1.0) || fwd_speed > 0.5) && !reverse_wanted {
        vs.gear = 1;
        vs.shift_timer = 0.0;
    } else if vs.gear == 0 {
        vs.gear = 1;
    }
    // (the pedals the pad's code will read after this tick's update: this tick's)
    let pad_pedals = (input.throttle, input.brake);
    let ratio_of = |g: i32| -> f32 {
        if g < 0 {
            -vt.ratios[0]
        } else {
            vt.ratios[(g.max(1) as usize).min(vt.ratios.len() - 1)]
        }
    };
    // Kick-down (mcPlayerInput + mcCarSim 0x8232d4b8): a pedal that jumps up faster than
    // ThrottleInputVelocityThreshold drops the box at once to the lowest gear that is not past
    // its shift point at this speed - MaxDownshifts gears at most, first gear below 10 m/s.
    // (the chassis' throttle is the pedal that drives: in reverse this port's brake)
    let drive_pedal = if vs.gear < 0 { pad_input.brake } else { pad_input.throttle };
    let pedal_now = drive_pedal.max(0.5);
    // (computer drivers too: their input object carries the same flag, 0x822b9504)
    // (RB1, mcPlayerInput::Update 0x822A2EBC..0x822A32A0: the player's test is made once a FRAME by the pad's
    // code - (max(new, 0.5) - max(old, 0.5)) x the frame's 1 / step (0x827D750C there: 30 a second, two
    // ticks) against ThrottleInputVelocityThreshold; flag 0x2000 of input +40. A pedal going 0 -> 0.677
    // (5.3 a second) or 0.677 -> 1 (9.7) is no stomp for a threshold of 10; 0 -> 1 (15) is. The pedal a
    // tick is handed only changes with the pad's frame, so the change seen here is the frame's.
    // MCLA_DBG_RB1: over one tick's step, as before. Computer drivers: as before.)
    let stomp_rate = if input_ai || dbg_flag("MCLA_DBG_RB1") { 1.0 / dt.max(1e-4) } else { 0.5 * (1.0 / dt.max(1e-4)) };
    let stomped = t.input.throttle_input_velocity_threshold > 0.0 && (pedal_now - vs.pedal_was.max(0.5)) * stomp_rate > t.input.throttle_input_velocity_threshold;
    vs.pedal_was = drive_pedal;
    // (mc4 step 13b: the pad's driver hands the stomp on only to an automatic - flag 8 of its state,
    // manual gears, stops the call of 0x8232D4B8 at 0x822A1120: no kick-down, no boost restart)
    let stomped = stomped && !(input.manual && !input_ai);
    if stomped {
        // (0x8232D4B8 begins by starting the engine's boost afresh, whatever the gearbox then
        // does - seen in a recording the moment reverse engages under a held pedal)
        vs.boost_timer = e.boost_duration;
    }
    if stomped && vs.gear > 1 && !vs.gear_change.active && vs.shift_timer > 0.12 && (vs.gear as usize) < vt.shift.len() {
        let r = t.wheel_back.radius.max(0.1);
        let mut target = 1;
        if fwd_speed >= 10.0 {
            while target < vs.gear && fwd_speed / r * vt.ratios[target as usize] / RPM > vt.shift[target as usize][0] {
                target += 1;
            }
        }
        let n = (vs.gear - target).min(t.trans.max_downshifts.clamp(1, 4) as i32);
        if n > 0 {
            vs.gear -= n;
            vs.shift_timer = 0.0;
        }
    }
    let forced = FORCE_GEAR.load(std::sync::atomic::Ordering::Relaxed);
    if forced != 0 {
        vs.gear = forced;
        vs.shift_timer = 1.0;
        vs.gear_change = GearChange::default();
    } else {
        // Input-requested reverse/kick-down changes happen before engine Update.
        vs.gear_change.start(old_gear, vs.gear, e.gcl);
    }
    let engine_ratio = ratio_of(vs.gear);
    // ---- the automatic brake (mcCarSim::Update 0x8232F1A0..0x8232F304, chassis +396): off the
    // throttle (0.1 or less; any pedal in neutral), slower than the engine's idle speed in this
    // gear plus 5 mph - by the body's speed and by the drive shaft's - every wheel is braked with
    // a quarter of its grip, rising by half a second^-1, and the transmission is held; above
    // that speed it stays as it is until the throttle passes 0.05 ----
    {
        let pedal = if vs.gear < 0 { input.brake } else { input.throttle };
        let first_radius = if vs.drive == 0 { t.wheel_back.radius } else { t.wheel_front.radius };
        // (0x8232F250: the body's speed is chassis +224 - what vehAero left there, the speed the car
        // came into the tick BEFORE with; the shaft's is chassis +572 as this update began, signed)
        let old = dbg_flag("MCLA_DBG_OLDAUTOBRAKE");
        let threshold = if engine_ratio == 0.0 { 4.47 } else if old { first_radius / engine_ratio.abs() * (e.idle_rpm * 0.104_719_76) + 2.235 } else { fma32(first_radius / engine_ratio.abs(), e.idle_rpm * 0.104_719_76, 2.235) };
        let slow = if old { st.vel.length() < threshold && (vs.omega[0] * first_radius).abs() < threshold } else { vs.last_speed < threshold && vs.omega[0] * first_radius < threshold };
        if (pedal <= 0.1 || vs.gear == 0) && slow {
            if vs.auto_brake == 0.0 {
                vs.auto_brake = 0.25;
            }
            vs.auto_brake = (vs.auto_brake + dt * 0.5).min(1.0);
        } else if pedal > 0.05 {
            vs.auto_brake = 0.0;
        }
    }
    // ---- the burnout (mcCarSim::Update 0x8232F768..0x8232FD7C, operation for operation; after the
    // automatic brake, ahead of the tyres' melt, the engine and the transmission). Throttle and
    // handbrake together, slower than BurnThreshold (BurnRevThreshold in reverse) by chassis +224 -
    // the speed the car came into the tick BEFORE with - and a wheel on the ground: first gear (or
    // reverse) is taken and held, the wheels the engine does not turn - by the layout as it stands
    // - are braked with twice their grip and the others not at all, an all-wheel-drive car goes to
    // one axle, the centre of mass to (x, 0, the held axle's place x BurnWeightxfer), the charge up
    // by BurnIncval a second. Otherwise, while there is charge: the launch - BurnBoost along the
    // driven wheels' way unless the handbrake is on - and the charge down by BurnDecval and by
    // (1 - throttle) x BurnIncval; at nothing the layout comes back. MCLA_DBG_Y6: the port's older
    // block further down. ----
    // (the drive layout as the tick came in: chassis +572 is formed by it at the head of mcCarSim::Update)
    let drive_before = vs.drive;
    // (RB6: the charge as the tick came in - what the head of mcCarSim::Update, 0x8232EF9C, steers a
    // motorcycle by, ahead of the burnout's own rules. V: se.1 k 170, the first steered tick of a launch.)
    let burn_in = if dbg_flag("MCLA_DBG_RB6") { None } else { Some(vs.burn) };
    vs.by = BurnTick { exact: !bike_old(vt) && !dbg_flag("MCLA_DBG_Y6"), state_in: vs.burn_state, held: false, brake_layout: None, cg: None, cg_return: true, launch: None, launch_layout: vs.drive };
    if dbg_flag("MCLA_DBG_TRACE") {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| eprintln!("TUNE engine {:?} trans {:?} sync {:?} gyro {:?}
TUNEINPUT {:?} nitro {:?} steering limit {:?}", t.engine.s, t.trans.s, vt.sync_torque, t.gyro, t.input, t.nitro, t.wheel_front.steering_limit));
    }
    if vs.by.exact {
        let (thr, brk) = if vs.gear < 0 { (input.brake, input.throttle) } else { (input.throttle, input.brake) };
        let hand: f32 = if input.handbrake { 1.0 } else { 0.0 };
        // (0x8232F62C: with charge left and the handbrake on, the centre of mass stays where it is)
        vs.by.cg_return = !(vs.burn > 0.0 && hand != 0.0);
        let grounded = st.wheels.iter().filter(|w| w.grounded).count();
        let way = if vs.gear < 0 { -1 } else if vs.gear == 0 { 0 } else { 1 };
        let limit = if way < 0 { t.s.burn_rev_threshold } else { t.s.burn_threshold };
        let wanted = thr > 0.0 && hand > 0.0 && vs.last_speed < limit && grounded > 0;
        if vs.two == 0 && wanted {
            let target = if way < 0 { -1 } else { 1 };
            if vs.gear != target {
                // (0x825617E8: the gear is set, its timer starts again)
                let was = vs.gear;
                vs.gear = target;
                vs.shift_timer = 0.0;
                if forced == 0 {
                    vs.gear_change.start(was, vs.gear, e.gcl);
                }
            }
            vs.by.held = true;
            vs.by.brake_layout = Some(vs.drive);
            vs.burn_state = 1;
            // (which axle is held: the front for a car the tuning drives at the rear or at all four)
            let rest_z = |j: usize| match vs.rest_exact { Some(r) => r[j][2], None => p.wheel_pos[j].z };
            let held_z = if matches!(t.drivetrain_type, 0 | 2) {
                if vs.drive == 2 { vs.drive = 0; }
                rest_z(0)
            } else {
                if vs.drive == 3 { vs.drive = 1; }
                rest_z(2)
            };
            // (0x8232F8E4: on the grid - chassis +592 bit 0x2 - the share is 0.75, 0x8201CCA4, not the tuning's)
            vs.by.cg = Some(held_z * if input.hold { 0.75 } else { t.s.burn_weightxfer });
            if vs.burn < 1.0 && grounded > 0 {
                vs.burn = fma32(t.s.burn_incval, dt, vs.burn);
                if vs.burn > 1.0 {
                    vs.burn = 1.0;
                }
            } else {
                vs.burn_state = 2;
            }
        } else if vs.burn > 0.0 {
            vs.burn_state = 3;
            if hand == 0.0 && grounded > 0 {
                vs.by.launch = Some((((way as f32 * (1.0 - hand)) * (1.0 - brk)) * t.s.burn_boost) * thr);
                vs.by.launch_layout = vs.drive;
            }
            vs.burn = fnms32(t.s.burn_decval, dt, vs.burn);
            vs.burn = fnms32((1.0 - thr) * t.s.burn_incval, dt, vs.burn);
            if vs.burn < 0.0 {
                vs.burn = 0.0;
                vs.burn_state = 0;
                if matches!(t.drivetrain_type, 2 | 3) {
                    vs.drive = t.drivetrain_type as u8;
                }
            }
        }
        st.burnout = if vs.burn_state == 1 || vs.burn_state == 2 { vs.burn } else { 0.0 };
        st.launch = if vs.burn_state == 3 { vs.burn } else { 0.0 };
    }
    // ---- the rev limiter (mcCarSim::Update 0x8232FD7C..0x8232FF28, after the burnout's rules): with the
    // transmission held, in neutral, in first gear with burnout charge, or in the air, an engine past
    // RevLimitMax of the way from its speed of best power to its greatest counts time; after
    // RevLimitDelay the throttle is cut - to RevLimitThrottleRate, or (a negative rate) from the pedal
    // down by it a second, from no more than a half where the engine drives nothing that stands on
    // the ground - until the engine is back under RevLimitMin. Everything after it in the tick -
    // tyres' melt, engine, gyro - sees the cut pedal (chassis +136). MCLA_DBG_Y6: none. ----
    let mut input = input;
    if vs.by.exact {
        let computer = input.ai || REPLAY_AI.load(std::sync::atomic::Ordering::Relaxed);
        let held = (vs.gear_change.direction != 0 && vs.gear >= 2) || vs.gear == 0 || (vs.auto_brake > 0.0 && (!computer || input.hold))
            || (input.hold && vs.burn < 0.95 && !input.handbrake);
        let thr = if vs.gear < 0 { input.brake } else { input.throttle };
        let mut allowed = thr;
        if held || vs.gear == 0 || (vs.gear == 1 && vs.burn > 0.0) || vs.air_time > 0.0 {
            let best = vt.torque_curve.as_ref().map_or(e.opt_rpm * 0.104_719_76, |c| c.peak().0);
            let top = e.max_rpm * 0.104_719_76;
            let share = (vs.engine_w - best) / (top - best);
            if share > t.s.rev_limit_max {
                vs.rev.timer += dt;
                if vs.rev.timer > t.s.rev_limit_delay && !vs.rev.on {
                    vs.rev.on = true;
                    vs.rev.cut = if t.s.rev_limit_throttle_rate < 0.0 { thr } else { t.s.rev_limit_throttle_rate };
                    let layout = vs.drive;
                    let standing = (0..4).filter(|i| match layout { 1 | 4 => *i < 2, 2 | 3 => true, _ => *i >= 2 } && vs.wheels[*i].record).count();
                    if vs.gear == 0 || held || standing == 0 {
                        vs.rev.cut = if 0.5 - vs.rev.cut >= 0.0 { vs.rev.cut } else { 0.5 };
                    }
                }
            } else if share < t.s.rev_limit_min {
                vs.rev.on = false;
            } else if !(share > 0.0) {
                vs.rev.timer = -dt;
            }
            let low = if vs.rev.timer >= 0.0 { vs.rev.timer } else { 0.0 };
            vs.rev.timer = if low - t.s.rev_limit_delay >= 0.0 { t.s.rev_limit_delay } else { low };
            if vs.rev.on {
                if t.s.rev_limit_throttle_rate < 0.0 {
                    vs.rev.cut = fma32(t.s.rev_limit_throttle_rate, dt, vs.rev.cut);
                    if vs.rev.cut < 0.0 {
                        vs.rev.cut = 0.0;
                    }
                }
                if vs.rev.cut < allowed {
                    allowed = vs.rev.cut;
                }
            }
        } else if vs.rev.timer > 0.0 {
            vs.rev.timer -= dt;
            if vs.rev.timer < 0.0 {
                vs.rev.timer = 0.0;
            }
        }
        if vs.gear < 0 { input.brake = allowed; } else { input.throttle = allowed; }
        vs.rev.left = allowed;
    }
    let engine_ratio = ratio_of(vs.gear);
    // (mcCarTireFric::Update runs ahead of the engine and the transmission, 0x82330378: the
    // layout it switches is the one the transmission's update finds. What mcCarSim::Update
    // takes from the melt - the air's damping factor - is the melt of the tick before.)
    let melt_before = vs.melt;
    if !dbg_flag("MCLA_DBG_OLDORDER") {
        let gear_in = vs.gear;
        tire_fric_update(&mut vs, t, input, gear_in, dt);
    }
    let drive_melted = vs.drive;
    // (the chassis update, 0x82330930, ahead of the engine: while the burnout's state is not 0 -
    // charging, charged, or the launch - the engine turns at no less than charge x the speed of
    // its best power (engine +72). When the tyres bite in the launch and the shaft drags the
    // engine down faster than the charge decays, the engine's torque, the converter and the
    // shaft's solve all see that floor. MCLA_DBG_Z1: no floor.)
    if vs.burn_state > 0 && !dbg_flag("MCLA_DBG_Z1") {
        let best = vt.torque_curve.as_ref().map_or(t.engine.s.opt_rpm * 0.104_719_76, |c| c.peak().0);
        let floor = vs.burn * best;
        if vs.engine_w < floor {
            vs.engine_w = floor;
        }
    }
    // (A1, the chassis update 0x82330378 ahead of the vehicle's: in the tick a car comes down - a wheel has
    // a contact record again, byte +529, after air time - the air time is negated for that one tick and the
    // transmission's timer, +28, starts again: no change of gear for GearChangeDelay after a landing)
    // (the air time itself, chassis +584, 0x82330554: counted while no wheel has a record and mcCarStuck
    // is at rest (its +56); the tick that ends, it is negated - what vfunc 13 hands out is negative for that
    // one tick - then nothing. mcCarSim::Update, which runs ahead, sees the value of the tick before.)
    vs.air584_before = vs.air584;
    if vs.by.exact && !dbg_flag("MCLA_DBG_A1") {
        let records = (0..4).filter(|i| (!vt.bike || i % 2 == 0) && vs.wheels[*i].record).count();
        if records == 0 && vs.stuck == 0 {
            vs.air584 += dt;
        } else if vs.air584 > 0.0 {
            vs.air584 = -vs.air584;
        } else {
            vs.air584 = 0.0;
        }
        if vs.air584 < 0.0 {
            vs.shift_timer = 0.0;
        }
    }
    update_engine(&mut vs, vt, input, pedal_released, engine_ratio, dt);
    let automatic_old_gear = vs.gear;
    // Transmission Update reads engine Update's actual speed and preceding
    // powered contacts. The original has no motorcycle-only shift multiplier.
    let layout = vs.drive as u32;
    let powered = |i: usize| match layout { 1 | 4 => i < 2, 2 | 3 => true, _ => i >= 2 };
    let mut contacts = [(false, 0.0); 4];
    let mut count = 0;
    for i in 0..4 {
        if powered(i) && (!vt.bike || i % 2 == 0) {
            contacts[count] = (st.wheels[i].grounded, vs.wheels[i].slide);
            count += 1;
        }
    }
    let automatic = super::transmission::update_prepared(super::transmission::AutomaticState {
        // (vehTransmission + 16, set each tick by sub_8232CB28: 0 manual, 1 automatic)
        mode: if forced == 0 && !(input.manual && !input_ai) { 1 } else { 0 }, gear: vs.gear,
        direction: vs.gear_change.direction as i32, inhibited: vs.trans_inhibited,
        held: vs.by.exact && vs.by.held, elapsed: vs.shift_timer, modulation: vs.shift_mod,
    }, super::transmission::AutomaticFrame {
        tune: &t.trans, points: &vt.shift, engine_speed: vs.engine_w,
        pedal: vs.engine_pedal, contacts: &contacts[..count], dt,
    });
    vs.gear = automatic.gear;
    vs.shift_timer = if forced == 0 { automatic.elapsed } else { 1.0 };
    vs.shift_mod = automatic.modulation;
    // (manual gears: a shift asked for by the pad, in a forward gear - up while there is a gear
    // above, sub_825619E8; down not below first, sub_8255AE20; the gear is set as the automatic's
    // own shifts set it, 0x825617E8)
    if input.manual && !input_ai && forced == 0 && vs.gear >= 1 {
        let to = vs.gear + input.shift.signum() as i32;
        if to != vs.gear && to >= 1 && to <= t.trans.gears as i32 - 2 {
            vs.gear = to;
            vs.shift_timer = 0.0;
        }
    }
    if forced == 0 {
        // The automatic shift is queued for the following engine Update.
        vs.gear_change.start(automatic_old_gear, vs.gear, e.gcl);
    }
    let ratio = ratio_of(vs.gear);
    // (Z2: the pad's code, between this update and the physics step: which way the box is to go.
    // The rule is this port's own - throttle and brake as it has them - only its place in the tick
    // is the original's: the step's contact callbacks find the new way, the next update drives in it.)
    vs.way_after = 0;
    if pad_exact && replay_way == 0 && forced == 0 {
        // (A2: the pad's own choice - made anew, on its frame's tick, just ahead of the physics step)
        vs.way_after = vs.pad.pedals.way as i32;
    } else if !dbg_flag("MCLA_DBG_Z2") && replay_way == 0 && forced == 0 {
        let (thr, brk) = pad_pedals;
        // (not on the grid - `hold`: see `reverse_wanted`)
        let wanted = !input.hold && brk > 0.1 && thr < 0.1 && fwd_speed < 1.0;
        vs.way_after = if wanted && vs.gear >= 0 && fwd_speed < 0.5 {
            -1
        } else if vs.gear < 0 && ((thr > 0.1 && fwd_speed > -1.0) || fwd_speed > 0.5) && !wanted {
            1
        } else if vs.gear < 0 { -1 } else { 1 };
    }

    // ---- steering input (mcPlayerInput): less lock at speed (SSSThreshold, SSSValue), turning
    // at SteerRate, returning at ReturnRate, faster across the centre (CountersteerFactor) ----
    let speed = st.vel.length();
    let sss = {
        let thr = t.input.sss_threshold;
        // (the computer drivers' steering is not the pad's: their tables already hold the lock they need)
        if thr > 0.0 && !input_ai {
            let k = (speed / thr).clamp(0.0, 1.0);
            1.0 + (k * std::f32::consts::FRAC_PI_2).sin() * (t.input.sss_value.clamp(0.1, 1.0) - 1.0)
        } else {
            1.0
        }
    };
    // (the chassis scales the lock once more: SSSThresholdMin..Max, SSSValue of the car's tuning)
    vs.sss_chassis = {
        let (lo, hi) = (t.s.sss_threshold_min, t.s.sss_threshold_max);
        if hi <= 0.0 || speed < lo {
            1.0
        } else if speed >= hi {
            t.s.sss_value.clamp(0.1, 1.0)
        } else {
            1.0 + (((speed - lo) / (hi - lo).max(0.1)) * std::f32::consts::FRAC_PI_2).sin() * (t.s.sss_value.clamp(0.1, 1.0) - 1.0)
        }
    };
    let sss = sss * vs.sss_chassis;
    // nitrous: a shot runs down over NitroBoostTime; while it burns the throttle is held open
    let grounded_now = st.wheels.iter().filter(|w| w.grounded).count();
    // (mcCarNitro: a shot takes one from the count; the next can go once the view has come
    // back from the last one, NitroFovOutTime after it ended. Nothing refills it on the road.)
    // (A1, the exact path: mcCarNitro's own update 0x82345A80 runs after the vehicle's - the level comes
    // down and the force is formed there, further down; what mcCarSim::Update sees here is the byte
    // "burning" as the last update or the pad's fire routine 0x82345820 left it. MCLA_DBG_A1: before.)
    let exact_nitro = vs.by.exact && !dbg_flag("MCLA_DBG_A1");
    // (the wheels with a contact record, byte +529, as the tick came in: mcCarNitro's update runs in the
    // chassis update, ahead of the pass that updates the wheels - 0x8255AC20 counts the last tick's)
    vs.nitro_records = (0..4).filter(|i| (!vt.bike || i % 2 == 0) && vs.wheels[*i].record).count();
    // (RB3: the player's shot is fired by the pad's code, between this update and the physics step - further
    // down, `nitro_try`. Here only: computer drivers, the older paths, and a replay's hint column of a
    // recording that did not log its pad, VEH_NITRO. MCLA_DBG_RB3: this rule for everyone, as before.)
    let fire_exact = exact_nitro && !dbg_flag("MCLA_DBG_RB3");
    let fire_in_pad = fire_exact && pad_exact;
    if !fire_in_pad && input.nitro && !vs.nitro_held && vs.nitro_level <= 0.0 && vs.nitro_rest <= 0.0 && st.nitro_shots > 0 && grounded_now > 0 {
        vs.nitro_level = 1.0;
        vs.nitro_burning = true;
        st.nitro_shots -= 1;
    }
    vs.nitro_held = input.nitro;
    if exact_nitro {
        if vs.nitro_burning {
            vs.nitro_rest = t.nitro.fov_out_time.max(0.0);
        } else {
            vs.nitro_rest = (vs.nitro_rest - dt).max(0.0);
        }
    } else if vs.nitro_level > 0.0 {
        vs.nitro_level = (vs.nitro_level - dt / t.nitro.boost_time.max(0.2)).max(0.0);
        vs.nitro_rest = t.nitro.fov_out_time.max(0.0);
    } else {
        vs.nitro_rest = (vs.nitro_rest - dt).max(0.0);
    }
    st.nitro = st.nitro_shots as f32 / st.nitro_max.max(1) as f32;
    if vs.sst_level > 0.0 {
        vs.sst_level = (vs.sst_level - dt / t.ss_turbo.use_time.max(0.2)).max(0.0);
    }
    // (either shot is cut short when the car lies on its side or roof with a wheel on the road)
    if (vs.nitro_level > 0.0 || vs.sst_level > 0.0) && st.up().y < 0.1 && st.wheels.iter().any(|w| w.grounded) {
        if !exact_nitro {
            vs.nitro_level = 0.0;
        }
        vs.sst_level = 0.0;
    }
    // (0x8232EF30: while the nitrous burns the chassis's throttle, +136, is 1 - whatever the gear; in
    // reverse this port's brake is that pedal)
    let input = if exact_nitro && vs.nitro_burning {
        if vs.gear < 0 { CarInput { brake: 1.0, ..input } } else { CarInput { throttle: 1.0, ..input } }
    } else if ((!exact_nitro && vs.nitro_level > 0.0) || vs.sst_level > 0.0) && vs.gear > 0 { CarInput { throttle: 1.0, ..input } } else { input };
    // angular damping: AeroDampFactor after AeroDampDelay in the air, LandingDampFactor on
    // touching down after a quarter second of flight, back to 1 over a second
    // (0x82330554: "in the air" is no wheel with a contact record, 0x8255AC20)
    if (if exact_nitro { vs.nitro_records } else { grounded_now }) == 0 {
        vs.air_time += dt;
        if vs.air_time > t.s.aero_damp_delay.max(0.0) && t.s.aero_damp_factor > 0.0 {
            vs.aero_damp = t.s.aero_damp_factor;
        }
    } else {
        if vs.air_time > 0.25 && t.s.landing_damp_factor > 0.0 {
            vs.aero_damp = t.s.landing_damp_factor;
        }
        vs.air_time = 0.0;
        if bike_old(vt) || dbg_flag("MCLA_DBG_OLDSUMS") {
            vs.aero_damp += (1.0 - vs.aero_damp).clamp(-dt, dt);
            // (mcCarSim::Update 0x8232FFEC: with melted tyres the damping of the body's turning
            // goes towards TireMeltDampFactor - a car leaving a burnt launch swings round freely)
            if !vt.bike && t.s.tire_melt_damp_factor < 1.0 && melt_before > 0.01 {
                vs.aero_damp = (t.s.tire_melt_damp_factor - 1.0) * melt_before + 1.0;
            }
        } else {
            // (the original's order and operations: mcCarSim::Update, 0x8232FFEC, sets the
            // factor from the melt of the tick before - fused - and the chassis's update that
            // follows, 0x823308E4, moves whatever it finds one time step towards 1; so with
            // melted tyres the factor in use is the melt's value plus dt)
            // (0x8232FFEC: not on two wheels - chassis +24 == 2 skips it)
            if !vt.bike && t.s.tire_melt_damp_factor < 1.0 && melt_before > 0.01 {
                vs.aero_damp = fma32(t.s.tire_melt_damp_factor - 1.0, melt_before, 1.0);
            }
            if vs.aero_damp < 1.0 {
                vs.aero_damp = (dt + vs.aero_damp).min(1.0);
            } else if vs.aero_damp > 1.0 {
                vs.aero_damp = (vs.aero_damp - dt).max(1.0);
            }
        }
    }
    // ---- burnout (mcCarSim): handbrake and throttle near standstill spin the driven wheels
    // (BurnIncval per second up to a full charge); letting go launches the car with
    // BurnBoost while the charge runs down (BurnDecval per second) ----
    if !vs.by.exact {
        let charging = input.handbrake && input.throttle > 0.5 && input.brake < 0.1 && vs.gear > 0 && speed < t.s.burn_threshold.max(3.0) * MPH && grounded_now >= 2 && vs.nitro_level <= 0.0;
        if charging {
            vs.gear = 1;
            vs.burn = (vs.burn + t.s.burn_incval.max(0.1) * dt).min(1.0);
            vs.burn_state = if vs.burn >= 1.0 { 2 } else { 1 };
            // (0x8232F96C / 0x8232F948: a burnout turns one axle only)
            if vs.drive == 2 { vs.drive = 0; } else if vs.drive == 3 { vs.drive = 1; }
            let want = -input.steer.clamp(-1.0, 1.0) * t.gyro.burnout_limit.max(0.0);
            let rate = t.gyro.burnout_rate.max(0.5) * dt;
            vs.burn_yaw += (want - vs.burn_yaw).clamp(-rate, rate);
        } else if vs.burn > 0.0 {
            vs.burn_state = 3;
            vs.burn -= (t.s.burn_decval.max(0.1) + (1.0 - input.throttle.clamp(0.0, 1.0)) * t.s.burn_incval.max(0.1)) * dt;
            vs.burn_yaw = 0.0;
            if vs.burn <= 0.0 {
                vs.burn = 0.0;
                vs.burn_state = 0;
                // (0x8232FD4C: the launch is over - all four wheels again)
                if matches!(t.drivetrain_type, 2 | 3) {
                    vs.drive = t.drivetrain_type as u8;
                }
            }
        } else {
            vs.burn_state = 0;
            vs.burn_yaw = 0.0;
        }
        st.burnout = if vs.burn_state == 1 || vs.burn_state == 2 { vs.burn } else { 0.0 };
        st.launch = if vs.burn_state == 3 { vs.burn } else { 0.0 };
    }
    // ---- two-wheel driving (mcCarGyro, four-wheeled cars): the weight-transfer button with
    // the steering over at more than LeanSpeedMin tips the car up - a kick, then a controller
    // that takes the roll to LeanAngle at LeanRateUp and holds it there (Lean, LeanLimit,
    // LeanDamp). Letting the button go, slowing down, rolling backwards or full lock puts it
    // back down with a kick; then it rests for ten frames. ----
    vs.two_acc = 0.0;
    if !vt.bike && t.gyro.lean > 0.0 && !input_ai {
        let gt = &t.gyro;
        let grounded = st.wheels.iter().filter(|w| w.grounded).count();
        let roll = st.right().y.clamp(-1.0, 1.0).asin();
        let burning = vs.burn_state == 1 || vs.burn_state == 2;
        vs.two_wait = vs.two_wait.saturating_sub(1);
        if vs.two == 0 {
            if input.xfer && vs.two_wait == 0 && grounded > 0 && st.up().y > 0.8 && !burning && st.speed() > gt.lean_speed_min && input.steer.abs() > 0.05 {
                // (steering right lifts the right side)
                vs.two = if input.steer > 0.0 { 1 } else { -1 };
                vs.two_target = roll;
                vs.two_err = 0.0;
                vs.two_trim = 0.0;
                st.ang_vel += st.forward() * (-(vs.two as f32) * gt.lean_impulse_up / 30.0);
            }
        } else {
            let fallen = st.up().y < 0.4;
            let let_go = !input.xfer || fallen;
            // (a nitrous shot ends it too)
            let lost = st.speed() < gt.lean_speed_min || fwd_speed < 0.0 || input.steer.abs() > 0.95 || (grounded == 0 && st.airborne_time > 1.0) || vs.nitro_level > 0.0;
            if let_go || lost {
                if grounded > 0 {
                    st.ang_vel += st.forward() * (vs.two as f32 * gt.lean_impulse_dn / 30.0);
                }
                vs.two = 0;
                vs.two_wait = 10;
            } else {
                let goal = gt.lean_angle * vs.two as f32;
                let step = gt.lean_rate_up.max(0.1) * dt;
                vs.two_target += (goal - vs.two_target).clamp(-step, step);
                let err = vs.two_target - roll;
                let d = if vs.two_err != 0.0 { (err - vs.two_err) / dt } else { 0.0 };
                vs.two_err = err;
                let (mut e, mut gain) = (err, gt.lean);
                if gt.lean_limit > 0.0 {
                    e = e.clamp(-gt.lean_limit, gt.lean_limit);
                    gain = gt.lean / gt.lean_limit;
                }
                // (the original moves the centre of mass over the wheels - LeanOffsetCG - so that
                // the car balances at the angle; here a slow trim takes up what gravity asks for)
                vs.two_trim = (vs.two_trim + err * dt * 60.0).clamp(-30.0, 30.0);
                vs.two_acc = gain * e + gain.sqrt() * gt.lean_damp * 2.0 * d + vs.two_trim;
            }
        }
    } else {
        vs.two = 0;
    }
    // (on two wheels the steering is held to LeanSteerLimit of its lock)
    let lock = if vs.two != 0 && t.gyro.lean_steer_limit > 0.0 { t.gyro.lean_steer_limit.min(1.0) } else { 1.0 };
    // (mcPlayerInput 0x822a3034 with mcCarDamage 0x82342718: a badly damaged car pulls to one
    // side at low speed - past MedDamage the pull grows as (share of the way to MaxDamage)^0.3,
    // times SteerDamageBias, fading out up to SteerDamageSpeedThreshold)
    let pad_pull;
    let pull = if !input_ai && t.input.steer_damage_bias > 0.0 && t.input.steer_damage_speed_threshold > 1.0 && t.damage.max_damage > t.damage.med_damage {
        let over = ((st.damage * t.damage.max_damage - t.damage.med_damage) / (t.damage.max_damage - t.damage.med_damage)).clamp(0.0, 1.0);
        if over > 0.0 { (1.0 - (speed / t.input.steer_damage_speed_threshold).clamp(0.0, 1.0)) * over.powf(0.3) * t.input.steer_damage_bias } else { 0.0 }
    } else {
        0.0
    };
    pad_pull = pull;
    // (Z2: the original's path in normal play too. The pad's code (mcPlayerInput, physics/pad.rs)
    // holds the steering value P - input object +24: the stick through its dead zone and
    // SteerInputExp, the damaged car's pull added, approached at SteerRate / ReturnRate about the
    // countersteer reference, with mcPlayerInput's own speed factor on the TARGET - and
    // mcCarSim::Update steers every tick with P x the chassis's factor for the speed along the
    // car (0x8232D800), leaving that product in chassis +132. A computer driver's input object
    // gives its P directly: its value is only rate limited here, as before, and scaled the same
    // way. MCLA_DBG_Z2: the port's older smoothing of stick x both factors.)
    let exact_pad = !bike_old(vt) && !dbg_flag("MCLA_DBG_Z2") && !REPLAY_STEER.load(std::sync::atomic::Ordering::Relaxed);
    if exact_pad {
        let row2 = vs.body.as_ref().filter(|b| b.snap == (st.pos, st.rot, st.vel, st.ang_vel)).map(|b| b.rows)
            .unwrap_or_else(|| { let m = Mat3::from_quat(st.rot); [m.x_axis, m.y_axis, m.z_axis] });
        if input.animated {
            // (an animated brain writes the steering value itself, sub_822B97E0: no pad code, no rate)
            vs.pad_steer = input.steer.clamp(-1.0, 1.0);
        } else if !input_ai {
            let sens = ((f32::from_bits(STEER_SENS.load(std::sync::atomic::Ordering::Relaxed)) - 1.0) / 0.5).clamp(-1.0, 1.0);
            let target = (super::pad::steer_target(input.steer, sens, &t.input) + pull).clamp(-1.0, 1.0);
            let sideways = super::exact::fma(row2[0].x, st.vel.x, super::exact::fma(row2[0].z, st.vel.z, row2[0].y * st.vel.y));
            let (brake_pad, hand_pad) = (if vs.gear < 0 { input.throttle } else { input.brake }, if input.handbrake { 1.0 } else { 0.0 });
            if pad_exact {
                // (A2: what the pad's last frame left, input +24 - see ahead of `physics_frame`)
                vs.pad_steer = vs.pad.steer_next;
            } else {
            vs.pad_steer = super::pad::steer_step(vs.pad_steer, target, &t.input, &super::pad::SteerIn {
                speed: vs.last_speed, sideways, chassis_factor: vs.sss_chassis_exact, steering_limit: t.wheel_front.steering_limit,
                brake: brake_pad, handbrake: hand_pad, sens, dt, cruise: false,
            });
            }
        } else {
            let want = input.steer.clamp(-1.0, 1.0);
            let returning = want.abs() < vs.pad_steer.abs() || want * vs.pad_steer < 0.0;
            let mut rate = if returning { t.input.return_rate.max(1.0) } else { t.input.steer_rate.max(1.0) };
            if want * vs.pad_steer < 0.0 {
                rate *= t.input.countersteer_factor.max(1.0);
            }
            vs.pad_steer += (want - vs.pad_steer).clamp(-rate * dt, rate * dt);
        }
        let along = super::exact::fma(row2[2].x, st.vel.x, super::exact::fma(row2[2].z, st.vel.z, row2[2].y * st.vel.y)).abs();
        let k = sss_factor(&t.s, along);
        vs.sss_chassis_exact = k;
        vs.sss_chassis = k;
        vs.steer = (vs.pad_steer * k).clamp(-lock, lock);
        if vt.bike && burn_in.unwrap_or(vs.burn) > 0.0 {
            vs.steer = super::exact::fnms(burn_in.unwrap_or(vs.burn) * vs.steer, BIKE_BURN_STEER, vs.steer);
        }
        vs.steer_back = vs.steer;
    } else {
    let want = ((input.steer + pull).clamp(-1.0, 1.0) * sss).clamp(-lock, lock);
    let returning = want.abs() < vs.steer.abs() || want * vs.steer < 0.0;
    let mut rate = if returning { t.input.return_rate.max(1.0) } else { t.input.steer_rate.max(1.0) };
    if !input_ai {
        rate *= f32::from_bits(STEER_SENS.load(std::sync::atomic::Ordering::Relaxed));
    }
    if want * vs.steer < 0.0 {
        rate *= t.input.countersteer_factor.max(1.0);
    }
    vs.steer += (want - vs.steer).clamp(-rate * dt, rate * dt);
    }
    // (P3, 0x8232FF6C: with the rider off the air's damping factor is AeroDampFactor - and the chassis's update
    // that follows moves it one time step towards one)
    if wiped && t.s.aero_damp_factor < 1.0 {
        vs.aero_damp = (dt + t.s.aero_damp_factor).min(1.0);
    }
    // (0x82330864, further down the chassis's update, the gyro at rest: lying past flat - the chassis's up below
    // level - or in the air longer than AeroDampDelay, the factor is AeroDampFactor itself)
    if wiped && vs.bg.state == 0 && vs.bg.intent == 0 && ((st.rot * Vec3::Y).y < 0.0 || vs.air584 > t.s.aero_damp_delay) {
        vs.aero_damp = t.s.aero_damp_factor;
    }
    // (a replay's steering is the chassis's own value of a recording, +132)
    if REPLAY_STEER.load(std::sync::atomic::Ordering::Relaxed) {
        if bike_old(vt) || dbg_flag("MCLA_DBG_OLDSTEER") {
            vs.steer = input.steer;
        } else {
            // (mcCarSim::Update, 0x8232EF50..0x8232EFC0 and 0x823302C4, operation for operation.
            // The pad's code leaves its steering in chassis +132 once a frame - every other
            // tick, as a rule; each update turns the wheels with that value times the factor
            // for the car's speed along its length, 0x8232D800, and leaves the PRODUCT in +132.
            // So a recording's +132 at a tick's entry is the pad's value where the pad has just
            // written it, and the last update's product where it has not - and then the update
            // steers with the frame's pad value again, taken from the input object, not with
            // that product. The pad has written where the entry value is not the product the
            // last update left.)
            if input.steer.to_bits() != vs.steer_back.to_bits() {
                vs.pad_steer = input.steer;
            }
            let row2 = vs.body_exact.as_ref().map(|b| b[2])
                .or(vs.body.as_ref().filter(|b| b.snap == (st.pos, st.rot, st.vel, st.ang_vel)).map(|b| b.rows[2]))
                .unwrap_or_else(|| Mat3::from_quat(st.rot).z_axis);
            let along = super::exact::fma(row2.x, st.vel.x, super::exact::fma(row2.z, st.vel.z, row2.y * st.vel.y)).abs();
            let k = sss_factor(&t.s, along);
            vs.steer = vs.pad_steer * k;
            // (P3, 0x8232EDEC: the steering walks at 2.5 a second from the pad's value towards the body's
            // turning rate about the world's z, within one either way)
            if wiped {
                let w = st.ang_vel.z;
                let target = if w - (-1.0) >= 0.0 { w } else { -1.0 };
                let target = if target - 1.0 >= 0.0 { 1.0 } else { target };
                let mut held = vs.pad_steer;
                super::exact::approach(&mut held, target, 2.5, dt);
                vs.steer = held * k;
            }
            // (0x8232EF9C: a motorcycle - two wheels - with burnout charge steers less by charge x 0.9)
            if vt.bike {
                vs.sss_chassis = k;
                if burn_in.unwrap_or(vs.burn) > 0.0 {
                    vs.steer = super::exact::fnms(burn_in.unwrap_or(vs.burn) * vs.steer, BIKE_BURN_STEER, vs.steer);
                }
            }
            vs.steer_back = vs.steer;
        }
    }

    // (chassis +572 is from the tick's entry: by the layout as the melt found it)
    let drive_then = if vs.drive == drive_melted { drive_before } else { vs.drive };
    vs.shaft_speed_in = vs.omega[0] * if drive_then == 0 { t.wheel_back.radius } else { t.wheel_front.radius };
    // (MCLA_DBG_OLDORDER: the tyres' melt after the transmission's update, as before)
    if dbg_flag("MCLA_DBG_OLDORDER") {
        let gear_then = if dbg_flag("MCLA_DBG_OLDMELT") { vs.gear } else { automatic_old_gear };
        tire_fric_update(&mut vs, t, input, gear_then, dt);
    }
    if dbg_flag("MCLA_DBG_GEAR") {
        eprintln!("GEAR was {} now {} drive {} melt {:?} burn {:?} omega {:?} engine {:?} timer {:?} change {:?}", automatic_old_gear, vs.gear, vs.drive, vs.melt, vs.burn, vs.omega, vs.engine_w, vs.shift_timer, vs.gear_change);
    }

    // ---- the turn gyro (mcCarGyro): pulls the yaw rate towards what the steering asks for ----
    // target = the yaw rate of a car rolling without slip at this lock, scaled by TurnBias
    // (and by TurnOversteer on the throttle); the error, limited to TurnLimit, becomes a
    // sideways force at the front axle, with a damping term on its change.
    {
        let gt = &t.gyro;
        let up = st.up();
        let yaw = st.ang_vel.dot(up);
        let grounded = st.wheels.iter().filter(|w| w.grounded).count() as f32;
        vs.turn_force = 0.0;
        // (with the sums kept in the original's own operations the turn is worked out in the
        // physics step, after the wheels, as the original does it - see `exact_turn` there)
        let exact_turn = !vt.bike && !dbg_flag("MCLA_DBG_OLDSUMS") && !dbg_flag("MCLA_DBG_OLDROWS") && !dbg_flag("MCLA_DBG_OLDTYREIN") && !dbg_flag("MCLA_DBG_OLDGYRO") && (input_ai || !input.handbrake || !dbg_flag("MCLA_DBG_OLDDRIFT"));
        if exact_turn {
        } else if gt.turn_factor > 0.0 && gt.turn > 0.0 && grounded > 0.0 && !no_gyro() {
            let wheelbase = (p.wheel_pos[2].z - p.wheel_pos[0].z).abs().max(1.0);
            let wf = &t.wheel_front;
            // (steering right turns the car clockwise seen from above: a negative rate about up)
            let target = -(1.0 - wf.optimum_slip) * wf.steering_limit * vs.steer * fwd_speed / wheelbase * gt.turn_bias * ((input.throttle - 0.5) * 2.0 * gt.turn_oversteer + 1.0);
            // (mcCarGyro 0x82367F40..0x82368038, gyro +72: melted tyres take the gyro's say away;
            // it grows with the speed up to SSSThresholdMin; a charged burnout gives it back)
            let mut x = 1.0 - vs.melt;
            if fwd_speed.abs() < t.s.sss_threshold_min {
                x = 1.0 / t.s.sss_threshold_min * x * fwd_speed.abs();
            }
            if vs.burn > x {
                x = vs.burn;
            }
            let x = x.max(0.0).min(1.0);
            let blend = if input_ai && gyro_full() { 1.0 } else { gt.turn_factor * x };
            let err = blend * (target - yaw);
            let d = if vs.turn_err != 0.0 { (err - vs.turn_err) / dt } else { 0.0 };
            vs.turn_err = err;
            let (mut e, mut gain) = (err, gt.turn);
            if gt.turn_limit > 0.0 {
                e = e.clamp(-gt.turn_limit, gt.turn_limit);
                gain = gt.turn / gt.turn_limit;
            }
            let mut out = gain * e + gain.sqrt() * gt.turn_damp * 2.0 * d;
            if dbg_flag("MCLA_DBG_GYRO") { eprintln!("GYRO target {target:.4} yaw {yaw:.4} x {x:.4} blend {blend:.4} err {err:.4} d {d:.4} out {out:.4} steer {:.4} v {fwd_speed:.3}", vs.steer); }
            if fwd_speed.abs() < 10.0 {
                out *= fwd_speed.abs() * 0.1;
            }
            // Spin: brake and handbrake together swing the car round with the steering
            // (full strength from 25 m/s); RevSpin: so does the handbrake while backing up at
            // more than 5 m/s in reverse gear (full from 10 m/s). Either stands in for the turn.
            let burning = vs.burn_state == 1 || vs.burn_state == 2;
            // (the player's car only: the original gates both with a flag of the gyro)
            // (Spin and RevSpin are angular accelerations: times the body's inertia about its
            // height, 0x82367D30 / 0x82367E48; the turn's output is taken times the mass)
            let mut scale = t.s.mass * st.wheels.iter().find(|w| w.grounded).map_or(1.0, |w| super::collision::surface(super::collision::Surface::from_u8(w.surface)).0);
            if input_ai {
            } else if input.handbrake && input.brake > 0.0 && !burning && gt.spin > 0.0 && vs.gear >= 0 {
                out = -vs.steer * (fwd_speed.abs() * 0.04).min(1.0) * gt.spin;
                scale = vt.inertia_yaw;
                vs.turn_err = 0.0;
            } else if input.handbrake && (!burning || input.throttle <= 0.0) && vs.gear < 0 && fwd_speed < -5.0 && gt.rev_spin > 0.0 {
                out = vs.steer * (fwd_speed.abs() * 0.1).min(1.0) * gt.rev_spin;
                scale = vt.inertia_yaw;
                vs.turn_err = 0.0;
            } else if input.handbrake && !burning {
                // (0x82367DD8 -> 0x82367EA4: on the handbrake the turn is left out altogether -
                // the recorded target and error are 0 for as long as it is held)
                out = 0.0;
                vs.turn_err = 0.0;
            }
            // a torque about the ground's normal (N m): positive turns the car anticlockwise seen from above
            vs.turn_force = out * scale * (grounded / if vt.bike { 2.0 } else { 4.0 });
        } else {
            vs.turn_err = 0.0;
        }
        // in the air the car levels its roll (Roll, RollLimit, RollDamp) and can be yawed (Yaw)
        let right = st.right();
        let roll = right.y.clamp(-1.0, 1.0).asin();
        vs.air_acc = [0.0; 3];
        // (R2: not a motorcycle on the original's path - this levelling is the port's own; the recorded sums of
        // a ZX-14 in the air hold the air's damping and the gyro's gravity pitch and nothing else: V se.0
        // 163.. with the rider off, 179 stood up again. MCLA_DBG_R2: as before.)
        if grounded == 0.0 && st.airborne_time > 0.15 && !(vt.bike && !bike_old(vt) && !dbg_flag("MCLA_DBG_R2")) {
            let (mut r, mut gain) = (roll, gt.roll);
            if gt.roll_limit > 0.0 {
                r = r.clamp(-gt.roll_limit, gt.roll_limit);
                gain = gt.roll / gt.roll_limit;
            }
            // GravityPitch: in flight the nose follows the way the car is falling (within 30 degrees)
            if gt.gravity_pitch != 0.0 && !vt.bike && speed > 1.0 {
                let path = (st.vel.y / speed).clamp(-1.0, 1.0).asin().clamp(-0.523599, 0.523599);
                let pitch = st.forward().y.clamp(-1.0, 1.0).asin();
                vs.air_acc[2] += gt.gravity_pitch * (path - pitch);
            }
            // (the stick turns and tips the car - Yaw, Pitch; left alone, the car levels its roll)
            let (sx, sy) = (input.steer.clamp(-1.0, 1.0), input.lean.clamp(-1.0, 1.0));
            if sx.abs() > 0.2 || sy.abs() > 0.2 {
                vs.air_acc[1] = -sx * gt.yaw;
                if !vt.bike {
                    vs.air_acc[2] += sy * gt.pitch;
                }
            } else if gt.roll > 0.0 {
                vs.air_acc[0] = -(gain * r + gain.sqrt() * gt.roll_damp * 2.0 * (roll - vs.air_roll) / dt);
            }
        }
        vs.air_roll = roll;
        // DriftTorque: the engine pulls harder the more the car steers or slides sideways
        // (mcCarGyro 0x82366CF8..0x82366E64: only while no burnout is charged and the car moves
        // faster than 1 m/s - below that the multiplier keeps its last value, so a car that
        // rolled to a stop up a hill pulls away with the hill's extra. The extra is a share of
        // the engine's torque at its speed of best power, vehEngine +20.)
        if !vt.bike && !dbg_flag("MCLA_DBG_OLDSUMS") && !dbg_flag("MCLA_DBG_OLDGYRO") {
            // (0x82366D1C..0x82366E64, operation for operation: the velocity made a unit
            // vector, its share along the car's right through the eased sine, the steering,
            // DriftTorque; the climb by the guest runtime's arcsine; the larger over the
            // engine's reference torque, plus one)
            use super::exact::{ease, fma, guest_asin, unit_or_zero};
            let v = st.vel;
            let rows = vs.body_exact.map(|m| [m[0], m[1], m[2]]).or(vs.body.filter(|b| b.snap == (st.pos, st.rot, st.vel, st.ang_vel)).map(|b| b.rows)).unwrap_or_else(|| { let m = Mat3::from_quat(st.rot); [m.x_axis, m.y_axis, m.z_axis] });
            if vs.burn == 0.0 && fma(v.z, v.z, fma(v.x, v.x, v.y * v.y)).sqrt() > 1.0 {
                let vn = unit_or_zero(v);
                let side = fma(vn.x, rows[0].x, fma(rows[0].z, vn.z, rows[0].y * vn.y)).abs();
                let side = if side - 1.0 >= 0.0 { 1.0 } else { side };
                let (slide, wheel) = (ease(side), vs.steer.abs());
                let drift = (if slide - wheel >= 0.0 { slide } else { wheel }) * gt.drift_torque;
                let climb = if input_ai {
                    0.0
                } else {
                    let a = (guest_asin(vn.y as f64) as f32) / gt.hill_grade;
                    let a = if a >= 0.0 { a } else { 0.0 };
                    (if a - 1.0 >= 0.0 { 1.0 } else { a }) * gt.hill_torque
                };
                vs.torque_mult = (if drift - climb >= 0.0 { drift } else { climb }) / vt.sync_torque + 1.0;
            }
        } else if !wiped && vs.burn == 0.0 && speed > 1.0 {
            let sideways = (st.vel / speed).dot(right).abs().min(1.0);
            let slide = (sideways * std::f32::consts::FRAC_PI_2).sin();
            // HillTorque: and the steeper the way the car is travelling climbs, up to the full
            // extra at HillGrade (the player's car only); the larger of the two counts
            let climb = if input_ai { 0.0 } else { ((st.vel.y / speed).clamp(-1.0, 1.0).asin() / gt.hill_grade).max(0.0).min(1.0) * gt.hill_torque };
            vs.torque_mult = (slide.max(vs.steer.abs()) * gt.drift_torque).max(climb) / vt.sync_torque + 1.0;
        }
    }


    // ---- motorcycles (mcBikeGyro) ----
    vs.wheelie_lift = 0.0;
    if bike_old(vt) {
        let gt = &t.gyro;
        let up = st.up();
        // ---- mcBikeGyro::Update 0x823688B8 (translated from the original, checked on recorded
        // rides). The velocity and its change, low-passed (0.4 and 0.15 a tick at 60 Hz) ----
        let filtered = st.vel * 0.4 + vs.bike_vel * 0.6;
        let acc = (filtered - vs.bike_vel) / dt;
        vs.bike_acc = acc * 0.15 + vs.bike_acc * 0.85;
        vs.bike_vel = filtered;
        let (front, rear) = (st.wheels[0], st.wheels[2]);
        let down = front.grounded as u32 + rear.grounded as u32;
        let wheelbase = (p.wheel_pos[2].z - p.wheel_pos[0].z).abs().max(0.5);
        let body = st.pos + st.rot * Vec3::new(t.s.model_offset_x, t.s.model_offset_y, t.s.model_offset_z);
        // ---- the turn (0x82368BB4): with Turn at 0, as the bikes have it, the rate of turning
        // about the ground under the rear wheel is moved straight towards what the steering
        // asks, by TurnFactor of the way where the chassis has taken steering lock away for
        // the speed (1 - its factor) - none at town speeds ----
        vs.bike_turn = Vec3::ZERO;
        vs.bike_push = Vec3::ZERO;
        if down > 0 && gt.turn <= 0.0 && !REPLAY_NO_BIKE_GYRO.load(std::sync::atomic::Ordering::Relaxed) {
            let n = if rear.grounded { rear.normal } else { Vec3::Y };
            let v = if fwd_speed < 0.0 && vs.omega[0] >= 0.0 { -fwd_speed } else { fwd_speed };
            let blend = gt.turn_factor * (1.0 - vs.sss_chassis);
            let yaw = st.ang_vel.dot(n);
            let base = t.wheel_front.steering_limit * vs.steer * v * gt.turn_bias * (-1.0 / wheelbase);
            let pedal = (input.throttle - if input.xfer { 1.0 } else { 0.0 }).clamp(-1.0, 1.0);
            let target = (pedal * gt.turn_oversteer + 1.0) * base * blend + (1.0 - blend) * yaw;
            let scale = if v.abs() < 5.0 { v.abs() * 0.2 } else { 1.0 };
            let grip = if front.grounded { super::collision::surface(super::collision::Surface::from_u8(front.surface)).0 } else { 1.0 };
            st.ang_vel += n * ((1.0 - gt.turn_damp) * (grip * scale) * (target - yaw));
        }
        // ---- the lean (0x82369A38): the rider leans to a share (LeanAngle; LeanAngleMax on
        // the weight button; six tenths in a wheelie) of the angle that balances the sideways
        // pull - the measured one, low-passed, less where the chassis steers fully, mixed by
        // LeanFactor with what the steering asks for at this speed. The roll asked of the
        // body moves there at LeanRateUp / LeanRateDn; and with Lean at 0 the body is simply
        // turned about the line through its tyres' contacts, (1 - LeanDamp) of the way a tick ----
        let (line, normal, middle) = if down == 2 {
            ((rear.contact - front.contact) / wheelbase, (front.normal + rear.normal) * 0.5, (front.contact + rear.contact) * 0.5)
        } else if down == 1 {
            let w = if rear.grounded { rear } else { front };
            (st.rot * Vec3::Z, w.normal, w.contact)
        } else {
            let back = st.rot * Vec3::Z;
            (Vec3::new(back.x, 0.0, back.z).normalize_or(Vec3::Z), Vec3::Y, st.pos)
        };
        if st.up().y > -0.7 {
            let mut share = if input.xfer { gt.lean_angle_max } else { gt.lean_angle };
            if vs.wheelie != 0 {
                share *= 0.6;
            }
            if vs.lean_scale == 0.0 {
                vs.lean_scale = share;
            }
            vs.lean_scale += (share - vs.lean_scale).clamp(-gt.lean_rate_dn * dt, gt.lean_rate_dn * dt);
            let side = line.cross(normal).normalize_or(-st.right());
            let mut spd = speed;
            if vs.burn > 0.0 && vs.gear > 0 {
                spd = spd.max(vs.engine_w / (t.engine.s.opt_rpm * 0.104_719_76).max(1.0) * gt.burnout_lean);
            }
            let g = vt.gravity;
            // (the front wheel's own angle, +492, is the other way round from this port's)
            let asked = vs.steer.abs() * (1.0 - t.wheel_front.optimum_slip) * -front.steer / wheelbase * spd * spd;
            let lean_steer = vs.lean_scale * asked.atan2(g);
            let pull = vs.bike_acc.dot(side) * (1.0 - vs.sss_chassis);
            vs.lean_lat = 0.85 * vs.lean_lat + 0.15 * pull;
            let lean_pull = vs.lean_scale * vs.lean_lat.atan2(g);
            let mut want = (1.0 - gt.lean_factor) * lean_pull + gt.lean_factor * lean_steer;
            if vs.burn > 0.0 {
                let yaw = (st.ang_vel.dot(normal) / gt.burnout_limit).clamp(-gt.burnout_limit, gt.burnout_limit);
                want += (yaw * gt.burnout_lean - want) * vs.burn;
            }
            let cur = vs.lean_target;
            let rate = if down == 0 || (cur <= 0.0 && want <= 0.0 && want <= cur) || (cur >= 0.0 && want >= 0.0 && want >= cur) { gt.lean_rate_dn } else { gt.lean_rate_up };
            vs.lean_target += (want - cur).clamp(-rate * dt, rate * dt);
            let roll = st.right().y.clamp(-1.0, 1.0).asin();
            let err = roll - vs.lean_target;
            if gt.lean <= 0.0 && !REPLAY_NO_BIKE_GYRO.load(std::sync::atomic::Ordering::Relaxed) {
                vs.bike_turn = line * -((1.0 - gt.lean_damp) * err * vs.aero_damp);
                vs.bike_push = (middle - body).cross(vs.bike_turn);
            }
        }
        // (the body itself leans now: nothing is added for the eye)
        vs.lean = 0.0;

        // the wheelie: leaning back on the throttle with the rear wheel down starts it; a
        // controller then holds the pitch the rider asks for (WheelieAngle x how far back and
        // how much throttle), and for WheelieBoostDuration the bike is pushed along
        let front_down = st.wheels[0].grounded || st.wheels[1].grounded;
        let rear_down = st.wheels[2].grounded || st.wheels[3].grounded;
        let speed = fwd_speed;
        if gt.wheelie_angle > 0.0 {
            if vs.wheelie == 0 && !input.ai && speed > 4.47 && rear_down && input.throttle > 0.5 && input.brake == 0.0 && input.lean > 0.8 {
                vs.wheelie = 1;
                vs.wheelie_boost = gt.wheelie_boost_duration;
                vs.wheelie_err = 0.0;
                vs.wheelie_pitch = st.forward().y.clamp(-1.0, 1.0).asin();
            }
            if vs.wheelie == 1 {
                let over = up.y < -0.7;
                if input.throttle <= 0.0 || input.brake > 0.9 || over || speed < 4.47 || input.lean < -0.5 || (front_down && vs.wheelie_boost <= 0.0) {
                    vs.wheelie_boost = 0.0;
                    vs.wheelie = 99;
                }
            }
            if vs.wheelie == 99 && front_down {
                vs.wheelie = 0;
            }
            // the stoppie (NosieAngle): leaning forward on the brake, off the throttle, with the
            // front wheel down lifts the tail the same way; it ends with the brake, the lean or
            // the speed, and is over when the rear wheel is back on the road
            if vs.wheelie == 0 && !input.ai && gt.nosie_angle > 0.0 && speed > 4.47 && front_down && input.brake > 0.0 && input.throttle < 0.11 && input.lean < -0.8 {
                vs.wheelie = 2;
                vs.wheelie_err = 0.0;
                vs.wheelie_pitch = st.forward().y.clamp(-1.0, 1.0).asin();
            }
            if vs.wheelie == 2 && (input.brake <= 0.0 || input.lean > -0.3 || speed < 4.47 || up.y < 0.2 || !front_down) {
                vs.wheelie = 98;
            }
            if vs.wheelie == 98 && rear_down {
                vs.wheelie = 0;
            }
            if vs.wheelie > 0 {
                let x = ((input.lean + 1.0) * 0.5 * input.throttle).clamp(0.0, 1.0);
                let mut amount = (x * std::f32::consts::FRAC_PI_2).sin() * 2.0 - 1.0;
                if vs.wheelie == 99 {
                    amount = -0.15;
                } else if amount < 0.0 {
                    amount *= 0.15;
                }
                let pitch = st.forward().y.clamp(-1.0, 1.0).asin();
                // (past 75 degrees the aim is to come back)
                let aim = match vs.wheelie {
                    2 => -gt.nosie_angle * ((-input.lean).clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).sin() * input.brake.clamp(0.0, 1.0) * (speed / 10.0).min(1.0),
                    98 => 0.0,
                    _ if pitch > 1.309 => -2.0944,
                    _ => gt.wheelie_angle * amount,
                };
                let step = gt.wheelie_rate.max(0.1) * dt;
                vs.wheelie_pitch += (aim - vs.wheelie_pitch).clamp(-step, step);
                let err = vs.wheelie_pitch - pitch;
                let d = if vs.wheelie_err != 0.0 { (err - vs.wheelie_err) / dt } else { 0.0 };
                vs.wheelie_err = err;
                let (mut e, mut gain) = (err, gt.wheelie);
                if gt.wheelie_limit > 0.0 {
                    e = e.clamp(-gt.wheelie_limit, gt.wheelie_limit);
                    gain = gt.wheelie / gt.wheelie_limit;
                }
                // an upward acceleration at the front wheel
                vs.wheelie_lift = gain * e + gain.sqrt() * gt.wheelie_damp * 2.0 * d;
                if vs.wheelie_boost > 0.0 {
                    vs.wheelie_boost = (vs.wheelie_boost - dt).max(0.0);
                }
            } else {
                vs.wheelie_err = 0.0;
                vs.wheelie_pitch = 0.0;
            }
        }
    } else {
        vs.lean = 0.0;
    }

    vs.hyd.step(&t.hydraulics, if vt.bike { 0 } else { input.hyd }, dt);
    st.upset = (st.upset - dt / t.damage.wipeout_delay.max(0.1)).max(0.0);
    // ---- A2: the pad's code (mcPlayerInput::Update 0x822A2988), on the first of a frame's two ticks,
    // between this update and the physics step - with the frame's step, 0x827D7558 = two ticks. It
    // reads the chassis as the update left it: +224 (the speed the tick came in with), the body's
    // velocity along and across the car, +568 (this update's speed factor). Its pedals and way are
    // for the next update - but the way is what this step's contact callbacks already find. ----
    if pad_exact {
        if !vs.pad.second_tick {
            let frame = dt + dt;
            let rows = vs.body.as_ref().filter(|b| b.snap == (st.pos, st.rot, st.vel, st.ang_vel)).map(|b| b.rows)
                .unwrap_or_else(|| { let m = Mat3::from_quat(st.rot); [m.x_axis, m.y_axis, m.z_axis] });
            let v = st.vel;
            let speed224 = fma32(v.z, v.z, fma32(v.x, v.x, v.y * v.y)).sqrt();
            let forward = -super::exact::fma(rows[2].x, v.x, super::exact::fma(rows[2].z, v.z, rows[2].y * v.y));
            // (RB4: what the pad's code is handed is the mapper's value of the pad's BYTE, 0x822A4BB8 - a trigger
            // over 255 through a dead zone of 0.05, a stick's axis (byte - 127.5) / 127.5 through 0.1 - ahead of
            // mcPlayerInput's own zones. V: every logged frame of the stage-R recordings, physics/pad.rs
            // `recorded_frames`. MCLA_DBG_RB4: this port's floats straight in, no cruise, the older lean.)
            let rb4 = !dbg_flag("MCLA_DBG_RB4");
            let (rt, lt, stick_x) = if rb4 {
                (super::pad::trigger_axis(super::pad::trigger_byte(raw_input.throttle)), super::pad::trigger_axis(super::pad::trigger_byte(raw_input.brake)), super::pad::stick_axis(super::pad::stick_byte(raw_input.steer)))
            } else {
                (raw_input.throttle, raw_input.brake, raw_input.steer)
            };
            vs.pad.pedals = super::pad::pedals(vs.pad.pedals, &t.input, &super::pad::PedalIn { rt, lt, speed: speed224, forward, bike: vt.bike });
            vs.pad.handbrake = raw_input.handbrake;
            if replay_way == 0 && forced == 0 {
                vs.way_after = vs.pad.pedals.way as i32;
            }
            // (RB4, the cruise 0x822A25E0, after the pedals and ahead of the steering: on a road with a speed
            // limit a half-pressed trigger holds that speed. The limit is the world's: `CRUISE_LIMIT_MPH`.)
            if rb4 {
                let driving = !wiped && vs.pad.pedals.way > 0 && !vs.nitro_burning && if vt.bike { vs.bg.intent == 0 } else { vs.two == 0 };
                let limit_mph = f32::from_bits(CRUISE_LIMIT_MPH.load(std::sync::atomic::Ordering::Relaxed));
                let (throttle, _) = super::pad::cruise(&mut vs.pad.cruise, vs.pad.pedals.throttle, &t.input, &super::pad::CruiseIn { speed: speed224, limit_mph, driving, modulation: vs.shift_mod[1] });
                vs.pad.pedals.throttle = throttle;
            } else {
                vs.pad.cruise = Default::default();
            }
            // (the steering, 0x822A2F74.. and 0x822A1520, after the pedals: it reads the brake just made)
            let sens = ((f32::from_bits(STEER_SENS.load(std::sync::atomic::Ordering::Relaxed)) - 1.0) / 0.5).clamp(-1.0, 1.0);
            let target = (super::pad::steer_target(stick_x, sens, &t.input) + pad_pull).clamp(-1.0, 1.0);
            let sideways = super::exact::fma(rows[0].x, v.x, super::exact::fma(rows[0].z, v.z, rows[0].y * v.y));
            vs.pad.steer_next = super::pad::steer_step(vs.pad_steer, target, &t.input, &super::pad::SteerIn {
                speed: speed224, sideways, chassis_factor: vs.sss_chassis_exact, steering_limit: t.wheel_front.steering_limit,
                brake: vs.pad.pedals.brake, handbrake: if raw_input.handbrake { 1.0 } else { 0.0 }, sens, dt: frame, cruise: vs.pad.cruise.on,
            });
            // (0x822A3110: the stick's other axis, input +32, goes towards the stick at 5 a second)
            if rb4 {
                // (RB4, 0x822A30AC: read only while the weight button was down the frame BEFORE - this port's
                // one key for the rider's weight is the button and the stick together - through the stick's two
                // dead zones; otherwise nothing at once)
                vs.pad.stick_next = super::pad::lean_step(vs.pad.stick_y, super::pad::stick_byte(raw_input.lean), vs.pad.button, frame);
                vs.pad.button = raw_input.xfer || raw_input.lean != 0.0;
            } else {
            let mut y = vs.pad.stick_y;
            super::exact::approach(&mut y, raw_input.lean.clamp(-1.0, 1.0), 5.0, frame);
            vs.pad.stick_next = y;
            }
        }
        vs.pad.second_tick = !vs.pad.second_tick;
    }
    // ---- RB3: the nitrous button (mcPlayerInput::Update 0x822A337C..0x822A3458: the button DOWN - a level,
    // not a press - sets flag 8 of input +40; 0x822A1058 at the end of the same frame calls the fire routine
    // 0x82345988 -> 0x82345698: not burning (nitro +32), the view back at rest (+24 == +20), a shot left
    // (+18 > 0), the car answering its pad, not lying on the ground (0x823455B8: air time <= 0 and the
    // chassis's up.y < 0.1) -> 0x82345820: level 1, burning, one shot less). The pad's frame stands between
    // this tick's vehicle update - mcCarNitro's own update with it - and the physics step, so what it reads is
    // the object as this tick's update leaves it: the port's mcCarNitro block is inside `physics_frame`, the
    // test is finished right after it. V: sd.0 (fired once, refused while burning and 2 and 20 ticks after
    // the burn's end), sd.1 (button pulsed: fired in the first pad frame after the view is back, 61 ticks
    // after each end). ----
    let nitro_try = {
        let pressed = if REPLAY_STEER.load(std::sync::atomic::Ordering::Relaxed) {
            REPLAY_NITRO_PAD.load(std::sync::atomic::Ordering::Relaxed)
        } else {
            // (the pad's frame is the first of every two ticks: `second_tick` was flipped just above)
            pad_exact && vs.pad.second_tick && raw_input.nitro
        };
        let up_y = vs.body.as_ref().filter(|b| b.snap == (st.pos, st.rot, st.vel, st.ang_vel)).map(|b| b.rows[1].y).unwrap_or_else(|| Mat3::from_quat(st.rot).y_axis.y);
        let upset = !(vs.air584 > 0.0) && up_y < 0.1;
        fire_exact && pressed && !upset && !wiped
    };
    let backup = (st.pos, st.rot, st.wheels);
    physics_frame(st, &mut vs, p, vt, input, ratio, world, dt);
    if nitro_try && !vs.nitro_burning && vs.nitro_view == 0.0 && st.nitro_shots > 0 {
        vs.nitro_level = 1.0;
        vs.nitro_burning = true;
        st.nitro_shots -= 1;
    }
    if dbg_flag("MCLA_DBG_TRACE") {
        // (the nitro object as the next sample of a recording shows it: stageR/agB/nitrocmp.py)
        eprintln!("NITRO {} {} {:08x} {:08x}", vs.nitro_burning as u8, st.nitro_shots, vs.nitro_level.to_bits(), vs.nitro_view.to_bits());
    }
    let sane = st.pos.is_finite() && st.rot.is_finite() && st.vel.is_finite() && st.ang_vel.is_finite() && st.vel.length() < 250.0 && st.ang_vel.length() < 60.0;
    if !sane {
        log::warn!("vehsim: state went wild at {:.0} {:.0} {:.0} (vel {:.1}, spin {:?}): put back; mass {:.0} inertia {:?} bike {} ai {} steer {:.2} gear {} omega {:?} loads {:?} comp {:?} turn force {:.0}", backup.0.x, backup.0.y, backup.0.z, st.vel.length(), (st.rot.inverse() * st.ang_vel).to_array().map(|v| v.round()), t.s.mass, [t.s.inertia_x, t.s.inertia_y, t.s.inertia_z], vt.bike, input_ai, vs.steer, vs.gear, vs.omega.map(|v| v.round()), vs.wheels.map(|w| w.load.round()), vs.wheels.map(|w| (w.comp * 100.0).round()), vs.turn_force);
        st.pos = backup.0;
        st.rot = backup.1;
        st.wheels = backup.2;
        st.vel = Vec3::ZERO;
        st.ang_vel = Vec3::ZERO;
        vs = VehState::default();
    }
    st.last_impact = (st.last_impact - dt).max(0.0);
    st.steer = vs.steer * t.wheel_front.steering_limit;
    st.rpm = (vs.engine_w / RPM).max(e.idle_rpm * 0.8);
    st.gear = vs.gear;
    st.engine_factor = 1.0;
    st.veh = Some(Box::new(vs));
}

#[derive(Clone, Copy)]
struct TyreFrame {
    contact: super::tyre::Contact,
    arm: Vec3,
    forward: Vec3,
    lateral: Vec3,
    surf_drag: f32,
    /// how far the tyre sinks into the surface (the material's depth, wheel +448)
    surf_sink: f32,
    mass: f32,
    sliding_fric: f32,
    /// the contact record's normal and point
    normal: Vec3,
    point: Vec3,
    /// the damping the wheel hands the body: its value (wheel +508) and the force with it
    damp: Option<(f32, Vec3)>,
}

#[allow(clippy::too_many_arguments)]
fn physics_frame(st: &mut CarState, vs: &mut VehState, p: &CarParams, vt: &VehTune, input: CarInput, ratio: f32, world: &WorldCollider, dt: f32) {
    // (the speed the car comes into this tick with - chassis +224, written before the step)
    let speed_in = st.vel.length();
    let air_then = st.airborne_time;
    // (the car's up as its last update had it: what the wheels' contacts are weighed against,
    // 0x8255D028 - they are found after the body has moved on, and judged before the car's
    // own matrix follows)
    let up_then = vs.prev_up.unwrap_or(st.rot * Vec3::Y);
    vs.prev_up = Some(st.rot * Vec3::Y);
    // (P3: the rider off a motorcycle, chassis +592 bit 0x10 - work/veh/gjkfix/p2/rider_decode.md. The damage
    // object's timer, set to WipeoutDelay, is counted down once a frame - every other tick - by the frame's
    // step; not while the bike still slides with a wheel in the air, nor while the thrown rider still moves,
    // which a replay is told: MCLA_REPLAY_WIPE="first,from,to" = the first tick after the bit whose update
    // finds a count, and the ticks after the bit between which none is made.)
    let wiped = vt.bike && !bike_old(vt) && !dbg_flag("MCLA_DBG_P3") && vs.wipe.is_some();
    if let (true, Some((timer, age))) = (wiped, vs.wipe.as_mut()) {
        *age += 1;
        static HINT: std::sync::OnceLock<Option<(u32, u32, u32)>> = std::sync::OnceLock::new();
        let hint = *HINT.get_or_init(|| {
            let v: Vec<u32> = std::env::var("MCLA_REPLAY_WIPE").ok()?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (v.len() == 3).then(|| (v[0], v[1], v[2]))
        });
        // (R2: a stage-R recording logs the damage object's frame update, 0x82342EF0 - which ticks it runs
        // ahead of, and the thrown rider's term, his velocity with the vertical part tenfold: no count while
        // the bike is faster than 4.47 m/s (chassis +224) with a wheel off the ground, nor while the rider's
        // term is 4.47 or more. MCLA_DBG_R2: the older hints.)
        let logged = REPLAY_RIDER_ON.load(std::sync::atomic::Ordering::Relaxed) && !dbg_flag("MCLA_DBG_R2");
        let counts = match hint.filter(|_| !logged) {
            _ if logged => {
                let col = f32::from_bits(REPLAY_RIDER.load(std::sync::atomic::Ordering::Relaxed));
                let wheels_down = (0..4).filter(|i| i % 2 == 0 && vs.wheels[*i].record).count();
                col >= 1.0 && !(vs.last_speed >= 4.47 && wheels_down != 2) && !(col - 1.0 >= 4.47)
            }
            Some((first, from, to)) => *age >= first && (*age - first) % 2 == 0 && !(from..=to).contains(age),
            None => {
                let wheels_down = (0..4).filter(|i| i % 2 == 0 && vs.wheels[*i].record).count();
                *age % 2 == 0 && !(speed_in >= 4.47 && wheels_down != 2)
            }
        };
        if counts && *timer > 0.0 {
            *timer -= dt + dt;
        }
        if dbg_flag("MCLA_DBG_CALLS") {
            eprintln!("WIPE age {} timer {:?} counts {} v {:?}", *age, *timer, counts, vs.last_speed);
        }
    }
    // (mcCarSim, 0x82330724: a car that stands - no throttle, no brake, every wheel with a
    // contact, no faster than 5 cm/s the tick before and turning no faster than 0.5 rad/s -
    // counts the time; anything else starts the count again. Past one second its body is
    // frozen each tick, further down.)
    {
        let still = input.throttle == 0.0 && input.brake == 0.0 && st.wheels.iter().enumerate().all(|(i, w)| w.grounded || (vt.bike && i % 2 == 1)) && vs.last_speed <= 0.05 && st.ang_vel.length() <= 0.5;
        vs.still_timer = if still && !dbg_flag("MCLA_DBG_NOFREEZE") { vs.still_timer + dt } else { 0.0 };
        vs.last_speed = speed_in;
    }
    let t = &vt.t;
    let e = &t.engine.s;
    let mass = t.s.mass;
    let g = vt.gravity;
    let rm = Mat3::from_quat(st.rot);
    let (right, up, fwd) = (rm.x_axis, rm.y_axis, -rm.z_axis);
    // (mcCarSim 0x82330378: wiped out, the centre of gravity jumps UpsetCG of the way from its
    // tuned place to the middle of the body and comes back as the wipeout's delay runs down -
    // the car tumbles; wrecked, it stays up there)
    // (mcCarSim::Update 0x8232F910..0x8232FAB4: a burnout puts the centre of mass on the road
    // under BurnWeightxfer of the way to the axle that is held - the weight goes onto the braked
    // wheels; afterwards it comes back, 1.5 m/s along the car and 0.5 m/s in height. Recorded:
    // (0, 0, -0.331) on the R8, static loads 6517 / 3910 N against 4741 / 5686.)
    let mut cg_before_burn: Option<f32> = None;
    {
        let tuned = [t.s.com_y, t.s.com_z];
        if !vs.cg_set {
            vs.cg = tuned;
            vs.cg_set = true;
        }
        // (a replay's recorded centre of mass, chassis +112 as the update finds it: the rules below work on it)
        if vs.by.exact {
            if let Some(c) = vs.com_exact.take() {
                vs.cg = [c.y, c.z];
            }
        }
        if vs.by.exact {
            // (0x8232F64C: back along the car at 1.5 m/s - unless charge and handbrake hold it; then
            // the burnout's own place, 0x8232F988; then DriftCG; then, in the chassis's update
            // 0x823304EC, back up at 0.5 m/s)
            // (Z3, a motorcycle in a stunt, 0x8232F35C..0x8232F5DC: by the gyro's state as its last
            // update left it - in a stoppie the centre of mass goes to a quarter of the front
            // wheel's place along the bike, once, from the tune's; and for as long as the state is
            // not 0 (or the turn's intent with the button is set) it is not brought back)
            let stunt = vt.bike && (vs.bg.state != 0 || vs.bg.intent != 0);
            if vt.bike && vs.bg.state == -1 && vs.cg[1] == tuned[1] {
                let front_z = vs.rest_exact.map_or((p.wheel_pos[0].z + p.wheel_pos[1].z) * 0.5, |r| r[0][2]);
                vs.cg[1] = front_z * 0.25;
            }
            if vs.by.cg_return && !stunt {
                super::exact::approach(&mut vs.cg[1], tuned[1], 1.5, dt);
            }
            cg_before_burn = Some(vs.cg[1]);
            if let Some(z) = vs.by.cg {
                vs.cg = [0.0, z];
            }
            if t.s.drift_cg > 0.0 && input.handbrake {
                let target = fma32(super::exact::fms(tuned[0], t.s.drift_cg, tuned[0]), 1.0, tuned[0]);
                super::exact::approach(&mut vs.cg[0], target, 1.5, dt);
            }
            super::exact::approach(&mut vs.cg[0], tuned[0], 0.5, dt);
        } else if vs.burn_state == 1 || vs.burn_state == 2 {
            let held = if matches!(t.drivetrain_type, 0 | 2) { p.wheel_pos[0].z } else { p.wheel_pos[2].z };
            vs.cg = [0.0, held * if input.hold { 0.75 } else { t.s.burn_weightxfer }];
        } else {
            vs.cg[1] += (tuned[1] - vs.cg[1]).clamp(-1.5 * dt, 1.5 * dt);
            // (the end of mcCarSim::Update, 0x823302D8: with the handbrake held a car with a
            // DriftCG - the Golf '07 and the Range Rovers, 0.75 - has its centre of mass go
            // towards that share of its height at 1.5 m/s. Read from the code; no recording
            // has it.)
            if t.s.drift_cg > 0.0 && input.handbrake {
                let target = tuned[0] * t.s.drift_cg;
                vs.cg[0] += (target - vs.cg[0]).clamp(-1.5 * dt, 1.5 * dt);
            }
            vs.cg[0] += (tuned[0] - vs.cg[0]).clamp(-0.5 * dt, 0.5 * dt);
        }
    }
    // (P3: the wheels are weighed by the centre of mass as mcCarSim::Update leaves it; the chassis's update
    // that follows then holds it at CenterOfMass + (ModelOffset - CenterOfMass) x (q x UpsetCG), q the
    // timer's share of WipeoutDelay)
    let weigh_z = vs.cg[1];
    let mut held_x: Option<f32> = None;
    if let (true, Some((timer, _))) = (wiped && vs.by.exact && t.s.upset_cg > 0.0, vs.wipe) {
        let q = timer / t.damage.wipeout_delay;
        let q = if q >= 0.0 { q } else { 0.0 };
        let q = if q - 1.0 >= 0.0 { 1.0 } else { q };
        let s = q * t.s.upset_cg;
        let com = Vec3::new(t.s.com_x, t.s.com_y, t.s.com_z);
        let held = (Vec3::new(t.s.model_offset_x, t.s.model_offset_y, t.s.model_offset_z) - com) * s + com;
        vs.cg = [held.y, held.z];
        held_x = Some(held.x);
    }
    let com_local = {
        let tuned = Vec3::new(held_x.unwrap_or(t.s.com_x), vs.cg[0], vs.cg[1]);
        let k = if st.damage >= 1.0 { 1.0 } else { st.upset };
        tuned + (Vec3::new(t.s.model_offset_x, t.s.model_offset_y, t.s.model_offset_z) - tuned) * (t.s.upset_cg.clamp(0.0, 1.0) * k)
    };
    let com_local = vs.com_exact.take().unwrap_or(com_local);
    let com = st.pos + rm * com_local;
    // (the body the original integrates sits at ModelOffset above the model's origin: its
    // velocity is that point's and it turns about it; only the arms of the forces are taken
    // from the centre of mass - vehWheel::Update 0x82566738 moves each force's point by the
    // difference. Seen in a recording: body position = chassis position + 0.708 up.)
    let pivot_local = Vec3::new(t.s.model_offset_x, t.s.model_offset_y, t.s.model_offset_z);
    let pivot = st.pos + rm * pivot_local;
    // (the body in the original's own numbers as the last tick left it - if nothing else has
    // moved the car since)
    let kept_body = vs.body.take().filter(|b| b.snap == (st.pos, st.rot, st.vel, st.ang_vel) && !bike_old(vt) && !dbg_flag("MCLA_DBG_OLDSTEP"));
    let rot_in = st.rot;
    // (the body's place as the original has it, body +256: what contact velocities turn about)
    let body_at = vs.body_at_exact.take().or(kept_body.map(|b| b.at)).unwrap_or(pivot);
    let inertia = moments(p, t);
    // ---- the force and torque sums. The original's body adds every force and torque as it
    // is applied, in single precision, so the sums' last bits depend on the order: the air
    // (vehAero, ahead of the wheels), each wheel in turn - front left, front right, rear
    // left, rear right; its damping's force, its tyre and spring force, that force's torque -
    // the axles, the gyro, and gravity last (0x82335AA8, 0x82336F8C). `calls` holds what is
    // applied with a key for its place in that order; they are summed at the end.
    // MCLA_DBG_OLDSUMS: the port's older sums. ----
    let exact_sums = !bike_old(vt) && !dbg_flag("MCLA_DBG_OLDSUMS") && !dbg_flag("MCLA_DBG_OLDROWS") && !dbg_flag("MCLA_DBG_OLDTYREIN");
    let mut calls: Vec<(u16, Vec3, Vec3)> = Vec::new();
    // (the launch's lift, finished where the sums are made: the axle's line, the way it is measured along, the rear contacts' slope, the point)
    let mut lift: Option<(Vec3, Vec3, f32, Vec3)> = None;
    // (0x8232FBF4..0x8232FD0C, in mcCarSim::Update - ahead of everything else in the tick's sums: the
    // launch pushes along the driven wheels' way - the sum of their contact frames' forwards as their
    // last update left them, wheel +352 of those with a record, made a unit - with BurnBoost x the mass)
    let launch_dir = if vs.by.exact && vs.by.launch.is_some() {
        let found = vs.by.launch_layout;
        let mut sum = Vec3::ZERO;
        for i in 0..4 {
            let driven = match found { 1 | 4 => i < 2, 2 | 3 => true, _ => i >= 2 };
            if driven && vs.wheels[i].record {
                sum = sum - vs.wheels[i].back_then;
            }
        }
        let l2 = super::exact::dot3(sum, sum);
        let mut e = 1.0 / l2.sqrt();
        if e == f32::INFINITY {
            e = 0.0;
        }
        Some(sum * ((-(l2 * (e * e) - 1.0)) * (0.5 * e) + e))
    } else {
        None
    };
    let engine_roll = std::mem::take(&mut vs.engine_roll);
    let mut force = if exact_sums { Vec3::ZERO } else { Vec3::new(0.0, -g * mass, 0.0) };
    let mut torque = Vec3::ZERO;
    let speed = st.vel.length();

    // static load of each wheel from its distance to the centre of mass
    let zf = (p.wheel_pos[0].z - com_local.z).abs().max(0.2);
    let zr = (p.wheel_pos[2].z - com_local.z).abs().max(0.2);
    let wheel_share = if vt.bike { 1.0 } else { 0.5 };
    let w_front = wheel_share * mass * g * zr / (zf + zr);
    let w_rear = wheel_share * mass * g * zf / (zf + zr);
    // (0x825673A0 -> 0x825646E0, operation for operation: each wheel's share of the weight by
    // its own place along the car - the weight over twice its distance from the model's
    // origin, times its distance from the centre of mass on the other side - and, where it
    // stands wider than its tyre, halved the same way across)
    let mut w_burn_brake = (w_front, w_rear);
    let (w_front, w_rear) = if bike_old(vt) || dbg_flag("MCLA_DBG_OLDSUSP") {
        (w_front, w_rear)
    } else {
        let weight = (if t.s.bound_gravity > 0.1 { t.s.bound_gravity } else { 1.0 } * mass) * 9.8f32;
        let share_at = |i: usize, width: f32, z: f32| {
            let at = p.wheel_pos[i];
            let w = weight / (at.z.abs() * 2.0) * (z + at.z).abs();
            if at.x.abs() > width { (com_local.x + at.x).abs() / (at.x.abs() * 2.0) * w } else { w }
        };
        // (wheel +428 as 0x8232F87C finds it: weighed before the burnout moved the centre of mass this tick)
        if let Some(z) = cg_before_burn {
            w_burn_brake = (share_at(0, t.wheel_front.width, z), share_at(2, t.wheel_back.width, z));
        }
        let z = if wiped { weigh_z } else { com_local.z };
        (share_at(0, t.wheel_front.width, z), share_at(2, t.wheel_back.width, z))
    };
    // (on one wheel - a wheelie or a stoppie - a motorcycle's wheels are each set up for the
    // whole weight: recorded, both wheels' static load is 3,636 N = 265 kg x 13.72 for as long
    // as the stunt lasts, springs and dampers twice as stiff with it)
    // (Z3, 0x8232F370: ((gravity factor x mass) x 9.8) x (2 / wheels), set while the gyro's state is
    // 1 or -1 and left standing while it is 99 / -99 - the weights are only made afresh at state 0)
    let (w_front, w_rear) = if vt.bike && vs.wheelie != 0 {
        if bike_old(vt) { (mass * g, mass * g) } else {
            let whole = (((if t.s.bound_gravity > 0.1 { t.s.bound_gravity } else { 1.0 }) * mass) * 9.8f32) * (2.0 / 2.0);
            (whole, whole)
        }
    } else { (w_front, w_rear) };

    // which wheels the engine turns
    // (the original's table of wheels per shaft: 0 rear, 1 and 4 front, 2 and 3 all four; for a
    // burnout an all-wheel-drive car is switched to one axle: type 2 to the rear, type 3 to the front)
    let burnout_now = if vs.by.exact { vs.by.brake_layout.is_some() } else { vs.burn_state == 1 || vs.burn_state == 2 };
    let layout = vs.drive as u32;
    let powered = |i: usize| match layout {
        1 | 4 => i < 2,
        2 | 3 => true,
        _ => i >= 2,
    };
    let train_of = |i: usize| if powered(i) { 0 } else { 1 };

    // Original dispatchers 0x82569AC8 / 0x82335AA8 run shafts before tyres.
    // Keep the preceding tyre torque/velocity/slip available while contacts are prepared.
    let exact_sums_early = !bike_old(vt) && !dbg_flag("MCLA_DBG_OLDSUMS");
    let previous_wheels = vs.wheels;
    let mut prepared = [super::drivetrain::PreparedWheel::default(); 4];
    let mut contacts: [Option<TyreFrame>; 4] = [None; 4];
    let active = |i: usize| !vt.bike || i % 2 == 0;
    let wheel_count = if vt.bike { 2.0 } else { 4.0 };
    // (chassis +572 as the gyro finds it: the drive shaft's speed of the tick before at its
    // first wheel's radius)
    let shaft_speed_then = if exact_sums_early { vs.shaft_speed_in } else { vs.omega[0] * if vs.drive == 0 { t.wheel_back.radius } else { t.wheel_front.radius } };
    let mut grounded_count = 0;
    let handbrake = if input.handbrake { 1.0 } else { 0.0 };
    // (rolling slowly off the throttle the original brakes by itself, see `auto_brake`)
    let hold = vs.auto_brake;
    let reversing = vs.gear < 0;
    // (in reverse the pedals swap: the brake pedal drives, the throttle brakes)
    let (throttle_in, brake_in) = if reversing { (input.brake, input.throttle) } else { (input.throttle, input.brake) };
    let brake = brake_in.clamp(0.0, 1.0);

    let vs_burn_state = vs.burn_state;
    let vs_melt = vs.melt;
    // ---- the road's roughness (vehChassis 0x8255AE68): the car runs along bumps as long as the
    // shortest `width` of the surfaces under its wheels, at a random 0.618..1.618 of its speed
    // (above 1 m/s the speed to the power 0.35); each new bump draws its own height and length ----
    {
        let mut speed = (st.vel.x * st.vel.x + st.vel.z * st.vel.z).sqrt();
        if speed > 1.0 {
            speed = speed.powf(0.35);
        }
        let rnd = vs.random_bits() * (1.0 / 8_388_608.0) + 0.618;
        let mut phase = rnd * dt * speed + vs.bump_phase;
        let (mut shortest, mut vary) = (f32::MAX, 0.0f32);
        for w in &vs.wheels {
            if let Some(s) = w.ground {
                let (width, _, depth) = super::collision::surface_bump(super::collision::Surface::from_u8(s));
                shortest = shortest.min(width);
                vary = vary.max(depth);
            }
        }
        let (mut height, mut width) = (0.0, 0.0);
        if phase > shortest {
            phase -= vs.random_bits() * vary * (1.0 / 8_388_608.0) + shortest;
            height = vs.random_normal(0.0, 0.5);
            width = vs.random_normal(0.0, 0.5);
        }
        vs.bump_phase = phase;
        for w in &mut vs.wheels {
            w.bump_phase = phase.max(0.0);
            if height != 0.0 { w.bump_height = height; }
            if width != 0.0 { w.bump_width = width; }
        }
    }
    // suspension damping the body's integration solves implicitly: (arm, normal, strength)
    let mut implicit: Vec<(Vec3, Vec3, f32)> = Vec::new();
    // A car fitted with hydraulics rides on their suspension (shorter, softer damped, a low
    // limit on its push); switched on, every wheel carries the same tuned load.
    let hyd_fitted = input.hyd & super::car::HYD_FITTED != 0 && !vt.bike;
    let hyd_wheels = hyd_fitted.then(|| {
        let fit = |w: &mcla_formats::rage::simtune::WheelTune| {
            let mut w = w.clone();
            w.travel_extent = t.hydraulics.suspension_extent;
            w.suspension_extent = t.hydraulics.suspension_extent;
            w.suspension_damp_coef = t.hydraulics.suspension_damp;
            w.suspension_clamp = 1.5;
            w
        };
        (fit(&t.wheel_front), fit(&t.wheel_back))
    });
    let hyd = vs.hyd;
    // (a live axle's bone pitches with the mean of how far its two wheels are pressed in)
    let rear_mean = (2..4).map(|i| {
        let wt = hyd_wheels.as_ref().map_or(&t.wheel_back, |w| &w.1);
        vs.wheels[i].comp.clamp(-wt.travel_extent, wt.travel_limit) - if hyd_fitted { hyd.height[i] } else { 0.0 } - wt.suspension_offset
    }).sum::<f32>() * 0.5;
    // (the body's matrix as the collision has it - rows and place)
    let seen_exact = vs.seen_exact.take();
    let seen_rows = if vs.body_exact.is_some() { seen_exact } else { kept_body.map(|b| b.seen) };
    let body_rows = vs.body_exact.take().or(kept_body.map(|b| [b.rows[0], b.rows[1], b.rows[2], b.origin])).unwrap_or_else(|| {
        let m = Mat3::from_quat(st.rot);
        [m.x_axis, m.y_axis, m.z_axis, st.pos]
    });
    let mut damping = super::exact::Damping::default();
    // (chassis +352: the chassis's own copy of the body's place - the bound's offset carried
    // through the model's matrix. Not always the body's place to the last bit: in one tick of
    // a hundred or so the two differ by a unit, and what the wheels hand the body is placed
    // by this one, measured from that one)
    let chassis_at = super::exact::point_through(pivot_local, &body_rows);
    // (the car's up of this update, to the bit, for the next tick's contacts)
    if !bike_old(vt) && !dbg_flag("MCLA_DBG_OLDSTEP") {
        vs.prev_up = Some(body_rows[1]);
    }
    if dbg_flag("MCLA_DBG_CALLS") && kept_body.is_none() {
        eprintln!("BODY made afresh from the car's pose");
    }
    let mut rebuilt: [Option<[Vec3; 4]>; 4] = [None; 4];
    // (each wheel's shape for the tick's collision: front, radius, width, SuspensionLimit, BottomoutElasticity)
    let mut wheel_geo: [Option<(bool, f32, f32, f32, f32)>; 4] = [None; 4];
    let mut wheel_reach: [Option<(f32, f32)>; 4] = [None; 4];
    let mut wheel_rest: [Option<Vec3>; 4] = [None; 4];
    let mut wheel_steered = [false; 4];
    // (wheel byte +530 as the last collision left it: how many wheels stand on the solver)
    let bottomed_in = (0..4).filter(|i| active(*i) && vs.wheels[*i].bottomed).count();
    let wheels_in = (0..4).filter(|i| active(*i)).count();
    let mut offsets = [0.0f32; 4];
    let (mut rest_zs, mut angles) = ([0.0f32; 4], [0.0f32; 4]);
    for i in 0..4 {
        if !active(i) { continue; }
        let front = i < 2;
        let wt = match hyd_wheels.as_ref() {
            Some((f, b)) => if front { f } else { b },
            None => if front { &t.wheel_front } else { &t.wheel_back },
        };
        let w_static = if hyd_fitted && hyd.on {
            (t.hydraulics.suspension_stiffness - 1.0).clamp(0.0, 10.0) * mass
        } else if front {
            w_front
        } else {
            w_rear
        };
        // (the ram's length moves the wheel's place of rest)
        // (and so does the tuning's SuspensionOffset, wheel +432: a lowered car's wheels sit
        // that much higher in its body at the same load - 0x8255C640, 0x82564E08)
        let ram = if hyd_fitted { hyd.height[i] } else { 0.0 } + wt.suspension_offset;
        let radius = wt.radius.max(0.1);
        // (what the ground is measured with: the wheel's collision shape)
        let tyre_width = vt.tyre_width[if front { 0 } else { 1 }];
        let round = if tyre_width > 0.0 { crown(radius, tyre_width, 0.0) } else { radius };
        if tyre_width > 0.0 && !bike_old(vt) && !dbg_flag("MCLA_DBG_OLDWHEEL") {
            wheel_geo[i] = Some((front, radius, tyre_width, wt.suspension_limit, wt.bottomout_elasticity));
            wheel_reach[i] = Some((wt.suspension_extent, wt.suspension_limit));
        }
        let local = if vt.bike { (p.wheel_pos[i] + p.wheel_pos[i + 1]) * 0.5 } else { p.wheel_pos[i] };
        let attach = st.pos + rm * local;
        // steering: the inner wheel turns a little more (SteeringOffset)
        let steer = if wt.steering_limit > 0.0 {
            let a = vs.steer * wt.steering_limit;
            a * (1.0 + local.x.signum() * wt.steering_offset * a)
        } else {
            0.0
        };
        // (the wheel as this update rebuilds it, 0x825652B8 and 0x82567508, in the original's
        // own operations: its angle, and its rows and place of rest carried through the
        // chassis's matrix. MCLA_DBG_OLDROWS: the port's older construction.)
        let exact_rows = !bike_old(vt) && !dbg_flag("MCLA_DBG_OLDROWS");
        let (rest_at, pivot_exact, offset_exact) = match vs.rest_exact {
            Some(r) => (Vec3::new(r[i][0], r[i][1], r[i][2]), r[i][3], r[i][4]),
            None => (Vec3::new(local.x, local.y - p.susp_rest, local.z), super::exact::pivot_arm(local.x, wt.pivot_offset, wt.width), ram),
        };
        let angle = super::exact::steer_angle(if front { vs.steer } else { -vs.steer }, rest_at.x, wt.steering_limit, wt.steering_offset);
        let steer = if exact_rows { -angle } else { steer };
        if exact_rows {
            rebuilt[i] = Some(super::exact::wheel_rebuilt_about(&body_rows, rest_at, pivot_exact, offset_exact, wt.steering_limit != 0.0, angle, t.steer_axis.filter(|_| front).map_or(Vec3::Y, Vec3::from)));
            wheel_rest[i] = Some(rest_at);
            wheel_steered[i] = wt.steering_limit != 0.0;
            offsets[i] = offset_exact;
            rest_zs[i] = rest_at.z;
            angles[i] = angle;
        }
        let ws = &mut vs.wheels[i];
        let w = &mut st.wheels[i];
        // (what the wheel's shape stood at when the collision ran: the wheel as the last
        // update left it - its steering then; and a steered wheel turns about a point
        // PivotOffset of its place across the car inboard of it, 0x825673A0 / 0x82567508:
        // at full lock its centre is 27 mm from where it stands straight)
        let steer_then = w.steer;
        let pivot_arm = if wt.steering_limit <= 0.0 || vt.bike { 0.0 } else if wt.pivot_offset > 0.0 { local.x * wt.pivot_offset } else if local.x.abs() > wt.width { local.x.signum() * wt.width * 0.25 } else { 0.0 };
        w.steer = steer;
        let extent = wt.travel_extent.clamp(0.02, 0.5);
        let limit = wt.suspension_limit.clamp(0.02, 0.5);
        // the wheel's centre at rest is `susp_rest` below the attachment point
        let top = limit + 0.05;
        let ray_len = top + p.susp_rest + ram.max(0.0) + extent + round;
        static DBG_TICK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if i == 0 { DBG_TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
        let hit = world.raycast(attach + up * top, -up, ray_len);
        // ---- the wheel's own shape. In the original every wheel is a part of the car's bound -
        // a rounded disc as wide as the tyre - and the suspension is pressed in by how deep
        // that shape stands in the ground (vehWheel 0x82564C48, chassis +544 == 2). On level
        // ground that is the distance under the axle; at a step it is the tyre's round that
        // meets the edge first, ahead of the axle, and the wheel climbs it over several ticks
        // instead of being knocked up in one; leaving a ledge it rests on the lip a while
        // longer. Here: where the tyre's round first touches the ground's triangles ----
        // (where the contact puts the suspension, by the original's own operations)
        let mut exact_x: Option<f32> = None;
        let hit = if dbg_flag("MCLA_DBG_RAYWHEEL") {
            hit
        } else {
            let axle = Quat::from_axis_angle(up, -steer_then) * right;
            let attach = if dbg_flag("MCLA_DBG_NOPIVOT") { attach } else { attach + (axle - right) * pivot_arm };
            let along = axle.cross(up).normalize_or(fwd);
            // (the shape leans as the wheel does - about the car's way forward, its top inwards)
            // (vehAxle::Update, 0x825681E0: about the axle's own third row - the car's, which
            // leans the top inwards; or, on a live rear axle, that of its bone, which is
            // authored turned right round and pitches with the mean of its two wheels, so
            // those wheels lean outwards. Recorded on an S281: rear 0.3667 rad a metre outwards,
            // front inwards.)
            let about = match t.axle_bone_z.filter(|_| !front) {
                Some(z) => Vec3::new(0.0, rear_mean / (z - local.z), -1.0).normalize(),
                None => Vec3::Z,
            };
            let lean = Quat::from_axis_angle(rm * about, ws.camber * local.x.signum());
            let (axle_w, up_w, along_w) = if bike_old(vt) || dbg_flag("MCLA_DBG_NOCAMBER") { (axle, up, along) } else { (lean * axle, lean * up, lean * along) };
            // (the shape's matrix as the original's collision forms it: the wheel's matrix
            // among the car's parts as the last update left it, times the body's matrix now -
            // 0x8255CEA0, 0x82575450. MCLA_DBG_OLDWHEELPOSE: built from the suspension's state
            // as before.)
            let posed = ws.part.filter(|_| !bike_old(vt) && !dbg_flag("MCLA_DBG_OLDWHEELPOSE")).map(|part| super::gjk::world_matrix(&part, seen_rows.as_ref().unwrap_or(&body_rows)));
            let (centre_w, axes_w) = match posed {
                Some(m) => (m[3], [m[0], m[1], m[2]]),
                None => (attach - up * (p.susp_rest + ram - ws.comp), [axle_w, up_w, along_w]),
            };
            // (a motorcycle's tyre is round across too: it has no shoulders to stand on when it leans)
            let shoulder = if vt.bike { 0.0 } else { (wt.width * 0.5).clamp(0.0, 0.25) };
            // (only what a wheel can roll up: an edge well below the axle. Anything higher is
            // a wall to it, and the body's business)
            let axle_down = top + p.susp_rest + ram - ws.comp;
            // (the round in the tyre's middle and at its two shoulders - a motorcycle's tyre is
            // round across too and has only the one)
            let mut found: [Option<super::collision::Hit>; 2] = [None, None];
            // (which triangles are tried at all: those that touch the wheel shape's box)
            let reach_box = (tyre_width > 0.0 && !dbg_flag("MCLA_DBG_NOBOX")).then(|| {
                let centre = centre_w;
                // the part's "last" place: the car a tick ago, the wheel TravelLimit + 0.1
                // above its place of rest, unsteered
                let turn = st.ang_vel * dt;
                let back = if turn.length_squared() > 1e-12 { Quat::from_axis_angle(turn.normalize(), -turn.length()) } else { Quat::IDENTITY };
                let (pos0, rot0) = (st.pos - st.vel * dt, (back * st.rot).normalize());
                let m0 = Mat3::from_quat(rot0);
                // (the body's last matrix as the tick's collision had it - which a tick with a deep push
                // does not renew, 0x8259E920: the box then reaches back to where the car stood before)
                let (pos0, m0) = match vs.last_seen.filter(|_| !dbg_flag("MCLA_DBG_Y5")) {
                    Some(l) => (l[3], Mat3::from_cols(l[0], l[1], l[2])),
                    None => (pos0, m0),
                };
                let last = pos0 + m0 * (local + Vec3::Y * (-p.susp_rest - ram + wt.travel_limit + 0.1));
                super::hull::WheelBox::new((centre, axes_w), (crown_half_width(radius, tyre_width), round), (last, [m0.x_axis, m0.y_axis, -m0.z_axis]))
            });
            // ---- one contact a wheel: of every triangle that touches the shape's box, what
            // the original's narrow phase makes of it; the deepest is the wheel's (0x8255D028).
            // Nothing is kept from tick to tick ----
            let touched = (tyre_width > 0.0 && !bike_old(vt) && !dbg_flag("MCLA_DBG_OLDWHEEL")).then(|| {
                let core = super::gjk::WheelCore::of(&super::gjk::wheel_shape(front, radius, tyre_width), centre_w, axes_w, radius, tyre_width);
                // (R4: set down afresh in the step before, the car has no pair with the city yet)
                let touch = if vs.set_down { None } else { super::hull::wheel_touch(world, &core, up_then, reach_box.as_ref(), vs.cell_ball) };
                if dbg_flag("MCLA_DBG_HANG") {
                    eprintln!("CON {} {i} comp {:.4} touch {:?}", DBG_TICK.load(std::sync::atomic::Ordering::Relaxed), ws.comp, touch.map(|t| (t.depth, t.surface, t.normal.dot(up), t.deep, rm.transpose() * (t.point - st.pos))));
                }
                // (0x82564C48: the depth goes into the suspension over how squarely the wheel
                // stands on the contact - the lesser of its axle's share of the contact's
                // sideways and its way's share of the contact's forwards, never under 0.75: a
                // wall's contact presses the wheel in by a third more than its depth. Recorded
                // on a Range Rover turned into a wall: 0.0301 from a face with a normal all but
                // level, the suspension from -0.007 to 0.026, 23 kN along that normal for one
                // tick, nothing the next, the ground again after.)
                let tilt = |n: Vec3| {
                    let f = n.cross(axle).normalize_or_zero();
                    let lat = f.cross(n);
                    axle.dot(lat).abs().min(along.dot(f).abs()).clamp(0.75, 1.0)
                };
                // (as a ray under the axle would have measured it)
                if let (Some(t), Some(_), false) = (touch.as_ref(), posed.as_ref(), dbg_flag("MCLA_DBG_OLDSUSP")) {
                    // (the wheel's rows as this update's rebuild has them, 0x82567508: the
                    // chassis's own for a wheel that is not turned)
                    let rows = match rebuilt[i] {
                        Some(m) => [m[0], m[1], m[2]],
                        None => if steer == 0.0 { [body_rows[0], body_rows[1], body_rows[2]] } else { let a = Quat::from_axis_angle(up, -steer) * right; [a, up, a.cross(up).normalize_or(fwd)] },
                    };
                    exact_x = suspension_target(rows, t.normal, t.depth, ws.comp);
                }
                touch.filter(|t| t.depth > -0.02).map(|t| super::collision::Hit { t: top + round + p.susp_rest + ram - ws.comp - t.depth / tilt(t.normal), point: t.point, normal: t.normal, surface: t.surface })
            });
            let mut planes = [(0.0f32, round); 5];
            let count = if vt.bike {
                1
            } else if tyre_width > 0.0 {
                for (plane, k) in planes.iter_mut().zip([0.0f32, -0.25, 0.25, -0.5, 0.5]) {
                    *plane = (k * tyre_width, crown(radius, tyre_width, k * tyre_width));
                }
                5
            } else {
                planes[1].0 = -shoulder;
                planes[2].0 = shoulder;
                3
            };
            // (the older reading of the ground - planes across the tread - only where the
            // original's own routine is switched off)
            let count = if touched.is_some() { 0 } else { count };
            // (and where a tyre that leans against its ground touches it: its crown is an arc
            // across the tread, and the point of it that faces the ground lies that arc's
            // radius times the lean off the middle)
            let mut probe = |found: &mut [Option<super::collision::Hit>; 2], aside: f32, rad: f32| {
                // (the city's collision stands 5 mm proud of its triangles - the mesh bound's
                // margin: every recorded contact point is exactly that far above them - which
                // is the same as a wheel that much larger)
                let rad = rad + super::collision::WORLD_MARGIN;
                let rest = super::hull::wheel_rest(world, attach - up * (p.susp_rest + ram - ws.comp) + up_w * axle_down + axle_w * aside, up_w, along_w, axle_w, rad, ray_len, axle_down + round * 0.25, axle_down, ws.kept.map(|k| (k.0, k.1)), reach_box.as_ref());
                for (best, h) in found.iter_mut().zip(rest) {
                    if let Some(mut h) = h {
                        // (as the middle of the tyre would measure it)
                        h.t += round - rad;
                        if best.map_or(true, |b| h.t < b.t - 1e-4) {
                            *best = Some(h);
                        }
                    }
                }
            };
            for (aside, rad) in planes.into_iter().take(count) {
                probe(&mut found, aside, rad);
            }
            if count == 5 && !dbg_flag("MCLA_DBG_NOLEANPROBE") {
                let arc = (tyre_width * tyre_width * 0.25 + 6.0 * (radius - 0.04) * (radius - 0.04)).sqrt();
                for normal in found.map(|h| h.map(|h| h.normal)).into_iter().flatten() {
                    let aside = (-arc * normal.dot(axle_w) / normal.dot(up_w).max(0.2)).clamp(-0.5 * tyre_width, 0.5 * tyre_width);
                    probe(&mut found, aside, crown(radius, tyre_width, aside));
                }
            }
            // ---- one contact a wheel: the deepest of those its shape is given (0x8255D028),
            // and it is given one for every triangle that touches the shape's box (above) and
            // is no farther than 2 cm from the tyre (0x825C65E8; recorded depths end at
            // -0.0199 in 23,889 samples). Nothing is kept from tick to tick: a wheel past the
            // edge of the polygon it stood on is found against that polygon again, at its edge.
            // The depth: how far the suspension would be pressed in from where it is, and the
            // 7 mm a wheel stands in the ground ----
            let depth = |h: &super::collision::Hit| (p.susp_rest + ram - (h.t - top - round)) - ws.comp;
            let best = match (found[0].filter(|h| depth(h) > -0.02), found[1].filter(|h| depth(h) > -0.02)) {
                (Some(a), Some(b)) => Some(if a.t <= b.t { a } else { b }),
                (a, b) => a.or(b),
            };
            let _ = hit;
            if let Some(touched) = touched {
                touched
            } else {
            if dbg_flag("MCLA_DBG_HANG") {
                eprintln!("CON {} {i} comp {:.4} kept {:?} same {:?} other {:?} -> {}", DBG_TICK.load(std::sync::atomic::Ordering::Relaxed), ws.comp, ws.kept.map(|k| k.2), found[0].map(|h| (depth(&h), h.surface)), found[1].map(|h| (depth(&h), h.surface)), best.map_or("none".to_string(), |h| format!("{:.4}", depth(&h))));
            }
            best
            }
        };
        // ---- and walls. The wheels are the widest thing about a car down low, and it is they
        // that meet a wall there: the shape's contact presses the suspension in like any other -
        // by its depth, over at least 0.75 for the tilt of the face against the wheel, less the
        // 7 mm a wheel stands in what it touches (0x82564C48) - and the spring then pushes
        // along the wall's normal. (Recorded: a front wheel on a wall polygon, normal level,
        // 23 kN at the suspension's clamp, the car turned aside by it; tyre grip 0.01.) ----
        let mut steep = false;
        let hit = if dbg_flag("MCLA_DBG_RAYWHEEL") || dbg_flag("MCLA_DBG_NOSTEEP") || (vt.tyre_width[if front { 0 } else { 1 }] > 0.0 && !bike_old(vt) && !dbg_flag("MCLA_DBG_OLDWHEEL") && !dbg_flag("MCLA_DBG_STEEPWHEEL")) {
            hit
        } else {
            let axle = Quat::from_axis_angle(up, -steer) * right;
            let centre = attach - up * (p.susp_rest + ram - ws.comp);
            let frame = Mat3::from_cols(axle, up, axle.cross(up).normalize_or(fwd));
            let ground_x = hit.map(|h| p.susp_rest + ram - (h.t - top - round));
            match super::hull::deepest_steep(&vt.wheel_shape[if front { 0 } else { 1 }], centre, frame, world, up, 0.7) {
                Some(wall) => {
                    // (what the wheel is told of a contact is its depth's share along the
                    // car's up, 0x8255D028 - next to nothing on an upright wall unless the wheel
                    // is far into it - and only of faces that do not look down; then the
                    // deepest such share is the wheel's contact. Recorded on a Range Rover
                    // turned into a wall: 0.0301 from a face with normal (0.52, 0.04, 0.86),
                    // one tick of 23 kN, the ground again the next)
                    let share = if wall.normal.y >= 0.0 { (wall.depth + super::collision::WORLD_MARGIN) * wall.normal.dot(up).max(0.0) } else { 0.0 };
                    let x = ws.comp + share / 0.75 - 0.007;
                    if share > 0.0 && x > -extent && ground_x.map_or(true, |g| x > g) {
                        steep = true;
                        Some(super::collision::Hit { t: top + round + p.susp_rest + ram - x, point: wall.point, normal: wall.normal, surface: wall.surface })
                    } else {
                        hit
                    }
                }
                None => hit,
            }
        };
        // (left wheels are the even ones)
        prepared[i].torque = previous_wheels[i].tyre_torque;
        prepared[i].brake = if let (true, Some(found)) = (vs.by.exact, vs.by.brake_layout) {
            // (0x8232F87C / 0x82565320: the wheels the engine does not turn - by the layout as the
            // rules found it, before they switched it - are braked with twice their grip, the
            // others not at all whatever the pedals say)
            let driven = match found { 1 | 4 => i < 2, 2 | 3 => true, _ => i >= 2 };
            let w_then = if front { w_burn_brake.0 } else { w_burn_brake.1 };
            if driven { 0.0 } else { ((wt.static_fric * w_then) * wt.radius) * 2.0 }
        } else if burnout_now {
            // (0x8232F87C: the wheels the engine does not turn are braked with twice their grip)
            if powered(i) { 0.0 } else { wt.static_fric * w_static * radius * 2.0 }
        } else if exact_rows && !dbg_flag("MCLA_DBG_OLDTYREIN") {
            // (0x82564758 and 0x825652B8, operation for operation: the two torques at full
            // pedal, wheel +484 / +488, and their sum by the pedals; the front wheels are
            // handed no handbrake at all, 0x8255C850)
            // (Z3: the pedals' torque, wheel +480, is made at the head of mcCarSim::Update (0x8232EFC0 ->
            // 0x825652B8) from +484 / +488 as they stand - which 0x82564758 only makes afresh further
            // down that update: the static weight they hold is the one of the tick before. Seen where
            // the weight jumps: a motorcycle's first tick of a stoppie.)
            let w_brake = if vt.bike && vs.w_then[i] > 0.0 { vs.w_then[i] } else { w_static };
            let full = ((wt.brake_coef * wt.static_fric) * wt.radius) * w_brake;
            let hand = ((wt.handbrake_coef * wt.static_fric) * wt.radius) * w_brake;
            fma32(full, brake_in, hand * if front { 0.0 } else { handbrake })
        } else {
            (wt.brake_coef * brake + wt.handbrake_coef * handbrake) * wt.static_fric * radius * w_static
        };
        let w_before = vs.w_then[i];
        vs.w_then[i] = w_static;
        // (the automatic brake replaces the pedals' torque: vehWheel 0x82565320, no BrakeCoef)
        if hold > 0.0 && !burnout_now && (input.hold || !(input.ai || REPLAY_AI.load(std::sync::atomic::Ordering::Relaxed))) {
            // (RB2: 0x8232F2D8 stands in mcCarSim::Update AHEAD of the centre of mass's way back
            // (0x8232F64C -> 0x82565758 weighs the wheels afresh): the weight in wheel +428 is still the
            // one of the tick before. Seen where a burnout is let go at a standstill - the centre of mass
            // comes back at 1.5 m/s while the automatic brake holds. MCLA_DBG_RB2: this tick's weight.)
            let w_hold = if w_before > 0.0 && vs.by.exact && !dbg_flag("MCLA_DBG_RB2") { w_before } else { w_static };
            prepared[i].brake = wt.static_fric * w_hold * radius * hold;
        }
        // (the grid hold, the player's car only: both front wheels' pedal torque is ten times the full
        // pedal's - +480 = 10 x +484 on every tick of the pre-race, a.0 k1..654 and a.2 with the throttle
        // down - and the normal one from the GO tick on; a burnout's own braking of the held axle replaces
        // it, a.3. The rear wheels keep the pedal's / the automatic brake's torque.)
        // (0x8232F304: 0x82565320 with 10.0, 0x82003A8C - grip x weight (+428, the tick before's, as the
        // automatic brake reads it) x radius x 10, no BrakeCoef; for the wheels of chassis +404's list: the
        // wheels the drive layout as it stands does not turn - the Saleen's fronts (a.0, a.2), the R8's fronts
        // while a burnout has put it on its rear axle (a.1 k608..616: 87 251 = 10 x 8 725) and none while it
        // drives all four (k150). A burnout's own braking replaces it, a.3.)
        if input.hold && !powered(i) && !burnout_now {
            let w_lock = if w_before > 0.0 && vs.by.exact { w_before } else { w_static };
            prepared[i].brake = ((wt.static_fric * w_lock) * radius) * 10.0;
        }
        // ---- suspension (vehWheel 0x82565858, constants from 0x82564758):
        // N = W + k (x + bump) (1 + k2 (x + bump)) + c v, v the compression's change over the tick ----
        let span = (wt.suspension_limit + wt.suspension_extent) * wt.suspension_extent;
        // (0x82564758, operation for operation: one division, by the span, and the products after it)
        let over_span = 1.0 / span;
        let k = fma32(wt.suspension_factor, wt.suspension_extent, wt.suspension_limit) * over_span * w_static;
        let (k2, c) = if k.abs() > 1e-6 {
            ((wt.suspension_factor - 1.0) * over_span * w_static / k, (w_static * k).sqrt() * wt.suspension_damp_coef * 2.0)
        } else {
            ((wt.suspension_factor - 1.0) / fma32(wt.suspension_factor, wt.suspension_extent, wt.suspension_limit), 0.0)
        };
        // (nothing under the wheel, 0x82566600: the same routine is asked for the full extension -
        // which the damper will not give at once: the wheel lets out against it, tick by tick)
        let hang_load = std::cell::Cell::new(0.0f32);
        let hang = |before: f32, bump: f32| -> (f32, f32) {
            if dbg_flag("MCLA_DBG_OLDSUSP") {
                let x = -extent;
                let xb = x + bump;
                let load = (k2 * xb + 1.0) * (k * xb) + c * ((x - before) / dt) + w_static;
                let comp = if load < 0.0 && c + k * dt > 0.0 { (c * before - k * dt * wt.travel_extent) / (c + k * dt) } else { x };
                return (comp, (comp - before) / dt);
            }
            // (0x82565858 with no contact, operation for operation)
            let per = 1.0 / dt;
            let x = -wt.travel_extent;
            let xb = bump + x;
            let load = fma32(fma32(k2, xb, 1.0), k * xb, c * ((x - before) * per)) + w_static;
            let comp = if load < 0.0 {
                let kd = k * dt;
                fma32(c, before, -(kd * wt.travel_extent)) / (c + kd)
            } else {
                x
            };
            // (what the routine leaves in wheel +500: the load, nothing if it came out negative)
            hang_load.set(if load < 0.0 { 0.0 } else { load });
            (comp, (comp - before) * per)
        };
        let Some(hit) = hit else {
            if dbg_flag("MCLA_DBG_HANG") { eprintln!("WHL {} {i} {:.5} none", DBG_TICK.load(std::sync::atomic::Ordering::Relaxed), ws.comp); }
            let (comp, rate) = hang(ws.comp, ws.bump);
            w.grounded = false;
            w.compression = ((comp + extent) / (limit + extent)).clamp(0.0, 1.0);
            w.world_pos = attach - up * (p.susp_rest + ram - comp);
            w.slip = 0.0;
            w.load = 0.0;
            w.f_lon = 0.0;
            // (0x82566F9C: a wheel with no contact loses its slide, its tyre's displacements, forces
            // and torque - not its slips or its contact speeds, which the next contact starts from.
            // MCLA_DBG_OLDHANG: those too.)
            let keep = if dbg_flag("MCLA_DBG_OLDHANG") { (0.0, 0.0, 0.0, 0.0) } else { (ws.slip_lat, ws.slip_long, ws.velocity_long, ws.velocity_lat) };
            *ws = WheelSim { comp, comp_rate: rate, bump: ws.bump, bump_phase: ws.bump_phase, bump_height: ws.bump_height, bump_width: ws.bump_width, camber: ws.camber, slip_lat: keep.0, slip_long: keep.1, velocity_long: keep.2, velocity_lat: keep.3, load: if dbg_flag("MCLA_DBG_Y5") { 0.0 } else { hang_load.get() }, ..Default::default() };
            continue;
        };
        let centre_drop = hit.t - top - round;
        // (a wheel stands 7 mm in what it touches: 0x82564C48 takes that off the contact's depth)
        let x = exact_x.unwrap_or((p.susp_rest + ram - centre_drop - if steep { 0.0 } else { 0.007 }).min(limit + 0.05));
        if dbg_flag("MCLA_DBG_HANG") { eprintln!("WHL {} {i} {:.5} {x:.5} {:.3} {:.3} {:.3} {}", DBG_TICK.load(std::sync::atomic::Ordering::Relaxed), ws.comp, hit.normal.dot(up), st.vel.dot(up), (st.vel + st.ang_vel.cross(hit.point - pivot)).dot(hit.normal), steep); }
        // (0x82565858: a contact that would let the wheel out past TravelExtent is taken at
        // TravelExtent - and, carrying nothing, the wheel then goes through the routine once
        // more as one in the air, below. MCLA_DBG_OLDHANG: once, as a wheel with no contact.)
        let clamp_exact = exact_x.is_some() && !dbg_flag("MCLA_DBG_OLDHANG");
        // (0x82565B28: and a contact taken at TravelExtent hands the body no damping, wheel +508 = 0)
        let let_out = clamp_exact && x < -wt.travel_extent && !dbg_flag("MCLA_DBG_Y5");
        let x = if clamp_exact && x < -wt.travel_extent { -wt.travel_extent } else { x };
        if x < -extent && !clamp_exact {
            let (comp, rate) = hang(ws.comp, ws.bump);
            w.grounded = false;
            w.compression = ((comp + extent) / (limit + extent)).clamp(0.0, 1.0);
            w.world_pos = attach - up * (p.susp_rest + ram - comp);
            w.slip = 0.0;
            w.load = 0.0;
            w.f_lon = 0.0;
            // (0x82566F9C: a wheel with no contact loses its slide, its tyre's displacements, forces
            // and torque - not its slips or its contact speeds, which the next contact starts from.
            // MCLA_DBG_OLDHANG: those too.)
            let keep = if dbg_flag("MCLA_DBG_OLDHANG") { (0.0, 0.0, 0.0, 0.0) } else { (ws.slip_lat, ws.slip_long, ws.velocity_long, ws.velocity_lat) };
            *ws = WheelSim { comp, comp_rate: rate, bump: ws.bump, bump_phase: ws.bump_phase, bump_height: ws.bump_height, bump_width: ws.bump_width, camber: ws.camber, slip_lat: keep.0, slip_long: keep.1, velocity_long: keep.2, velocity_lat: keep.3, load: if dbg_flag("MCLA_DBG_Y5") { 0.0 } else { hang_load.get() }, ..Default::default() };
            continue;
        }
        grounded_count += 1;
        w.grounded = true;
        w.contact = hit.point;
        w.normal = hit.normal;
        w.world_pos = attach - up * (p.susp_rest + ram - x);
        w.compression = ((x + extent) / (limit + extent)).clamp(0.0, 1.0);
        w.surface = hit.surface as u8;
        let n = hit.normal;

        // ---- suspension: N = W + k x (1 + k2 x) + c v ----
        let arm = hit.point - com;
        // (0x82566738: the arm of everything the wheel applies - from the model's origin plus
        // the centre of mass carried through the body's rows, to the contact record's point)
        let arm = if exact_sums {
            let cols = [Vec3::new(body_rows[0].x, body_rows[1].x, body_rows[2].x), Vec3::new(body_rows[0].y, body_rows[1].y, body_rows[2].y), Vec3::new(body_rows[0].z, body_rows[1].z, body_rows[2].z)];
            use super::exact::dot3;
            (hit.point - body_rows[3]) - Vec3::new(dot3(com_local, cols[0]), dot3(com_local, cols[1]), dot3(com_local, cols[2]))
        } else {
            arm
        };
        let cvel = st.vel + st.ang_vel.cross(hit.point - pivot);
        let m_w = w_static / g;

        // ---- tyre frame ----
        let axle = Quat::from_axis_angle(up, -steer) * right;
        let wheel_fwd = n.cross(axle).normalize_or(fwd) * -1.0;
        let wheel_fwd = if wheel_fwd.dot(fwd) < 0.0 { -wheel_fwd } else { wheel_fwd };
        let wheel_lat = wheel_fwd.cross(n).normalize_or(right);
        let wheel_lat = if wheel_lat.dot(right) < 0.0 { -wheel_lat } else { wheel_lat };
        let v_lat = cvel.dot(wheel_lat);
        let v_fwd = cvel.dot(wheel_fwd);
        // (0x82565E00, operation for operation: the contact's own directions from the wheel's
        // axle as this update rebuilt it - forwards = unit(axle x n), which points to the car's
        // rear, sideways = n x forwards - and the body's velocity at the contact record's
        // point, turning about the body's own place. MCLA_DBG_OLDTYREIN: the port's vectors.)
        let (wheel_fwd, wheel_lat, v_lat, v_fwd) = match rebuilt[i].filter(|_| !dbg_flag("MCLA_DBG_OLDTYREIN")) {
            Some(m) => {
                use super::exact::{cross, fma, recip};
                let f = cross(m[0], n);
                let back = f * recip(fma(f.z, f.z, fma(f.x, f.x, f.y * f.y)).sqrt());
                let side = cross(n, back);
                let v = cross(st.ang_vel, hit.point - body_at) + st.vel;
                (-back, side, fma(v.x, side.x, fma(side.y, v.y, side.z * v.z)), -fma(back.x, v.x, fma(back.y, v.y, back.z * v.z)))
            }
            None => (wheel_fwd, wheel_lat, v_lat, v_fwd),
        };
        vs.wheels[i].contact_in = [hit.point, n];
        vs.wheels[i].velocity_lat = v_lat;
        let ws = &mut vs.wheels[i];
        let (mut surf_grip, surf_drag) = super::collision::surface(hit.surface);
        // (vehWheel 0x82565B90: a tyre without SurfaceDragCoefLat keeps its whole grip on a
        // surface that drags)
        if wt.surface_drag_lat == 0.0 && surf_drag > 0.0 {
            surf_grip = 1.0;
        }
        // (0x82565FC0: against a face too steep for the wheel the tyre has no hold - the
        // recorded grip there is 0.01)
        if steep {
            surf_grip = 0.01;
        }
        // (replays of a recording stand on the recording's surfaces)
        {
            let replay_grip = f32::from_bits(REPLAY_SURFACE[i.min(3)].load(std::sync::atomic::Ordering::Relaxed));
            if replay_grip > 0.0 {
                surf_grip = replay_grip;
            }
        }
        // (P3: wheel +524 = Wipeout + (Standard - Wipeout) x the air's damping factor as mcCarSim::Update set it,
        // AeroDampFactor; the grip is divided by it - or, under one, brought that share of the way to one)
        if wiped {
            let h = fma32(t.s.friction_handling_standard - t.s.friction_handling_wipeout, t.s.aero_damp_factor, t.s.friction_handling_wipeout);
            surf_grip = if h < 1.0 { fma32(1.0 - h, 1.0 - surf_grip, surf_grip) } else { surf_grip / h };
        }
        // (step 19e: the Ice power-up - vehicle +624 bit 0x80, sub_8232EA78 0x823300EC: every
        // wheel's +524 is 10 when the car is not wiped out, so the grip is divided by 10)
        if input.ice && !wiped {
            surf_grip /= 10.0;
        }
        // (wheel +444 is set for every wheel that has a contact, whether it carries or not)
        vs.wheel_grip[i.min(3)] = surf_grip;

        // ---- the surface's roughness under this wheel (vehWheel 0x82565B90): one bump is a
        // sine over an eased share of its length; the wheels of the axle behind (the front one
        // in reverse) are a quarter of a wave behind; sideways sliding and low speed flatten it ----
        {
            let (width, height, _) = super::collision::surface_bump(hit.surface);
            let speed = (v_fwd * v_fwd + v_lat * v_lat).sqrt();
            let straight = if speed > 1.0 {
                if v_lat.abs() < speed { 1.0 - v_lat.abs() / speed } else { 0.0 }
            } else {
                1.0
            };
            let speed = if speed > 1.0 { speed.powf(0.35) } else { speed };
            if height != 0.0 {
                if ws.bump_phase != 0.0 {
                    let length = (ws.bump_width + 1.0) * width;
                    ws.bump_phase %= length;
                    let share = ws.bump_phase / length;
                    let sign = |v: f32| -> f32 { if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 } };
                    let behind = (-sign(local.z - com_local.z) * sign(v_fwd) * 0.25).min(0.0);
                    let eased = if share > 0.0 && share < 1.0 { 1.0 - (share * std::f32::consts::FRAC_PI_2).cos() } else { 0.0 };
                    ws.bump = ((eased + behind) * std::f32::consts::TAU).sin() * (straight * straight);
                }
                ws.bump *= (ws.bump_height + 1.0) * height;
            } else {
                ws.bump_phase = 0.0;
                ws.bump = 0.0;
            }
            ws.bump *= up.dot(n).abs().clamp(0.25, 1.0);
            if speed < 5.0 {
                ws.bump *= speed * 0.2;
            }
            if REPLAY_BUMP_ON.load(std::sync::atomic::Ordering::Relaxed) {
                ws.bump = f32::from_bits(REPLAY_BUMP[i.min(3)].load(std::sync::atomic::Ordering::Relaxed));
            }
            ws.ground = Some(hit.surface as u8);
        }
        // (the contact this wheel has now, for the next tick's choice)
        ws.kept = if steep { None } else { Some((hit.normal, hit.normal.dot(hit.point), x + 0.007 - ws.comp)) };

        let before = ws.comp;
        let mut comp = x;
        let exact = !dbg_flag("MCLA_DBG_OLDSUSP");
        // (0x82565858, operation for operation: the rate is the change times 1 / dt, the
        // products are fused as the original's are)
        let per = 1.0 / dt;
        let mut comp_rate = if exact { (comp - before) * per } else { (comp - before) / dt };
        let xb = ws.bump + comp;
        let mut load = if exact { fma32(fma32(k2, xb, 1.0), k * xb, c * comp_rate) + w_static } else { (k2 * xb + 1.0) * (k * xb) + c * comp_rate + w_static };
        if dbg_flag("MCLA_DBG_HANG") {
            eprintln!("SUS {} {i} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}", DBG_TICK.load(std::sync::atomic::Ordering::Relaxed), k.to_bits(), c.to_bits(), k2.to_bits(), w_static.to_bits(), ws.bump.to_bits(), x.to_bits(), load.to_bits());
        }
        // what the body's integration is told of this spring (+508): its damping and the
        // stiffness it gains over a tick; none at the ends of the travel
        let mut impedance = 0.0;
        if load < 0.0 {
            // the wheel hangs: no load, the suspension lets out against its damper
            load = 0.0;
            if exact {
                let kd = k * dt;
                comp = fma32(c, before, -(kd * wt.travel_extent)) / (c + kd);
                comp_rate = (comp - before) * per;
            } else {
                comp = (c * before - k * dt * wt.travel_extent) / (c + k * dt);
                comp_rate = (comp - before) / dt;
            }
        } else {
            if comp > limit {
                // past the limit the spring goes on as stiff as it is there
                if exact {
                    comp_rate = (limit - before) * per;
                    load = fma32(fma32(k2, limit, 1.0), k * comp, c * comp_rate) + w_static;
                } else {
                    comp_rate = (limit - before) / dt;
                    load = (k2 * limit + 1.0) * (k * comp) + c * comp_rate + w_static;
                }
                comp = limit;
            } else {
                impedance = k * dt / up.dot(n).abs().clamp(0.25, 1.0) + c;
                // (0x825662CC and 0x82565B0C, operation for operation: the stiffness over how
                // squarely the contact faces the car's up - between a quarter and one - times
                // the time step, plus the damping, fused)
                if let (true, Some(_)) = (exact_sums, rebuilt[i]) {
                    // (0x82565E80: against the CHASSIS's up, chassis +256 - a car's wheel has the same
                    // second row; a motorcycle's front wheel, in its raked fork, has not)
                    let m = [body_rows[0], body_rows[1], body_rows[2]];
                    let facing = fma32(n.x, m[1].x, fma32(m[1].z, n.z, m[1].y * n.y)).abs();
                    let facing = if facing - 0.25 >= 0.0 { facing } else { 0.25 };
                    let facing = if facing - 1.0 >= 0.0 { 1.0 } else { facing };
                    impedance = fma32(k * (1.0 / facing), dt, c);
                }
                if let_out {
                    impedance = 0.0;
                }
            }
            // (SuspensionClamp times the wheel's share of the weight is all it pushes with)
            load = load.min(wt.suspension_clamp * w_static);
        }
        ws.comp = comp;
        // (the end of vehWheel::Update, 0x825666A0, and the axle after it, 0x825681E0: a wheel
        // pressed in past its SuspensionOffset leans its top in by that share of
        // SuspensionLimit times CamberLimit - and keeps the lean it last had while it hangs)
        if rebuilt[i].is_none() && wt.camber_limit > 0.0 && comp - ram > 0.0 && wt.suspension_limit > 0.0 {
            ws.camber = (comp - ram) / wt.suspension_limit * wt.camber_limit;
        }
        ws.comp_rate = comp_rate;
        ws.load = load;
        if load == 0.0 {
            // (0x825665D8: a wheel that carries nothing is off the ground for everything that
            // follows - no tyre, no grip, a slide of 0 that lets the gearbox shift - and the
            // suspension routine runs once more as for a wheel in the air)
            if dbg_flag("MCLA_DBG_HANG") { eprintln!("HANG t {} wheel {i} before {before:.4} ground {x:.4} relaxed {comp:.4}", DBG_TICK.load(std::sync::atomic::Ordering::Relaxed)); }
            let (comp, rate) = hang(comp, ws.bump);
            // (the second run's own answer stays in wheel +500: let out all the way against a damper that
            // now moves slower, the spring can come out carrying something after all - and the tyre, which
            // 0x825666A0 runs next, works with that load in the frame 0x8256663C has just left: the wheel's
            // own axle, up and way. Seen in rh 161: 154 N on a rear wheel at -TravelExtent.)
            let second = if dbg_flag("MCLA_DBG_Y5") { 0.0 } else { hang_load.get() };
            // (to everything that asks whether the wheel is on the ground - wheel +288, set
            // from the contact alone at the head of 0x825666A0 - it still is)
            if dbg_flag("MCLA_DBG_ZEROTYRE") {
                grounded_count -= 1;
                w.grounded = false;
            }
            w.compression = ((comp + extent) / (limit + extent)).clamp(0.0, 1.0);
            w.world_pos = attach - up * (p.susp_rest + ram - comp);
            w.slip = 0.0;
            w.load = 0.0;
            w.f_lon = 0.0;
            // (but vehWheel::Update, 0x825666A0, still runs its tyre for it - the wheel has a
            // contact, and a load of nothing: no force, and the tyre's displacements, which a
            // wheel in the air loses, go on by what the tread slips. Recorded on an R8's front
            // wheel over a lip: 0.01244 -> 0.00898 in the tick it carried nothing, and 2,900 N
            // of drag from that the tick it came down - 1,100 N if it starts from nothing.)
            let (disp_lat, disp_long) = if dbg_flag("MCLA_DBG_ZEROTYRE") { (0.0, 0.0) } else { (ws.disp_lat, ws.disp_long) };
            // (and its slide value stays what it was: only a wheel without any contact loses it)
            *ws = WheelSim { comp, comp_rate: rate, bump: ws.bump, bump_phase: ws.bump_phase, bump_height: ws.bump_height, bump_width: ws.bump_width, ground: ws.ground, kept: ws.kept, camber: ws.camber, disp_lat, disp_long, slide: if dbg_flag("MCLA_DBG_OLDTYREIN") { 0.0 } else { ws.slide }, velocity_lat: if dbg_flag("MCLA_DBG_Y5") { 0.0 } else { v_lat }, ..Default::default() };
            if !dbg_flag("MCLA_DBG_ZEROTYRE") {
                let k_lat = 2.0 * w_static / wt.disp_limit_lat.max(0.005);
                let k_long = 2.0 * w_static / wt.disp_limit_long.max(0.005);
                contacts[i] = Some(TyreFrame {
                    contact: super::tyre::Contact {
                        radius, spin: 0.0, velocity_lat: v_lat, velocity_long: v_fwd,
                        load: second, surface: surf_grip, spring_lat: k_lat, spring_long: k_long,
                        damper_lat: 2.0 * wt.damp_coef_lat * (k_lat * m_w).sqrt(), damper_long: 2.0 * wt.damp_coef_long * (k_long * m_w).sqrt(), dt,
                    }, arm, forward: wheel_fwd, lateral: wheel_lat, surf_drag: 0.0, surf_sink: 0.0, mass: m_w, sliding_fric: wt.sliding_fric, normal: n, point: hit.point, damp: None,
                });
                if let (true, Some(m), Some(c)) = (second != 0.0, rebuilt[i], contacts[i].as_mut()) {
                    c.forward = Vec3::ZERO - m[2];
                    c.lateral = m[0];
                    c.normal = m[1];
                    c.point = m[3] - m[1] * wt.radius;
                    // (and the arm of what it applies is to that point)
                    let cols = [Vec3::new(body_rows[0].x, body_rows[1].x, body_rows[2].x), Vec3::new(body_rows[0].y, body_rows[1].y, body_rows[2].y), Vec3::new(body_rows[0].z, body_rows[1].z, body_rows[2].z)];
                    c.arm = (c.point - body_rows[3]) - Vec3::new(super::exact::dot3(com_local, cols[0]), super::exact::dot3(com_local, cols[1]), super::exact::dot3(com_local, cols[2]));
                    vs.wheels[i].contact_in = [c.point, c.normal];
                    vs.wheels[i].load = second;
                    w.load = second;
                }
            }
            continue;
        }
        let f_susp = n * load;
        if !exact_sums {
            force += f_susp;
            torque += arm.cross(f_susp);
        }
        if dbg_flag("MCLA_DBG_TERMS") {
            eprintln!("SUSP {i} local {:?} n local {:?} rate {:.4} fix {:.1}", rm.transpose() * f_susp, rm.transpose() * n, comp_rate, comp_rate * dt * k * wt.suspension_damp_factor);
        }
        // (vehWheel::Update 0x825666A0 hands the body SuspensionDampFactor of that spring as a
        // damping matrix along the ground's normal, with the force the spring gains in a tick)
        let mut damp_call: Option<(f32, Vec3)> = None;
        if impedance > 0.0 && wt.suspension_damp_factor > 0.0 {
            if exact_sums {
                // (0x825667C4..0x825668A8: the matrix's strength, and the force - the rate
                // times the step, times the stiffness, times the factor - along the normal,
                // nothing past the limit)
                let fd = if comp < wt.suspension_limit { n * (((comp_rate * dt) * k) * wt.suspension_damp_factor) } else { Vec3::ZERO };
                damp_call = Some((impedance, fd));
            } else if comp < limit {
                let f = n * (comp_rate * dt * k * wt.suspension_damp_factor);
                force += f;
                torque += arm.cross(f);
            }
            implicit.push((arm, n, impedance * wt.suspension_damp_factor));
        }
        // (replays of a recording stand on the recording's surfaces)
        let replay_grip = f32::from_bits(REPLAY_SURFACE[i.min(3)].load(std::sync::atomic::Ordering::Relaxed));
        if replay_grip > 0.0 {
            surf_grip = replay_grip;
        }
        let burning = vs_burn_state == 1 || vs_burn_state == 2;
        // (a recorded burnout: the spinning tyres push with their whole sliding friction -
        // 10,970 N on 3,720 N of load - the car is held by its other axle's brakes alone)
        let _ = burning;
        // melted driven tyres slide on SpinningFric
        // (0x82380BF8 at 0x82381224: the difference times the melt plus the sliding friction, fused)
        let sliding_fric = if vs_melt > 0.0 && powered(i) { fma32(t.tire_fric.spinning_fric - wt.sliding_fric, vs_melt, wt.sliding_fric) } else { wt.sliding_fric };
        let k_lat = 2.0 * w_static / wt.disp_limit_lat.max(0.005);
        let k_long = 2.0 * w_static / wt.disp_limit_long.max(0.005);
        let c_lat = 2.0 * wt.damp_coef_lat * (k_lat * m_w).sqrt();
        let c_long = 2.0 * wt.damp_coef_long * (k_long * m_w).sqrt();
        prepared[i].contact = true;
        prepared[i].response = super::drivetrain::WheelResponse {
            spring: k_long, damper: c_long, inverse_step: 1.0 / dt,
            // (wheel +296: sub_82565E00 writes this tick's contact velocity at 0x82566134, ahead
            // of the response block - only the spin and the slip are the preceding tyre update's.
            // Seen in a recorded launch of the original: with last tick's velocity the wheels
            // of a standing car hook up instead of spinning.)
            velocity: v_fwd, radius,
            optimum_slip: wt.optimum_slip, friction: wt.static_fric * surf_grip,
            load, spin: -previous_wheels[i].spin_rate, slip: previous_wheels[i].slip_long,
        };
        contacts[i] = Some(TyreFrame {
            contact: super::tyre::Contact {
                radius, spin: 0.0, velocity_lat: v_lat, velocity_long: v_fwd,
                load, surface: surf_grip, spring_lat: k_lat, spring_long: k_long,
                damper_lat: c_lat, damper_long: c_long, dt,
            }, arm, forward: wheel_fwd, lateral: wheel_lat, surf_drag, surf_sink: super::collision::surface_depth(hit.surface), mass: m_w, sliding_fric, normal: n, point: hit.point, damp: damp_call,
        });
        w.load = load;
        vs.wheel_grip[i.min(3)] = surf_grip;
    }

    // (wheel +320..+383 as this update leaves them: the contact's frame - sideways, normal, forwards
    // (which points to the car's rear), the record's point; 0x8256663C: a wheel that stands on nothing -
    // no record, or one that carries nothing - has its own matrix there instead, as this update
    // rebuilt it before lift and lean: axle, up, way, and its place less its up x its radius. Seen in rn's samples.)
    for i in 0..4 {
        vs.wheels[i].record = contacts[i].is_some();
        match (contacts[i], rebuilt[i]) {
            (Some(c), _) if c.contact.load != 0.0 => vs.wheels[i].frame = [c.lateral, c.normal, Vec3::ZERO - c.forward, c.point],
            (_, Some(m)) => vs.wheels[i].frame = [m[0], m[1], m[2], m[3] - m[1] * if i < 2 { t.wheel_front.radius } else { t.wheel_back.radius }],
            _ => {}
        }
        vs.wheels[i].back_then = vs.wheels[i].frame[2];
        vs.wheels[i].side_then = vs.wheels[i].frame[0];
    }
    // ---- what the wheels' collision shapes stand at until the next update: the last thing
    // the original's vehicle update does (0x8255CEA0) is to write each wheel's matrix - its
    // steering about its pivot (0x82567508), its place with the new compression, its lean
    // (0x825681E0) - taken back through the chassis's matrix of this update into the car's
    // bound, as that wheel part's matrix among the car's parts. The collision after the
    // physics step multiplies it with the body's matrix as that then stands (0x82575450).
    // Here the matrix is formed among the car's parts directly, from this update's own
    // steering, compression and lean. ----
    if !bike_old(vt) && !dbg_flag("MCLA_DBG_OLDWHEELPOSE") {
        let rear_mean = (2..4).map(|i| {
            let wt = hyd_wheels.as_ref().map_or(&t.wheel_back, |w| &w.1);
            vs.wheels[i].comp.clamp(-wt.travel_extent, wt.travel_limit) - if hyd_fitted { hyd.height[i] } else { 0.0 } - wt.suspension_offset
        }).sum::<f32>() * 0.5;
        for i in 0..4 {
            let front = i < 2;
            let wt = match hyd_wheels.as_ref() {
                Some((f, b)) => if front { f } else { b },
                None => if front { &t.wheel_front } else { &t.wheel_back },
            };
            let ram = if hyd_fitted { hyd.height[i] } else { 0.0 } + wt.suspension_offset;
            let local = p.wheel_pos[i];
            let pivot_arm = if wt.steering_limit <= 0.0 { 0.0 } else if wt.pivot_offset > 0.0 { local.x * wt.pivot_offset } else if local.x.abs() > wt.width { local.x.signum() * wt.width * 0.25 } else { 0.0 };
            let axle = Quat::from_axis_angle(Vec3::Y, -st.wheels[i].steer) * Vec3::X;
            let along = axle.cross(Vec3::Y).normalize_or(Vec3::Z);
            let about = match t.axle_bone_z.filter(|_| !front) {
                Some(z) => Vec3::new(0.0, rear_mean / (z - local.z), -1.0).normalize(),
                None => Vec3::Z,
            };
            let lean = if dbg_flag("MCLA_DBG_NOCAMBER") { Quat::IDENTITY } else { Quat::from_axis_angle(about, vs.wheels[i].camber * local.x.signum()) };
            let pivot = if dbg_flag("MCLA_DBG_NOPIVOT") { Vec3::ZERO } else { (axle - Vec3::X) * pivot_arm };
            vs.wheels[i].part = Some([lean * axle, lean * Vec3::Y, lean * along, local + pivot - Vec3::Y * (p.susp_rest + ram - vs.wheels[i].comp)]);
            vs.wheels[i].world = None;
        }
        // ---- the same in the original's own operations: the end of vehWheel::Update
        // (0x825666A0: the lean's angle from the new compression - kept where the wheel is not
        // pressed in past its offset - and the place lifted along the wheel's up by the
        // compression), vehAxle::Update (0x825681E0: the rows turned by that angle about the
        // axle's third row - the chassis's, or that of a live axle's bone, pitched by the mean
        // of its two wheels) and 0x8255CEA0 (the matrix taken back through the chassis's). ----
        for i in 0..4 {
            let Some(m) = rebuilt[i] else { continue };
            let front = i < 2;
            let wt = match hyd_wheels.as_ref() {
                Some((f, b)) => if front { f } else { b },
                None => if front { &t.wheel_front } else { &t.wheel_back },
            };
            let side = p.wheel_pos[i].x;
            let comp = vs.wheels[i].comp;
            if wt.camber_limit > 0.0 {
                let d = comp - offsets[i];
                if d > 0.0 {
                    vs.wheels[i].camber = (d / wt.suspension_limit) * wt.camber_limit;
                }
            }
            let place = m[1] * comp + m[3];
            let mut rows = [m[0], m[1], m[2]];
            if wt.camber_limit > 0.0 && !dbg_flag("MCLA_DBG_NOCAMBER") {
                let axis = match t.axle_bone.filter(|_| !front) {
                    Some(bone) => {
                        // (0x82567DA0 and 0x825681E0: the bone's third row with its height
                        // set from the mean of the two wheels over the bone's distance from
                        // the left wheel along that row, carried through the chassis's matrix)
                        let l2 = Vec3::from(bone[2]);
                        let rest_of = |j: usize| match vs.rest_exact {
                            Some(r) => Vec3::new(r[j][0], r[j][1], r[j][2]),
                            None => Vec3::new(p.wheel_pos[j].x, p.wheel_pos[j].y - p.susp_rest, p.wheel_pos[j].z),
                        };
                        let d = rest_of(2) - Vec3::from(bone[3]);
                        let along = super::exact::dot_xyz(l2, d);
                        let over = if along.abs() > 0.01 { 1.0 / along } else { 0.0 };
                        let disp = |j: usize| super::exact::axle_disp(vs.wheels[j].comp, wt.travel_extent, wt.travel_limit, offsets[j]);
                        let (dl, dr) = (disp(2), disp(3));
                        let y = if over != 0.0 { super::exact::fms(dr + dl, 0.5, 0.0) * over } else { l2.y };
                        super::exact::row_through(Vec3::new(l2.x, y, l2.z), &body_rows)
                    }
                    None => body_rows[2],
                };
                rows = super::exact::leaned(&rows, axis, super::exact::sign(side) * vs.wheels[i].camber);
            }
            let world = [rows[0], rows[1], rows[2], place];
            vs.wheels[i].world = Some(world);
            vs.wheels[i].part = Some(super::gjk::part_matrix(&world, &body_rows));
        }
    }

    // ---- engine and drivetrain ----
    // engine throttle: above one half it drives, below the engine holds the car back
    let thr = throttle_in.clamp(0.0, 1.0);
    let thr_eng = vs.engine_pedal;
    for k in 0..2 {
        let mut indices = [0usize; 4];
        let mut wheels = [super::drivetrain::PreparedWheel::default(); 4];
        let mut n = 0;
        for i in 0..4 {
            if active(i) && train_of(i) == k {
                indices[n] = i;
                wheels[n] = prepared[i];
                n += 1;
            }
        }
        if n == 0 { vs.omega[k] = 0.0; vs.diff[k] = 1.0; continue; }
        let mut tune = if k == 0 { t.powertrain.clone() } else { t.freetrain.clone() };
        if bike_old(vt) && k == 1 { tune.ang_drag = 0.0; } // (the older bike path: no drag on the front shaft; Z3 exact: the freetrain as tuned)
        tune.ang_drag *= dbg_angdrag();
        let engine = (k == 0 && vs.clutch).then_some(super::drivetrain::EngineDrive {
            ratio, speed: vs.engine_w, torque: vs.engine_torque, inertia: e.inertia,
            sync_torque: vt.sync_torque, idle_speed: e.idle_rpm * 0.104_719_76,
            // (the computer drivers' update, 0x822B9848 and 0x827453EC, writes IdleRPM over their
            // car's EngageRPM every frame: their engines never get the converter's extra ratio)
            max_speed: e.max_rpm * 0.104_719_76, engage_rpm: if input.ai || REPLAY_AI.load(std::sync::atomic::Ordering::Relaxed) { e.idle_rpm } else { e.engage_rpm }, cvt: e.engage_cvt,
            pedal: thr_eng, gear_change_inertia: t.trans.s.gear_change_inertia,
            direction: vs.gear_change.direction, disconnected: vs.trans_inhibited,
        });
        let solved = super::drivetrain::update_prepared(&tune, -vs.omega[k], vs.diff[k], engine, dt, &wheels[..n]);
        vs.omega[k] = -solved.speed;
        vs.diff[k] = solved.differential;
        if let Some(speed) = solved.engine_speed { vs.engine_w = speed; }
        for j in 0..n { vs.wheels[indices[j]].spin_rate = -solved.wheel_speeds[j]; }
    }
    // New tyre forces use the newly solved wheel speed; surface drag is applied
    // to the chassis after storing the original raw wheel torque (+548).
    for i in 0..4 {
        if !active(i) { continue; }
        let ws = &mut vs.wheels[i];
        let w = &mut st.wheels[i];
        w.spin += ws.spin_rate * dt;
        let Some(mut frame) = contacts[i] else { continue };
        let wt = match hyd_wheels.as_ref() {
            Some((f, b)) => if i < 2 { f } else { b },
            None => if i < 2 { &t.wheel_front } else { &t.wheel_back },
        };
        frame.contact.spin = -ws.spin_rate;
        let tyre = super::tyre::update(wt, frame.sliding_fric, frame.contact, &mut ws.disp_lat, &mut ws.disp_long);
        ws.tyre_torque = frame.contact.radius * tyre.longitudinal;
        ws.velocity_long = frame.contact.velocity_long;
        ws.slip_lat = tyre.slip_lat;
        ws.slip_long = tyre.slip_long;
        ws.slide = if bike_old(vt) || dbg_flag("MCLA_DBG_OLDTYREIN") {
            tyre.slide
        } else {
            // (the end of the tyre's update, 0x82566C24..0x82566CF8, as written: which of the
            // two friction evaluations' slide values counts depends on which displacement
            // held and on the contact's speeds; a value that crosses one half from below
            // jumps to one; under 2 m/s all round it is nothing, else at least 0.01)
            let (v_lat, v_long) = (frame.contact.velocity_lat, frame.contact.velocity_long);
            let patch = v_long + frame.contact.spin * frame.contact.radius;
            let old = ws.slide;
            let mut now = if !tyre.grip_long && (v_long.abs() > 2.0 || patch.abs() > 2.0) {
                if tyre.grip_lat { tyre.slide_long } else if tyre.slide_lat - tyre.slide_long >= 0.0 { tyre.slide_lat } else { tyre.slide_long }
            } else if tyre.grip_lat {
                0.0
            } else {
                tyre.slide_lat
            };
            if old < 0.5 && now > 0.5 && 1.0 - now >= 0.0 {
                now = 1.0;
            }
            if v_lat.abs() < 2.0 && v_long.abs() < 2.0 && patch.abs() < 2.0 {
                0.0
            } else if 0.01 - now >= 0.0 {
                0.01
            } else {
                now
            }
        };
        // (the end of vehWheel::Update, 0x82566D38: the surface's drag on the tyre - the
        // material's drag times the wheel's load times the square of the contact's speed,
        // sideways by SurfaceDragCoefLat and along the wheel by SurfaceDragCoefLong, the
        // latter grown by the share the tyre sinks in. The cars' tunes have 0.05 sideways
        // and nothing along the wheel.)
        let drag = frame.surf_drag * frame.contact.load;
        let f_lat = tyre.lateral - wt.surface_drag_lat * drag * frame.contact.velocity_lat * frame.contact.velocity_lat.abs();
        let f_long = tyre.longitudinal - wt.surface_drag_long * drag * frame.contact.velocity_long * frame.contact.velocity_long.abs() * (1.0 + frame.surf_sink);
        ws.f_lat = f_lat;
        ws.f_long = f_long;
        let f = frame.forward * f_long + frame.lateral * f_lat;
        if dbg_flag("MCLA_DBG_TERMS") {
            let inv = rm.transpose();
            eprintln!("TERM {i} camber {:.4} comp {:.4} arm {:?} tyre force {:?} load {:.1} flong {:.1} flat {:.1} fwd {:?}", ws.camber, ws.comp, inv * frame.arm, inv * f, ws.load, f_long, f_lat, inv * frame.forward);
        }
        if exact_sums {
            // ---- what vehWheel::Update applies to the body, operation for operation
            // (0x825667A4..0x82566F80) ----
            use super::exact::{cross, dot3, fnms};
            let key = 300 + 3 * i as u16;
            // the damping's hand-off (body vfunc 11, 0x8259EA40): its force at the arm set
            // off from the body's place
            ws.damp_value = frame.damp.map_or(0.0, |d| d.0);
            if let Some((value, fd)) = frame.damp {
                let r = (frame.arm + chassis_at) - body_at;
                calls.push((key, fd, cross(r, fd)));
                damping.add(frame.normal, (frame.normal * value) * wt.suspension_damp_factor, r);
            }
            // the tyre's force in the contact's directions with the surface's drag, and the
            // spring's along the normal
            let (back, side, n) = (-frame.forward, frame.lateral, frame.normal);
            let (v_lat, v_long, load) = (frame.contact.velocity_lat, frame.contact.velocity_long, frame.contact.load);
            let sink = frame.surf_sink + 1.0;
            let along = ((wt.surface_drag_long * frame.surf_drag) * load) * v_long.abs() * v_long;
            let across = ((wt.surface_drag_lat * frame.surf_drag) * v_lat) * load * v_lat.abs();
            let b = fnms(-along, sink, -tyre.longitudinal);
            let a = -across + tyre.lateral;
            let fv = n * load + (back * b + side * a);
            calls.push((key + 1, fv, Vec3::ZERO));
            // its torque: the arm taken into the car's frame and back (where the tuning's
            // WheelPositionScale would scale it), crossed with the force, and that through
            // the car's frame once more (chassis +80, all ones)
            let rows = [body_rows[0], body_rows[1], body_rows[2]];
            let cols = [Vec3::new(rows[0].x, rows[1].x, rows[2].x), Vec3::new(rows[0].y, rows[1].y, rows[2].y), Vec3::new(rows[0].z, rows[1].z, rows[2].z)];
            let local = Vec3::new(dot3(rows[0], frame.arm), dot3(rows[1], frame.arm), dot3(rows[2], frame.arm));
            let r = Vec3::new(dot3(local, cols[0]), dot3(local, cols[1]), dot3(local, cols[2]));
            let tq = cross(r, fv);
            let local = Vec3::new(dot3(rows[0], tq), dot3(rows[1], tq), dot3(rows[2], tq));
            calls.push((key + 2, Vec3::ZERO, Vec3::new(dot3(local, cols[0]), dot3(local, cols[1]), dot3(local, cols[2]))));
        } else {
            force += f;
            torque += frame.arm.cross(f);
        }
        w.slip = (ws.slide * 2.0).min(2.0);
        w.f_lon = f_long;
    }
    if vt.bike {
        // Rendering still has four slots and averages each pair. Only the even
        // slots are physical wheels; the odd slots are visual mirrors.
        for i in [0, 2] {
            st.wheels[i + 1] = st.wheels[i];
            st.wheels[i + 1].grounded = false;
            st.wheels[i + 1].load = 0.0;
            vs.wheels[i + 1] = vs.wheels[i];
            vs.wheels[i + 1].load = 0.0;
        }
    }
    if grounded_count == 0 {
        st.airborne_time += dt;
    } else {
        st.airborne_time = 0.0;
    }
    // nitrous pushes the car along its length: NitroBoostAmount m/s^2 times level^NitroBoostExp
    let exact_nitro = vs.by.exact && !dbg_flag("MCLA_DBG_A1");
    if exact_nitro {
        // ---- mcCarNitro::Update 0x82345A80 (from the chassis update 0x82330378 after the vehicle's update,
        // and so after the launch and the engine's roll, ahead of the air and the wheels - call 150): level -= dt / NitroBoostTime; out at nothing, or lying on the ground with the
        // up under 0.1 (0x823455B8); the force along the car's length -(NitroBoostAmount x level^NitroBoostExp)
        // x throttle (+136) x wheels with a record / wheels x the gear's way, times the mass (0x8259BEF8) ----
        if vs.nitro_burning {
            vs.nitro_level = fnms32(1.0 / t.nitro.boost_time, dt, vs.nitro_level);
            let upset = !(vs.air584 > 0.0) && body_rows[1].y < 0.1;
            if !(vs.nitro_level > 0.0) || upset {
                vs.nitro_level = 0.0;
                vs.nitro_burning = false;
            }
            let p = (vs.nitro_level as f64).powf(t.nitro.boost_exp as f64) as f32;
            let k = -(t.nitro.boost_amount * p);
            let share = vs.nitro_records as f32 / wheels_in as f32;
            let way: f32 = if vs.gear < 0 { -1.0 } else if vs.gear == 0 { 0.0 } else { 1.0 };
            calls.push((150, ((((body_rows[2] * k) * throttle_in) * share) * way) * mass, Vec3::ZERO));
            // (RB3, 0x82345C10: the view goes out towards NitroFov at NitroFov / NitroFovOutTime a second -
            // also in the tick the burn ends)
            if vs.nitro_view != t.nitro.fov {
                super::exact::approach(&mut vs.nitro_view, t.nitro.fov, t.nitro.fov / t.nitro.fov_out_time, dt);
            }
        } else if vs.nitro_view != 0.0 {
            // (RB3, 0x82345C80: and comes back to its rest, nitro +20, at NitroFov / NitroFovInTime)
            super::exact::approach(&mut vs.nitro_view, 0.0, t.nitro.fov / t.nitro.fov_in_time, dt);
        }
    } else if vs.nitro_level > 0.0 && grounded_count > 0 {
        force += fwd * (t.nitro.boost_amount.max(0.0) * vs.nitro_level.powf(t.nitro.boost_exp.max(0.1)) * mass * grounded_count as f32 / wheel_count);
    }
    // the slipstream turbo pushes like nitrous, with its own strength (turboBoostAmount)
    if vs.sst_level > 0.0 && grounded_count > 0 {
        force += fwd * (t.ss_turbo.boost_amount.max(0.0) * vs.sst_level.powf(if t.ss_turbo.boost_exp > 0.1 { t.ss_turbo.boost_exp } else { 3.0 }) * mass * grounded_count as f32 / wheel_count);
    }
    // burnout: the launch pushes the car forward (BurnBoost), the charge swings it with the steering
    // (0x8232FC64: BurnBoost times the mass for as long as the charge lasts, whatever is left
    // of it - 13,680 N on the R8 - less with the brake or the handbrake on)
    if !vs.by.exact && vs.burn_state == 3 && grounded_count > 0 && !input.handbrake {
        force += fwd * (t.s.burn_boost * thr * (1.0 - brake_in.clamp(0.0, 1.0)) * mass);
    }
    if !vs.by.exact && (vs.burn_state == 1 || vs.burn_state == 2) && grounded_count > 0 {
        let have = st.ang_vel.dot(up);
        torque += up * ((vs.burn_yaw - have) * inertia.y * 8.0);
    }
    // The wheelie and the stoppie (mcBikeGyro 0x8236A5C0 / 0x8236AA28 through vehGyro
    // 0x82563018): a torque about the axle of the wheel that stays down - the controller's
    // output times the mass - with the force that leaves that wheel's contact where it is,
    // less what of it would push down into the road. The wheelie's boost pushes at the rear
    // contact along the wheel (0x82562D20), so it lifts the nose as well.
    if vs.wheelie != 0 && bike_old(vt) {
        let stoppie = vs.wheelie == 2 || vs.wheelie == 98;
        let w = st.wheels[if stoppie { 0 } else { 2 }];
        let contact = if w.grounded { w.contact } else { st.pos + rm * ((p.wheel_pos[if stoppie { 0 } else { 2 }] + p.wheel_pos[if stoppie { 1 } else { 3 }]) * 0.5) };
        let n = if w.grounded { w.normal } else { Vec3::Y };
        let t_gyro = right * (vs.wheelie_lift * mass);
        torque += t_gyro;
        let spin_up = rm * ((rm.transpose() * t_gyro) / inertia);
        let mut f = spin_up.cross(contact - pivot) * -mass;
        let into = f.dot(n);
        if into > 0.0 {
            f -= n * into;
        }
        if dbg_flag("MCLA_DBG_GYRO") { eprintln!("WHEELIE lift {:.4} err {:.4} aim {:.4} torque {:?} force {:?} off {:?}", vs.wheelie_lift, vs.wheelie_err, vs.wheelie_pitch, t_gyro, f, contact - pivot); }
        force += f;
        if !stoppie && vs.wheelie_boost > 0.0 && w.grounded && vs.gear > 0 && (vs.gear as usize) < vt.ratios.len() - 1 {
            let k = (1.0 - st.vel.length() * 2.237 / t.trans.s.high_gear_mph) * t.gyro.wheelie_boost_acc * mass;
            // (along the rear wheel's way over the ground, wheel +352 - not along the reared-up
            // body: the higher the bike stands, the longer this push's arm)
            let push = (fwd - n * fwd.dot(n)).normalize_or(fwd) * k;
            force += push;
            // (a plain force on the body, 0x82562D20: its arm is from the body's own place - the
            // point high in the bike it turns about - not from the low centre of mass the wheels'
            // forces are moved to; reared up, that arm is a metre)
            // (that helper's arm ends at chassis +112 carried into the world - the tuned centre
            // of mass set off from the body's place, 15 cm higher still on a ZX-14: recorded
            // 1,662 N m from a push of 2,290 N, an arm of 0.73 m where the body's place gives 0.58)
            torque += (contact - (pivot + rm * com_local)).cross(push);
            if dbg_flag("MCLA_DBG_GYRO") { eprintln!("BOOST push {:.1} torque {:.1} v {:.2}", k, (contact - (pivot + rm * com_local)).cross(push).length(), st.vel.length()); }
        }
    }
    // DriftThrust (mcCarGyro, 0x82366B3C..0x82366D1C): a push along the car's length of
    // DriftThrust x the drive shaft's speed at its wheel (chassis +572 - with the wheels
    // spinning far more than the car's own speed) x a level x the share of the wheels on the
    // ground x sin(pi/2 x the sideways share of the velocity) x the throttle x the grip of the
    // surface under the shaft's first wheel, halved in top gear, times the mass. The level
    // (gyro +56) jumps to the handbrake when that is more and falls back at DriftDecay - which
    // the data has at 0, so once pulled it stays: every recording has it at 1 from its
    // second sample on, the handbrake never touched. Recorded on a Range Rover leaving at
    // full lock with its rear wheels spinning at 20 m/s: 140 N at a sideways share of 0.04,
    // 455 at 0.11, 710 at 0.21 - DriftThrust 0.05 to 2%.
    // ---- Z3: mcBikeGyro::Update (0x823688B8), after the wheels and the axles as in the original,
    // from the listing: the filters, the turn (Turn = 0: the body's turning rate is SET), the
    // handbrake's level and the engine's torque multiplier, the lean (Lean = 0: the body is turned
    // about the line through its contacts by the collider's pending turn and push), the wheelie
    // and the stoppie, the burnout, the spin and wobble, the gravity pitch and air control in the
    // air, the reverse drive - every section of the listing. ----
    if vt.bike && exact_sums { 'gyro: {
        use super::exact::{self as ex, approach, ease, fma, fnma, unit_or_zero};
        let gt = &t.gyro;
        let inv_dt = 1.0 / dt;
        let rows = [body_rows[0], body_rows[1], body_rows[2]];
        let vel = st.vel;
        // (chassis +224 as vehAero wrote it this tick)
        let speed224 = fma(vel.z, vel.z, fma(vel.x, vel.x, vel.y * vel.y)).sqrt();
        let max0 = |x: f32| if x >= 0.0 { x } else { 0.0 };
        let min1 = |x: f32| if x - 1.0 >= 0.0 { 1.0 } else { x };
        let clamp = |x: f32, lo: f32, hi: f32| { let t = if x - lo >= 0.0 { x } else { lo }; if t - hi >= 0.0 { hi } else { t } };
        // (section 0: velocity and its change, low-passed; the coefficients double under 60 Hz)
        let fps = if !(inv_dt < 0.0) { (inv_dt + 0.5) as i32 } else { (inv_dt - 0.5) as i32 };
        let a = if fps < 60 { 0.4f32 * 2.0 } else { 0.4 };
        let old = vs.bg.vel;
        // (R4: the vector unit gives nothing for a result too small to be a whole number of its kind - a
        // velocity filtered down at a standstill ends at exactly 0, not in the denormals: V se.0 426.
        // MCLA_DBG_R4: left as they come.)
        let fz = |v: Vec3| if dbg_flag("MCLA_DBG_R4") { v } else { Vec3::new(if v.x.is_subnormal() { 0.0 } else { v.x }, if v.y.is_subnormal() { 0.0 } else { v.y }, if v.z.is_subnormal() { 0.0 } else { v.z }) };
        let new_vel = fz(fz(vel * a) + fz(old * (1.0 - a)));
        let acc = fz(fz(new_vel - old) * inv_dt);
        let b = if fps < 60 { 0.15f32 * 2.0 } else { 0.15 };
        let new_acc = fz(fz(acc * b) + fz(vs.bg.acc * (1.0 - b)));
        vs.bg.vel = new_vel;
        vs.bg.acc = new_acc;
        // (P3, 0x82368A44: with the rider off the gyro does nothing more)
        if wiped {
            // (the engine's torque multiplier, +32, stays as the last whole update left it: V rp.0 129..)
            break 'gyro;
        }
        // (section 2)
        let g_neg = (if t.s.bound_gravity > 0.1 { t.s.bound_gravity } else { 1.0 }) * f32::from_bits(0xc11c_cccd);
        let back = rows[2];
        let fwd_speed = fnma(back.x, vel.x, fma(back.z, vel.z, back.y * vel.y));
        let (wf, wr) = (vs.wheels[0], vs.wheels[2]);
        let ground = wf.record as i32 + wr.record as i32;
        let steer = vs.pad_steer;
        // (input +36 / +32: the stick's two axes - X raw, as the camera's turn event also reads it)
        let (stick_x, stick_y) = (input.steer, input.lean);
        // (the weight button, input +40 bit 0 - not with a burnout state. The port's game has one key
        // for "lean back" / "lean forward": stick and button together; a replay hands them apart)
        let btn = (input.xfer || (input.lean != 0.0 && !vs.pad.live && !REPLAY_STEER.load(std::sync::atomic::Ordering::Relaxed))) && vs.burn_state == 0;
        let hand: f32 = handbrake;
        let wb = rest_zs[2] - rest_zs[0];
        let inverse = Vec3::new(ex::recip(inertia.x), ex::recip(inertia.y), ex::recip(inertia.z));
        let chassis = [rows[0], rows[1], rows[2], chassis_at];
        // (0x82563018 with a ground normal: the torque through the chassis's frame, the force that
        // leaves the point where it is, less what of it points along the normal)
        let torque_about = |tq: Vec3, off: Vec3, n: Vec3| -> (Vec3, Vec3) {
            let (tq, f) = ex::gyro_turn(tq, off, &chassis, &rows, inverse, mass);
            let lift = fma(f.x, n.x, fma(n.y, f.y, n.z * f.z));
            (tq, if lift > 0.0 { f - n * lift } else { f })
        };
        // (0x82562D20: a force at a point, its arm from the centre of mass carried through the chassis's frame)
        let cg = ex::point_through(com_local, &chassis);
        let mut w = st.ang_vel;
        let mut spin_set = false;
        // (section 3: the turn)
        if ground != 0 {
            let n = if wr.record { wr.frame[1] } else { Vec3::Y };
            let v = if fwd_speed < 0.0 && !(-vs.omega[0] > 0.0) { -fwd_speed } else { fwd_speed };
            let blend = fma(-gt.turn_factor, vs.sss_chassis, gt.turn_factor);
            let yaw = fma(w.x, n.x, fma(w.y, n.y, w.z * n.z));
            let base = (((t.wheel_front.steering_limit * vs.steer) * v) * gt.turn_bias) * (-1.0 / wb);
            let pedal = clamp((if btn { -1.0 } else { 0.0 }) + throttle_in, -1.0, 1.0);
            let target = fma(fma(pedal, gt.turn_oversteer, 1.0) * base, blend, (1.0 - blend) * yaw);
            vs.bg.yaw_target = target;
            vs.bg.intent = if btn { if target > 0.1 { 1 } else if target < -0.1 { -1 } else { 0 } } else { 0 };
            let mut err = target - yaw;
            let rate = (err - vs.bg.yaw_err) * inv_dt;
            if gt.turn > 0.0 {
                let mut gain = gt.turn;
                if gt.turn_limit > 0.0 {
                    err = clamp(err, -gt.turn_limit, gt.turn_limit);
                    gain /= gt.turn_limit;
                }
                let mut out = gain * err + ((gain.sqrt() * gt.turn_damp) * 2.0) * rate;
                if v.abs() < 10.0 {
                    out = (v.abs() * out) * 0.1;
                }
                let (tq, f) = torque_about(n * ((vs.wheel_grip[0] * out) * mass), wr.frame[3] - body_at, n);
                calls.push((530, Vec3::ZERO, tq));
                calls.push((531, f, Vec3::ZERO));
            } else {
                let sc = if v.abs() < 5.0 { v.abs() * 0.2 } else { 1.0 };
                err = ((1.0 - gt.turn_damp) * (vs.wheel_grip[0] * sc)) * err;
                w = n * err + w;
                // (body vfunc 43 is called whatever the error: below SSSThresholdMin the blend is 0, the
                // error exactly 0, and the call still makes the angular momentum afresh from the
                // turning rate through the rows as they stand - which are not quite square)
                spin_set = true;
            }
            vs.bg.yaw_err = err;
        }
        // (RB6, section 4, 0x82368F44..0x82369194: the burnout - handbrake on, throttle (chassis +136) above
        // 0.1, charge (chassis +580), the gearbox forward. With a wheel on the ground: the burnout's rate,
        // gyro +120, goes towards -(sqrt(engine speed / its speed of best power) x charge x the pad's
        // steering x BurnoutLimit) at BurnoutRate; its error against the body's turning about the FRONT
        // wheel's ground normal turns the bike - BurnoutTurn > 0: a torque through vehGyro's routine at
        // the front contact; the bikes' 0: the turning rate is set, x (1 - TurnDamp) - and the front
        // wheel's sideways speed is braked: the acceleration that stops it over half of
        // (1 - BurnoutGrip) seconds, times the mass, along the contact's sideways (no charge in it, as
        // coded). With no wheel on the ground: nothing of the section. V: se.1. MCLA_DBG_RB6: as before.)
        if hand > 0.0 && throttle_in > 0.1 && vs.burn > 0.0 && vs.gear > 0 && !dbg_flag("MCLA_DBG_RB6") {
            if ground != 0 {
                let best = vt.torque_curve.as_ref().map_or(t.engine.s.opt_rpm * 0.104_719_76, |c| c.peak().0);
                let root = ((vs.engine_w / best) as f64).powf(0.5) as f32;
                let n2 = wf.frame[1];
                let off2 = wf.frame[3] - body_at;
                let yaw2 = fma(n2.x, w.x, fma(w.y, n2.y, w.z * n2.z));
                approach(&mut vs.bg.burn, -(((root * vs.burn) * steer) * gt.burnout_limit), gt.burnout_rate, dt);
                let e = vs.bg.burn - yaw2;
                if gt.burnout_turn > 0.0 {
                    let (tq, f) = torque_about(n2 * ((gt.burnout_turn * mass) * e), off2, n2);
                    calls.push((532, Vec3::ZERO, tq));
                    calls.push((533, f, Vec3::ZERO));
                } else {
                    w = n2 * ((1.0 - gt.turn_damp) * e) + w;
                    spin_set = true;
                }
                let lat = wf.velocity_lat;
                let stop = ex::stop_rate(1.0 - gt.burnout_grip, lat * dt, lat, 0.0, 0.0);
                calls.push((533, wf.frame[0] * (mass * stop), Vec3::ZERO));
            }
        } else
        // (section 4, off the handbrake: the burnout's rate follows the body's)
        {
            let sgn = |x: f32| if x >= 0.0 { if -x >= 0.0 { 0.0 } else { 1.0 } } else { -1.0 };
            if sgn(vs.bg.burn) != sgn(w.y) {
                vs.bg.burn = 0.0;
            }
            approach(&mut vs.bg.burn, w.y, gt.burnout_rate, dt);
            if gt.drift > 0.0 && hand > 0.0 && brake_in == 0.0 && fwd_speed > 10.0 {
                let sp = if fwd_speed < 25.0 { (fwd_speed * fwd_speed) * f32::from_bits(0x3d23_d70a) } else { fwd_speed };
                let n = wr.frame[1];
                let k = (((((ground as f32 / wheel_count) * steer) * sp) * hand) * gt.drift) * mass;
                let (tq, f) = torque_about(n * -k, wr.frame[3] - body_at, n);
                calls.push((532, Vec3::ZERO, tq));
                calls.push((533, f, Vec3::ZERO));
            }
        }
        // (section 5: the front tyre's slide is cleared every tick)
        vs.wheels[0].slide = 0.0;
        // (section 6, 0x82369378..0x82369708: the spin on both brakes and the wobble. With the brake or
        // the handbrake on and Spin, RevSpin or WobbleAmplitude above nothing: with both on, a torque
        // through vehGyro's routine about the front wheel's ground normal at its contact,
        // -(|sideways speed| x the share of the wheels down - a whole-number division: 1 only with both -
        // x the pad's steering x the brake x the mass x Spin; -RevSpin while the shaft turns forward).
        // The wobble's clock, gyro +40, is -1 while it is off. All four bikes have RevSpin and
        // WobbleAmplitude 0.)
        if (brake_in > 0.0 || hand > 0.0) && (gt.spin > 0.0 || gt.rev_spin > 0.0 || gt.wobble_amplitude > 0.0) {
            let k = if vs.omega[0] < 0.0 { -gt.rev_spin } else { gt.spin }; // (-pt +48 < 0: the shaft forward)
            if brake_in > 0.0 && hand > 0.0 && k != 0.0 {
                let r = rows[0];
                let side = fma(vel.x, r.x, fma(r.z, vel.z, r.y * vel.y));
                let share = (ground / wheel_count as i32) as f32;
                let s = -(((((side.abs() * share) * steer) * brake_in) * mass) * k);
                let n = wf.frame[1];
                let (tq, f) = torque_about(n * s, wf.frame[3] - body_at, n);
                calls.push((533, f, tq));
            }
            if gt.wobble_amplitude != 0.0 {
                if vs.bg.wobble < 0.0 {
                    if fwd_speed > f32::from_bits(0x4232_cccd) {
                        vs.bg.wobble = 0.0;
                    }
                } else if fwd_speed < f32::from_bits(0x417a_51ec) {
                    vs.bg.wobble = -1.0;
                }
                if !(vs.bg.wobble < 0.0) {
                    vs.bg.wobble += dt;
                    vs.bg.wobble = ((vs.bg.wobble as f64) % (gt.wobble_frequency as f64)) as f32;
                    let mut amp = if vs.bg.wobble > gt.wobble_frequency * 0.5 { -gt.wobble_amplitude } else { gt.wobble_amplitude };
                    if brake_in != 0.0 {
                        amp *= brake_in;
                    } else if hand != 0.0 {
                        amp *= hand;
                    }
                    amp = (1.0 - steer.abs()) * amp;
                    if fwd_speed < 10.0 {
                        amp = (amp * fwd_speed) * 0.1;
                    }
                    if wr.record && !btn {
                        let f = wr.frame[0] * (mass * amp);
                        calls.push((533, f, ex::cross(wr.frame[3] - cg, f)));
                    }
                }
            }
        } else if !(vs.bg.wobble < 0.0) {
            vs.bg.wobble = -1.0;
        }
        // (section 7: the handbrake's level, DriftThrust, and the engine's torque multiplier)
        if gt.drift_thrust > 0.0 {
            let hb = if hand > 0.0 { hand } else { 0.0 };
            if hb > vs.bg.level {
                vs.bg.level = hb;
            } else if hb < vs.bg.level {
                approach(&mut vs.bg.level, hb, gt.drift_decay, dt);
            }
            if vs.bg.level > 0.0 {
                let d = unit_or_zero(vel);
                let side = fma(rows[0].x, d.x, fma(rows[0].y, d.y, rows[0].z * d.z));
                let k = (ground as f32 / wheel_count) * ((gt.drift_thrust * shaft_speed_then) * vs.bg.level);
                let e = ease(min1(max0(side.abs())));
                calls.push((534, (rows[2] * -(throttle_in * (e * k))) * mass, Vec3::ZERO));
            }
        }
        if vs.burn == 0.0 && speed224 > 1.0 {
            let d = unit_or_zero(vel);
            let side = fma(d.x, rows[0].x, fma(rows[0].z, d.z, rows[0].y * d.y));
            let e = ease(min1(max0(side.abs())));
            let a = (if e - steer.abs() >= 0.0 { e } else { steer.abs() }) * gt.drift_torque;
            let h = (ex::guest_asin(d.y as f64) as f32) / gt.hill_grade;
            let h = min1(max0(h)) * gt.hill_torque;
            let m = if a - h >= 0.0 { a } else { h };
            vs.torque_mult = m / vt.sync_torque + 1.0;
        }
        // (section 8: the lean)
        {
            let (n, c, fd) = if ground == 2 {
                let r0 = 1.0 / wb;
                let r = r0 * (1.0 - r0 * wb) + r0;
                ((wf.frame[1] + wr.frame[1]) * 0.5, (wf.frame[3] + wr.frame[3]) * 0.5, (wr.frame[3] - wf.frame[3]) * r)
            } else if ground == 1 {
                let wh = if wr.record { wr } else { wf };
                (wh.frame[1], wh.frame[3], wh.frame[2])
            } else {
                (Vec3::Y, body_rows[3], unit_or_zero(Vec3::new(rows[2].x, 0.0, rows[2].z)))
            };
            if rows[1].y > -0.7 {
                let mut share = if btn { gt.lean_angle_max } else { gt.lean_angle };
                if vs.bg.state != 0 {
                    share *= 0.6;
                }
                approach(&mut vs.bg.share, share, gt.lean_rate_dn, dt);
                let s = unit_or_zero(ex::cross(fd, n));
                let mut spd = speed224;
                if vs.burn > 0.0 && vs.gear > 0 {
                    let best = vt.torque_curve.as_ref().map_or(t.engine.s.opt_rpm * 0.104_719_76, |c| c.peak().0);
                    let b = (vs.engine_w / best) * gt.burnout_lean;
                    if b > spd {
                        spd = b;
                    }
                }
                let grav = -g_neg;
                let x1 = ((((steer.abs() * (1.0 - t.wheel_front.optimum_slip)) * angles[0]) / wb) * spd) * spd;
                let lean_steer = vs.bg.share * (ex::guest_atan2(x1 as f64, grav as f64) as f32);
                let pull = fma(vs.bg.acc.x, s.x, fma(vs.bg.acc.y, s.y, vs.bg.acc.z * s.z)) * (1.0 - vs.sss_chassis);
                let c15 = if inv_dt < 60.0 { 0.15f32 * 2.0 } else { 0.15 };
                vs.bg.lat = fma(1.0 - c15, vs.bg.lat, c15 * pull);
                let lean_acc = vs.bg.share * (ex::guest_atan2(vs.bg.lat as f64, grav as f64) as f32);
                let mut target = fma(1.0 - gt.lean_factor, lean_acc, gt.lean_factor * lean_steer);
                if vs.burn > 0.0 {
                    let yaw_n = clamp(fma(w.x, n.x, fma(w.y, n.y, w.z * n.z)) / gt.burnout_limit, -gt.burnout_limit, gt.burnout_limit);
                    target = fma(ex::fms(yaw_n, gt.burnout_lean, target), vs.burn, target);
                }
                let cur = vs.bg.roll;
                let rate = if ground == 0 {
                    gt.lean_rate_dn
                } else if !(cur > 0.0) && !(target > 0.0) && !(target > cur) {
                    gt.lean_rate_dn
                } else if !(cur < 0.0) && !(target < 0.0) && !(target < cur) {
                    gt.lean_rate_dn
                } else {
                    gt.lean_rate_up
                };
                approach(&mut vs.bg.roll, target, rate, dt);
                let roll = ex::guest_asin(rows[0].y as f64) as f32;
                let off = c - body_at;
                let mut err = roll - vs.bg.roll;
                if gt.lean > 0.0 {
                    let rate = (err - vs.bg.roll_err) * inv_dt;
                    let mut gain = gt.lean;
                    if gt.lean_limit > 0.0 {
                        err = clamp(err, -gt.lean_limit, gt.lean_limit);
                        gain /= gt.lean_limit;
                    }
                    let out = gain * err + ((gain.sqrt() * gt.lean_damp) * 2.0) * rate;
                    let (tq, f) = torque_about(fd * -((out * vs.aero_damp) * mass), off, n);
                    calls.push((535, Vec3::ZERO, tq));
                    calls.push((536, f, Vec3::ZERO));
                } else {
                    // (0x82562E18: no force - the body's pending turn and push)
                    let turn = fd * -(((1.0 - gt.lean_damp) * err) * vs.aero_damp);
                    let turn_all = ex::pending_add(vs.push_turn_left, turn);
                    let push = ex::pending_add(Vec3::ZERO, ex::cross(off, turn));
                    vs.bike_pending = Some((turn_all, push));
                }
                vs.bg.roll_err = err;
            }
        }
        // (sections 9 and 10: the wheelie and the stoppie)
        let pitch_now = ex::guest_asin(rows[2].y as f64) as f32;
        // (chassis +584 as this tick's chassis update left it: by the wheels of the tick before)
        // (A1: the chassis's own count, which also stands still while mcCarStuck is armed)
        let air = if dbg_flag("MCLA_DBG_A1") { air_then } else { vs.air584 };
        if gt.wheelie_angle > 0.0 {
            if vs.bg.state == 0 && btn && -vs.omega[0] < 0.0 && speed224 > 5.0 && wr.record && throttle_in > 0.1 && brake_in == 0.0 && stick_y > 0.8 {
                vs.bg.state = 1;
                vs.bg.boost = gt.wheelie_boost_duration;
                vs.bg.pitch_err = 0.0;
                vs.bg.pitch = pitch_now;
            }
            if vs.bg.state == 1 {
                let end = (air < 0.25 && !btn) || air > 0.75 || throttle_in == 0.0 || brake_in > 0.9 || rows[1].y < -0.7 || speed224 < 4.47 || stick_y < -0.5 || (wf.record && !(vs.bg.boost > 0.0));
                if end {
                    vs.bg.boost = 0.0;
                    vs.bg.state = 99;
                }
            }
            if vs.bg.state == 99 && (wf.record || air > 0.25) {
                vs.bg.boost = 0.0;
                vs.bg.state = 0;
            }
            if vs.bg.state > 0 {
                let x = (((stick_y + 1.0) * 0.5) * throttle_in) * if btn { 1.0 } else { 0.0 };
                let mut aim = ex::fms(ease(min1(max0(x))), 2.0, 1.0);
                if vs.bg.state == 99 {
                    aim = -1.0 * 0.15;
                } else if aim < 0.0 {
                    aim *= 0.15;
                }
                let mut tp = -(gt.wheelie_angle * aim);
                if pitch_now < -1.308_996_9 {
                    tp = -2.094_395_2;
                }
                approach(&mut vs.bg.pitch, tp, gt.wheelie_rate, dt);
                let mut err = pitch_now - vs.bg.pitch;
                let rate = if vs.bg.pitch_err != 0.0 { (err - vs.bg.pitch_err) * inv_dt } else { 0.0 };
                vs.bg.pitch_err = err;
                let mut gain = gt.wheelie;
                if gt.wheelie_limit > 0.0 {
                    err = clamp(err, -gt.wheelie_limit, gt.wheelie_limit);
                    gain /= gt.wheelie_limit;
                }
                let out = fma((gain.sqrt() * gt.wheelie_damp) * 2.0, rate, gain * err) * mass;
                let (tq, f) = torque_about(wr.frame[0] * out, wr.frame[3] - body_at, wr.frame[1]);
                calls.push((537, Vec3::ZERO, tq));
                calls.push((538, f, Vec3::ZERO));
                if vs.bg.boost > 0.0 {
                    if wr.record && vs.gear > 0 && (vs.gear as usize) < vt.ratios.len() - 1 {
                        let k = ((1.0 - (speed224 * 2.237) / t.trans.s.high_gear_mph) * gt.wheelie_boost_acc) * mass;
                        let f = wr.frame[2] * -k;
                        calls.push((539, f, ex::cross(wr.frame[3] - cg, f)));
                    }
                    vs.bg.boost -= dt;
                    if vs.bg.boost < 0.0 {
                        vs.bg.boost = 0.0;
                    }
                }
            }
        }
        if gt.nosie_angle > 0.0 {
            if vs.bg.state == 0 && btn && wf.record && brake_in > 0.0 && throttle_in < 0.11 && steer.abs() < 0.5 && stick_y < 0.1 && stick_x.abs() < 0.5 {
                vs.bg.state = -1;
                vs.bg.boost = gt.wheelie_boost_duration;
                vs.bg.pitch_err = 0.0;
            }
            if vs.bg.state == -1 {
                let end = (air < 0.25 && !btn) || air > 0.75 || rows[1].y < -0.7 || speed224 < 4.47 || (wr.record && (stick_y > 0.9 || !(vs.bg.boost > 0.0)));
                if end {
                    vs.bg.boost = 0.0;
                    vs.bg.state = -99;
                }
            }
            if vs.bg.state == -99 && (wr.record || air > 0.25) {
                vs.bg.boost = 0.0;
                vs.bg.state = 0;
            }
            if vs.bg.state < 0 {
                let x = (1.0 - stick_y) * 0.5;
                // (0x82227C70: nothing at or below 0, one from 1 up, 1 - cos of that share of a quarter turn between)
                let ease_cos = if !(x > 0.0) { 0.0 } else if !(x < 1.0) { 1.0 } else { 1.0 - ex::guest_cos((x * f32::from_bits(0x3fc90fdb)) as f64) as f32 };
                let mut m = ease_cos * brake_in;
                if speed224 < 10.0 {
                    m = (speed224 * m) * 0.1;
                }
                let mut tp = gt.nosie_angle * m;
                if pitch_now > 1.308_996_9 {
                    tp = 2.094_395_2;
                }
                let mut err = pitch_now - tp;
                let rate = if vs.bg.pitch_err != 0.0 { (err - vs.bg.pitch_err) * inv_dt } else { 0.0 };
                vs.bg.pitch_err = err;
                let mut gain = gt.wheelie;
                if gt.wheelie_limit > 0.0 {
                    err = clamp(err, -gt.wheelie_limit, gt.wheelie_limit);
                    gain /= gt.wheelie_limit;
                }
                let out = fma((gain.sqrt() * gt.wheelie_damp) * 2.0, rate, gain * err) * mass;
                let (tq, f) = torque_about(wf.frame[0] * out, wf.frame[3] - body_at, wf.frame[1]);
                calls.push((540, Vec3::ZERO, tq));
                calls.push((541, f, Vec3::ZERO));
                if vs.bg.boost > 0.0 {
                    vs.bg.boost -= dt;
                    if vs.bg.boost < 0.0 {
                        vs.bg.boost = 0.0;
                    }
                }
            }
        }
        // (section 11, 0x8236AA90: in the air the nose follows the flight path - GravityPitch x (the
        // path's climb, within 30 degrees, less the nose's) as a turning acceleration about the
        // bike's right, body vfunc 30)
        if gt.gravity_pitch != 0.0 && air > 0.0 {
            let d = unit_or_zero(vel);
            let climb = clamp(ex::guest_asin(d.y as f64) as f32, f32::from_bits(0xbf06_0a92), f32::from_bits(0x3f06_0a92));
            let a = climb - ex::guest_asin(-rows[2].y as f64) as f32;
            calls.push((542, Vec3::ZERO, rows[0] * (inertia.x * (gt.gravity_pitch * a))));
        }
        // (section 12, 0x8236AB84..0x8236ADAC, air control: in the air with the weight button, no stunt
        // and no turn intent left from the ground (gyro +16, +18), Yaw, Pitch or Roll above nothing -
        // once past a tenth of the timer's +84 (0x827D7554: the main loop's global time scale, never
        // under 0.1) - the stick turns the bike: -(Yaw / scale x stick X) about the body's up and
        // Pitch / scale x stick Y about its right, as turning accelerations (body vfunc 30). Roll only
        // takes part in the test. Chassis +576 bit 0 (a script's mask, sub_822ADF90) is never set in
        // the port. Before the tenth the original writes gyro +104, which nothing reads.)
        if air > 0.0 && btn && vs.bg.intent == 0 && vs.bg.state == 0 && (gt.yaw > 0.0 || gt.pitch > 0.0 || gt.roll > 0.0) {
            let scale = f32::from_bits(TIME_SCALE84.load(std::sync::atomic::Ordering::Relaxed));
            if air > scale * 0.1 {
                let inv = 1.0 / if scale - 0.1 >= 0.0 { scale } else { 0.1 };
                calls.push((543, Vec3::ZERO, rows[1] * (inertia.y * -((inv * gt.yaw) * stick_x))));
                calls.push((544, Vec3::ZERO, rows[0] * (inertia.x * ((gt.pitch * inv) * stick_y))));
            }
        }
        // (RB6, section 13, 0x8236ADB0..0x8236AFD4, the reverse drive - the engine never drives a motorcycle
        // backwards, the gyro pushes it: in reverse gear, throttle (chassis +136) above nothing, the handbrake
        // off, air time under 0.25 s, the rear wheel with a contact record: the speed along the bike is
        // driven towards -RevGearMPH x 0.447 - the error kept within ReverseLimit - by a force
        // -(ReverseAcc x mass x error) along the bike's back (a tenth of it while mcCarStuck is busy),
        // at the point halfway between the body's place and the chassis's first axle's (its matrix is the
        // chassis's: the model's origin) - body vfunc 31, 0x8259E0F0: the torque's two products go into
        // the sum one after the other. V: sf.2 k 180.. . MCLA_DBG_RB6: none.)
        if vs.gear < 0 && throttle_in > 0.0 && hand == 0.0 && vs.air584 < 0.25 && wr.record && !dbg_flag("MCLA_DBG_RB6") {
            let e = ex::fms(-t.trans.s.rev_gear_mph, f32::from_bits(0x3EE4_DD2F), fwd_speed);
            let lim = gt.reverse_limit;
            let e = if e - -lim >= 0.0 { e } else { -lim };
            let e = if e - lim >= 0.0 { lim } else { e };
            let mut f = rows[2] * -((gt.reverse_acc * mass) * e);
            if vs.stuck > 0 {
                f = f * 0.1;
            }
            // (body_rows[3]: the chassis matrix's place, chassis +288 - the model's origin)
            let at = (body_at + body_rows[3]) * 0.5;
            let d = at - body_at;
            calls.push((545, f, Vec3::new(d.y * f.z, d.z * f.x, d.x * f.y)));
            calls.push((545, Vec3::ZERO, -Vec3::new(d.z * f.y, d.x * f.z, d.y * f.x)));
        }
        // (the port's flag for "on one wheel": both wheels are then set up for the whole weight)
        vs.wheelie = match vs.bg.state { 1 => 1, -1 => 2, 0 => 0, _ => vs.wheelie };
        if spin_set {
            vs.bike_spin = Some(w);
        }
    } }
    let exact_gyro = exact_sums && !dbg_flag("MCLA_DBG_OLDGYRO");
    if exact_gyro {
        // ---- mcCarGyro, 0x82366B3C..0x82366D1C, operation for operation ----
        use super::exact::{approach, ease, fma, unit_or_zero};
        if t.gyro.drift_thrust > 0.0 && vs.burn == 0.0 {
            let hb: f32 = if input.handbrake { 1.0 } else { 0.0 };
            if hb > vs.drift_level {
                vs.drift_level = hb;
            } else if hb < vs.drift_level {
                approach(&mut vs.drift_level, hb, t.gyro.drift_decay, dt);
            }
            if vs.drift_level > 0.0 {
                let vn = unit_or_zero(st.vel);
                let side = fma(vn.x, body_rows[0].x, fma(body_rows[0].y, vn.y, body_rows[0].z * vn.z)).abs();
                let side = if side - 1.0 >= 0.0 { 1.0 } else { side };
                let first = if vs.drive == 0 { 2 } else { 0 };
                let mut k = (grounded_count as f32 / wheel_count) * ((t.gyro.drift_thrust * shaft_speed_then) * vs.drift_level);
                k = vs.wheel_grip[first] * ((ease(side) * k) * throttle_in);
                if vs.gear > 0 && vs.gear as usize == vt.ratios.len() - 1 {
                    k *= 0.5;
                }
                calls.push((510, (body_rows[2] * -k) * mass, Vec3::ZERO));
            }
        }
    } else if t.gyro.drift_thrust > 0.0 && vs.burn_state == 0 {
        let hb = if input.handbrake { 1.0 } else { 0.0 };
        if hb > vs.drift_level {
            vs.drift_level = hb;
        } else if hb < vs.drift_level {
            vs.drift_level = (vs.drift_level - t.gyro.drift_decay.max(0.0) * dt).max(hb);
        }
        let speed = st.vel.length();
        if vs.drift_level > 0.0 && speed > 0.0 {
            let side = (st.vel / speed).dot(right).abs().min(1.0);
            let ease = if side <= 0.0 { 0.0 } else if side >= 1.0 { 1.0 } else { (side * std::f32::consts::FRAC_PI_2).sin() };
            let first = if vs.drive == 0 { 2 } else { 0 };
            let mut k = t.gyro.drift_thrust * shaft_speed_then * vs.drift_level * (grounded_count as f32 / wheel_count) * ease * thr;
            // (the bikes' gyro, 0x8236970C, has neither of these two)
            if !vt.bike {
                k *= vs.wheel_grip[first];
            }
            if !vt.bike && vs.gear > 0 && vs.gear as usize == vt.ratios.len() - 1 {
                k *= 0.5;
            }
            force += fwd * (k * mass);
        }
    }
    // the drift gyro (mcCarGyro 0x82366950): on the handbrake, off the foot brake, faster than
    // 10 m/s, the car is turned the way it is steered - a torque about the ground under the
    // rear wheels like the turn's, Drift times the mass, the steering and the speed (eased in
    // up to 25 m/s: v^2 / 25). Recorded on the R8: +4,000 N m at 0.8 of lock and 13 m/s.
    let exact_drift = exact_gyro && !dbg_flag("MCLA_DBG_OLDDRIFT");
    if !exact_drift && input.handbrake && brake_in < 0.01 && t.gyro.drift > 0.0 && grounded_count > 0 && vs.burn_state == 0 {
        let v = st.vel.dot(fwd);
        if v > 10.0 {
            let s = if v >= 25.0 { v } else { v * v * 0.04 };
            vs.turn_force += -vs.steer * t.gyro.drift * mass * s * (grounded_count as f32 / wheel_count);
        }
    }
    vs.last_gyro = Vec3::ZERO;
    // the turn gyro (vehGyro 0x82563018): a torque about the ground's normal under the rear
    // wheels, and with it the force that leaves the middle of the rear axle where it is - the
    // car is turned about its rear axle, not about its middle
    if vs.turn_force != 0.0 && grounded_count > 0 {
        let n = (st.wheels[2].normal + st.wheels[3].normal).try_normalize().unwrap_or(up);
        let t_gyro = n * vs.turn_force;
        torque += t_gyro;
        let spin_up = rm * ((rm.transpose() * t_gyro) / inertia);
        let rear_mid = rm * ((p.wheel_pos[2] + p.wheel_pos[3]) * 0.5 - pivot_local);
        force -= spin_up.cross(rear_mid) * mass;
        vs.last_gyro = t_gyro;
    }

    // ---- the turn in the original's own operations (mcCarGyro 0x82367BCC..0x823681A8 and
    // vehGyro 0x82563018): about the mean of the two rear wheels' normals - the wheel's own
    // up where it carries nothing - the turning rate is moved towards what the steering asks;
    // the torque goes through the car's frame and back (and loses its last digits to the
    // body's place on the way), and with it the force that leaves the middle of the rear
    // contacts where it is. On the handbrake (the player's car) the older code above stands in. ----
    let input_computer = input.ai || REPLAY_AI.load(std::sync::atomic::Ordering::Relaxed);
    if exact_gyro && (t.gyro.turn > 0.0 || exact_drift) && (input_computer || !input.handbrake || exact_drift) && !no_gyro() {
        use super::exact::{dot3, fma, fnma, gyro_turn};
        let gt = &t.gyro;
        let (v, w) = (st.vel, st.ang_vel);
        let hand: f32 = if input.handbrake && !input_computer && exact_drift { 1.0 } else { 0.0 };
        let seen = |j: usize| -> (Vec3, Vec3) {
            match (contacts[j].filter(|c| c.contact.load != 0.0), rebuilt[j]) {
                (Some(c), _) => (c.normal, c.point),
                (None, Some(m)) => (m[1], m[3] - m[1] * if j < 2 { t.wheel_front.radius } else { t.wheel_back.radius }),
                (None, None) => (body_rows[1], body_rows[3]),
            }
        };
        let ((nl, pl), (nr, pr), (_, pfl), (_, pfr)) = (seen(2), seen(3), seen(0), seen(1));
        let sum = (nr + nl) * 0.5;
        let rear = (pr + pl) * 0.5;
        let front = (pfr + pfl) * 0.5;
        let l2 = dot3(sum, sum);
        let mut e = 1.0 / l2.sqrt();
        if e == f32::INFINITY {
            e = 0.0;
        }
        let n = sum * ((-(l2 * (e * e) - 1.0)) * (0.5 * e) + e);
        // (gyro +60, the share of the way to the front axle the car is turned about: 0)
        let offset = ((front - rear) * 0.0 + rear) - body_at;
        let yaw = fma(w.x, n.x, fma(w.y, n.y, w.z * n.z));
        let mut size = 0.0;
        let back = body_rows[2];
        let along = fnma(v.x, back.x, fma(back.z, v.z, back.y * v.y));
        let inverse = Vec3::new(super::exact::recip(inertia.x), super::exact::recip(inertia.y), super::exact::recip(inertia.z));
        // ---- the handbrake's drift (0x8236694C, the player's car, ahead of DriftThrust): on the
        // handbrake, off the brake, faster than 10 m/s forwards, a torque about the same normal
        // and point as the turn's - Drift times the mass, times the pad's steering of the frame
        // (the input object's, not the chassis's scaled value) and the speed (v^2 / 25 under
        // 25 m/s), by the share of the wheels down. MCLA_DBG_OLDDRIFT: the port's older code. ----
        // ---- the burnout's turn (0x82366558..0x82366944, ahead of the drift): BurnoutTurn > 0, on the
        // handbrake, with charge, every wheel down. The steering - times the root of the engine's speed
        // over its speed of best power, times the charge, twice that where the front axle is the held
        // one - asks for a turning rate of BurnoutLimit times it, approached at BurnoutRate; the torque
        // is (that rate + the car's own about the held wheels' ground normal) x the first driven
        // wheel's grip x BurnoutTurn x the mass, about the held wheel on the side steered to (both,
        // steering straight) through vehGyro's routine; BurnoutLean rolls the car by the same number;
        // and the held wheel's sideways speed is braked: the acceleration that would stop it over
        // half of (1 - BurnoutGrip) seconds, times the charge and the mass, along the contact's
        // sideways. The two held wheels' slide (+520) is zeroed. ----
        if !dbg_flag("MCLA_DBG_Y6") && gt.burnout_turn > 0.0 && hand > 0.0 && vs.burn > 0.0 {
            if grounded_count as f32 == wheel_count {
                // (engine +72: the speed of its best power, as vehEngine::Init finds it on the curve)
                let best = vt.torque_curve.as_ref().map_or(t.engine.s.opt_rpm * 0.104_719_76, |c| c.peak().0);
                let root = ((vs.engine_w / best) as f64).powf(0.5) as f32;
                let mut k = (vs.steer * root) * vs.burn;
                let way: f32 = if vs.gear < 0 { -1.0 } else if vs.gear == 0 { 0.0 } else { 1.0 };
                let first = if vs.drive == 0 { 2 } else { 0 };
                let hold = vs.wheel_grip[first] * if way < 0.0 { gt.burnout_turn_rev } else { gt.burnout_turn };
                // (each held wheel's record as it stands: normal +336, sideways +320, sideways speed +292, point +368)
                let of = |j: usize| -> (Vec3, Vec3, f32, Vec3) {
                    let w = &vs.wheels[j];
                    (w.frame[1], w.frame[0], w.velocity_lat, w.frame[3])
                };
                let front_held = !matches!(t.drivetrain_type, 1 | 3);
                let (a, b) = if front_held { (0usize, 1usize) } else { (2, 3) };
                if front_held {
                    k *= 2.0;
                }
                let (da, db) = (of(a), of(b));
                let n = (db.0 + da.0) * 0.5;
                let one = if front_held { if k > 0.0 { Some(db) } else if k < 0.0 { Some(da) } else { None } } else if k < 0.0 { Some(db) } else if k > 0.0 { Some(da) } else { None };
                let (side, v_side, at) = match one {
                    Some(d) => (d.1, d.2, d.3),
                    None => ((db.1 + da.1) * 0.5, (db.2 + da.2) * 0.5, (db.3 + da.3) * 0.5),
                };
                vs.wheels[a].slide = 0.0;
                vs.wheels[b].slide = 0.0;
                let arm = at - body_at;
                let turning = fnma(w.x, n.x, fma(w.y, n.y, w.z * n.z));
                let target = (way * gt.burnout_limit) * k;
                let sign = |x: f32| if x >= 0.0 { if -x >= 0.0 { 0.0 } else { 1.0 } } else { -1.0 };
                if sign(target) != sign(vs.burn_yaw) {
                    vs.burn_yaw = 0.0;
                }
                super::exact::approach(&mut vs.burn_yaw, target, gt.burnout_rate, dt);
                let err = vs.burn_yaw - turning;
                let rows = [body_rows[0], body_rows[1], body_rows[2]];
                let (tq, f) = gyro_turn(n * -((err * hold) * mass), arm, &[rows[0], rows[1], rows[2], chassis_at], &rows, inverse, mass);
                // (0x82563238, with the normal handed in - here only: what of the force points along it is left out)
                let lift = fma(f.x, n.x, fma(n.y, f.y, n.z * f.z));
                let f = if lift > 0.0 { f - n * lift } else { f };
                calls.push((490, Vec3::ZERO, tq));
                calls.push((491, f, Vec3::ZERO));
                if gt.burnout_lean != 0.0 {
                    calls.push((492, Vec3::ZERO, rows[2] * (inertia.z * (gt.burnout_lean * k))));
                }
                let stop = super::exact::stop_rate(1.0 - gt.burnout_grip, dt * v_side, v_side, 0.0, 0.0);
                let push = vs.burn * stop;
                calls.push((493, side * (push * mass), Vec3::ZERO));
            }
        } else
        if hand > 0.0 && gt.drift > 0.0 && brake_in == 0.0 && along > 10.0 && vs.burn == 0.0 {
            let speed = if along < 25.0 { (along * along) * f32::from_bits(0x3d23_d70a) } else { along };
            let pad = if (REPLAY_STEER.load(std::sync::atomic::Ordering::Relaxed) && !dbg_flag("MCLA_DBG_OLDSTEER")) || (!vt.bike && !dbg_flag("MCLA_DBG_Z2")) { vs.pad_steer } else { vs.steer };
            let k = (((grounded_count as f32 / wheel_count) * pad) * speed) * hand;
            let (tq, f) = gyro_turn(n * -((gt.drift * mass) * k), rear - body_at, &[body_rows[0], body_rows[1], body_rows[2], chassis_at], &[body_rows[0], body_rows[1], body_rows[2]], inverse, mass);
            calls.push((500, Vec3::ZERO, tq));
            calls.push((501, f, Vec3::ZERO));
        }
        // ---- the launch's lift (0x82367894..0x82367BB8, gyro flag 0x100000; ahead of the turn): with
        // charge and the handbrake off, the car's pitch is driven to the lean its acceleration asks
        // for - a torque about the rear axle's line (from the left rear contact to the right; one
        // wheel's sideways if only one has a record), at the rear contacts' middle, through vehGyro's
        // routine: Wheelie x error + sqrt(Wheelie) x WheelieDamp x 2 x its rate, times the mass, the
        // error clamped to WheelieLimit (the gain over it). error = (the car's length's height - the
        // rear contact frames' mean) - sin(0.25 x atan2(a, g) x WheelieAngle x throttle - 0.75 x
        // atan2(speed^2, g) x WheelieAngle x throttle), a = the tick's force sum so far over the mass,
        // along (the axle's line x the car's up), taken only when it is negative (forwards).
        // Otherwise gyro +80 is the car's pitch, asin of its length's height. ----
        if !dbg_flag("MCLA_DBG_Y6") {
            // (RB6: a motorcycle's gyro, mcBikeGyro, has no such lift - se.1's launch tick, k 120, has no
            // torque from its gyro at all)
            if vs.burn > 0.0 && hand == 0.0 && !(vt.bike && !dbg_flag("MCLA_DBG_RB6")) {
                let of = |j: usize| -> Option<(Vec3, Vec3, f32)> {
                    let w = &vs.wheels[j];
                    w.record.then(|| (w.frame[3], w.frame[0], w.frame[2].y))
                };
                let (rl, rr) = (of(2), of(3));
                let (line, slope) = match (rl, rr) {
                    (Some(l), Some(r)) => {
                        let d = r.0 - l.0;
                        (super::exact::unit_or_zero(d), (r.2 + l.2) * 0.5)
                    }
                    (Some(l), None) => (l.1, l.2),
                    (None, Some(r)) => (r.1, r.2),
                    (None, None) => (Vec3::ZERO, 0.0),
                };
                let up1 = body_rows[1];
                // (the axle's line x the car's up, by components as the vector unit has it)
                let b = Vec3::new(line.y * up1.z - line.z * up1.y, line.z * up1.x - line.x * up1.z, line.x * up1.y - line.y * up1.x);
                // (the point: the two rear wheels' record points' middle, records or not)
                let pt = |j: usize| vs.wheels[j].frame[3];
                let at = (pt(2) + pt(3)) * 0.5;
                lift = Some((line, b, slope, at));
            } else {
                vs.lift_err = super::exact::guest_asin(body_rows[2].y as f64) as f32;
            }
        }
        if t.gyro.turn > 0.0 {
        if hand > 0.0 && brake_in > 0.0 && vs.burn == 0.0 {
            // (0x82367CE0, brake and handbrake together: Spin - the car is turned by its steering alone)
            let mut k = vs.steer;
            if along.abs() < 25.0 {
                k = (along.abs() * k) * f32::from_bits(0x3d23_d70a);
            }
            size = -((((grounded_count as f32 / wheel_count) * k) * gt.spin) * inertia.y);
            vs.turn_err = 0.0;
        } else if hand > 0.0 && (vs.burn == 0.0 || throttle_in == 0.0) {
            // (0x82367DCC: on the handbrake the turn stands down - its last error forgotten -
            // and what goes through vehGyro's routine is a torque of nothing, which is not
            // quite nothing once it has been through the body's place.)
            vs.turn_err = 0.0;
            // (RB5, 0x82367DF0..0x82367EA4, RevSpin: in reverse gear (transmission +20 == 0), backing up
            // faster than 5 m/s (0x820390F8), the handbrake turns the car by its steering alone, the other
            // way round from Spin: -steering (chassis +132), times |speed| x 0.1 under 10 m/s, times
            // RevSpin x the inertia about the up, by the wheels on the ground. V: sc.0 k 290..360 in the
            // one-step replay. MCLA_DBG_RB5: nothing, as before.)
            if vs.gear < 0 && along < -5.0 && !dbg_flag("MCLA_DBG_RB5") {
                let mut k = -vs.steer;
                if along.abs() < 10.0 {
                    k = (along.abs() * k) * f32::from_bits(0x3dcc_cccd);
                }
                size = -((((grounded_count as f32 / wheel_count) * k) * gt.rev_spin) * inertia.y);
            }
        } else if gt.turn_factor > 0.0 {
            let fast = along.abs();
            let rest_z = |j: usize| match vs.rest_exact { Some(r) => r[j][2], None => p.wheel_pos[j].z };
            let wheelbase = rest_z(2) - rest_z(0);
            let wf = &t.wheel_front;
            let ideal = (((((1.0 - wf.optimum_slip) * wf.steering_limit) * vs.steer) * along) * (-1.0 / wheelbase)) * gt.turn_bias;
            let ideal = ideal * fma((throttle_in - 0.5) * 2.0, gt.turn_oversteer, 1.0);
            let mut x = 1.0 - vs.melt;
            if fast < t.s.sss_threshold_min {
                x = ((1.0 / t.s.sss_threshold_min) * x) * fast;
            }
            if vs.burn > x {
                x = vs.burn;
            }
            let x = if x >= 0.0 { x } else { 0.0 };
            let x = if x - 1.0 >= 0.0 { 1.0 } else { x };
            let blend = if input_computer && gyro_full() { 1.0 } else { gt.turn_factor * x };
            let aim = fma(1.0 - blend, yaw, blend * ideal);
            let mut err = aim - yaw;
            let d = if vs.turn_err != 0.0 { (err - vs.turn_err) * (1.0 / dt) } else { 0.0 };
            vs.turn_err = err;
            let mut gain = gt.turn;
            if gt.turn_limit > 0.0 {
                let low = -gt.turn_limit;
                let held = if err - low >= 0.0 { err } else { low };
                err = if held - gt.turn_limit >= 0.0 { gt.turn_limit } else { held };
                gain /= gt.turn_limit;
            }
            let mut out = gain * err + ((gain.sqrt() * gt.turn_damp) * 2.0) * d;
            if fast < 10.0 {
                out = (out * fast) * 0.1;
            }
            if dbg_flag("MCLA_DBG_TRACE") { eprintln!("TURN yaw {yaw:?} ideal {ideal:?} x {x:?} blend {blend:?} err {err:?} d {d:?} gain {gain:?} out {out:?} fast {fast:?} grounded {grounded_count} grip {:?} thr {throttle_in:?} steer {:?} burn {:?} melt {:?}", vs.wheel_grip[0], vs.steer, vs.burn, vs.melt); }
            size = (((grounded_count as f32 / wheel_count) * out) * vs.wheel_grip[0]) * mass;
        }
        let rows = [body_rows[0], body_rows[1], body_rows[2]];
        let (tq, f) = gyro_turn(n * size, offset, &[rows[0], rows[1], rows[2], chassis_at], &rows, inverse, mass);
        calls.push((530, Vec3::ZERO, tq));
        calls.push((531, f, Vec3::ZERO));
        vs.last_gyro = tq;
        }
    }

    // ---- aerodynamics: drag and downforce grow with the square of the share of top speed ----
    let top = vt.top_speed.max(10.0);
    if exact_sums {
        // ---- vehAero::Update (0x8256ADA8) with mcCarAero's numbers (0x82380750, 0x82380838),
        // operation for operation. It runs ahead of the wheels. The body's turning as it came
        // into the tick, in the car's frame; about each axis an acceleration
        // -(C sign(w) + V w + V2 |w| w), no more than stops it within the tick, times |w|
        // under 1 rad/s, times the factor; then the downforce along the car's up and the
        // drag against its velocity, both by the square of the speed over the top speed -
        // which the original takes as HighGearMPH times 0.447 ----
        use super::exact::fnms;
        let (w, v) = (st.ang_vel, st.vel);
        let rows = [body_rows[0], body_rows[1], body_rows[2]];
        // (the engine's rocking of the body comes first of all: the engine's update is ahead of the air's)
        if let (Some(dir), Some(k)) = (launch_dir, vs.by.launch) {
            calls.push((50, (dir * k) * mass, Vec3::ZERO));
        }
        if engine_roll != 0.0 {
            calls.push((100, Vec3::ZERO, rows[2] * (inertia.z * engine_roll)));
        }
        let local = [
            fma32(rows[0].x, w.x, fma32(rows[0].y, w.y, rows[0].z * w.z)),
            fma32(w.x, rows[1].x, fma32(rows[1].y, w.y, rows[1].z * w.z)),
            fma32(w.x, rows[2].x, fma32(rows[2].y, w.y, rows[2].z * w.z)),
        ];
        let a = &t.aero;
        let tuned = [(a.ang_c_damp_x, a.ang_vel_damp_x, a.ang_vel2_damp_x), (a.ang_c_damp_y, a.ang_vel_damp_y, a.ang_vel2_damp_y), (a.ang_c_damp_z, a.ang_vel_damp_z, a.ang_vel2_damp_z)];
        let per = 1.0 / dt;
        let factor = vs.aero_damp;
        if factor > 0.0 && !dbg_flag("MCLA_DBG_NODAMP") {
            for axis in 0..3 {
                let x = local[axis];
                if !(x.abs() > 0.001) {
                    continue;
                }
                let (c0, c1, c2) = tuned[axis];
                let mut acc = fnms(x.abs() * x, c2, fnms(x, c1, -(c0 * super::exact::sign(x))));
                if acc.abs() * dt > x.abs() {
                    acc = -(per * x);
                }
                if x.abs() < 1.0 {
                    acc *= x.abs();
                }
                if factor != 1.0 {
                    acc = factor * acc;
                }
                if dbg_flag("MCLA_DBG_CALLS") {
                    eprintln!("AEROT axis {axis} w {x:?} tune {:?} factor {factor:?} torque {:?}", tuned[axis], inertia[axis] * acc);
                }
                calls.push((200, Vec3::ZERO, rows[axis] * (inertia[axis] * acc)));
            }
        }
        let speed = fma32(v.z, v.z, fma32(v.x, v.x, v.y * v.y)).sqrt();
        let top = t.trans.s.high_gear_mph * f32::from_bits(0x3ee4dd2f);
        let over = 1.0 / ((top as f64 * top as f64) as f32);
        // (mcCarSim::Update 0x82330024: the aero's byte +40 picks DragMin. Four wheels: a slipstream target.
        // Two wheels (0x8233005C): the rider's stick, input +32, under -0.5 with an air time under 0.5 -
        // the stoppie's lean; the ZX-14's DragMin is 3 against DragMax 2. MCLA_DBG_A1: cars' rule for all.)
        let other = if vt.bike && !dbg_flag("MCLA_DBG_A1") { input.lean < -0.5 && vs.air584_before < 0.5 } else { input.draft };
        // (R2: vehAero's drag, 0x8256B14C, asks only for a speed above 1 m/s - nothing about the wheels: V se.0
        // 168.., a quarter second after the last wheel left the ground. MCLA_DBG_R2: none in the air, as before.)
        let drag = if st.airborne_time > 0.25 && dbg_flag("MCLA_DBG_R2") { 0.0 } else if other { t.aero.drag_min * over } else { t.aero.drag_max * over };
        // (0x8256B070: the force along the car's up is given whether or not a wheel is down, less
        // the share of the wheels the last collision handed to the solver - bottomed out, byte
        // +530, 0x8255AD28. MCLA_DBG_OLDDOWN: none with no wheel down, as before.)
        let old_down = dbg_flag("MCLA_DBG_OLDDOWN");
        if speed > 1.5 && (grounded_count > 0 || !old_down) {
            let mut down = rows[1] * -(((speed * speed) * mass) * (t.aero.down * over));
            if !old_down && wheels_in > 0 && bottomed_in != 0 {
                down *= 1.0 - bottomed_in as f32 / wheels_in as f32;
            }
            if factor < 1.0 {
                down *= factor;
            }
            calls.push((201, down, Vec3::ZERO));
        }
        if speed > 1.0 {
            calls.push((202, v * -((mass * drag) * speed), Vec3::ZERO));
        }
    } else if speed > 1.0 {
        // (mcCarAero: DragMax on the ground whatever the pedal does, DragMin in a slipstream,
        // nothing once the car has been in the air for a quarter of a second)
        // (the switch to DragMin: a car in a slipstream; a bike whose rider leans back - the
        // bikes' DragMin is the larger of their two values)
        let other = if vt.bike { input.lean > 0.5 } else { input.draft };
        let drag = if st.airborne_time > 0.25 { 0.0 } else if other { t.aero.drag_min.max(0.0) } else { t.aero.drag_max.max(0.0) };
        force -= st.vel * (speed * mass * drag / (top * top));
        if speed > 1.5 && grounded_count > 0 {
            force -= up * (speed * speed * mass * t.aero.down.max(0.0) / (top * top));
        }
    }

    // Contact/drivetrain/tyre math runs once per 60 Hz simulation frame.
    // Preserve the existing smaller motion/collision steps until the original
    // chassis constraint solver has been ported as well.
    // The change of velocity over the tick (the body's 0x8259EC10): with the suspension's damping
    // D = sum s [n; r x n][n; r x n]^T,  (diag(m, I) + dt D) [dv; dw] = dt [F; T].
    // (vehAero, 0x8256ADA8: the air's damping of the body's turning is a torque like any
    // other, worked out once a tick from the turning the body had BEFORE this tick's forces:
    // about each of the car's axes an acceleration -(C sign(w) + V w + V2 |w| w), no more than
    // stops it within the tick, times |w| under 1 rad/s, times its moment of inertia. Checked
    // against the recorded torque sums of an R8's launch to 0.1 N m: -16.2, -109.0, -209.8.)
    let tick_damping = !dbg_flag("MCLA_DBG_NODAMP") && !dbg_flag("MCLA_DBG_SUBDAMP");
    if exact_sums {
        // ---- vehAxle::Update (0x825681E0), operation for operation: where an axle's wheels
        // are pressed in more than a centimetre apart and the one pressed in more has a
        // contact, a turning acceleration about the car's length. Front axle, then rear. ----
        if !dbg_flag("MCLA_DBG_NOROLLBAR") {
            for (a, (l, r)) in [(0usize, 1usize), (2, 3)].into_iter().enumerate() {
                let (coef, wt) = if a == 0 { (t.axle_front, &t.wheel_front) } else { (t.axle_back, &t.wheel_back) };
                if coef[0] == 0.0 || !coef[0].is_finite() || coef[0] > 100.0 || coef[0] < 0.0 {
                    continue;
                }
                let disp = |j: usize| super::exact::axle_disp(vs.wheels[j].comp, wt.travel_extent, wt.travel_limit, offsets[j]);
                let d = disp(l) - disp(r);
                let rates = vs.wheels[l].comp_rate - vs.wheels[r].comp_rate;
                if (d > 0.01 && st.wheels[l].grounded) || (d < -0.01 && st.wheels[r].grounded) {
                    let damp = (coef[0].sqrt() * coef[1]) * 2.0;
                    let acc = super::exact::fnma(d / (wt.suspension_extent + wt.suspension_limit), coef[0], damp * rates);
                    if dbg_flag("MCLA_DBG_CALLS") {
                        eprintln!("AXLE {a} d {:?} rates {:?} ({:?} {:?}) coef {:?} damp {:?} span {:?} izz {:?} acc {:?}", d, rates, vs.wheels[l].comp_rate, vs.wheels[r].comp_rate, coef[0], damp, wt.suspension_extent + wt.suspension_limit, inertia.z, acc);
                    }
                    calls.push((if dbg_flag("MCLA_DBG_AXLEORDER") { 401 - a as u16 } else { 400 + a as u16 }, Vec3::ZERO, body_rows[2] * (inertia.z * acc)));
                }
            }
        }
    } else if tick_damping {
        let w = rm.transpose() * st.ang_vel;
        let a = &t.aero;
        let acc = |w: f32, c0: f32, c1: f32, c2: f32| -> f32 {
            let mut a = (-(c0 * w.signum()) - c1 * w - c2 * w.abs() * w) * vs.aero_damp;
            if a.abs() * dt > w.abs() {
                a = -w / dt;
            }
            if w.abs() < 1.0 {
                a *= w.abs();
            }
            a
        };
        let acc = Vec3::new(acc(w.x, a.ang_c_damp_x, a.ang_vel_damp_x, a.ang_vel2_damp_x), acc(w.y, a.ang_c_damp_y, a.ang_vel_damp_y, a.ang_vel2_damp_y), acc(w.z, a.ang_c_damp_z, a.ang_vel_damp_z, a.ang_vel2_damp_z));
        torque += rm * (inertia * acc);
        // (vehAxle::Update, 0x825681E0, the same way - a turning acceleration about the car's
        // length, once a tick: where an axle's wheels are pressed in more than a centimetre
        // apart and the one pressed in more has a contact, -(difference / (SuspensionLimit +
        // SuspensionExtent) * coefficient + 2 damping sqrt(coefficient) * (their rates'
        // difference)). Fitted on the torque sums of four recorded runs, S281 and R8: 1.89 and
        // 2.02 for the rates where 2 x 0.4 x sqrt(5.5) and sqrt(6.5) are 1.88 and 2.04; and
        // nothing in the ticks where the wheel pressed in more hung free.)
        if !dbg_flag("MCLA_DBG_NOROLLBAR") {
            for (a, (l, r)) in [(0usize, 1usize), (2, 3)].into_iter().enumerate() {
                let (coef, wt) = if a == 0 { (t.axle_front, &t.wheel_front) } else { (t.axle_back, &t.wheel_back) };
                if vt.bike || coef[0] <= 0.0 || !coef[0].is_finite() || coef[0] > 100.0 {
                    continue;
                }
                let (wl, wr) = (&vs.wheels[l], &vs.wheels[r]);
                let at = |w: &WheelSim| w.comp.clamp(-wt.travel_extent, wt.travel_limit);
                let dx = at(wl) - at(wr);
                if dx.abs() > 0.01 && st.wheels[if dx > 0.0 { l } else { r }].grounded {
                    let span = (wt.suspension_limit + wt.suspension_extent).max(0.05);
                    let damp = if coef[1].is_finite() && coef[1].abs() < 100.0 { 2.0 * coef[1] * coef[0].sqrt() } else { 0.0 };
                    let acc = -(dx / span * coef[0] + damp * (wl.comp_rate - wr.comp_rate));
                    torque += rm * Vec3::new(0.0, 0.0, inertia.z * acc);
                }
            }
        }
    }
    if dbg_flag("MCLA_DBG_TERMS") {
        eprintln!("FSUM local {:?} gravity local {:?}", rm.transpose() * force, rm.transpose() * Vec3::new(0.0, -g * mass, 0.0));
    }
    if exact_sums {
        // (what the code above still added in its own order - the gyro's forces and the
        // port's own extras - goes in after the axles as one; gravity last, 0x8259D7F0:
        // the world's gravity times the car's factor, times the mass)
        calls.push((520, force, torque));
        // (the port's own extras that act on the turning rate - levelling and steering in the
        // air, the two-wheel controller - as torques about the car's axes)
        if grounded_count == 0 {
            for (axis, acc) in [(2usize, vs.air_acc[0]), (1, vs.air_acc[1]), (0, vs.air_acc[2])] {
                if acc != 0.0 {
                    calls.push((525, Vec3::ZERO, body_rows[axis] * (inertia[axis] * acc)));
                }
            }
        }
        if vs.two_acc != 0.0 {
            calls.push((525, Vec3::ZERO, body_rows[2] * (inertia.z * vs.two_acc)));
        }
        calls.push((900, Vec3::new(0.0, -((9.8f32 * if t.s.bound_gravity > 0.1 { t.s.bound_gravity } else { 1.0 }) * mass), 0.0), Vec3::ZERO));
        if lift.is_some() {
            calls.push((527, Vec3::ZERO, Vec3::ZERO));
            calls.push((528, Vec3::ZERO, Vec3::ZERO));
        }
        calls.sort_by_key(|c| c.0);
        if let Some((line, b, slope, at)) = lift {
            // (the force sum as the gyro finds it there: everything ahead of it in the tick)
            let mut so_far = Vec3::ZERO;
            for (key, df, _) in calls.iter() {
                if *key >= 527 { break; }
                so_far = so_far + *df;
            }
            let gt = &t.gyro;
            let grav = -((if t.s.bound_gravity > 0.1 { t.s.bound_gravity } else { 1.0 }) * f32::from_bits(0xc11c_cccd));
            let a = (so_far + Vec3::ZERO) * (1.0f32 / mass);
            let along = fma32(a.z, b.z, fma32(a.x, b.x, a.y * b.y));
            let angle = gt.wheelie_angle * throttle_in;
            let fast = -((((speed_in * speed_in) as f64).atan2(grav as f64) as f32) * angle);
            let pull = if along < 0.0 { ((along as f64).atan2(grav as f64) as f32) * angle } else { 0.0 };
            let lean = super::exact::guest_sin(fma32(pull, 0.25, fast * 0.75) as f64) as f32;
            let err = (body_rows[2].y - slope) - lean;
            let rate = (err - vs.lift_err) * (1.0 / dt);
            vs.lift_err = err;
            let (mut gain, mut e) = (gt.wheelie, err);
            if gt.wheelie_limit > 0.0 {
                let low = -gt.wheelie_limit;
                let held = if e - low >= 0.0 { e } else { low };
                e = if held - gt.wheelie_limit >= 0.0 { gt.wheelie_limit } else { held };
                gain /= gt.wheelie_limit;
            }
            let out = fma32((gain.sqrt() * gt.wheelie_damp) * 2.0, rate, gain * e);
            let rows = [body_rows[0], body_rows[1], body_rows[2]];
            let inverse = Vec3::new(super::exact::recip(inertia.x), super::exact::recip(inertia.y), super::exact::recip(inertia.z));
            let (tq, f) = super::exact::gyro_turn(line * (mass * out), at - body_at, &[rows[0], rows[1], rows[2], chassis_at], &rows, inverse, mass);
            for c in calls.iter_mut() {
                if c.0 == 527 { c.2 = tq; }
                if c.0 == 528 { c.1 = f; }
            }
        }
        if dbg_flag("MCLA_DBG_CALLS") {
            for (key, df, dq) in &calls {
                eprintln!("CALL {key} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}", df.x.to_bits(), df.y.to_bits(), df.z.to_bits(), dq.x.to_bits(), dq.y.to_bits(), dq.z.to_bits());
            }
            eprintln!("CALL end");
        }
        let (mut f, mut q) = (Vec3::ZERO, Vec3::ZERO);
        let mut stage = 0;
        for (key, df, dq) in calls.iter().copied() {
            if key >= 490 && stage == 0 {
                vs.sums_gyro[0] = f;
                vs.sums_gyro[1] = q;
                stage = 1;
            }
            if key >= 900 && stage == 1 {
                // (a replay's: the recorded sums after the gyro in place of this port's)
                if let Some(given) = vs.gyro_sums_exact.take() {
                    f = given[0];
                    q = given[1];
                }
                vs.sums_gyro[2] = f;
                vs.sums_gyro[3] = q;
                stage = 2;
            }
            f = f + df;
            q = q + dq;
        }
        force = f;
        torque = q;
    }
    // (step 19f: a Pulse power-up, sub_82380418: Force x the archetype's mass and the world torque
    // x its inertia (component by component, as the original multiplies them) go into the body's
    // force and torque sums, +336 / +352, through the collider's slots 31 / 33 (0x8259E0F0,
    // 0x8259C0F0) - the sums the next tick integrates over its dt (0x8259BE30) and then clears.)
    if let Some((pf, pt)) = vs.pulse.take() {
        force += pf * mass;
        torque += pt * inertia;
    }
    vs.last_force = force;
    vs.last_torque = torque;
    let (dv, dw) = {
        let iw = rm * Mat3::from_diagonal(inertia) * rm.transpose();
        let mut a = [[0.0f64; 7]; 6];
        for r in 0..3 {
            a[r][r] = mass as f64;
            for c in 0..3 {
                a[3 + r][3 + c] = iw.col(c)[r] as f64;
            }
        }
        for (arm, n, s) in &implicit {
            let q = arm.cross(*n);
            let j = [n.x as f64, n.y as f64, n.z as f64, q.x as f64, q.y as f64, q.z as f64];
            for r in 0..6 {
                for c in 0..6 {
                    a[r][c] += dt as f64 * *s as f64 * j[r] * j[c];
                }
            }
        }
        let b = [force.x, force.y, force.z, torque.x, torque.y, torque.z];
        for r in 0..6 {
            a[r][6] = (b[r] * dt) as f64;
        }
        for col in 0..6 {
            let pivot = (col..6).max_by(|x, y| a[*x][col].abs().total_cmp(&a[*y][col].abs())).unwrap();
            a.swap(col, pivot);
            let d = a[col][col];
            if d.abs() < 1e-12 {
                continue;
            }
            for r in 0..6 {
                if r != col {
                    let f = a[r][col] / d;
                    for c in col..7 {
                        a[r][c] -= f * a[col][c];
                    }
                }
            }
        }
        let x: Vec<f32> = (0..6).map(|r| if a[r][r].abs() < 1e-12 { 0.0 } else { (a[r][6] / a[r][r]) as f32 }).collect();
        let dw = Vec3::new(x[3], x[4], x[5]);
        // (the integrator, 0x8259EC10, solves the turning by elimination - the same answer -
        // and then goes back for the velocity with the coupling's sign the other way:
        // (m + dt K) dv = dt F + dt sum(s n (arm x n . dw)), where the solve itself has it
        // minus. With the four wheels' damping in balance about the centre of mass nothing
        // shows; with a wheel hanging the body is given what looks like a push of 500-800 N
        // up through its centre for the tick. Recorded on every such tick of three cars: the
        // change of velocity is this to 0.01 N s.)
        let dv = if dbg_flag("MCLA_DBG_SYMSOLVE") || implicit.is_empty() {
            Vec3::new(x[0], x[1], x[2])
        } else {
            let mut k = Mat3::from_diagonal(Vec3::splat(mass));
            let mut rhs = force * dt;
            for (arm, n, s) in &implicit {
                let ds = dt * *s;
                k += Mat3::from_cols(*n * (ds * n.x), *n * (ds * n.y), *n * (ds * n.z));
                rhs += *n * (ds * arm.cross(*n).dot(dw));
            }
            k.inverse() * rhs
        };
        (dv, dw)
    };
    // (0x82336F8C: the frozen body - velocities, sums, impulses and push all nothing; its
    // wheels have run, and what they asked for is thrown away. Recorded: a car creeping at
    // 4 cm/s reads exactly nothing on the tick its count passes a second.)
    // (mcCarStuck, 0x823509F0, between the car's update and the physics step: a car whose
    // body has touched something and that, 0.3 s on, is still within 3.5 m of where it was,
    // under throttle in a driving gear with a wheel on the ground, is turned by hand - its
    // matrix, not its turning rate - by 1.05 rad/s times the pad's steering (input +24), until it is 5 m from
    // that place, the throttle is let go, or the handbrake is pulled. Recorded on a Range
    // Rover scraping a gate pillar at full lock: 37 ticks in which its heading turns
    // 0.95 rad/s faster than its angular velocity says.)
    // (the turn as an angle about the world's up, for the body's rows in the original's own
    // operations below; where the body is not kept so, the car's quaternion is turned)
    let mut stuck_turn: Option<f32> = None;
    if vs.stuck != 0 && !dbg_flag("MCLA_DBG_NOSTUCK") {
        let pedal = if vs.gear < 0 { input.brake } else { input.throttle };
        let far2 = {
            let d = vs.stuck_at - body_at;
            super::exact::fma(d.x, d.x, super::exact::fma(d.z, d.z, d.y * d.y))
        };
        if far2 > 25.0 && vs.stuck != 2 {
            vs.stuck = 0;
            vs.stuck_timer = 0.0;
        } else if vs.stuck == 1 {
            vs.stuck_timer += dt;
            if far2 <= 3.5 * 3.5 && vs.stuck_timer >= 0.3 && pedal > 0.0 && vs.gear >= 1 && st.wheels.iter().any(|w| w.grounded) {
                // (the turning begins with the next tick)
                vs.stuck = 2;
            } else if vs.stuck_timer >= 1.0 {
                vs.stuck = 0;
                vs.stuck_timer = 0.0;
            }
        } else if vs.stuck == 2 {
            if far2 > 25.0 || vs.gear == 0 || pedal <= 0.0 || input.handbrake {
                vs.stuck = 0;
                vs.stuck_timer = 0.0;
            } else {
                // (the steering of the input object, +24 - the pad's value of the frame, not the
                // chassis's product with the speed factor: 0x82350F50, V s_rtv s256 - s270)
                let rate = vs.pad_steer * f32::from_bits(0x3f86_6666);
                let rate = if vs.gear < 0 { -rate } else { rate };
                stuck_turn = Some(-(dt * rate));
            }
        }
    }
    if dbg_flag("MCLA_DBG_STUCK") {
        eprintln!("STUCK state {} timer {:.3} far {:.2} steer {:e} turn {:?} cg {:.4} walls {}", vs.stuck, vs.stuck_timer, (vs.stuck_at - (st.pos + rm * pivot_local)).length(), vs.steer, stuck_turn, vs.cg[0], super::hull::WALLS.with(|t| t.get()));
    }
    let frozen = vs.still_timer > 1.0;
    // (R4: and the sums a frozen tick leaves are nothing - what the recorder's hook at the integration reads)
    if frozen && exact_sums && !dbg_flag("MCLA_DBG_R4") {
        vs.last_force = Vec3::ZERO;
        vs.last_torque = Vec3::ZERO;
    }
    if let Some(angle) = stuck_turn.filter(|_| !(exact_sums && !dbg_flag("MCLA_DBG_OLDSTEP"))) {
        st.rot = (Quat::from_rotation_y(angle) * st.rot).normalize();
        stuck_turn = None;
    }
    let (dv, dw) = if frozen { (Vec3::ZERO, Vec3::ZERO) } else { (dv, dw) };
    if frozen {
        st.vel = Vec3::ZERO;
        st.ang_vel = Vec3::ZERO;
    }
    let dbg_yaw0 = st.ang_vel;
    let body_step = dt / 4.0;
    // ---- the body's step in the original's own operations (MCLA_DBG_OLDSTEP: the port's four
    // smaller steps). First the move, 0x8259F300: the place by the velocity the tick came in
    // with (0x8259E768), the rows turned by the turning rate it came in with times the step
    // (0x8259D030) and, every 33rd time, made square again (0x8259E2C0); the model's origin
    // from the place (0x8259D1E8). Then the integrator, 0x8259BE30: the sums times the step,
    // through the damping the wheels handed over (0x8259EC10), into the momenta; and the
    // velocities from those - the turning rate through the rows as they now stand. ----
    let exact_step = exact_sums && !dbg_flag("MCLA_DBG_OLDSTEP");
    // (whether the hull's contacts were answered by an impulse, and how many of them stand all but upright)
    let mut hull_touched: Option<(bool, u32)> = None;
    let (dv, dw) = if exact_step {
        use super::exact as ex;
        let rows0 = if st.rot == rot_in {
            [body_rows[0], body_rows[1], body_rows[2]]
        } else {
            let m = Mat3::from_quat(st.rot);
            [m.x_axis, m.y_axis, m.z_axis]
        };
        // (mcCarStuck's turn of the body's rows, 0x82350940 about the world's up)
        let rows0 = match stuck_turn.take() {
            Some(angle) => ex::leaned(&rows0, Vec3::Y, angle),
            None => rows0,
        };
        // (body +176: each moment's reciprocal as the vector unit's estimate and one step - which
        // is not always the divided 1 / I; the three recorded cars' nine values agree)
        let inverse = Vec3::new(ex::recip(inertia.x), ex::recip(inertia.y), ex::recip(inertia.z));
        let (v_in, w_in) = (st.vel, st.ang_vel);
        // (the momenta: the replay's, the last tick's, or made from the velocities)
        let momentum_carried = vs.momentum_exact.is_some() || kept_body.is_some();
        let (mut pm, mut lm, count) = match (vs.momentum_exact.take(), kept_body) {
            (Some(m), _) => m,
            (None, Some(b)) => (b.p, b.l, b.count),
            (None, None) => {
                let iw = ex::world_inertia(&rows0, inertia);
                (v_in * mass, Vec3::new(ex::dot3(Vec3::new(iw[0].x, iw[1].x, iw[2].x), w_in), ex::dot3(Vec3::new(iw[0].y, iw[1].y, iw[2].y), w_in), ex::dot3(Vec3::new(iw[0].z, iw[1].z, iw[2].z), w_in)), 0)
            }
        };
        let exact_collide = !dbg_flag("MCLA_DBG_OLDCOLLIDE") && p.hull.is_some();
        let _ = momentum_carried;
        // (Z3, a motorcycle: the turning rate its gyro set in the update is the body's from then on -
        // body vfunc 43, 0x8259E180: +288, and the angular momentum made from it through the
        // inertia in the world; and what the gyro queued in body +416 / +400 is applied at the
        // head of the step, 0x8259F300 -> 0x8259E790, before the move)
        // (R4: a frozen body, 0x8259BC48, has lost all of it - the rate the gyro set and what it queued: V se.0
        // 269, a motorcycle creeping at 4 cm/s as its still timer passes a second. MCLA_DBG_R4: kept.)
        if frozen && !dbg_flag("MCLA_DBG_R4") {
            vs.bike_spin = None;
            if vs.bike_pending.take().is_some() {
                vs.push_turn_left = Vec3::ZERO;
            }
        }
        let w_in = match vs.bike_spin.take() {
            Some(w) => {
                let iw = ex::world_inertia(&rows0, inertia);
                lm = Vec3::new(ex::dot3(Vec3::new(iw[0].x, iw[1].x, iw[2].x), w), ex::dot3(Vec3::new(iw[0].y, iw[1].y, iw[2].y), w), ex::dot3(Vec3::new(iw[0].z, iw[1].z, iw[2].z), w));
                w
            }
            None => w_in,
        };
        let (body_at, rows0, w_in) = match vs.bike_pending.take() {
            Some((turn, push)) => {
                vs.push_turn_left = if super::solver::turn_applies(turn) { Vec3::ZERO } else { turn };
                (body_at + push, super::solver::rows_pushed(&rows0, turn), w_in)
            }
            None => (body_at, rows0, w_in),
        };
        let mut at1 = v_in * dt + body_at;
        let mut rows1 = ex::rows_turned(&rows0, w_in * dt);
        // (the rows as the turn left them, before they are made square again)
        let mut rows_turned = rows1;
        let mut count = count + 1;
        if count > 32 {
            rows1 = ex::rows_squared(&rows1);
            count = 0;
        }
        let mut origin1 = ex::origin_of(at1, &rows1, pivot_local);
        if dbg_flag("MCLA_DBG_CALLS") {
            let h = |v: Vec3| format!("{:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
            eprintln!("STEP {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}", h(pm), h(lm), h(force), h(torque), h(damping.k[0]), h(damping.k[1]), h(damping.k[2]), h(damping.b[0]), h(damping.b[1]), h(damping.b[2]), h(damping.a[0]), h(damping.a[1]), h(damping.a[2]), h(rows1[0]), h(rows1[1]), h(rows1[2]), h(inertia));
        }
        // (P3: the rider comes off - the bit, and the damage object's timer at WipeoutDelay; the update of the
        // next tick is the first to find it.)
        // (R4: the timer run out, the bit goes, mcCar::Update 0x823363D0: the tyres' melt, the nitrous, mcCarStuck
        // and the gyro are made new (0x82380B38, 0x823457D8, 0x82350638, 0x82368780), a gearbox in reverse or
        // neutral goes to first (0x82561D90), and the motorcycle is stood up, 0x822A9930: its length along the
        // direction saved as the rider came off (chassis +608: the velocity then, a unit vector; under 1 m/s
        // the car's own forwards), level - X = unit(d x up), Z = unit(X x up) - at the body's place; a probe
        // of the level from a metre above that place to three below; on a hit the place is the hit, the up the
        // polygon's normal and the rows made square about it. That is the MODEL's matrix (0x8232C3D0 ->
        // 0x82588990: every manifold of the car emptied, the instance's matrix and its last one, the body's
        // place from it through the bound's offset, 0x8259D280); the velocities are nothing, the shaft and its
        // wheels stand, the gear is the one for no speed, the engine at its idle (0x8255A1B0); the still
        // timer and the air time start again. The tick's forces are integrated from there; the car's pairs
        // are made again only in the next step. MCLA_DBG_R4: only the bit goes, as before.)
        let mut stood = false;
        vs.set_down = false;
        if vt.bike && !bike_old(vt) && !dbg_flag("MCLA_DBG_P3") && vs.by.exact {
            match vs.wipe {
                None if body_rows[1].y <= 0.1 && !(vs.air584 > 0.0) => {
                    vs.wipe = Some((t.damage.wipeout_delay, 0));
                    // (chassis +112 = chassis +96: the centre of mass at ModelOffset's height on the model's origin)
                    vs.cg = [t.s.model_offset_y, 0.0];
                    // (0x8232C788: chassis +608)
                    let v = v_in;
                    let len = ex::fma(v.z, v.z, ex::fma(v.x, v.x, v.y * v.y)).sqrt();
                    vs.wipe_dir = if len > 1.0 { v * ex::recip(len) } else { Vec3::ZERO - body_rows[2] };
                }
                Some((timer, _)) if timer <= 0.0 => {
                    vs.wipe = None;
                    if !dbg_flag("MCLA_DBG_R4") {
                        vs.melt = 0.0;
                        vs.nitro_level = 0.0;
                        vs.nitro_burning = false;
                        vs.stuck = 0;
                        vs.stuck_timer = 0.0;
                        vs.bg = BikeGyro { level: vs.bg.level, share: t.gyro.lean_angle, ..BikeGyro::default() };
                        if vs.gear <= 0 {
                            let was = vs.gear;
                            vs.gear = 1;
                            vs.gear_change.start(was, 1, e.gcl);
                        }
                        vs.shift_timer = 0.0;
                        let (d, up) = (vs.wipe_dir, Vec3::Y);
                        let x = ex::unit_or_zero(ex::cross(d, up));
                        let z = ex::unit_or_zero(ex::cross(x, up));
                        let place = body_at;
                        let (mut m, mut hit) = ([x, up, z, place], false);
                        let (from, to) = (Vec3::new(place.x, place.y + 1.0, place.z), Vec3::new(place.x, place.y - 3.0, place.z));
                        if let Some((point, normal)) = level_probe(world, from, to) {
                            let unit_or = |v: Vec3, other: Vec3| if ex::dot3(v, v) > f32::from_bits(0x3727_c5ac) { ex::unit_or_zero(v) } else { other };
                            let y = unit_or(normal, Vec3::Y);
                            let x = unit_or(ex::cross(y, z), Vec3::X);
                            let z = unit_or(ex::cross(x, y), Vec3::Z);
                            m = [x, y, z, point];
                            hit = true;
                        }
                        if dbg_flag("MCLA_DBG_HIT") || dbg_flag("MCLA_DBG_CALLS") {
                            let h = |v: Vec3| format!("{:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
                            eprintln!("STOODUP hit {hit} d {} rows {} | {} | {} place {} from {}", h(d), h(m[0]), h(m[1]), h(m[2]), h(m[3]), h(place));
                        }
                        rows1 = [m[0], m[1], m[2]];
                        rows_turned = rows1;
                        at1 = super::loose::xf(&m, pivot_local);
                        origin1 = ex::origin_of(at1, &rows1, pivot_local);
                        pm = Vec3::ZERO;
                        lm = Vec3::ZERO;
                        vs.manifolds.clear();
                        vs.hull_kept.clear();
                        vs.hull_last = Some([rows1[0], rows1[1], rows1[2], origin1]);
                        vs.still_timer = 0.0;
                        vs.air584 = 0.0;
                        vs.air_time = 0.0;
                        // (0x8255A1B0 with no velocity: the shaft, its wheels, the gear, the engine)
                        vs.omega[0] = 0.0;
                        vs.diff[0] = 1.0;
                        vs.wheels[2].spin_rate = 0.0;
                        if vs.gear != 1 {
                            let was = vs.gear;
                            vs.gear = 1;
                            vs.shift_timer = 0.0;
                            vs.gear_change.start(was, 1, e.gcl);
                        }
                        vs.engine_w = e.idle_rpm * 0.104_719_76;
                        vs.aero_damp = 1.0;
                        stood = true;
                        vs.set_down = true;
                    }
                }
                _ => {}
            }
        }
        let (v_in, w_in) = if stood { (Vec3::ZERO, Vec3::ZERO) } else { (v_in, w_in) };
        let exact_collide = exact_collide && !stood;
        // ---- the body against the city (0x825CCE00, 0x82594068): what the hull touches where
        // the move has brought it, the contacts kept from earlier ticks, the car's callback,
        // and the velocity solve - whose impulses go into the momenta with the tick's forces ----
        let mut origin1 = origin1;
        let mut at1 = at1;
        let seen = [rows1[0], rows1[1], rows1[2], origin1];
        let mut solved_contacts: Option<(super::solver::Body, Vec<super::solver::Contact>, super::solver::Solved)> = None;
        let mut loose_island: Option<(super::solver::Body, Vec<super::solver::Contact>, super::loose::IslandOut)> = None;
        // (the body as the tick's solve starts from it: a scrape reads the car's velocities before the impulses)
        let mut solve_body: Option<super::solver::Body> = None;
        let mut loose_answered = false;
        // (the normals of the car's contacts with the city that the callbacks took from the solver - the wheels'
        // - which mcCarDamage's scrape normal still sums, 0x82343F8C: every contact of its run)
        let mut city_off_n = Vec3::ZERO;
        let (mut impulse, mut turning) = (Vec3::ZERO, Vec3::ZERO);
        let mut reordered: Option<Vec<usize>> = None;
        if exact_collide {
            let hull = p.hull.as_ref().unwrap();
            let last = vs.hull_last.unwrap_or([rows0[0], rows0[1], rows0[2], body_rows[3]]);
            // (stage P: the loose bodies' move, gravity and damping, 0x8259F300)
            if let Some(lw) = world.loose.as_ref().filter(|_| !dbg_flag("MCLA_DBG_P1")) {
                if !vs.loose.skip_pre {
                    vs.loose.pre_collide(lw, dt);
                }
            }
            let bbox = (hull.min - Vec3::splat(0.041), hull.max + Vec3::splat(0.041));
            if dbg_flag("MCLA_DBG_HIT") {
                static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                eprintln!("HULLTICK {}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
                if std::env::var_os("MCLA_DBG_HULLBOX").is_some() { eprintln!("HULLBOX min {:?} max {:?} verts {:?} parts {:?}", hull.min, hull.max, hull.verts, vs.wheels.map(|w| w.part)); }
            }
            // (which of the city's cells the car is handed, 0x825CD4E8: by the ball of the whole car's box -
            // the hull's and the wheels' - swept from the body's last matrix. The box's floor, 0.48 below the
            // model's origin, is what three live cars' composite bounds held; its rule is not decoded.)
            let mut car_box = bbox;
            let cell_ball = if !dbg_flag("MCLA_DBG_P2") {
                // (P2: the composite bound's box as the wheels' fit leaves it, 0x8255BB80's tail -> 0x8259ADA8 ->
                // 0x82599FB0: the union of every part's own box swept from its "last" matrix to its current one,
                // with every wheel part put at its place of rest less max(SuspensionExtent, 0.5) and its last
                // matrix at (SuspensionLimit + rest) + 0.1. A wheel bound's box is its own support along each
                // axis and the margin, 0x825D3808; the hull's its corners' box and 0.041. It stands as it is
                // between fits. A decision only: written plainly.)
                use super::gjk::Support;
                let (mut lo, mut hi) = bbox;
                for i in 0..4 {
                    if let (Some((front, radius, width, _, _)), Some((extent, limit)), Some(rest)) = (wheel_geo[i], wheel_reach[i], wheel_rest[i]) {
                        let core = super::gjk::WheelCore::of(&super::gjk::wheel_shape(front, radius, width), Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], radius, width);
                        let m = f32::from_bits(0x3d23_d70a);
                        let e = Vec3::new(core.support(Vec3::X).x + m, core.support(Vec3::Y).y + m, core.support(Vec3::Z).z + m);
                        let down = rest.y - if extent - 0.5 >= 0.0 { extent } else { 0.5 };
                        let up = (limit + rest.y) + 0.1;
                        // (R1: the part's own rows as the fit finds them. A car's wheel stands square in
                        // the car; a motorcycle's front wheel part has the fork's rake in its rows, and
                        // 0x82599FB0's two absolute-value passes - the last pose in the part's axes, then
                        // that box in the composite's - make its swept box larger than the true sweep:
                        // live ZX-14 min (-0.4270, -0.5947, -1.4288). MCLA_DBG_R1: rows taken as square.)
                        let rows = if dbg_flag("MCLA_DBG_R1") || !(i < 2) { super::exact::IDENTITY } else {
                            let f = super::exact::steer_frame(t.steer_axis.map_or(Vec3::Y, Vec3::from));
                            if t.steer_axis.is_some() && wheel_steered[i] { f } else { super::exact::IDENTITY }
                        };
                        if rows == super::exact::IDENTITY {
                            lo = lo.min(Vec3::new(rest.x - e.x, down - e.y, rest.z - e.z));
                            hi = hi.max(Vec3::new(rest.x + e.x, up + e.y, rest.z + e.z));
                        } else {
                            let (mp, lp) = (Vec3::new(rest.x, down, rest.z), Vec3::new(rest.x, up, rest.z));
                            let dp = lp - mp;
                            let r: [Vec3; 3] = std::array::from_fn(|j| Vec3::new(rows[0].dot(rows[j]), rows[1].dot(rows[j]), rows[2].dot(rows[j])));
                            let tr = Vec3::new(rows[0].dot(dp), rows[1].dot(dp), rows[2].dot(dp));
                            let e1 = r[0].abs() * e.x + r[1].abs() * e.y + r[2].abs() * e.z;
                            let (h1, l1) = (e.max(tr + e1), (-e).min(tr - e1));
                            let cs = (h1 + l1) * 0.5;
                            let hs = h1 - cs;
                            let c = rows[0] * cs.x + rows[1] * cs.y + rows[2] * cs.z + mp;
                            let ee = rows[0].abs() * hs.x + rows[1].abs() * hs.y + rows[2].abs() * hs.z;
                            lo = lo.min(c - ee);
                            hi = hi.max(ee + c);
                        }
                    }
                }
                car_box = (lo, hi);
                // (the deform pieces' places from this box, once (0x8235CBD0, `sub_8255B9A8(car + 8)`): V the R8's
                // window pieces at mid y 0.4007 of the box -0.4759 .. 1.2772, work/dmgmap)
                if !st.dmg.pieces_set {
                    st.dmg.set_up(p.hub_bones.unwrap_or(p.wheel_pos), lo, hi);
                }
                if dbg_flag("MCLA_DBG_HIT") {
                    eprintln!("CELLBOX lo {:?} hi {:?} ball {:?} geo {:?} rest {:?} reach {:?}", lo, hi, super::hull::swept_ball(&seen, &last, lo, hi), wheel_geo, wheel_rest, wheel_reach);
                }
                (!dbg_flag("MCLA_DBG_Y3")).then(|| super::hull::swept_ball(&seen, &last, lo, hi))
            } else {
                let (mut lo, mut hi) = bbox;
                for i in 0..4 {
                    if let (Some((_, radius, width, _, _)), Some(part)) = (wheel_geo[i], vs.wheels[i].part) {
                        let e = Vec3::new(width * 0.5 + 0.08, radius + 0.012, radius + 0.012);
                        let ext = part[0].abs() * e.x + part[1].abs() * e.y + part[2].abs() * e.z;
                        lo = lo.min(part[3] - ext);
                        hi = hi.max(part[3] + ext);
                    }
                }
                lo.y = lo.y.min(-0.48);
                (!dbg_flag("MCLA_DBG_Y3")).then(|| super::hull::swept_ball(&seen, &last, lo, hi))
            };
            vs.cell_ball = cell_ball;
            vs.last_seen = Some(last);
            // (R3: a motorcycle whose thrown rider is in its island - a jointed body: every solve of the tick
            // sweeps eight times, not four. Whether he is, a replay is told. MCLA_DBG_R3: four.)
            super::solver::SWEEPS_NOW.store(if REPLAY_JOINTED.load(std::sync::atomic::Ordering::Relaxed) && !dbg_flag("MCLA_DBG_R3") { super::solver::SWEEPS_JOINTED } else { 0 }, std::sync::atomic::Ordering::Relaxed);
            // (R3: and the island's mean elasticity, island +40, is over ALL its enabled contacts - the rider's
            // own against the ground count in it)
            let r3 = !dbg_flag("MCLA_DBG_R3");
            super::solver::ISLAND_OTHERS.store(if r3 { REPLAY_OTHERS.load(std::sync::atomic::Ordering::Relaxed) as usize } else { 0 }, std::sync::atomic::Ordering::Relaxed);
            super::solver::ISLAND_OTHERS_E.store(REPLAY_OTHERS_E.load(std::sync::atomic::Ordering::Relaxed), std::sync::atomic::Ordering::Relaxed);
            super::hull::collide(hull, bbox, &seen, &last, world, cell_ball, &mut vs.manifolds);
            // (the wheels' parts after the hull's: every contact of a wheel's shape where the
            // move has brought it. What the wheel itself is told of them the next update reads
            // afresh from the same pose; here they are kept for the one case the solver gets
            // one - a wheel in deeper than its suspension goes, 0x8255D360.
            // MCLA_DBG_NOBOTTOM: the wheels' contacts never reach the solver.)
            let bottoming = !dbg_flag("MCLA_DBG_NOBOTTOM");
            // (0x8255CEA0 ends every update with the wheels' bytes +530 cleared)
            for w in vs.wheels.iter_mut() {
                w.bottomed = false;
            }
            if bottoming {
                let order: [usize; 4] = if dbg_flag("MCLA_DBG_WHEELORDER") { [0, 2, 1, 3] } else { [0, 1, 2, 3] };
                for (k, i) in order.into_iter().enumerate() {
                    if let (Some((front, radius, width, _, _)), Some(part)) = (wheel_geo[i], vs.wheels[i].part) {
                        let m = super::gjk::world_matrix(&part, &seen);
                        let core = super::gjk::WheelCore::of(&super::gjk::wheel_shape(front, radius, width), m[3], [m[0], m[1], m[2]], radius, width);
                        let moved = match (wheel_reach[i], wheel_rest[i]) {
                            (Some((_, limit)), Some(rest)) if !dbg_flag("MCLA_DBG_RA6") => {
                                let top = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(rest.x, (limit + rest.y) + 0.1, rest.z)];
                                use super::gjk::Support;
                                let own = super::gjk::WheelCore::of(&super::gjk::wheel_shape(front, radius, width), Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], radius, width);
                                let mg = f32::from_bits(0x3d23_d70a);
                                let half = Vec3::new(own.support(Vec3::X).x + mg, own.support(Vec3::Y).y + mg, own.support(Vec3::Z).z + mg);
                                Some((m, super::gjk::world_matrix(&top, &last), half, super::gjk::world_matrix(&part, &last)))
                            }
                            _ => None,
                        };
                        super::hull::wheel_collide(world, &core, k as u8 + 1, &seen, cell_ball, &mut vs.manifolds, moved);
                    }
                }
            }
            let wheel_of_part = |part: u8| -> usize { if dbg_flag("MCLA_DBG_WHEELORDER") { [0, 2, 1, 3][part as usize - 1] } else { part as usize - 1 } };
            let up_update = vs.prev_up.unwrap_or(rows0[1]);
            let car_city_pair = !vs.manifolds.is_empty();
            super::hull::refresh(&seen, &mut vs.manifolds);
            // (stage P: the hull against the props and pieces about it, their kept contacts, the callbacks)
            let loose_world = world.loose.as_ref().filter(|_| !dbg_flag("MCLA_DBG_P1") && !vs.loose.ghost);
            // (RA7: the wheels' parts for the props' side - MCLA_DBG_RA7: the hull alone, as before)
            let mut loose_wheels: Vec<Option<super::loose::WheelIn>> = Vec::new();
            if !dbg_flag("MCLA_DBG_RA7") && wheel_geo.iter().filter(|g| g.is_some()).count() == 4 {
                for i in 0..4 {
                    loose_wheels.push(match (wheel_geo[i], vs.wheels[i].part, wheel_reach[i], wheel_rest[i]) {
                        (Some((front, radius, width, _, _)), Some(part), Some((_, limit)), Some(rest)) => {
                            use super::gjk::Support;
                            let m = super::gjk::world_matrix(&part, &seen);
                            let shape = super::gjk::wheel_shape(front, radius, width);
                            let core = super::gjk::WheelCore::of(&shape, m[3], [m[0], m[1], m[2]], radius, width);
                            let own = super::gjk::WheelCore::of(&shape, Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], radius, width);
                            let mg = f32::from_bits(0x3d23_d70a);
                            let half = Vec3::new(own.support(Vec3::X).x + mg, own.support(Vec3::Y).y + mg, own.support(Vec3::Z).z + mg);
                            let top = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(rest.x, (limit + rest.y) + 0.1, rest.z)];
                            Some(super::loose::WheelIn { core, now: m, last: super::gjk::world_matrix(&top, &last), half })
                        }
                        _ => None,
                    });
                }
            }
            let car_in = super::loose::CarIn { hull, hull_cg: hull.cg, hull_margin: f32::from_bits(0x3d23_d70a), now: seen, last, speed: speed_in, bound: car_box, wheels: loose_wheels, vel: v_in, touch: (t.s.bound_friction, t.s.bound_elasticity), wiped: vs.wipe.is_some() };
            if let Some(lw) = loose_world {
                vs.loose.tick += 1;
                vs.loose.collide(lw, &car_in, world, car_city_pair);
                vs.loose.refresh(lw, &car_in);
            }
            let loose_ids = if loose_world.is_some() { vs.loose.car_island() } else { Vec::new() };
            let loose_touch = !loose_ids.is_empty();
            let up = rows1[1];
            let slippery = vs.stuck == 2 && gear_not_reverse(vs.gear, vs.way_after);
            let head_on = (t.s.bound_friction, t.s.bound_elasticity, t.s.head_on_cg_offset);
            let mut contacts: Vec<super::solver::Contact> = Vec::new();
            // (every contact of the car with the city in the manifolds' order: is it one the solver gets)
            let mut city_rows: Vec<bool> = Vec::new();
            let mut hull_at: Vec<usize> = Vec::new();
            for m in vs.manifolds.iter_mut() {
                for slot in m.slots.iter_mut() {
                    city_rows.push(true);
                    // (0x825A2C88: the two materials' friction and elasticity multiplied; the
                    // impulse of the tick before forgotten)
                    let c = &mut slot.c;
                    c.impulse = Vec3::ZERO;
                    if slot.part != 0 {
                        // (the wheel's callback, 0x8255D028: its contact is the suspension's
                        // business and is taken from the solver - unless it stands deeper along
                        // the car's up, as the update had it, than SuspensionLimit against what
                        // is fixed: then the solver has it, less that limit, without friction
                        // and with the tuning's BottomoutElasticity; wheel byte +530 says which)
                        let i = wheel_of_part(slot.part);
                        let (_, _, _, limit, bounce) = wheel_geo[i].unwrap_or((false, 0.0, 0.0, f32::MAX, 0.0));
                        let d = c.depth * ex::dot3(c.normal, up_update);
                        if d > limit {
                            vs.wheels[i].bottomed = true;
                            c.depth = d - limit;
                            c.friction = 0.0;
                            c.elasticity = bounce;
                            slot.enabled = true;
                            if dbg_flag("MCLA_DBG_HIT") {
                                eprintln!("WHEELCON wheel {i} cell {} polygon {} age {} depth {:?} elasticity {:?} on_world {:?} on_body {:?} normal {:?}", m.cell, slot.polygon, slot.age, c.depth, c.elasticity, c.on_world, c.on_body, c.normal);
                            }
                            contacts.push(*c);
                        } else {
                            if dbg_flag("MCLA_DBG_HIT") {
                                eprintln!("WHEELOFF wheel {i} cell {} polygon {} age {} depth {:?} along {:?} normal {:?}", m.cell, slot.polygon, slot.age, c.depth, d, c.normal);
                            }
                            vs.wheels[i].bottomed = false;
                            slot.enabled = false;
                            *city_rows.last_mut().unwrap() = false;
                            city_off_n += c.normal;
                        }
                        continue;
                    }
                    let own = super::collision::surface_touch(hull.surface_of(slot.element));
                    let _ = up;
                    let other = super::collision::surface_touch(slot.surface);
                    let f = own.0 * other.0;
                    c.friction = if f >= 0.0 { f } else { 0.0 };
                    let e = own.1 * other.1;
                    c.elasticity = if e - 1.0 >= 0.0 { 1.0 } else { e };
                    slot.enabled = true;
                    // (the car's callback, 0x82332D30, for a contact with what is fixed: head-on
                    // and fast, the tuning's own friction and elasticity and the body's point
                    // taken lower; while mcCarStuck turns the car, no friction)
                    let n = c.normal;
                    let along = ex::fma(n.x, rows1[2].x, ex::fma(rows1[2].y, n.y, rows1[2].z * n.z));
                    if wiped {
                        c.friction = head_on.0;
                        c.elasticity = head_on.1;
                    } else if along > (ex::guest_cos(f64::from_bits(0x3FDB_ECDE_6000_0000)) as f32) && speed_in > 22.35 {
                        c.friction = head_on.0;
                        c.elasticity = head_on.1;
                        c.on_body.y -= head_on.2;
                    } else if slippery {
                        c.friction = 0.0;
                    }
                    if dbg_flag("MCLA_DBG_HIT") {
                        eprintln!("HULLCON cell {} polygon {} element {} age {} depth {:?} friction {:?} elasticity {:?} local {:?} on_world {:?} on_body {:?} normal {:?}", m.cell, slot.polygon, slot.element, slot.age, c.depth, c.friction, c.elasticity, c.local, c.on_world, c.on_body, c.normal);
                    }
                    hull_at.push(contacts.len());
                    contacts.push(*c);
                }
            }
            // (0x825A2E00, called for every wheel contact that is not handed to the solver: its
            // record is emptied - a wheel's contacts are not kept from tick to tick)
            for m in vs.manifolds.iter_mut() {
                m.slots.retain(|s| s.part == 0 || s.enabled);
            }
            vs.manifolds.retain(|m| !m.slots.is_empty());
            if !loose_touch {
                if let Some(lw) = loose_world {
                    vs.loose.wake_car_island(lw);
                }
            }
            if !contacts.is_empty() || loose_touch {
                let mut body = super::solver::Body { at: at1, rows: rows1, vel: if frozen { Vec3::ZERO } else { v_in }, ang: if frozen { Vec3::ZERO } else { w_in }, inv_mass: 1.0 / mass, inv_moments: inverse, force: if frozen { Vec3::ZERO } else { force }, torque: if frozen { Vec3::ZERO } else { torque } };
                // (the move's turn of the rows, 0x8259D030, ends by taking the turning rate from
                // the angular momentum again through the rows as they then stand - before the
                // 33rd move makes them square: that, not the rate the tick came in with, is what
                // the solver starts from)
                if !frozen && ex::dot3(w_in * dt, w_in * dt) > f32::from_bits(0x2b8c_bccc) {
                    let mut w0 = ex::spin_of(lm, &rows_turned, inverse);
                    if w0.length_squared() > 12.566_371 * 12.566_371 {
                        w0 *= 12.566_371 / w0.length();
                    }
                    body.ang = w0;
                }
                // (MCLA_DBG_SOLVER=<file>: the solve as a case of tools/solver_reference.py)
                if let Some(path) = std::env::var_os("MCLA_DBG_SOLVER") {
                    use std::io::Write;
                    let h = |v: Vec3| format!("{:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
                    let mut line = format!("{:08x} {} {} {} {} {} {} {:08x} {:08x} {} {} {} {:x}", dt.to_bits(), h(body.at), h(body.rows[0]), h(body.rows[1]), h(body.rows[2]), h(body.vel), h(body.ang), mass.to_bits(), body.inv_mass.to_bits(), h(body.inv_moments), h(body.force), h(body.torque), contacts.len());
                    for c in &contacts {
                        line.push_str(&format!(" {:08x} {:08x} {:08x} {} {} {} {} {} {} {}", c.depth.to_bits(), c.friction.to_bits(), c.elasticity.to_bits(), h(c.on_body), h(c.on_world), h(c.normal), h(c.kept), h(c.target), h(c.pushed), h(c.local)));
                    }
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                        let _ = writeln!(f, "{line} # live");
                    }
                }
                if loose_touch {
                    // (an island with something loose in it: the solve, the breaking pass, the solve again)
                    let mut dynm = super::loose::CarDyn { p: &mut pm, l: &mut lm, mass, inverse, rows: rows1 };
                    let out = vs.loose.solve_car_island(loose_world.unwrap(), &car_in, &mut body, &mut dynm, &contacts, &city_rows, &loose_ids, dt);
                    impulse = out.impulse;
                    turning = out.turning;
                    // (each back to its own contact: the island's sort can have moved the car's rows among
                    // themselves - V s_rtv s333, a separating wall contact warm-started with another's
                    // multipliers)
                    for (l, t) in out.links.iter().zip(out.touches.iter()) {
                        if l.from.is_none() {
                            contacts[l.car_row].kept = t.kept;
                            contacts[l.car_row].impulse = t.impulse;
                        } else if t.impulse != Vec3::ZERO {
                            loose_answered = true;
                        }
                    }
                    if dbg_flag("MCLA_DBG_HIT") {
                        eprintln!("LOOSEISLAND impulse {:?} turning {:?} momenta {:?} {:?}", impulse, turning, pm, lm);
                    }
                    solve_body = Some(body);
                    loose_island = Some((body, contacts, out));
                } else {
                // (R3: an island of more than 16 contacts is sorted by a sort that does not keep equal keys
                // in their order, 0x82593CC8 - with the thrown rider's contacts among them the hull's own
                // can come out exchanged. A replay is told the order the recording's island had them in.)
                let code = REPLAY_ORDER.load(std::sync::atomic::Ordering::Relaxed);
                if code != 0 && r3 {
                    let mut digits: Vec<usize> = Vec::new();
                    let mut c = code;
                    while c > 0 { digits.insert(0, (c % 10) as usize); c /= 10; }
                    let hulls: Vec<usize> = hull_at.clone();
                    let mut sorted = digits.clone();
                    sorted.sort();
                    if digits.len() == hulls.len() && sorted.iter().enumerate().all(|(i, d)| *d == i + 1) {
                        let was = contacts.clone();
                        let mut order: Vec<usize> = (0..contacts.len()).collect();
                        for (j, d) in digits.iter().enumerate() {
                            order[hulls[j]] = hulls[d - 1];
                            contacts[hulls[j]] = was[hulls[d - 1]];
                        }
                        reordered = Some(order);
                    }
                }
                let solved = super::solver::velocity(&body, &mut contacts, dt, 1.0 / dt);
                impulse = solved.impulse;
                turning = solved.turning;
                if dbg_flag("MCLA_DBG_HIT") {
                    for c in &contacts { eprintln!("HULLIMP multipliers {:?} impulse {:?}", c.kept, c.impulse); }
                }
                solve_body = Some(body);
                solved_contacts = Some((body, contacts, solved));
                }
            }
            // (the islands the car is not in: loose bodies on their own)
            if let Some(lw) = loose_world.filter(|_| !vs.loose.skip_end) {
                vs.loose.step_rest(lw, &car_in, dt);
            }
        }
        if frozen {
            pm = Vec3::ZERO;
            lm = Vec3::ZERO;
        }
        if !frozen || impulse != Vec3::ZERO || turning != Vec3::ZERO {
            let (f, tq) = if frozen { (Vec3::ZERO, Vec3::ZERO) } else { (force, torque) };
            (pm, lm) = ex::integrate_with(pm, lm, f, tq, impulse, turning, &damping, &rows1, inertia, mass, dt);
        }
        // (0x8259C178 / 0x8259E230 with their caps, body +200 and +204: 300 mph and two turns a second)
        let mut v1 = pm * (1.0 / mass);
        if v1.length_squared() > 134.1 * 134.1 {
            let k = 134.1 / v1.length();
            v1 *= k;
            pm *= k;
        }
        let mut w1 = ex::spin_of(lm, &rows1, inverse);
        if w1.length_squared() > 12.566_371 * 12.566_371 {
            let k = 12.566_371 / w1.length();
            w1 *= k;
            lm *= k;
        }
        // ---- and the second solve, which only moves the body out (0x82591108, 0x8259E920) ----
        let turn_left = if dbg_flag("MCLA_DBG_Y4") { Vec3::ZERO } else { vs.push_turn_left };
        // (in a tick with something loose in the car's island: mcCarDamage's calls, one per run of the
        // car's rows in each of the island's two lists - 0x82590DB0 walks the list sorted by instance A,
        // then the one sorted by instance B, and calls the instance once per run of rows with it on that
        // side. The car is A against the city and against what has a higher level index, B against what
        // has a lower one. V work/proppath k311 / k329: the scrape is the A run's, whose first row is a
        // piece the car is A against, not the city's or the first piece of the island.)
        let mut loose_runs: Vec<DamageRun> = Vec::new();
        let pushed_now = if let Some((body, mut contacts, mut solved)) = solved_contacts.take() {
            let pushed = super::solver::push(&body, &mut contacts, &mut solved, dt, 1.0 / dt, (0.04, 0.04), false, turn_left);
            if let Some(order) = reordered.take() {
                let now = contacts.clone();
                for (new, old) in order.iter().enumerate() {
                    contacts[*old] = now[new];
                }
            }
            Some((pushed, contacts))
        } else if let Some((body, mut contacts, mut out)) = loose_island.take() {
            // (mcCarDamage's calls: the car's run of the A-sorted list - its rows with the city and with what it is
            // A against, in the island's order - then its run of the B-sorted list; each sums the impulses on the car
            // over its mass, 0.4 of those against the fixed city (0x82567A08), and takes the first row's point and other)
            let a_rows: Vec<usize> = (0..out.links.len()).filter(|&i| out.links[i].a == Some(super::loose::Who::Car)).collect();
            let b_rows: Vec<usize> = out.by_b.iter().copied().filter(|&i| out.links[i].b == Some(super::loose::Who::Car)).collect();
            for (rows, off) in [(a_rows, city_off_n + out.off_a), (b_rows, out.off_b)] {
                let mut run: Option<DamageRun> = None;
                for i in rows {
                    let (l, t) = (&out.links[i], &out.touches[i]);
                    let car_a = l.a == Some(super::loose::Who::Car);
                    let other = if car_a { l.b } else { l.a };
                    let fixed = matches!(other, Some(super::loose::Who::City) | None);
                    let (mine, point, n_car, other_side) = if car_a { (t.impulse, t.a.point, t.normal, &t.b) } else { (Vec3::ZERO - t.impulse, t.b.point, t.back, &t.a) };
                    let r = run.get_or_insert_with(|| {
                        let ob = other_side.body.and_then(|k| out.bodies.get(k));
                        DamageRun { dv: Vec3::ZERO, first: point, last: point, mult: vs.loose.vehicle_factor(other), any: false, nsum: Vec3::ZERO, n0: n_car, v2: ob.map_or(Vec3::ZERO, |b| b.vel + b.ang.cross(point - b.at)), v2c: ob.map_or(Vec3::ZERO, |b| b.vel) }
                    });
                    r.dv += if fixed { mine * body.inv_mass * f32::from_bits(0x3ecc_cccd) } else { mine * body.inv_mass };
                    r.any |= t.impulse != Vec3::ZERO;
                    r.nsum += n_car;
                    r.last = point;
                }
                loose_runs.extend(run.map(|mut r| {
                    r.nsum += off;
                    r
                }));
            }
            // (the ambient car's post-solve callback, slot 25 sub_826FAF98, for each contact it has with this car - an
            // mcCarInstance: v = its impulse (negated on the pair's second side) x its collider's inverse mass (+196),
            // x 0.4 were this car fixed (level state 2); w = sat((sqrt|v| - 0.5) x 0.2); w > 0 is added to one of
            // its render instance's four side sums (+560) by the contact's point on it in its frame (the instance's
            // matrix inverted, sub_821713C0): x for (x > 0, z > 0), y (x > 0, z <= 0), z (x <= 0, z > 0), w (x <= 0,
            // z <= 0). A car without a collider (riding, not made active) is skipped.)
            for (l, t) in out.links.iter().zip(out.touches.iter()) {
                let (amb_side, j, o) = match (l.a, l.b) {
                    (Some(super::loose::Who::Car), Some(super::loose::Who::Obj(o))) => (&t.b, Vec3::ZERO - t.impulse, o),
                    (Some(super::loose::Who::Obj(o)), Some(super::loose::Who::Car)) => (&t.a, t.impulse, o),
                    _ => continue,
                };
                let Some(ob) = vs.loose.objs.get(o) else { continue };
                let Some(uid) = ob.amb.as_ref().map(|a| a.uid) else { continue };
                let Some(b) = amb_side.body.and_then(|k| out.bodies.get(k)) else { continue };
                let v = j * b.inv_mass;
                let w = (((v.length_squared().sqrt()).sqrt() - 0.5) * 0.2).max(0.0).min(1.0);
                if w > 0.0 {
                    let lp = super::loose::inv(&ob.matrix, amb_side.point - ob.matrix[3]);
                    let side = match (lp.x > 0.0, lp.z > 0.0) {
                        (true, true) => 0,
                        (true, false) => 1,
                        (false, true) => 2,
                        (false, false) => 3,
                    };
                    vs.loose.amb_dmg.push((uid, side, w));
                }
            }
            for (l, t) in out.links.iter().zip(out.touches.iter()) {
                let other = match (l.a, l.b) {
                    (Some(super::loose::Who::Car), o) => o,
                    (o, Some(super::loose::Who::Car)) => o,
                    _ => continue,
                };
                if l.from.is_none() || matches!(other, Some(super::loose::Who::City) | None) {
                    continue;
                }
                // (the collision audio's view of a touch with another car, as the city's below)
                if let Some(super::loose::Who::Obj(o)) = other {
                    if vs.loose.objs.get(o).is_some_and(|x| x.veh.is_some()) {
                        let (own_side, other_side, n) = if matches!(l.a, Some(super::loose::Who::Car)) { (&t.a, &t.b, t.normal) } else { (&t.b, &t.a, t.back) };
                        let vel_of = |s: &super::solver::Side| s.body.and_then(|k| out.bodies.get(k)).map_or(Vec3::ZERO, |b| b.vel + b.ang.cross(s.point - b.at));
                        let side = super::collision::Surface::CarSide;
                        st.touches.push(super::car::AudioTouch { call: st.touch_calls + 1, pos: own_side.point, normal: Vec3::ZERO - n, vel: vel_of(own_side) - vel_of(other_side), own: side, other: side, vehicle: true });
                    }
                }
            }
            let pushed = vs.loose.push_car_island(&mut out, &body, super::solver::Pushed { push: Vec3::ZERO, turn: turn_left, deep: false }, dt);
            for (l, t) in out.links.iter().zip(out.touches.iter()) {
                if l.from.is_none() {
                    contacts[l.car_row].pushed = t.pushed;
                }
            }
            Some((pushed, contacts))
        } else {
            None
        };
        if let Some((pushed, contacts)) = pushed_now {
            // (where the velocity solve's move left the body, before it is pushed out)
            let (moved_rows, moved_origin) = (rows1, origin1);
            at1 = at1 + pushed.push;
            rows1 = super::solver::rows_pushed(&rows1, pushed.turn);
            // (0x8259E790: a turn of under a millionth of a radian is not made and not cleared)
            vs.push_turn_left = if super::solver::turn_applies(pushed.turn) { Vec3::ZERO } else { pushed.turn };
            origin1 = ex::origin_of(at1, &rows1, pivot_local);
            if dbg_flag("MCLA_DBG_HIT") {
                eprintln!("HULLPUSH push {:?} turn {:?} deep {}", pushed.push, pushed.turn, pushed.deep);
            }
            // (the contacts' multipliers and impulses back into what is kept)
            let mut k = 0;
            // (RA4, V sa.0 211: an impulse on a piece alone does not count - only what is fixed answers.
            // MCLA_DBG_RA4: any impulse of the island, as before.)
            let mut answered = loose_answered && dbg_flag("MCLA_DBG_RA4");
            let mut walls = 0u32;
            st.touch_calls += 1;
            // (mcCarDamage's view, 0x82343880 / 0x82567A08: every enabled contact's impulse over the
            // mass, 0.4 of it against the fixed city; the first one's point, the last one's on a car
            // lying over)
            let (mut hit_dv, mut hit_first, mut hit_last, mut hit_any) = (Vec3::ZERO, None, Vec3::ZERO, false);
            let (mut hit_n, mut hit_n0) = (Vec3::ZERO, None);
            for m in vs.manifolds.iter_mut() {
                for slot in m.slots.iter_mut() {
                    if !slot.enabled {
                        continue;
                    }
                    let c = contacts[k];
                    k += 1;
                    hit_dv += c.impulse * (1.0 / mass) * f32::from_bits(0x3ecc_cccd);
                    hit_any |= c.impulse != Vec3::ZERO;
                    hit_first.get_or_insert(c.on_body);
                    hit_last = c.on_body;
                    hit_n += c.normal;
                    hit_n0.get_or_insert(c.normal);
                    // (the collision audio's view of it: sub_82381FB0 reads every enabled contact)
                    st.touches.push(super::car::AudioTouch {
                        call: st.touch_calls,
                        pos: c.on_body,
                        normal: -c.normal,
                        vel: v_in + w_in.cross(c.on_body - at1),
                        own: if slot.part != 0 { super::collision::Surface::CarWheel } else { p.hull.as_ref().map_or(super::collision::Surface::CarSide, |h| h.surface_of(slot.element)) },
                        other: slot.surface,
                        vehicle: false,
                    });
                    slot.c.kept = c.kept;
                    slot.c.pushed = c.pushed;
                    slot.c.impulse = c.impulse;
                    // (A1, 0x8232E8E0: SettleCG counts every enabled contact with what is fixed that stands
                    // all but upright - a wheel's too, when it is in past its suspension against a wall: a
                    // motorcycle's front wheel in a crash. MCLA_DBG_A1: the hull's only.)
                    if slot.part != 0 {
                        if c.normal.y.abs() < 0.1 && !dbg_flag("MCLA_DBG_A1") {
                            walls += 1;
                        }
                        continue;
                    }
                    answered |= c.impulse != Vec3::ZERO;
                    if c.normal.y.abs() < 0.1 {
                        walls += 1;
                    }
                }
            }
            hull_touched = Some((answered, walls));
            // (the callback runs only in a tick the solver gave some contact of the car an impulse; with something
            // loose in the island the calls are the runs below)
            if hit_any && loose_runs.is_empty() && !dbg_flag("MCLA_DBG_NOWORLDHIT") {
                let at = if st.up().y < f32::from_bits(0x3dcc_cccd) { hit_last } else { hit_first.unwrap_or(hit_last) };
                // (0x82343378: in the instance's matrix, which the tick's move has already set)
                let d = at - moved_origin;
                let local = Vec3::new(d.dot(moved_rows[0]), d.dot(moved_rows[1]), d.dot(moved_rows[2]));
                if dbg_flag("MCLA_DBG_WORLDHIT") {
                    eprintln!("WORLDHIT dv {:.6} at local {:.6} {:.6} {:.6} zones {:?} damage {:.6} zone box {:?}", hit_dv.length(), local.x, local.y, local.z, st.zones, st.damage * p.dmg[0], st.zone_half);
                }
                // (a scrape's inputs, 0x82343CA8 .. 0x823441F0: the car's velocity at the point as the solve started
                // from it - no impulses, no pending forces - and at its centre after the solve; the city's none; the
                // normals summed, unit over 0.001, else the first's. V work/dmgmap: a 26 m/s head-on 26.08445 |
                // 26.084 m/s at the point, the scrape then let through by the centre's 4.54 m/s after the solve)
                let hit_n = hit_n + city_off_n;
                let n = if hit_n.length() > f32::from_bits(0x3a83_126f) { hit_n.normalize() } else { hit_n0.unwrap_or(Vec3::Y) };
                let v_pt = start_velocity(solve_body.as_ref(), at).unwrap_or(v1 + w1.cross(at - (at1 - pushed.push)));
                // (the gate's velocity, 0x823441FC: the car's body +272 as the callback reads it - the solve leaves it as
                // it began, its impulses go in at the Update (V s_rtv, car forced: of every tick with |dv| over 0.1 the
                // scrape ticks are the original's exactly - k167 and k224 included - with it and with no other))
                let v_c = solve_body.filter(|_| !dbg_flag("MCLA_DBG_SCRAPEPOST")).map_or(v1, |b| b.vel);
                let scrape = super::car::ScrapeIn { v1: v_pt, v2: Vec3::ZERO, rel: v_c, n };
                if dbg_flag("MCLA_DBG_WORLDHIT") {
                    if let Some(b) = solve_body.as_ref() {
                        let c = |v: Vec3| (v.length(), v.dot(n) / v.length().max(1e-9));
                        eprintln!("GATE t{} dv {:.5} start {:?} imp {:?} force {:?} end {:?}", super::loose::REPLAY_TICK.load(std::sync::atomic::Ordering::Relaxed), hit_dv.length(), c(b.vel), c(b.vel + impulse * (1.0 / mass)), c(b.vel + force * (dt / mass)), c(v1));
                    }
                }
                if dbg_flag("MCLA_DBG_WORLDHIT") {
                    eprintln!("SCRAPE t{} dv {:.5} |v1| {:.5} |rel| {:.4} c {:.4} n {:.4?} amount {:.6}", super::loose::REPLAY_TICK.load(std::sync::atomic::Ordering::Relaxed), hit_dv.length(), v_pt.length(), v_c.length(), v_c.dot(n) / v_c.length().max(1e-9), n, scrape.amount());
                }
                st.world_hit(p, hit_dv.length(), at, local, Some(scrape));
            }
            // (the island's calls, the A run's first: in a recorded tick with both, mcCarDamage +56 - the last
            // call's - is the sum of the pieces the car is B against, exactly - sa.0 k230..k292)
            for r in loose_runs.iter().filter(|r| r.any && !dbg_flag("MCLA_DBG_NOWORLDHIT")) {
                let at = if st.up().y < f32::from_bits(0x3dcc_cccd) { r.last } else { r.first };
                let d = at - moved_origin;
                let local = Vec3::new(d.dot(moved_rows[0]), d.dot(moved_rows[1]), d.dot(moved_rows[2]));
                let dv = r.dv.length();
                if dbg_flag("MCLA_DBG_WORLDHIT") {
                    eprintln!("LOOSEHIT dv {:.6} at local {:.6} {:.6} {:.6} mult {}", dv, local.x, local.y, local.z, r.mult);
                }
                let n = if r.nsum.length() > f32::from_bits(0x3a83_126f) { r.nsum.normalize() } else { r.n0 };
                // (the car's own velocity at the point as the solve started, as above; the other's after it: I)
                let v_pt = start_velocity(solve_body.as_ref(), at).unwrap_or(v1 + w1.cross(at - (at1 - pushed.push)));
                let v_c = solve_body.filter(|_| !dbg_flag("MCLA_DBG_SCRAPEPOST")).map_or(v1, |b| b.vel);
                let scrape = super::car::ScrapeIn { v1: v_pt, v2: r.v2, rel: v_c - r.v2c, n };
                if dbg_flag("MCLA_DBG_WORLDHIT") {
                    eprintln!("LOOSESCRAPE t{} dv {:.5} v1 {:.4?} v2 {:.4?} rel {:.4?} n {:.4?} amount {:.6}", super::loose::REPLAY_TICK.load(std::sync::atomic::Ordering::Relaxed), dv, v_pt, r.v2, scrape.rel, n, scrape.amount());
                }
                st.damage_call(p, dv, r.mult, at, local, Some(scrape));
            }
            // (0x8259E920: the matrix the next tick's collision takes for the last one)
            let end = [rows1[0], rows1[1], rows1[2], origin1];
            if !pushed.deep {
                vs.hull_last = Some(end);
                vs.last_set = true;
                vs.deep_count = 0;
            } else {
                vs.deep_count = vs.deep_count.saturating_add(1).min(254);
                if !vs.last_set || vs.deep_count > 8 {
                    vs.hull_last = Some(end);
                }
            }
        } else if exact_collide {
            vs.hull_last = Some([rows1[0], rows1[1], rows1[2], origin1]);
            vs.last_set = true;
            vs.deep_count = 0;
        }
        // (stage P: the loose bodies' integration, their push applied, the entries' ages)
        if exact_collide && !dbg_flag("MCLA_DBG_P1") {
            if let Some(lw) = world.loose.as_ref() {
                if !vs.loose.skip_end {
                    vs.loose.end_tick(lw, dt);
                }
            }
        }
        st.vel = v1;
        st.ang_vel = w1;
        st.pos = origin1;
        st.rot = Quat::from_mat3(&Mat3::from_cols(rows1[0], rows1[1], rows1[2])).normalize();
        vs.body_now = Some([rows1[0], rows1[1], rows1[2], origin1]);
        if std::env::var_os("MCLA_DBG_POSE").is_some() {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let h = |v: Vec3| format!("{:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
            eprintln!("POSE {} seen {} {} {} {} after {} {} {} {} at {}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed), h(seen[0]), h(seen[1]), h(seen[2]), h(seen[3]), h(rows1[0]), h(rows1[1]), h(rows1[2]), h(origin1), h(at1));
        }
        vs.body = Some(BodyExact { rows: rows1, origin: origin1, at: at1, p: pm, l: lm, count, snap: (st.pos, st.rot, st.vel, st.ang_vel), seen });
        (v1 - v_in, w1 - w_in)
    } else {
        (dv, dw)
    };
    // (a car without a hull shape still meets the world with its spheres, once a tick here)
    if exact_step && !dbg_flag("MCLA_DBG_NORESOLVE") && (p.hull.is_none() || dbg_flag("MCLA_DBG_SPHERES")) {
        st.resolve_body_pub(p, world);
    }
    let mut given = 0.0f32;
    for _ in 0..if exact_step { 0 } else { 4 } {
        let dt = body_step;
        let rm = Mat3::from_quat(st.rot);
        let com = st.pos + rm * pivot_local;
        st.vel += dv * 0.25;
        given += 0.25;
        let mut ang_local = rm.transpose() * (st.ang_vel + dw * 0.25);
        // angular damping per axis: constant, linear and quadratic terms (AngCDamp, AngVelDamp, AngVel2Damp)
        let damp = |w: f32, c0: f32, c1: f32, c2: f32| -> f32 {
            let mut a = (-(c0 * w.signum()) - c1 * w - c2 * w.abs() * w) * vs.aero_damp;
            if a.abs() * dt > w.abs() {
                a = -w / dt;
            }
            if w.abs() < 1.0 {
                a *= w.abs();
            }
            w + a * dt
        };
        let a = &t.aero;
        if !dbg_flag("MCLA_DBG_NODAMP") && !tick_damping {
        ang_local = Vec3::new(damp(ang_local.x, a.ang_c_damp_x, a.ang_vel_damp_x, a.ang_vel2_damp_x), damp(ang_local.y, a.ang_c_damp_y, a.ang_vel_damp_y, a.ang_vel2_damp_y), damp(ang_local.z, a.ang_c_damp_z, a.ang_vel_damp_z, a.ang_vel2_damp_z));
        }
        // anti-roll (vehAxle): each axle turns the body against the difference between its
        // wheels' compressions (torque coefficient, damping)
        for (a, (l, r)) in [(0usize, 1usize), (2, 3)].into_iter().enumerate() {
            let (coef, wt) = if a == 0 { (t.axle_front, &t.wheel_front) } else { (t.axle_back, &t.wheel_back) };
            let (wl, wr) = (&vs.wheels[l], &vs.wheels[r]);
            if tick_damping || dbg_flag("MCLA_DBG_NOROLLBAR") || coef[0] == 0.0 || !coef[0].is_finite() || coef[0].abs() > 100.0 || !(st.wheels[l].grounded || st.wheels[r].grounded) {
                continue;
            }
            let dx = wl.comp - wr.comp;
            if dx.abs() > 0.01 {
                let span = (wt.suspension_limit + wt.suspension_extent).max(0.05);
                let damp = if coef[1].is_finite() && coef[1].abs() < 100.0 { coef[1] } else { 0.0 };
                // (the left side lower means the right side is up: turn it back down)
                ang_local.z -= (dx / span * coef[0] + damp * (wl.comp_rate - wr.comp_rate)) * dt;
            }
        }
        if grounded_count == 0 {
            // levelling about the car's length, steering about its height
            ang_local.z += vs.air_acc[0] * dt;
            ang_local.y += vs.air_acc[1] * dt;
            ang_local.x += vs.air_acc[2] * dt;
        }
        // (the two-wheel controller's roll)
        ang_local.z += vs.two_acc * dt;
        st.ang_vel = rm * ang_local;
        let new_com = com + (st.vel - dv * given) * dt;
        let turn = st.ang_vel - dw * given;
        if turn.length_squared() > 0.0 {
            st.rot = (Quat::from_scaled_axis(turn * dt) * st.rot).normalize();
        }
        // (a motorcycle's lean: the collider's turn and push of this tick, a quarter a step)
        let mut new_com = new_com;
        if vt.bike && vs.bike_turn != Vec3::ZERO {
            st.rot = (Quat::from_scaled_axis(vs.bike_turn * 0.25) * st.rot).normalize();
            new_com += vs.bike_push * 0.25;
        }
        st.pos = new_com - st.rot * pivot_local;

        if !dbg_flag("MCLA_DBG_NORESOLVE") && (p.hull.is_none() || dbg_flag("MCLA_DBG_SPHERES")) {
            st.resolve_body_pub(p, world);
        }
    }
    // ---- the body against the world, once a tick as the original collides: the hull's
    // contacts, each with the product of its two materials' friction and of their elasticity
    // (0x825A2C88: the hull is car_side, car_roof on top and car_undercarriage below, told
    // here by the way the body is pushed) ----
    let new_collide = exact_step && !dbg_flag("MCLA_DBG_OLDCOLLIDE") && p.hull.is_some();
    if new_collide {
        // (the car's callbacks after the solve, 0x8232E8E0: a contact the solver has starts the
        // standing count again; if the solver answered any, mcCarStuck is armed, and for every
        // contact that stands all but upright the centre of mass comes 0.8 of the way down to
        // SettleCG x ModelOffset's height)
        if let Some((answered, walls)) = hull_touched {
            vs.still_timer = 0.0;
            if answered {
                if vs.stuck == 0 {
                    vs.stuck = 1;
                    // (0x8256A078: the chassis's own place as the tick's update had it - the model's
                    // origin - which mcCarStuck then measures the BODY's place from)
                    vs.stuck_at = body_rows[3];
                    vs.stuck_timer = 0.0;
                }
                if !dbg_flag("MCLA_DBG_NOSETTLE") {
                    for _ in 0..walls {
                        vs.cg[0] = super::exact::fma(super::exact::fms(t.s.settle_cg, t.s.model_offset_y, vs.cg[0]), f32::from_bits(0x3f4c_cccd), vs.cg[0]);
                    }
                }
            }
        }
    } else if !dbg_flag("MCLA_DBG_NORESOLVE") && !dbg_flag("MCLA_DBG_SPHERES") {
        if let Some(hull) = p.hull.as_ref() {
            let up = st.up();
            // (0x82333C2C: while mcCarStuck turns the car, going forwards, what its body rubs
            // on does not hold it)
            let slippery = vs.stuck == 2 && gear_not_reverse(vs.gear, vs.way_after);
            let touch = |surface: super::collision::Surface, normal: Vec3, height: f32| -> (f32, f32) {
                // (the bumper bar and the floor are undercarriage: recorded friction 0.010 and elasticity
                // 0.100 for the bar against a wall = 0.1 x 0.1 and 0.2 x 0.5)
                let own = super::collision::surface_touch(if normal.dot(up) > 0.7 || height < 0.4 { super::collision::Surface::CarUndercarriage } else if normal.dot(up) < -0.5 { super::collision::Surface::CarRoof } else { super::collision::Surface::CarSide });
                let other = super::collision::surface_touch(surface);
                static DBG: std::sync::OnceLock<(f32, f32)> = std::sync::OnceLock::new();
                let k = *DBG.get_or_init(|| { let g = |n: &str| std::env::var(n).ok().and_then(|v| v.parse().ok()).unwrap_or(1.0); (g("MCLA_DBG_HULLFRIC"), g("MCLA_DBG_HULLBOUNCE")) });
                if dbg_flag("MCLA_DBG_GYRO") { eprintln!("TOUCH own {:?} other {:?}", own, other); }
                (if slippery { 0.0 } else { (own.0 * other.0 * k.0).max(0.0) }, (own.1 * other.1 * k.1).min(1.0))
            };
            if let Some((speed, at)) = super::hull::resolve(st, hull, world, t.s.mass, inertia, pivot_local, dt, touch, (speed_in > 22.35).then_some((t.s.bound_friction, t.s.bound_elasticity, t.s.head_on_cg_offset)), &mut vs.hull_kept, (dv, dw)) {
                st.blow(p, speed, at);
            }
            // (the car's contact callback, 0x8232E8E0: a contact the solver has - the hull's,
            // not the wheels' - starts the standing count again)
            if super::hull::TOUCHED.with(|t| t.get()) {
                vs.still_timer = 0.0;
                // (and, if the solver had to answer it, arms mcCarStuck with where the car is now)
                if vs.stuck == 0 && super::hull::HIT.with(|t| t.get()) {
                    vs.stuck = 1;
                    vs.stuck_at = st.pos;
                    vs.stuck_timer = 0.0;
                }
            }
            // (the same callback, 0x8232E8E0, after the solve: for every contact with something
            // fixed that stands all but upright - if the solver gave any impulse at all - the
            // centre of mass comes 0.8 of the way down to SettleCG x ModelOffset's height. With
            // it on the road the tyres' side force no longer rolls the car: a Range Rover
            // scraping a wall goes back to the road's 2.6 degrees. It climbs back 0.5 m/s.
            // Recorded: 0.3647 -> 0.0815 -> 0.0265 on one contact a tick; an R8 head-on, two
            // contacts, 0.1125 -> 0.0113 in the tick.)
            if !dbg_flag("MCLA_DBG_NOSETTLE") {
                let target = t.s.settle_cg * t.s.model_offset_y;
                for _ in 0..super::hull::WALLS.with(|t| t.get()) {
                    vs.cg[0] += (target - vs.cg[0]) * 0.8;
                }
            }
            st.catch_fall(world);
        }
    }
    if dbg_flag("MCLA_DBG_YAW") {
        eprintln!("YAW before {:?} dw {:?} torque {:?} after {:?} implicit {}", dbg_yaw0, dw, torque, st.ang_vel, implicit.len());
        eprintln!("VEL inertia {:?} mass {} local F {:?} T {:?} dv {:?} dw {:?} cg {:?} pivot {:?} | v {:?} w {:?} dv {:?} dw {:?} F {:?} T {:?} com {:?}", inertia, mass, rm.transpose() * force, rm.transpose() * torque, rm.transpose() * dv, rm.transpose() * dw, com_local, pivot_local, st.vel, st.ang_vel, dv, dw, force, torque, st.pos + Mat3::from_quat(st.rot) * pivot_local);
        for (arm, n, s) in &implicit { eprintln!("IMPL arm {:?} n {:?} s {:.3}", rm.transpose() * *arm, rm.transpose() * *n, s); }
    }
    // Contacts belong to the force calculation at the beginning of this frame.
    // The rendered hubs belong to the final chassis pose: retaining their old
    // world positions makes them trail the axle by roughly velocity * dt.
    for i in 0..4 {
        let physical = if vt.bike { i / 2 * 2 } else { i };
        let attach = if vt.bike {
            (p.wheel_pos[physical] + p.wheel_pos[physical + 1]) * 0.5
        } else { p.wheel_pos[physical] };
        // (the set-up's ride height, SuspensionOffset, as in the contact above: a lowered car's
        // wheels sit that much higher in its body. Left out, a lowered rival's wheels were drawn
        // that far into the road - N-VP-3, the Range Rover's 0.125 / 0.1125 m)
        let wt = match hyd_wheels.as_ref() {
            Some((f, b)) => if physical < 2 { f } else { b },
            None => if physical < 2 { &t.wheel_front } else { &t.wheel_back },
        };
        let ram = if hyd_fitted { hyd.height[physical] } else { 0.0 } + wt.suspension_offset;
        let local = attach - Vec3::Y * (p.susp_rest + ram - vs.wheels[physical].comp);
        st.wheels[i].world_pos = st.pos + st.rot * local;
    }
}

/// diagnostics: MCLA_NO_GYRO switches the turn gyro off
/// test hook: hold this gear (0 = the gearbox decides)
/// the player's steering sensitivity (the options' scale; f32 bits, 1.0 = the tuning's rate)
pub static STEER_SENS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);
/// RB4, mcPlayerInput +104: the speed limit, in mph, of the road the player's car is on (f32 bits; the
/// world's - 40 and 55 seen in a recording). Nothing (0, the default): the cruise never comes on.
pub static CRUISE_LIMIT_MPH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static FORCE_GEAR: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// test switches (set = on), read once
/// a * b + c in one rounding to double, then to single - the original's scalar fused
/// multiply-add as its code runs it (fmadds).
#[inline]
fn fma32(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// c - a * b likewise (fnmsubs).
#[inline]
fn fnms32(a: f32, b: f32, c: f32) -> f32 {
    (-(a as f64).mul_add(b as f64, -(c as f64))) as f32
}

/// How far in a wheel's contact puts its suspension (0x82565E00 and 0x82564C48, operation for
/// operation): `rows` the wheel's axle, up and way as this update rebuilt them; `n`, `depth`
/// the contact record's normal and depth (0x8255D028); `comp` the compression before. The
/// contact's own forward and sideways directions, how squarely the wheel stands on them
/// (never under 0.75), the depth over that, less the 7 mm a wheel stands in what it touches.
fn suspension_target(rows: [Vec3; 3], n: Vec3, depth: f32, comp: f32) -> Option<f32> {
    use super::gjk::{cross_as_original as cross, recip_as_original as recip};
    let [w0, w1, w2] = rows;
    let f = cross(w0, n);
    let l2 = fma32(f.z, f.z, fma32(f.x, f.x, f.y * f.y));
    if l2 < 0.0 {
        return None;
    }
    let forward = f * recip(l2.sqrt());
    let lateral = cross(n, forward);
    let a = fma32(w0.x, lateral.x, fma32(w0.z, lateral.z, w0.y * lateral.y)).abs();
    let b = fma32(w2.x, forward.x, fma32(w2.z, forward.z, w2.y * forward.y)).abs();
    let up = fma32(n.x, w1.x, fma32(w1.z, n.z, w1.y * n.y));
    let least = if b - a >= 0.0 { a } else { b };
    let least = if least - 0.75 >= 0.0 { least } else { 0.75 };
    let tilt = if least - 1.0 >= 0.0 { 1.0 } else { least };
    let over = 1.0 / tilt;
    // (wheel +448 and the global at 0x828DA450 are both nothing in every recording)
    Some(fnms32(over, 0.0, fnms32(up, 0.0, fma32(depth, over, comp) - f32::from_bits(0x3be56042))))
}

/// A car set down where a cine car event puts it, with a velocity (docs/reference/cutscenes/cine_car_event.md, snap):
/// sub_822A9930 with its probe - from a metre above the place to three below; on a hit the place is the hit, the up
/// the polygon's normal and the rows made square about it (as the stand-up, R4) - the velocities as given, the still
/// timer (chassis +588) 0, a burning nitrous shot and a running slipstream turbo stopped (sub_82345680, sub_82345E20);
/// then sub_8255A1B0: both shafts and their wheels at the speed along the car (-row 2 . v) over the first wheel's
/// radius, the differentials even, the gear the one for that speed (sub_82561050: reverse under -1 m/s, first under
/// 10, else the lowest gear the speed does not over-rev), the engine at the gear's speed held between IdleRPM and the
/// engine's +72 (the port: the torque curve's peak, else OptRPM - approximate). The car's pairs are made again only
/// in the next step (0x82588990, as the stand-up).
pub fn set_pose(st: &mut CarState, vt: &VehTune, world: &WorldCollider, m: [Vec3; 4], vel: Vec3, ang_vel: Vec3) {
    use super::exact as ex;
    let [x0, y0, z0, place] = m;
    let mut rows = [x0, y0, z0];
    let mut at = place;
    let (from, to) = (Vec3::new(place.x, place.y + 1.0, place.z), Vec3::new(place.x, place.y - 3.0, place.z));
    if let Some((point, normal)) = level_probe(world, from, to) {
        let unit_or = |v: Vec3, other: Vec3| if ex::dot3(v, v) > f32::from_bits(0x3727_c5ac) { ex::unit_or_zero(v) } else { other };
        let y = unit_or(normal, Vec3::Y);
        let x = unit_or(ex::cross(y, z0), Vec3::X);
        let z = unit_or(ex::cross(x, y), Vec3::Z);
        rows = [x, y, z];
        at = point;
    }
    st.relocate(at, Quat::from_mat3(&Mat3::from_cols(rows[0], rows[1], rows[2])).normalize());
    st.vel = vel;
    st.ang_vel = ang_vel;
    let Some(vs) = st.veh.as_deref_mut() else { return };
    vs.manifolds.clear();
    vs.hull_kept.clear();
    vs.hull_last = None;
    vs.last_seen = None;
    vs.set_down = true;
    vs.still_timer = 0.0;
    if vs.nitro_burning {
        vs.nitro_burning = false;
        vs.nitro_level = 0.0;
    }
    vs.sst_level = 0.0;
    let t = &vt.t;
    let e = &t.engine.s;
    let fwd = -ex::fma(rows[2].x, vel.x, ex::fma(rows[2].z, vel.z, rows[2].y * vel.y));
    let drive = if vs.drive == 255 { t.drivetrain_type as u8 } else { vs.drive };
    let (power_r, free_r, all) = match drive {
        0 => (t.wheel_back.radius, t.wheel_front.radius, false),
        1 => (t.wheel_front.radius, t.wheel_back.radius, false),
        _ => (t.wheel_front.radius, 0.0, true),
    };
    let power = fwd / power_r;
    let free = if all { fwd } else { fwd / free_r };
    vs.omega = [power, free];
    vs.diff = [1.0, 1.0];
    for (i, w) in vs.wheels.iter_mut().enumerate() {
        let front = i < 2;
        w.spin_rate = if all || (front == (drive == 1)) { power } else { free };
    }
    let g = if fwd < -1.0 {
        -1
    } else if fwd < 10.0 {
        1
    } else {
        let radii = match t.drivetrain_type {
            1 | 4 => vec![t.wheel_front.radius; 2],
            2 | 3 => vec![t.wheel_front.radius, t.wheel_front.radius, t.wheel_back.radius, t.wheel_back.radius],
            _ => vec![t.wheel_back.radius; 2],
        };
        let needed = super::transmission::ratio_for_speed(e.max_rpm * RPM, fwd * 2.237, &radii);
        let top = vt.ratios.len() as i32 - 1;
        (1..top).find(|&g| needed > vt.ratios[g as usize]).unwrap_or(top)
    };
    if g != vs.gear {
        let was = vs.gear;
        vs.gear = g;
        vs.shift_timer = 0.0;
        vs.gear_change.start(was, g, e.gcl);
    }
    let ratio = if g < 0 { -vt.ratios[0] } else { vt.ratios[g as usize] };
    let peak = vt.torque_curve.as_ref().map_or(e.opt_rpm * RPM, |c| c.peak().0);
    vs.engine_w = (ratio * power).min(peak).max(e.idle_rpm * RPM);
}

/// R4: the level's probe as the stand-up asks it (0x82574418), plainly written: the first polygon of the city
/// a segment meets - the point, and the polygon's normal as the collision forms it.
fn level_probe(world: &WorldCollider, from: Vec3, to: Vec3) -> Option<(Vec3, Vec3)> {
    let mut best: Option<(f32, Vec3, Vec3)> = None;
    let dir = to - from;
    world.triangles_of(from.min(to) - Vec3::splat(0.01), from.max(to) + Vec3::splat(0.01), |mesh, i| {
        let tri = &mesh.tris[i];
        let n = if mesh.faces.len() == mesh.tris.len() { mesh.faces[i] } else { (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero() };
        let den = n.dot(dir);
        if !(den < 0.0) {
            return;
        }
        let t = n.dot(tri[0] - from) / den;
        if !(0.0..=1.0).contains(&t) || best.is_some_and(|b| b.0 <= t) {
            return;
        }
        let p = from + dir * t;
        let inside = (0..3).all(|k| (tri[(k + 1) % 3] - tri[k]).cross(p - tri[k]).dot(n) >= -1e-6);
        if inside {
            if std::env::var_os("MCLA_DBG_PROBE").is_some() {
                let h = |v: Vec3| format!("{:08x} {:08x} {:08x}", v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
                eprintln!("PROBE t {:08x} point {} normal {} tri {} | {} | {}", t.to_bits(), h(p), h(n), h(tri[0]), h(tri[1]), h(tri[2]));
            }
            best = Some((t, p, n));
        }
    });
    best.map(|b| (b.1, b.2))
}

/// One of mcCarDamage's calls in a tick with something loose in the car's island (a run of the car's rows in
/// one of the island's two lists): the change of velocity, the first and last points, the first other's
/// multiplier, whether any row has an impulse, the normals' sum and the first's, and the first other's velocity
/// at the point and at its centre (none for the city).
struct DamageRun {
    dv: Vec3,
    first: Vec3,
    last: Vec3,
    mult: f32,
    any: bool,
    nsum: Vec3,
    n0: Vec3,
    v2: Vec3,
    v2c: Vec3,
}

pub(super) fn dbg_flag(name: &'static str) -> bool {
    static FLAGS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    FLAGS.get_or_init(|| ["MCLA_DBG_R1", "MCLA_DBG_R2", "MCLA_DBG_R3", "MCLA_DBG_R4", "MCLA_DBG_RB2", "MCLA_DBG_RB3", "MCLA_DBG_RB4", "MCLA_DBG_RB5", "MCLA_DBG_RB6", "MCLA_DBG_P1", "MCLA_DBG_P2", "MCLA_DBG_P3", "MCLA_DBG_RA1", "MCLA_DBG_RA2", "MCLA_DBG_RA3", "MCLA_DBG_RA4", "MCLA_DBG_RA5", "MCLA_DBG_RA6", "MCLA_DBG_RA7", "MCLA_DBG_RA8", "MCLA_DBG_RB1", "MCLA_DBG_RD1", "MCLA_DBG_A1", "MCLA_DBG_A2", "MCLA_DBG_A3", "MCLA_DBG_A4", "MCLA_DBG_A6", "MCLA_DBG_Z1", "MCLA_DBG_Z2", "MCLA_DBG_HIT", "MCLA_DBG_OLDCOLLIDE", "MCLA_DBG_NORESOLVE", "MCLA_DBG_NODAMP", "MCLA_DBG_NOROLLBAR", "MCLA_DBG_YAW", "MCLA_DBG_SPHERES", "MCLA_DBG_RAYWHEEL", "MCLA_DBG_HANG", "MCLA_DBG_GYRO", "MCLA_DBG_NOBOX", "MCLA_DBG_NOSTEEP", "MCLA_DBG_NOCAMBER", "MCLA_DBG_NOLEANPROBE", "MCLA_DBG_OLDWHEEL", "MCLA_DBG_ZEROTYRE", "MCLA_DBG_TERMS", "MCLA_DBG_SUBDAMP", "MCLA_DBG_NOFREEZE", "MCLA_DBG_STEEPWHEEL", "MCLA_DBG_NOPIVOT", "MCLA_DBG_NOSETTLE", "MCLA_DBG_NOSTUCK", "MCLA_DBG_STUCK", "MCLA_DBG_SYMSOLVE", "MCLA_DBG_OLDWHEELPOSE", "MCLA_DBG_OLDSUSP", "MCLA_DBG_OLDROWS", "MCLA_DBG_OLDTYREIN", "MCLA_DBG_OLDSUMS", "MCLA_DBG_OLDSTEP", "MCLA_DBG_OLDPOSE", "MCLA_DBG_AXLEORDER", "MCLA_DBG_CALLS", "MCLA_DBG_OLDGYRO", "MCLA_DBG_NOBOTTOM", "MCLA_DBG_WHEELORDER", "MCLA_DBG_OLDMELT", "MCLA_DBG_OLDHANG", "MCLA_DBG_GEAR", "MCLA_DBG_OLDORDER", "MCLA_DBG_OLDSTEER", "MCLA_DBG_OLDDOWN", "MCLA_DBG_OLDDRIFT", "MCLA_DBG_NOENGINEROLL", "MCLA_DBG_OLDAUTOBRAKE", "MCLA_DBG_Y1", "MCLA_DBG_Y2", "MCLA_DBG_Y3", "MCLA_DBG_Y4", "MCLA_DBG_Y5", "MCLA_DBG_Y6", "MCLA_DBG_Y7", "MCLA_DBG_Y8", "MCLA_DBG_Y9", "MCLA_DBG_TRACE", "MCLA_DBG_WORLDHIT", "MCLA_DBG_NOWORLDHIT", "MCLA_DBG_SCRAPEPOST"].into_iter().filter(|n| std::env::var(n).is_ok()).collect()).contains(&name)
}

fn dbg_angdrag() -> f32 {
    static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_ANGDRAG").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0))
}

fn no_gyro() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_NO_GYRO").is_ok())
}

/// experiment: MCLA_GYRO_FULL gives computer drivers the turn gyro at full blend
fn gyro_full() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_GYRO_FULL").is_ok())
}

#[cfg(test)]
mod crown_tests {
    /// The wheels' collision shapes as measured in the running game (depth + distance of the
    /// centre from the contact, on level ground): the middle of the tread, and the shoulders
    /// at the tyre's own radius.
    #[test]
    fn a_crowned_tyre_has_the_recorded_radius() {
        for (r, w, measured) in [(0.2953, 0.245, 0.3072), (0.3011, 0.305, 0.3190), (0.2950, 0.285, 0.3111), (0.3101, 0.335, 0.3309), (0.3312, 0.275, 0.3443), (0.3417, 0.305, 0.3573), (0.3803, 0.315, 0.3951), (0.3923, 0.355, 0.4104), (0.2791, 0.235, 0.2908), (0.3026, 0.235, 0.3134)] {
            assert!((super::crown(r, w, 0.0) - measured).abs() < 2.5e-4, "{r} {w}: {} for {measured}", super::crown(r, w, 0.0));
            assert!((super::crown(r, w, w * 0.5) - r).abs() < 1e-5);
        }
    }
}

#[cfg(test)]
mod contact_integration_tests {
    use super::*;

    #[test]
    #[ignore = "requires MCLA_DATA_DIR containing Complete Edition vehicle resources"]
    fn render_schedules_preserve_every_car_and_bike_physics_tick() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
        let assets = crate::assets::Assets::new(&root);
        let mut world = WorldCollider::default();
        world.sectors.insert(0, super::super::collision::SectorMesh::build(vec![
            [Vec3::new(-512.0, 0.0, -512.0), Vec3::new(-512.0, 0.0, 512.0), Vec3::new(512.0, 0.0, -512.0)],
            [Vec3::new(512.0, 0.0, -512.0), Vec3::new(-512.0, 0.0, 512.0), Vec3::new(512.0, 0.0, 512.0)],
        ]));
        for name in ["vp_vw_golf_83", "vp_kaw_ninja_zx14_06"] {
            let car = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle").join(name)).unwrap();
            let mut params = crate::vehicles::car_params(&car);
            let mut tune = VehTune::new(car.simtune.as_ref().unwrap().base.clone());
            tune.bike = car.bike;
            params.veh = Some(std::sync::Arc::new(tune));
            let advance = |state: &mut CarState, tick: usize| {
                // Paused ticks advance UI time but not physics; Zone scales the fixed tick.
                if (120..150).contains(&tick) { return; }
                let dt = crate::fixed_step::DT * if (150..210).contains(&tick) { 0.2 } else { 1.0 };
                let input = if tick < 30 { CarInput::default() }
                    else if tick < 260 { CarInput { throttle: 1.0, ..Default::default() } }
                    else { CarInput { brake: 1.0, steer: 0.25, ..Default::default() } };
                state.step(&params, input, &world, dt);
                if tick == 300 { state.correct_position(Vec3::new(0.4, 0.1, -0.3)); }
            };
            let mut initial = CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            for _ in 0..180 { initial.step(&params, CarInput::default(), &world, crate::fixed_step::DT); }
            let mut baseline = initial.clone();
            let expected: Vec<_> = (0..360).map(|tick| {
                advance(&mut baseline, tick);
                // Includes complete engine, shaft, contact and tyre state, not just position.
                format!("{baseline:?}")
            }).collect();
            for fps in [30, 60, 144, 240, 400, 0] {
                let mut state = initial.clone();
                let mut clock = crate::fixed_step::Clock::default();
                let mut history = crate::vehicle_pose::History::default();
                let mut tick = 0;
                let frames = if fps == 0 { 180 * 3 } else { fps * 6 };
                for frame in 0..frames {
                    let elapsed = if fps == 0 { [0.001, 0.029, 1.0 / 300.0][frame % 3] } else { 1.0 / fps as f64 };
                    for _ in 0..clock.advance(elapsed) {
                        history.begin(&state, 0.0);
                        advance(&mut state, tick);
                        history.finish(&state, 0.0);
                        assert_eq!(format!("{state:?}"), expected[tick], "{name}, {fps} fps, tick {tick}");
                        tick += 1;
                    }
                    let before_render = format!("{state:?}");
                    let pose = history.sample(&state, 0.0, clock.alpha());
                    for i in 0..4 {
                        let axle = if car.bike { (params.wheel_pos[i / 2 * 2] + params.wheel_pos[i / 2 * 2 + 1]) * 0.5 } else { params.wheel_pos[i] };
                        assert!((pose.hubs[i].x - axle.x).abs() < 0.001 && (pose.hubs[i].z - axle.z).abs() < 0.001,
                            "{name}, {fps} fps, wheel {i} detached during interpolation");
                    }
                    assert_eq!(format!("{state:?}"), before_render);
                }
                assert_eq!(tick, 360, "{name}, {fps} fps");
            }
        }
    }

    #[test]
    #[ignore = "requires MCLA_DATA_DIR containing Complete Edition vehicle resources"]
    fn automatic_shift_queues_the_next_engine_frame_without_spending_its_timer() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
        let assets = crate::assets::Assets::new(&root);
        let car = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle/vp_vw_golf_83")).unwrap();
        let mut params = crate::vehicles::car_params(&car);
        let tune = VehTune::new(car.simtune.as_ref().unwrap().base.clone());
        let gcl = tune.t.engine.s.gcl;
        params.veh = Some(std::sync::Arc::new(tune));
        let mut world = WorldCollider::default();
        world.sectors.insert(0, super::super::collision::SectorMesh::build(vec![
            [Vec3::new(-64.0, 0.0, -512.0), Vec3::new(-64.0, 0.0, 512.0), Vec3::new(64.0, 0.0, -512.0)],
            [Vec3::new(64.0, 0.0, -512.0), Vec3::new(-64.0, 0.0, 512.0), Vec3::new(64.0, 0.0, 512.0)],
        ]));
        let mut state = CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
        let dt = 1.0 / 60.0;
        for _ in 0..180 { state.step(&params, CarInput::default(), &world, dt); }
        let input = CarInput { throttle: 1.0, ..Default::default() };
        for _ in 0..600 {
            let old_gear = state.gear;
            state.step(&params, input, &world, dt);
            if state.gear > old_gear {
                let engine = state.veh.as_ref().unwrap();
                assert_eq!(engine.engine_pedal, 1.0);
                assert_eq!(engine.gear_change.remaining, gcl);
                assert_eq!(engine.gear_change.direction, 1);
                state.step(&params, input, &world, dt);
                assert_eq!(state.veh.as_ref().unwrap().gear_change.remaining, gcl - dt);
                return;
            }
        }
        panic!("Golf failed to reach its first automatic upshift");
    }

    /// A2: the player's triggers through mcPlayerInput's pedal path in normal play - the brake trigger
    /// brakes (with a hundredth of throttle), takes reverse once the car is slow, then drives it
    /// backwards; the throttle trigger brakes that, takes the forward gears again and drives off.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR containing Complete Edition vehicle resources"]
    fn the_players_triggers_brake_reverse_and_drive_off_again() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
        let assets = crate::assets::Assets::new(&root);
        let car = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle/vp_vw_golf_83")).unwrap();
        let mut params = crate::vehicles::car_params(&car);
        params.veh = Some(std::sync::Arc::new(VehTune::new(car.simtune.as_ref().unwrap().base.clone())));
        let mut world = WorldCollider::default();
        world.sectors.insert(0, super::super::collision::SectorMesh::build(vec![
            [Vec3::new(-64.0, 0.0, -512.0), Vec3::new(-64.0, 0.0, 512.0), Vec3::new(64.0, 0.0, -512.0)],
            [Vec3::new(64.0, 0.0, -512.0), Vec3::new(-64.0, 0.0, 512.0), Vec3::new(64.0, 0.0, 512.0)],
        ]));
        let mut state = CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
        let dt = 1.0 / 60.0;
        for _ in 0..180 { state.step(&params, CarInput::default(), &world, dt); }
        for _ in 0..300 { state.step(&params, CarInput { throttle: 1.0, ..Default::default() }, &world, dt); }
        let fast = state.fwd_speed();
        assert!(fast > 10.0 && state.gear > 0, "the car drives off: {fast}");
        // the brake trigger at speed: state 3, the chassis's throttle a hundredth through ThrottleInputExp
        for _ in 0..4 { state.step(&params, CarInput { brake: 1.0, ..Default::default() }, &world, dt); }
        let pad = state.veh.as_ref().unwrap().pad;
        assert_eq!((pad.pedals.state, pad.pedals.way), (3, 1));
        assert!(pad.pedals.throttle > 0.0 && pad.pedals.throttle < 0.2 && pad.pedals.brake == 1.0, "{:?}", pad.pedals);
        let mut reversed_at = None;
        for k in 0..900 {
            state.step(&params, CarInput { brake: 1.0, ..Default::default() }, &world, dt);
            if reversed_at.is_none() && state.gear < 0 { reversed_at = Some((k, state.fwd_speed())); }
            if std::env::var_os("MCLA_PAD_TRACE").is_some() && k % 30 == 0 { let vs = state.veh.as_ref().unwrap(); println!("k {k} v {:.3} gear {} pad {:?} way_after {}", state.fwd_speed(), state.gear, vs.pad.pedals, vs.way_after); }
        }
        let (_, v) = reversed_at.expect("reverse is taken");
        assert!(v.abs() < 1.5, "reverse only once the car is slow: {v}");
        assert!(state.fwd_speed() < -2.0, "and the brake trigger then drives it backwards: {}", state.fwd_speed());
        assert_eq!(state.veh.as_ref().unwrap().pad.pedals.state, 2);
        for _ in 0..600 { state.step(&params, CarInput { throttle: 1.0, ..Default::default() }, &world, dt); }
        assert!(state.gear > 0 && state.fwd_speed() > 5.0, "the throttle trigger stops it and drives off: gear {} {}", state.gear, state.fwd_speed());
    }

    #[test]
    #[ignore = "requires MCLA_DATA_DIR containing Complete Edition vehicle resources"]
    fn wheel_hubs_stay_on_axles_during_acceleration_braking_and_airborne_turns() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
        let assets = crate::assets::Assets::new(&root);
        let mut world = WorldCollider::default();
        world.sectors.insert(0, super::super::collision::SectorMesh::build(vec![
            [Vec3::new(-64.0, 0.0, -512.0), Vec3::new(-64.0, 0.0, 512.0), Vec3::new(64.0, 0.0, -512.0)],
            [Vec3::new(64.0, 0.0, -512.0), Vec3::new(-64.0, 0.0, 512.0), Vec3::new(64.0, 0.0, 512.0)],
        ]));
        for name in ["vp_vw_golf_83", "vp_kaw_ninja_zx14_06"] {
            let car = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle").join(name)).unwrap();
            let mut params = crate::vehicles::car_params(&car);
            let mut tune = VehTune::new(car.simtune.as_ref().unwrap().base.clone());
            tune.bike = car.bike;
            params.veh = Some(std::sync::Arc::new(tune));
            for (speed, height) in [(0.0, 0.02), (30.0, 0.02), (90.0, 10.0)] {
                let mut state = CarState::new(Vec3::new(0.0, height, 0.0), 0.0);
                state.vel = state.forward() * speed;
                if height > 1.0 { state.ang_vel = Vec3::new(0.5, 0.8, 0.2); }
                for frame in 0..180 {
                    let input = if frame < 90 { CarInput { throttle: 1.0, ..Default::default() } }
                        else { CarInput { brake: 1.0, steer: 0.25, ..Default::default() } };
                    state.step(&params, input, &world, 1.0 / 60.0);
                    if frame == 60 || frame == 120 {
                        let contacts = state.wheels.map(|w| w.contact);
                        // Car separation runs after the physics update, before upload.
                        state.correct_position(Vec3::new(0.4, 0.1, -0.3));
                        assert_eq!(state.wheels.map(|w| w.contact), contacts);
                    }
                    for i in 0..4 {
                        let local = state.rot.inverse() * (state.wheels[i].world_pos - state.pos);
                        let axle = if car.bike {
                            let even = i / 2 * 2;
                            (params.wheel_pos[even] + params.wheel_pos[even + 1]) * 0.5
                        } else { params.wheel_pos[i] };
                        assert!((local.x - axle.x).abs() < 0.001 && (local.z - axle.z).abs() < 0.001,
                            "{name}, initial speed {speed}, frame {frame}, wheel {i}: hub {local:?}, axle {axle:?}");
                    }
                }
            }
        }
    }

    /// N-VP-3: the drawn tyre (radius + 6.35 mm about the drawn hub) of a standing car meets the
    /// ground as the crown does: its lowest point 0 - 10 mm above the plane, lowered set-ups too.
    /// (Before 2026-10-19 the drawn hubs left out SuspensionOffset: the Range Rover's tournament
    /// set-up, -0.125 / -0.1125, had its tyres drawn 0.12 / 0.10 m into the road.)
    #[test]
    #[ignore = "requires MCLA_DATA_DIR containing Complete Edition vehicle resources"]
    fn drawn_tyres_of_lowered_set_ups_stand_on_the_ground() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
        let assets = crate::assets::Assets::new(&root);
        let mut world = WorldCollider::default();
        world.sectors.insert(0, super::super::collision::SectorMesh::build(vec![
            [Vec3::new(-64.0, 0.0, -64.0), Vec3::new(-64.0, 0.0, 64.0), Vec3::new(64.0, 0.0, -64.0)],
            [Vec3::new(64.0, 0.0, -64.0), Vec3::new(-64.0, 0.0, 64.0), Vec3::new(64.0, 0.0, 64.0)],
        ]));
        for (name, cfg) in [("vpd_lr_sport_08", "SC_tournament_vpd_lr_sport_08"), ("vpd_lr_sport_08", "vpd_lr_sport_08"), ("vp_aud_r8_08", "vp_aud_r8_08"), ("vp_aud_r8_08", "SC_GH_33_3_vp_aud_r8_08"), ("vp_sln_s281_06", "GH_08_3_vp_sln_s281_06"), ("vp_lam_murcielago_06", "GH_04_3_vp_lam_murcielago_06"), ("vp_duc_999r_06", "GH_16_4_vp_duc_999r_06")] {
            let car = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle").join(name)).unwrap();
            let mut params = crate::vehicles::car_params(&car);
            let s = car.simtune.as_ref().unwrap();
            let bytes = std::fs::read(root.join("resources/vehicle/configs").join(format!("{cfg}.xcc"))).unwrap();
            let fit = (**s).clone().fit(&bytes).unwrap();
            let mut t = s.base.clone();
            for (axle, wt) in [&mut t.wheel_front, &mut t.wheel_back].into_iter().enumerate() {
                wt.radius = fit.radius[axle];
                wt.width = fit.width[axle];
                wt.suspension_offset = fit.offset[axle];
            }
            for (i, w) in params.wheel_pos.iter_mut().enumerate() {
                if fit.half_track[i / 2] > 0.0 { w.x = w.x.signum() * fit.half_track[i / 2]; }
            }
            let mut tune = VehTune::new(t);
            tune.bike = car.bike;
            if let Some(hubs) = car.hubs { tune.seat((hubs[0].y + hubs[2].y) * 0.5); }
            let radius = [tune.t.wheel_front.radius, tune.t.wheel_back.radius];
            params.veh = Some(std::sync::Arc::new(tune));
            let mut state = CarState::new(Vec3::new(0.0, 0.3, 0.0), 0.0);
            for _ in 0..240 { state.step(&params, CarInput::default(), &world, 1.0 / 60.0); }
            for i in 0..4 {
                let up = state.rot * Vec3::Y;
                let bottom = state.wheels[i].world_pos - up * (radius[i / 2] + 0.00635);
                eprintln!("{name} {cfg} offset {:?} wheel {i}: drawn tyre bottom {:.4} m", fit.offset, bottom.y);
                assert!((0.0..0.012).contains(&bottom.y), "{name} {cfg} wheel {i}: drawn tyre bottom at {:.4} m", bottom.y);
            }
        }
    }

    #[test]
    #[ignore = "requires MCLA_DATA_DIR containing Complete Edition vehicle resources"]
    fn disc_cars_and_bikes_have_their_physical_wheel_count_and_static_load() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR"));
        let assets = crate::assets::Assets::new(&root);
        let mut world = WorldCollider::default();
        world.sectors.insert(0, super::super::collision::SectorMesh::build(vec![
            [Vec3::new(-32.0, 0.0, -32.0), Vec3::new(-32.0, 0.0, 32.0), Vec3::new(32.0, 0.0, -32.0)],
            [Vec3::new(32.0, 0.0, -32.0), Vec3::new(-32.0, 0.0, 32.0), Vec3::new(32.0, 0.0, 32.0)],
        ]));
        for name in ["vp_vw_golf_83", "vp_kaw_ninja_zx14_06", "vpd_duc_1098_07"] {
            let car = crate::vehicle_model::load_car(&assets, &root.join("resources/vehicle").join(name)).unwrap();
            let mut params = crate::vehicles::car_params(&car);
            let mut tune = VehTune::new(car.simtune.as_ref().unwrap().base.clone());
            tune.bike = car.bike;
            let weight = tune.t.s.mass * tune.gravity;
            params.veh = Some(std::sync::Arc::new(tune));
            let mut state = CarState::new(Vec3::new(0.0, 0.02, 0.0), 0.0);
            for _ in 0..180 { state.step(&params, CarInput::default(), &world, 1.0 / 60.0); }
            let load: f32 = state.wheels.iter().map(|w| w.load).sum();
            assert_eq!(state.wheels.iter().filter(|w| w.grounded).count(), if car.bike { 2 } else { 4 }, "{name}");
            assert!((load / weight - 1.0).abs() < 0.02, "{name}: load {load}, weight {weight}");
            assert!(state.speed() < 0.05 && state.ang_vel.length() < 0.01, "{name}: {state:?}");
            if car.bike {
                for i in [0, 2] {
                    assert_eq!(state.wheels[i].world_pos, state.wheels[i + 1].world_pos);
                    assert_eq!(state.wheels[i + 1].load, 0.0);
                    assert_eq!(state.veh.as_ref().unwrap().wheels[i + 1].load, 0.0);
                }
            }
            // Leaving the collision surface must clear contact forces and elastic state.
            state.pos.y += 10.0;
            state.step(&params, CarInput::default(), &world, 1.0 / 60.0);
            assert!(state.wheels.iter().all(|w| !w.grounded && w.load == 0.0));
            assert!(state.veh.as_ref().unwrap().wheels.iter().all(|w|
                w.load == 0.0 && w.tyre_torque == 0.0 && w.disp_lat == 0.0 && w.disp_long == 0.0));
        }
    }
}

#[cfg(test)]
mod transmission_reference_tests {
    use super::*;

    #[test]
    fn portable_vantage_gearing_matches_original_capture() {
        let mut tune = SimTune::default();
        tune.engine.s.idle_rpm = 1000.0; tune.engine.s.max_rpm = 7800.0;
        tune.engine.torque = vec![170.0,199.0,232.0,261.0,286.0,302.0,312.0,308.0,254.0,0.0];
        tune.engine.idle = vec![100.0,0.0,-100.0,-200.0,-300.0,-400.0,-500.0,-600.0,-700.0,-800.0];
        tune.trans.gears = 8;
        tune.trans.s.rev_gear_mph = 45.0; tune.trans.s.low_gear_mph = 45.0;
        tune.trans.s.high_gear_mph = 214.0; tune.trans.s.gear_bias = 0.55;
        tune.trans.s.upshift_bias_low = 0.4; tune.trans.s.downshift_bias_min = 0.1;
        tune.trans.s.downshift_bias_max = 0.5;
        tune.wheel_front.radius = 0.318; tune.wheel_back.radius = 0.318;
        let vt = VehTune::new(tune);
        let ratios: [f32; 7] = [12.9119387,12.9119387,8.24057388,5.63274813,4.12363338,3.23323035,2.71512699];
        for (&a, e) in vt.ratios.iter().zip(ratios) { assert_eq!(a.to_bits(), e.to_bits()); }
        let shifts: [[f32;4];6] = [
            [7175.31348,4305.18848,0.0,0.0],
            [7123.02783,4273.81689,4121.44336,2289.69092],
            [7063.44727,4238.06836,4381.97607,2434.43115],
            [6991.70703,4195.02441,4653.91992,2585.51123],
            [6893.8252,4136.29541,4933.80859,2741.00488],
            [7800.0,7800.0,5210.22217,2894.56812],
        ];
        assert_eq!(vt.shift[0], [0.0;4]);
        for (a, e) in vt.shift[1..].iter().zip(shifts) {
            for (&a, e) in a.iter().zip(e) { assert_eq!(a.to_bits(), e.to_bits()); }
        }
    }

    #[test]
    #[ignore = "requires MCLA_TRANSMISSION_REFERENCE from tools/transmission_reference.py"]
    fn transmission_initialization_matches_original_generated_code() {
        let csv = std::fs::read_to_string(std::env::var("MCLA_TRANSMISSION_REFERENCE").expect("MCLA_TRANSMISSION_REFERENCE")).unwrap();
        let (mut cases, mut ratios, mut shifts) = (0, 0, 0);
        let (mut ratio_error, mut shift_error) = (0.0_f32, 0.0_f32);
        for line in csv.lines().skip(1) {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            let (n, gears) = (p[0] as usize, p[3] as usize);
            assert_eq!(p.len(), 42+gears*5+2);
            let mut tune = SimTune::default();
            tune.engine.s.idle_rpm = p[1];
            tune.engine.s.max_rpm = p[2];
            tune.engine.torque = p[18..18+n].to_vec();
            tune.engine.idle = p[30..30+n].to_vec();
            tune.trans.gears = gears as u32;
            let tr = &mut tune.trans.s;
            tr.rev_gear_mph = p[4]; tr.low_gear_mph = p[5]; tr.high_gear_mph = p[6];
            tr.upshift_bias = p[7]; tr.upshift_bias_low = p[8];
            tr.downshift_bias_min = p[9]; tr.downshift_bias_max = p[10]; tr.gear_bias = p[12];
            tune.drivetrain_type = if p[13] == 4.0 { 2 } else { 0 };
            tune.wheel_front.radius = p[14];
            tune.wheel_back.radius = if p[13] == 4.0 { p[16] } else { p[14] };
            let radii = &p[14..14+p[13] as usize];
            let prepared_ratios = super::super::transmission::ratios(&tune.trans, p[2] * RPM, radii);
            let mut vt = VehTune::new(tune);
            if cases < 128 {
                // Actual disc layouts must also select/average the same radii.
                assert_eq!(vt.ratios, prepared_ratios);
            }
            vt.ratios = prepared_ratios;
            (vt.shift, vt.sync_torque) = vt.shift_points();
            for (g, &ratio) in vt.ratios.iter().enumerate() {
                let original = if g == 0 { 0 } else { g+1 };
                let expected = p[42+original*5];
                let value = if g == 0 { -ratio } else { ratio };
                ratios += (value.to_bits()!=expected.to_bits()) as usize;
                ratio_error = ratio_error.max((value-expected).abs());
                if g > 0 {
                    for (a, e) in vt.shift[g].iter().zip(&p[43+original*5..47+original*5]) {
                        shifts += (a.to_bits()!=e.to_bits()) as usize;
                        shift_error = shift_error.max((a-e).abs());
                    }
                }
            }
            assert_eq!(vt.sync_torque.to_bits(), p[p.len()-1].to_bits());
            cases += 1;
        }
        println!("{cases} transmission cases; ratio mismatches {ratios} max error {ratio_error}; shift mismatches {shifts} max error {shift_error}");
        assert_eq!(cases, 1152);
        assert_eq!((ratios, shifts), (0,0));
    }
}

#[cfg(test)]
mod engine_curve_tests {
    #[test]
    #[ignore = "requires MCLA_ENGINE_CURVE_REFERENCE from tools/engine_curve_reference.py"]
    fn engine_curves_match_original_generated_code() {
        let path = std::env::var("MCLA_ENGINE_CURVE_REFERENCE").expect("MCLA_ENGINE_CURVE_REFERENCE");
        let csv = std::fs::read_to_string(path).unwrap();
        let (mut count, mut mismatches, mut max_error) = (0, 0, 0.0_f32);
        for line in csv.lines().skip(1) {
            let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            assert_eq!(p.len(), 80);
            let n = p[1] as usize;
            let offset = if p[0] == 0.0 { 5 } else { 17 };
            let curve = super::super::engine_curve::Curve::new(&p[offset..offset+n], p[2], p[3], p[0] != 0.0);
            for (i, c) in curve.coefficients.iter().enumerate() {
                for (j, value) in c.iter().enumerate() {
                    assert_eq!(value.to_bits(), p[29+i*4+j].to_bits(), "coefficient {i}/{j}: {line}");
                }
            }
            let result = curve.evaluate(p[4]);
            let torque_curve = super::super::engine_curve::Curve::new(&p[5..5+n], p[2], p[3], false);
            let (peak, torque) = torque_curve.peak();
            assert_eq!(peak.to_bits(), p[78].to_bits(), "peak: {line}");
            assert_eq!(torque.to_bits(), p[79].to_bits(), "sync torque: {line}");
            if result.to_bits() != p[77].to_bits() { mismatches += 1; }
            max_error = max_error.max((result-p[77]).abs());
            count += 1;
        }
        println!("{count} curve cases, {mismatches} mismatches, maximum absolute error {max_error}");
        assert_eq!(count, 14080);
        assert_eq!(mismatches, 0);
    }
}

#[cfg(test)]
mod gear_change_tests {
    use super::*;
    use mcla_formats::rage::simtune::EngineScalars;

    fn engine() -> EngineScalars {
        EngineScalars { gcl: 1.0, upshift_throttle: 0.5, downshift_throttle: 1.0, ..Default::default() }
    }

    #[test]
    fn upshift_waits_for_rpm_match_and_downshift_blips_the_pedal() {
        let tune = engine();
        let mut up = GearChange::default();
        up.start(1, 2, tune.gcl);
        for _ in 0..12 {
            up.update(1.0, 2, 5, 8.0, 700.0, 50.0, &tune, 25.0, 1.0 / 60.0);
        }
        assert!(up.active, "RPM mismatch must keep the shift running past the old 0.12 s cap");
        up.update(1.0, 2, 5, 8.0, 400.0, 50.0, &tune, 25.0, 1.0 / 60.0);
        assert_eq!(up.remaining, 0.0);
        up.update(1.0, 2, 5, 8.0, 400.0, 50.0, &tune, 25.0, 1.0 / 60.0);
        assert!(!up.active);
        assert_eq!(up.direction, 0);

        let mut down = GearChange::default();
        down.start(4, 2, tune.gcl);
        assert_eq!(down.update(0.3, 2, 5, 8.0, 100.0, 50.0, &tune, 25.0, 1.0 / 60.0), 1.0);
        assert!(down.active);
    }

    #[test]
    fn shift_duration_is_bounded_and_first_gear_preserves_the_pedal() {
        let tune = EngineScalars { gcl: 0.12, ..engine() };
        let mut shift = GearChange::default();
        shift.start(2, 1, tune.gcl);
        for _ in 0..8 {
            assert_eq!(shift.update(0.3, 1, 5, 8.0, 100.0, 50.0, &tune, 25.0, 1.0 / 60.0), 0.3);
        }
        assert!(!shift.active);
        assert_eq!(shift.direction, 0);
    }

    #[test]
    #[ignore = "requires tools/shift_reference.py; set MCLA_SHIFT_REFERENCE"]
    fn shift_state_matches_original_generated_code() {
        let path = std::env::var("MCLA_SHIFT_REFERENCE").expect("set MCLA_SHIFT_REFERENCE to the original CSV");
        let csv = std::fs::read_to_string(path).unwrap();
        let mut count = 0;
        for line in csv.lines().skip(1) {
            let p: Vec<f32> = line.split(',').map(|x| x.parse().unwrap()).collect();
            let mut shift = GearChange { direction: p[0] as i8, remaining: p[3], active: true };
            let tune = EngineScalars { gcl: p[5], upshift_throttle: p[6], downshift_throttle: p[7], ..Default::default() };
            let pedal = shift.update(p[4], p[1] as i32, p[2] as usize, p[8], p[10], p[11], &tune, p[9], p[12]);
            assert!((pedal - p[13]).abs() < 1e-6, "pedal case {count}: {line}, Rust {pedal}");
            assert!((shift.remaining - p[14]).abs() < 1e-6, "timer case {count}: {line}");
            assert_eq!(shift.active, p[15] != 0.0, "latch case {count}: {line}");
            assert_eq!(shift.direction, p[16] as i8, "direction case {count}: {line}");
            count += 1;
        }
        assert_eq!(count, 1200);
    }
}

#[cfg(test)]
mod primitive_reference_tests {
    use super::*;

    #[test]
    fn portable_regression_values_from_original_scalar_code() {
        // Captured by physics_reference.py from the original scalar instructions.
        assert_eq!(synchronize_engine(186.759445, 917.835693, 376.0, 0.06, 1.0 / 30.0).to_bits(),
            395.648346f32.to_bits());
        assert_eq!(synchronize_engine(905.476318, 193.574249, 376.0, 0.01, 1.0 / 30.0).to_bits(),
            193.574249f32.to_bits());
        let wheel = WheelTune { optimum_slip: 0.07, static_fric: 1.85, ..Default::default() };
        assert_eq!(friction(&wheel, 0.0, 0.01, 0.035).0.to_bits(), 0.013875002f32.to_bits());
    }

    #[test]
    #[ignore = "requires tools/physics_reference.py; set MCLA_PHYSICS_REFERENCE"]
    fn scalar_vehicle_primitives_match_original_generated_code() {
        let path = std::env::var("MCLA_PHYSICS_REFERENCE").expect("set MCLA_PHYSICS_REFERENCE to the original CSV");
        let csv = std::fs::read_to_string(path).unwrap();
        let mut counts = std::collections::BTreeMap::<String, (usize, usize, u32)>::new();
        for line in csv.lines().skip(1) {
            let fields: Vec<_> = line.split(',').collect();
            let kind = fields[0];
            let p: Vec<f32> = fields[1..].iter().map(|s| s.parse().unwrap()).collect();
            let (actual, flag) = match kind {
                "sync" => ([synchronize_engine(p[0], p[1], p[2], p[3], p[4]), 0.0, 0.0], false),
                "slip" => ([slip(p[0], p[1]), 0.0, 0.0], false),
                "friction" => {
                    let wt = WheelTune { optimum_slip: p[0], static_fric: p[1], ..Default::default() };
                    let (mu, slide) = friction(&wt, p[2], p[3], p[4]);
                    ([mu, slide, 0.0], false)
                }
                "displace" => {
                    let mut disp = p[0];
                    let (step, grip) = displace(&mut disp, p[1], p[2], p[3]);
                    ([disp, step, 0.0], grip)
                }
                _ => panic!("unknown primitive {kind}"),
            };
            let expected = [p[6], p[7], p[8]];
            let max_ulp = actual.iter().zip(expected).map(|(a, b)| a.to_bits().abs_diff(b.to_bits())).max().unwrap();
            let mismatch = max_ulp != 0 || flag != (p[9] != 0.0);
            let entry = counts.entry(kind.to_string()).or_default();
            entry.0 += 1;
            entry.1 += usize::from(mismatch);
            entry.2 = entry.2.max(max_ulp);
            if mismatch && entry.1 <= 2 {
                eprintln!("{kind} mismatch: {line}, Rust {actual:?} flag {flag}");
            }
        }
        eprintln!("primitive cases (total, mismatches, maximum ULP): {counts:?}");
        assert_eq!(counts.len(), 4);
        assert_eq!(counts["sync"].0, 1600);
        assert_eq!(counts["slip"].0, 196);
        assert_eq!(counts["friction"].0, 11776);
        assert_eq!(counts["displace"].0, 1600);
        assert!(counts.values().all(|c| c.1 == 0), "original-code differences: {counts:?}");
    }
}

/// A body's velocity at the point `p` as the tick's solve starts from it (its velocity and turning, no contact
/// impulses, not the tick's pending forces) - what mcCarDamage's contact callback reads at the point from the
/// collider (0x82343CA8, vslot 21). MCLA_DBG_SCRAPEPOST: the velocity after the solve, as before 2026-10-20.
fn start_velocity(body: Option<&super::solver::Body>, p: Vec3) -> Option<Vec3> {
    let b = body.filter(|_| !dbg_flag("MCLA_DBG_SCRAPEPOST"))?;
    Some(b.vel + b.ang.cross(p - b.at))
}
