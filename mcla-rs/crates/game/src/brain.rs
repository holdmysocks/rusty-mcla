//! A computer racer's brain as the original's mcBrainRacing runs it (decoded 2026-10-21, docs/AI.md;
//! rule file work/racerai/brain.md): goals from the race's checkpoints, routes over the navigation mesh,
//! the aiv race line (`aiv.rs`), the rubber band's cap (`rubber.rs`), nitro, stuck recovery, and the
//! pedals through mcOpponentInput. Being ported.

/// The pedals a brain asks for, as mcOpponentInput holds them (+12 throttle, +16 brake, +20 handbrake,
/// +24 steer) and as it has moved them so far (+152 .. +164).
#[derive(Debug, Clone, Copy, Default)]
pub struct Pedals {
    pub throttle: f32,
    pub brake: f32,
    pub handbrake: f32,
    pub steer: f32,
}

/// mcOpponentInput (vtable 0x820384CC, update sub_822B91F0): moves its pedals towards the brain's at
/// fixed rates (its tune block, sub_822B90B8): steer 4 /s towards the centre and 6 /s away from it (half
/// while the brain's collision timer runs), throttle 35 /s, brake 2 /s on and 30 /s off, handbrake 10 /s;
/// any throttle takes the brake off.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpponentInput {
    pub cur: Pedals,
}

fn approach(cur: f32, target: f32, step: f32) -> f32 {
    if cur < target {
        (cur + step).min(target)
    } else {
        (cur - step).max(target)
    }
}

impl OpponentInput {
    pub const STEER_BACK: f32 = 4.0;
    pub const STEER_OUT: f32 = 6.0;
    pub const THROTTLE: f32 = 35.0;
    pub const BRAKE_ON: f32 = 2.0;
    pub const BRAKE_OFF: f32 = 30.0;
    pub const HANDBRAKE: f32 = 10.0;

    pub fn update(&mut self, target: Pedals, collision: bool, dt: f32) -> Pedals {
        let c = &mut self.cur;
        // ((cur - target) x cur >= 0: the wheel comes back towards the middle)
        let mut rate = if (c.steer - target.steer) * c.steer >= 0.0 { Self::STEER_BACK } else { Self::STEER_OUT };
        if collision {
            rate *= 0.5;
        }
        c.steer = approach(c.steer, target.steer, rate * dt);
        c.throttle = approach(c.throttle, target.throttle, Self::THROTTLE * dt);
        let br = if target.brake > c.brake { Self::BRAKE_ON } else { Self::BRAKE_OFF };
        c.brake = approach(c.brake, target.brake, br * dt);
        c.handbrake = approach(c.handbrake, target.handbrake, Self::HANDBRAKE * dt);
        if c.throttle > 0.0 {
            c.brake = 0.0;
        }
        *c
    }
}

/// The brain's stuck detector (sub_822BA468; +340 state, +344 timer, +352 anchor; +368 2.0 s, +372 1.0 s,
/// +376 5.0 m): a racer that has stayed within 5 m of a spot at no more than 10 m/s for 2 s backs up
/// straight for 1 s, then turns towards the line on full lock with the handbrake until it points within
/// 45 degrees of it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stuck {
    pub state: u8,
    pub timer: f32,
    pub anchor: glam::Vec3,
}

/// What the stuck detector wants this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Manoeuvre {
    /// +336: back up
    pub reverse: bool,
    /// +332: 0 none, 1 turn one way (steer -1), 2 the other (steer +1)
    pub turn: u8,
    /// +337: with the handbrake
    pub handbrake: bool,
}

impl Stuck {
    pub const TIME: f32 = 2.0;
    pub const REVERSE: f32 = 1.0;
    pub const RADIUS: f32 = 5.0;
    pub const MOVING: f32 = 10.0;

    /// `angle_to_line`: signed plan angle from the car's forward to the race line's next point
    /// (sub_8216C9C0 of sub_8254C888), `angle_to_corridor` to the corridor point ahead (sub_8254C8C0),
    /// used when the first is past 135 degrees.
    pub fn update(&mut self, m: &mut Manoeuvre, pos: glam::Vec3, speed: f32, angle_to_line: f32, angle_to_corridor: f32, dt: f32) {
        self.timer += dt;
        // (moved: more than 5 m from the anchor, or faster than 10 m/s - tested in states 0 and 1 only;
        // a state change falls through into the next state's test in the same call)
        let moved = |s: &Self| pos.distance_squared(s.anchor) > Self::RADIUS * Self::RADIUS || speed > Self::MOVING;
        if self.state == 0 {
            if moved(self) {
                self.timer = 0.0;
                self.anchor = pos;
            } else if self.timer > Self::TIME {
                self.timer = 0.0;
                self.state = 1;
            }
        }
        if self.state == 1 {
            if moved(self) {
                self.timer = 0.0;
                self.anchor = pos;
                self.state = 0;
                m.reverse = false;
            } else if self.timer > Self::REVERSE {
                self.timer = 0.0;
                self.state = 2;
                m.reverse = false;
            } else {
                m.reverse = true;
            }
        }
        if self.state == 2 {
            // (the corridor's angle only chooses the way to turn; the end test is on the line's)
            let angle = angle_to_line;
            if m.turn == 0 {
                let a = if angle.abs() > 135f32.to_radians() { angle_to_corridor } else { angle };
                m.turn = if a >= 0.0 { 1 } else { 2 };
            }
            let matching = (m.turn == 1) == (angle >= 0.0);
            if angle.abs() < 45f32.to_radians() && matching {
                // (+344 = 0, +332 = 0, +340 = 0; +337 is left as it is - it only counts while +332 / +336 is set)
                self.timer = 0.0;
                m.turn = 0;
                self.state = 0;
            } else {
                m.handbrake = true;
            }
        }
    }
}

/// The pre-race revving (sub_822BCFE0 case 2; brain +254 enabled, +255 handbrake, +276 rpm seen, +280 timer,
/// +284 rev-up target, +288 rev-down threshold): the engine's rpm as the brain follows it (6000 rpm/s
/// towards the real one), throttle up to a random target and held a random time, then off down to a
/// random threshold and a random pause. The draws are the shared MWC's uniforms.
#[derive(Debug, Clone, Copy)]
pub struct Revving {
    pub enabled: bool,
    pub handbrake: bool,
    pub rpm: f32,
    pub timer: f32,
    pub up: f32,
    pub down: f32,
}

impl Default for Revving {
    fn default() -> Self {
        Revving { enabled: false, handbrake: false, rpm: 0.0, timer: 0.0, up: -1.0, down: -1.0 }
    }
}

/// The engine figures the rev cycle uses (vehEngine +76 IdleRPM, +72 OptRPM, +68 MaxRPM; mcCarSimTune
/// +252 RevLimitMin, +256 RevLimitMax).
#[derive(Debug, Clone, Copy, Default)]
pub struct RevEngine {
    pub idle: f32,
    pub opt: f32,
    pub max: f32,
    pub limit_min: f32,
    pub limit_max: f32,
}

impl Revving {
    /// SetRevvingEnabled (sub_822BB0C0): on resets the cycle; either way the timer is drawn anew, U(0, 1) s.
    pub fn set_enabled(&mut self, on: bool, u: f32) {
        if on {
            self.rpm = 0.0;
            self.up = -1.0;
            self.down = -1.0;
        }
        self.enabled = on;
        self.timer = u;
    }

    /// Whether the cycle drives the car this frame (revving on, or its timer still running).
    pub fn active(&self) -> bool {
        self.enabled || self.timer > 0.0
    }

    /// One frame: the throttle (0 or 1). `rpm` = the engine's, `rnd` = U(0, 1).
    pub fn update(&mut self, e: &RevEngine, rpm: f32, dt: f32, rnd: &mut dyn FnMut() -> f32) -> f32 {
        let step = 6000.0 * dt;
        self.rpm = if self.rpm < rpm { (self.rpm + step).min(rpm) } else { (self.rpm - step).max(rpm) };
        self.timer -= dt;
        if self.up < 0.0 {
            if self.timer >= 0.0 || (self.down >= 0.0 && self.rpm >= self.down) {
                return 0.0;
            }
            if !self.enabled {
                // (0x822BD1CC: revving switched off - throttle 1 for this last frame, no new target)
                return 1.0;
            }
            if e.limit_max > 0.0 {
                self.timer = rnd() * 3.0;
                self.up = e.idle + (e.opt + (e.max - e.opt) * e.limit_min - e.idle) * (0.75 + rnd() * 0.25);
            } else {
                self.timer = rnd() * 0.35;
                self.up = e.idle + (e.max - e.idle) * (0.75 + rnd() * 0.2);
            }
            self.down = -1.0;
            1.0
        } else {
            // (0x822BD3E0: revving switched off - the rev-up holds, throttle 1, until the cycle's timer
            // has run out; no down phase follows)
            if !self.enabled || !(self.timer < 0.0 && self.rpm > self.up) {
                return 1.0;
            }
            self.timer = rnd();
            self.down = e.idle + (e.max - e.idle) * (0.05 + rnd() * 0.7);
            self.up = -1.0;
            0.0
        }
    }
}

use crate::vehicles::parked::Mwc;
use glam::Vec3;

/// A goal of the brain's array (32 bytes): `pos` (+0), `w` (+12: the radius, or for a gate the fraction
/// of the way to `p1` where it is crossed), `p1` (+16), `speed` (+28, -1 none), and the flags word:
/// bit 0 gate, bit 1 a checkpoint (not an alt path's point), bits 2..5 the lap, 6..12 the checkpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Goal {
    pub pos: Vec3,
    pub w: f32,
    pub p1: Vec3,
    pub speed: f32,
    pub flags: u32,
}

impl Goal {
    pub fn gate(&self) -> bool {
        self.flags & 1 != 0
    }
    pub fn is_checkpoint(&self) -> bool {
        self.flags & 2 != 0
    }
    pub fn lap(&self) -> usize {
        ((self.flags >> 2) & 0xF) as usize
    }
    pub fn checkpoint(&self) -> usize {
        ((self.flags >> 6) & 0x7F) as usize
    }
    /// Where the goal lies (sub_822BAD78: a gate's point is lerp(pos, p1, w)).
    pub fn centre(&self) -> Vec3 {
        if self.gate() { self.pos + (self.p1 - self.pos) * self.w } else { self.pos }
    }
}

/// A checkpoint of the race's CheckpointList as the brain reads it (+64 pos, +80 second point, +168 radius,
/// +172 the gate's fraction, +188 max speed (-1), +330 gate).
#[derive(Debug, Clone, Copy)]
pub struct CpDef {
    pub pos: Vec3,
    pub second: Vec3,
    pub radius: f32,
    pub frac: f32,
    pub speed: f32,
    pub gate: bool,
}

/// An alt path (racescript.rs `AiRoute`) and its alt checkpoints (`Course::nodes`).
#[derive(Debug, Clone, Copy)]
pub struct AltPath {
    pub before: usize,
    pub weight: f32,
    pub first: usize,
    pub count: usize,
    pub mask: u8,
    pub skip: usize,
    pub shortcut: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct AltCp {
    pub pos: Vec3,
    pub radius: f32,
    pub speed: f32,
}

/// What BrainRacing_SetupRace gives the goal builder.
#[derive(Debug, Clone, Default)]
pub struct Course {
    pub cps: Vec<CpDef>,
    pub paths: Vec<AltPath>,
    pub alt: Vec<AltCp>,
}

impl Course {
    /// The port's reading of a race script (racescript.rs `course`).
    pub fn from_script(c: &crate::racescript::Course) -> Course {
        Course {
            cps: c
                .cps
                .iter()
                .map(|k| match k.gate {
                    Some((b, f)) => CpDef { pos: k.pos, second: b, radius: k.radius, frac: f, speed: -1.0, gate: true },
                    // (+80 is only set for a line checkpoint; nothing reads it for the others)
                    None => CpDef { pos: k.pos, second: k.pos, radius: k.radius, frac: 0.0, speed: -1.0, gate: false },
                })
                .collect(),
            paths: c
                .routes
                .iter()
                .map(|r| AltPath { before: r.before, weight: r.weight, first: r.start, count: r.count, mask: r.mask, skip: r.kind.max(0) as usize, shortcut: r.shortcut })
                .collect(),
            // (every race script's alt checkpoint max speed is -1: work/racerai/altpaths.md)
            alt: c.nodes.iter().map(|&(pos, radius)| AltCp { pos, radius, speed: -1.0 }).collect(),
        }
    }
}

/// The goal-building part of the brain (sub_822BE538 SetupRace, sub_822BAF18, sub_822BA758).
#[derive(Debug, Clone)]
pub struct GoalPlan {
    pub laps: usize,
    /// +328: the racer's bit in the alt paths' masks
    pub opponent: u32,
    /// +260: 0.75 at difficulty 2 or 3, else 0
    pub shortcut_chance: f32,
    /// +380: the rubber band is on for this racer
    pub rubber: bool,
    /// +228 / +232: the checkpoint / lap of a shortcut the racer has been switched onto (-1 none)
    pub lock: (i32, i32),
    pub goals: Vec<Goal>,
}

impl GoalPlan {
    pub fn new(laps: usize, opponent: u32, difficulty: i32, rubber: bool) -> GoalPlan {
        GoalPlan { laps: laps.max(1), opponent, shortcut_chance: if difficulty == 2 || difficulty == 3 { 0.75 } else { 0.0 }, rubber, lock: (-1, -1), goals: Vec::new() }
    }

    fn open_to(&self, p: &AltPath) -> bool {
        (p.mask as u32 >> (self.opponent & 0xFF)) & 1 != 0
    }

    fn matches(&self, p: &AltPath, shortcut: bool) -> bool {
        self.open_to(p) && (!self.rubber || p.shortcut == shortcut)
    }

    /// sub_822BA758: the alt path driven before checkpoint `cp` (in lap `lap`), from the path cursor `alt`.
    fn alt_before(&mut self, c: &Course, cp: &mut usize, alt: &mut usize, lap: usize, final_wrap: bool, rng: &mut Mwc) {
        let forced = *cp as i32 == self.lock.0 && lap as i32 == self.lock.1;
        let mut shortcut = forced;
        if !forced && self.rubber {
            let r = rng.float();
            if r < self.shortcut_chance {
                // (only if a shortcut of this checkpoint is open to the racer)
                shortcut = c.paths.iter().skip(*alt).take_while(|p| p.before <= *cp).any(|p| p.before == *cp && self.open_to(p) && p.shortcut);
            }
        }
        let w_of = |p: &AltPath| if final_wrap { if p.weight == 0.0 { 1.0 } else { 0.0 } } else { p.weight };
        // the sum of the weights of the paths of this checkpoint (the cursor ends past them)
        let mut total = 0.0f32;
        let mut end = *alt;
        while end < c.paths.len() && c.paths[end].before <= *cp {
            let p = &c.paths[end];
            if p.before == *cp && self.matches(p, shortcut) {
                total += w_of(p);
            }
            end += 1;
        }
        if total > 0.0 {
            let mut x = (rng.next() & 0x7F_FFFF) as f32 * total * (1.0 / 8_388_608.0);
            let mut k = *alt;
            while k < c.paths.len() && c.paths[k].before <= *cp {
                let p = c.paths[k];
                if p.before == *cp {
                    if self.matches(&p, shortcut) {
                        x -= w_of(&p);
                    }
                    // (the test is made on every path of the checkpoint, matching or not: 0x822BAC44)
                    if x < 1e-6 {
                        *cp += p.skip;
                        for j in 0..p.count {
                            let a = c.alt.get(p.first + j).copied().unwrap_or(AltCp { pos: Vec3::ZERO, radius: 0.0, speed: -1.0 });
                            let flags = ((lap as u32) << 2 & 0x3C) | ((*cp as u32) << 6 & 0x1FC0);
                            self.goals.push(Goal { pos: a.pos, w: a.radius, p1: a.pos, speed: if a.speed == 0.0 { -1.0 } else { a.speed }, flags });
                        }
                        break;
                    }
                }
                k += 1;
            }
        }
        *alt = end;
    }

    /// sub_822BAF18: goals from checkpoint `cp` of lap `lap` to the end of the race.
    pub fn build(&mut self, c: &Course, mut cp: usize, lap: usize, rng: &mut Mwc) {
        let n = c.cps.len();
        for lap in lap..self.laps {
            let mut alt = 0usize;
            while cp < n {
                self.alt_before(c, &mut cp, &mut alt, lap, false, rng);
                if cp >= n {
                    break;
                }
                let k = c.cps[cp];
                let flags = (k.gate as u32) | 2 | ((lap as u32) << 2 & 0x3C) | ((cp as u32) << 6 & 0x1FC0);
                self.goals.push(Goal { pos: k.pos, w: if k.gate { k.frac } else { k.radius }, p1: k.second, speed: if k.speed == 0.0 { -1.0 } else { k.speed }, flags });
                cp += 1;
            }
            self.alt_before(c, &mut cp, &mut alt, lap, lap + 1 >= self.laps, rng);
            cp -= n.min(cp);
        }
    }

    /// BrainRacing_AppendGoal (sub_822BE478): a goal of its own - checkpoint bit set, gate clear, the
    /// checkpoint field its index, lap 0; +16 and +28 not written (taken as the point itself and -1).
    pub fn append(&mut self, pos: Vec3, radius: f32) {
        let k = self.goals.len() as u32;
        self.goals.push(Goal { pos, w: radius, p1: pos, speed: -1.0, flags: 2 | (k << 6 & 0x1FC0) });
    }
}

// ---------------------------------------------------------------------------------------------
// the brain itself

use crate::aiv::{Aiv, AivCar};
use crate::physics::CarInput;
use mcla_formats::rage::aitune::AiTune;
use mcla_formats::rage::navmesh::{NavMesh, QuickestPath};
use std::sync::Arc;

/// The one random generator the original's brains share (MWC at 0x82860890). (approx.: here only the
/// brains draw from it, and its seed is the port's.)
pub fn rng() -> std::sync::MutexGuard<'static, Mwc> {
    static R: std::sync::OnceLock<std::sync::Mutex<Mwc>> = std::sync::OnceLock::new();
    R.get_or_init(|| std::sync::Mutex::new(Mwc::default())).lock().unwrap_or_else(|e| e.into_inner())
}

/// What the brain reads of its car each frame.
#[derive(Debug, Clone, Copy)]
pub struct BrainCar {
    pub pos: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub vel: Vec3,
    pub ang_vel: Vec3,
    /// car +224 (|v|)
    pub speed: f32,
    /// car +572: the drivetrain's shaft speed x its first wheel's radius
    pub shaft_speed: f32,
    pub wheels_down: usize,
    pub wheel_count: usize,
    /// the port's gear numbering (-1 reverse, 0 neutral, 1.. forward)
    pub gear: i32,
    pub rpm: f32,
    pub engine: RevEngine,
    /// sub_8254A280's f3: the gyro's DriftTorque / the engine's torque at its best power
    pub drift_ratio: f32,
    /// 2 x the bound's half extents
    pub size: Vec3,
    /// g = -9.8 x the car's gravity factor
    pub gravity: f32,
    pub nitro_tune: [f32; 3],
    pub nitro_t: f32,
    pub sling_tune: [f32; 3],
    pub sling_t: f32,
    /// mcCarNitro can fire (sub_82345698) / is burning
    pub nitro_ready: bool,
    pub nitro_burning: bool,
    /// mcCarSSTurbo charged and not active (sub_82345FE0) / active
    pub sling_ready: bool,
    pub sling_active: bool,
    /// the grid flag (car +592 == 2: Racer_PreRaceBurnout)
    pub grid: bool,
}

/// What the brain does with its car this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct BrainOut {
    pub input: CarInput,
    /// fire the nitrous (flag 8) / the slingshot (flag 4)
    pub nitro: bool,
    pub sling: bool,
    /// put the car here (sub_822A9930: velocities zeroed, onto the ground)
    pub teleport: Option<Vec3>,
    /// the car's nitrous is unlimited (car +664 byte +33)
    pub unlimited_nitro: bool,
}

/// A racer's brain in its driver (`AiDriver::brain`): a clone of a driver starts without one.
#[derive(Default)]
pub struct BrainSlot(pub Option<Box<Brain>>);

impl Clone for BrainSlot {
    fn clone(&self) -> Self {
        BrainSlot(None)
    }
}

impl std::fmt::Debug for BrainSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BrainSlot({})", if self.0.is_some() { "racing" } else { "-" })
    }
}

impl BrainSlot {
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }
}

pub struct Brain {
    pub course: Arc<Course>,
    pub plan: GoalPlan,
    pub aiv: Aiv,
    pub input: OpponentInput,
    pub stuck: Stuck,
    pub man: Manoeuvre,
    pub rev: Revving,
    /// +192 / +208
    pub final_heading: Vec3,
    pub final_speed: f32,
    /// +236 / +248
    pub allow_nitro: bool,
    pub nitro_off: f32,
    /// +240 / +244
    pub nitro_score: f32,
    pub sling_score: f32,
    /// +257 / +258 / +292 / +296 / +338
    pub hold_still: bool,
    pub looping: bool,
    pub max_speed: f32,
    pub collision: f32,
    pub avoid_handbrake: bool,
    /// +212 base, +216 current goal, +220 next goal to plan
    pub base: usize,
    pub cur: usize,
    pub next: usize,
    /// +128: 0 idle, 3 failed, 4 a path waiting for the line
    nav_state: u8,
    path: Option<QuickestPath>,
    /// +264 / +268
    off_corridor: i32,
    off_mesh: i32,
    /// diagnostics
    pub last: crate::aiv::AivOut,
    /// the race's difficulty (results +3100) and how far the core's grid timeline has got
    pub difficulty: i32,
    pub stage: u8,
    /// the brain's copy of the race's rubber band tuning (+412..+551; None: rubber banding off)
    pub rubber: Option<crate::rubber::RubberBand>,
    /// +408: obstacle kinds to ignore (bit k = kind k)
    pub ignore: u32,
    dbg_t: f32,
}

impl Brain {
    /// BrainFactory_CreateBrainRacing + BrainRacing_Reset + BrainRacing_SetupRace(list, opponent, laps,
    /// difficulty) and the core's set-up calls (SetFinalHeading 0, SetFinalSpeed -1, SetRevvingEnabled 1,
    /// SetHoldStill 0: OrderedCheckpointCore 0x1143..0x11d6).
    pub fn setup_race(course: Arc<Course>, tune: Arc<AiTune>, opponent: u32, laps: usize, difficulty: i32, rubber: bool) -> Brain {
        let mut plan = GoalPlan::new(laps, opponent, difficulty, rubber);
        plan.build(&course, 0, 0, &mut rng());
        let mut b = Brain {
            course,
            plan,
            aiv: Aiv::new(tune),
            input: OpponentInput::default(),
            stuck: Stuck::default(),
            man: Manoeuvre::default(),
            rev: Revving::default(),
            final_heading: Vec3::ZERO,
            final_speed: -1.0,
            allow_nitro: true,
            nitro_off: 0.0,
            nitro_score: 0.5,
            sling_score: 0.5,
            hold_still: false,
            looping: false,
            max_speed: -1.0,
            collision: 0.0,
            avoid_handbrake: false,
            base: 0,
            cur: 0,
            next: 0,
            nav_state: 0,
            path: None,
            off_corridor: 30,
            off_mesh: 30,
            last: crate::aiv::AivOut { throttle: 0.0, steer: 0.0, brake: 1.0, handbrake: 0.0, valid: false },
            difficulty,
            stage: 0,
            rubber: None,
            ignore: 0,
            dbg_t: 0.0,
        };
        let u = rng().float();
        b.rev.set_enabled(true, u);
        b
    }

    fn goal_at(&self, i: usize) -> Option<Goal> {
        self.plan.goals.get(i).copied()
    }

    /// sub_822B9F18: plan again from the car (nav cancelled, the line reset, all three indices = `at`).
    fn replan(&mut self, at: usize) {
        self.nav_state = 0;
        self.path = None;
        self.aiv.reset();
        self.base = at;
        self.cur = at;
        self.next = at;
    }

    /// sub_822BCC90: the racer is far behind - onto the next shortcut.
    fn catch_up(&mut self) {
        let n = self.course.cps.len();
        let flags_of = |b: &Brain| -> u32 {
            let g = &b.plan.goals;
            if g.is_empty() {
                0
            } else if b.cur == g.len() {
                g[g.len() - 1].flags
            } else {
                g.get(b.cur).map_or(0, |x| x.flags)
            }
        };
        let f = flags_of(self);
        let c = ((f >> 6) & 0x7F) as usize;
        let cur_lap = ((f >> 2) & 0xF) as usize;
        let mut found: Option<(usize, usize)> = None;
        for (k, p) in self.course.paths.iter().enumerate() {
            if p.before > c + 1 {
                break;
            }
            if !p.shortcut {
                continue;
            }
            if p.before == c + 1 {
                if cur_lap + 1 >= self.plan.laps && p.before >= n {
                    return;
                }
                found = Some((k, cur_lap));
                break;
            }
            if cur_lap + 1 < self.plan.laps && p.before == 0 && c + 1 == n {
                found = Some((k, cur_lap + 1));
                break;
            }
        }
        let Some((k, lap)) = found else { return };
        let before = self.course.paths[k].before;
        self.plan.lock = (before as i32, lap as i32);
        log::info!("BrainRacing: opponent {} switching to catch up on shorter alt path leading to checkpoint {}", self.plan.opponent, before);
        // (the goals from the first at or after (lap, before) are rebuilt with the shortcut forced)
        let mut cut = self.plan.goals.len();
        for i in self.cur..self.plan.goals.len() {
            let g = &self.plan.goals[i];
            if g.lap() > lap || (g.lap() == lap && g.checkpoint() >= before) {
                cut = i;
                break;
            }
        }
        self.plan.goals.truncate(cut);
        let course = self.course.clone();
        self.plan.build(&course, before, lap, &mut rng());
        let at = self.cur;
        self.replan(at);
        let mut cp = before + self.course.paths[k].skip;
        let mut lp = lap;
        if cp >= n && n > 0 {
            cp -= n;
            lp += 1;
        }
        self.plan.lock = (cp as i32, lp as i32);
    }

    /// One frame: sub_822BE670 (goals, routes, the line) then sub_822BCFE0 (the pedals) and
    /// mcOpponentInput. `rb` = the rubber band's tuning and this racer's measures (None: off).
    pub fn update(&mut self, car: &BrainCar, turn: &crate::brainturn::TurnCar, mesh: &NavMesh, obstacles: &[crate::aiv::Obstacle], rb: Option<(&crate::rubber::RubberBand, crate::ai::RbState)>, dt: f32) -> BrainOut {
        let mut out = BrainOut::default();
        // ---- sub_822BE670 ----
        if self.nav_state == 4 && self.aiv.goal_count() < 8 {
            if let Some(path) = self.path.take() {
                let mut goals = std::mem::take(&mut self.plan.goals);
                // (false: the line's portal ring has no room yet - the path waits, state 4)
                let fed = crate::brainpath::path_to_aiv_at(&mut self.aiv, mesh, &path, &mut goals, self.cur, &mut self.next, self.looping, self.final_heading, self.final_speed);
                self.plan.goals = goals;
                if fed {
                    self.nav_state = 0;
                } else {
                    self.path = Some(path);
                }
            } else {
                self.nav_state = 0;
            }
        }
        self.aiv.set_obstacles(obstacles);
        let acar = AivCar {
            pos: car.pos,
            right: car.right,
            forward: car.forward,
            vel: car.vel,
            ang_vel: car.ang_vel,
            nitro_tune: car.nitro_tune,
            nitro_t: car.nitro_t,
            sling_tune: car.sling_tune,
            sling_t: car.sling_t,
            speed_cap: self.max_speed,
            size: car.size,
            gravity: car.gravity,
        };
        self.aiv.update(&acar, dt);
        // (unlimited nitrous: far enough behind - rubber.md 2.7)
        if let Some((t, s)) = rb {
            out.unlimited_nitro = s.gap < -t.unlimited_nitro_min_dist_behind_player;
        }
        // off the navigation mesh for 30 updates: onto the nearest polygon
        if mesh.on_mesh(car.pos) {
            self.off_mesh = 30;
        } else {
            self.off_mesh -= 1;
            if self.off_mesh <= 0 {
                self.off_mesh = 30;
                out.teleport = mesh.teleport_target(car.pos);
            }
        }
        // out of the planned corridor for 30 updates: plan again from the car
        if self.aiv.off_line().0 {
            self.off_corridor -= 1;
            if self.off_corridor <= 0 {
                self.off_corridor = 30;
                let at = self.cur;
                self.replan(at);
            }
        } else {
            self.off_corridor = 30;
        }
        let ng = self.plan.goals.len();
        self.cur = self.base + self.aiv.goals_done().max(0) as usize;
        if self.looping && ng > 0 {
            self.cur %= ng;
        }
        // far behind: take the next shortcut
        if let Some((t, s)) = rb {
            let f = self.goal_at(self.cur.min(ng.saturating_sub(1))).map_or(0, |g| g.flags);
            let (lap, cp) = (((f >> 2) & 0xF) as i32, ((f >> 6) & 0x7F) as i32);
            if s.gap < -t.dist_behind_before_shortcuts && (lap > self.plan.lock.1 || (lap == self.plan.lock.1 && cp > self.plan.lock.0)) {
                self.catch_up();
            }
        }
        // the next goal's route over the mesh
        let ng = self.plan.goals.len();
        if self.next < ng && (self.nav_state == 0 || self.nav_state == 3) {
            let goal = self.plan.goals[self.next].centre();
            let start = if self.cur == self.next && !self.aiv.has_path() {
                car.pos
            } else if self.next == 0 {
                self.plan.goals[ng - 1].centre()
            } else {
                self.plan.goals[self.next - 1].centre()
            };
            let cost = mcla_formats::rage::navmesh::racer_cost(mesh.point_flags(start), mesh.point_flags(goal));
            match mesh.quickest_path(start, goal, &cost) {
                Some(p) => {
                    self.path = Some(p);
                    self.nav_state = 4;
                }
                None => self.nav_state = 3,
            }
        }
        self.collision = (self.collision - dt).max(0.0);
        self.nitro_off = (self.nitro_off - dt).max(0.0);

        // ---- sub_822BCFE0 ----
        let gear_arg = car.gear.max(1) - 1;
        let a = self.aiv.output(gear_arg, 0.0, car.drift_ratio, self.collision > 0.0);
        self.last = a;
        let mut t = Pedals { throttle: a.throttle, brake: a.brake, handbrake: 0.0, steer: a.steer };
        let mut reverse = false;
        if self.hold_still {
            t = Pedals { throttle: 0.0, brake: 0.0, handbrake: 1.0, steer: 0.0 };
        } else if self.rev.active() {
            let thr = self.rev.update(&car.engine, car.rpm, dt, &mut || rng().float());
            t = Pedals { throttle: thr, brake: 0.0, handbrake: if self.rev.handbrake { 1.0 } else { 0.0 }, steer: 0.0 };
        } else if car.grid {
            t = Pedals { throttle: 1.0, brake: 0.0, handbrake: 1.0, steer: 0.0 };
        } else if a.valid {
            let to_line = crate::aiv::ang_xz(car.forward, self.aiv.next_point() - car.pos);
            let to_corridor = crate::aiv::ang_xz(car.forward, self.aiv.corridor_ahead() - car.pos);
            if self.stuck.state == 0 {
                crate::brainturn::turnaround_car(&mut self.man, self.avoid_handbrake, to_line, to_corridor, car.speed, turn, self.aiv.max_curvature(), mesh);
            }
            if self.stuck.state != 0 || (!self.man.reverse && self.man.turn == 0) {
                self.stuck.update(&mut self.man, car.pos, car.speed, to_line, to_corridor, dt);
            }
            if !self.man.reverse && self.man.turn == 0 {
                let cap = rb.map_or(1.0, |(_, s)| s.cap);
                t.throttle = a.throttle.min(cap);
                // nitrous / slingshot
                let gates = rb.map_or(true, |(r, s)| s.from_start > r.nitro_min_dist_from_start && s.to_finish > r.nitro_min_dist_from_finish && s.gap < -r.nitro_min_dist_behind_player);
                if car.wheels_down == car.wheel_count && self.allow_nitro && a.throttle > 0.999_999 && self.nitro_off == 0.0 && gates {
                    if car.sling_ready && !car.nitro_burning {
                        out.sling = self.aiv.sling_benefit() > self.sling_score;
                    } else if car.nitro_ready && !car.sling_active {
                        out.nitro = self.aiv.nitro_benefit() > self.nitro_score;
                    }
                }
                // (approx.: in the air the original also hands the pitch control +32 to the input -
                // the port's computer cars have no air control)
            } else {
                t = Pedals { throttle: 1.0, brake: 0.0, handbrake: 0.0, steer: match self.man.turn { 1 => -1.0, 2 => 1.0, _ => 0.0 } };
                if self.man.reverse {
                    t.steer = -t.steer;
                    reverse = true;
                } else if self.man.handbrake {
                    t.handbrake = 1.0;
                }
            }
        } else {
            self.man.reverse = false;
            self.man.turn = 0;
            if self.nav_state == 4 && car.speed > 8.94 {
                t.throttle = 1.0;
                t.brake = 0.0;
            }
        }
        let p = self.input.update(t, self.collision > 0.0, dt);
        // (the chassis takes the pedals as written - CarInput::animated: no pad code, no computer-driver
        // share, the gearbox kept at first or above, as the brain's max(gear, 2); backing up goes
        // through the port's reverse: in reverse the brake is the pedal that drives)
        out.input = if reverse {
            CarInput { throttle: 0.0, brake: p.throttle.max(0.11), steer: p.steer, handbrake: p.handbrake > 0.5, ai: true, ..Default::default() }
        } else {
            CarInput { throttle: p.throttle, brake: p.brake, steer: p.steer, handbrake: p.handbrake > 0.5, ai: true, animated: true, ..Default::default() }
        };
        out
    }

    /// Mark a collision with another vehicle (mcOpponent sub_822B9AC0: +296 = 1.5 s).
    pub fn hit(&mut self) {
        self.collision = 1.5;
    }
}

// ---------------------------------------------------------------------------------------------
// the port's side: a racer's brain run on its car

/// The race core's grid timeline for a racer (OrderedCheckpointCore 0xF7D..0x11A6, 0x15d8..0x1605;
/// Race_GetTimer in ms, negative before GO): from -3000 ms every computer racer gets the handbrake for
/// its revving, at -1000 ms revving is switched off (both only at a difficulty above 0), and at the
/// start (BeginRace, Racer_Activate) SetRevvingEnabled(0) once more.
pub fn script_grid(b: &mut Brain, to_go: Option<f32>) {
    match to_go {
        Some(t) => {
            if b.difficulty != 0 && t <= 3.0 && b.stage < 1 {
                b.stage = 1;
                b.rev.handbrake = true;
            }
            if b.difficulty != 0 && t <= 1.0 && b.stage < 2 {
                b.stage = 2;
                let u = rng().float();
                b.rev.set_enabled(false, u);
            }
        }
        None => {
            if b.stage < 3 {
                b.stage = 3;
                let u = rng().float();
                b.rev.set_enabled(false, u);
            }
        }
    }
}

/// One frame of car `i`'s brain (vehicles.rs): its view of the car, the obstacles, the update, and
/// what it does to the car besides the pedals (nitrous, the off-mesh drop). `to_go`: seconds to GO
/// while the grid holds the racers.
pub fn tick(v: &mut crate::vehicles::Vehicles, i: usize, to_go: Option<f32>, dt: f32) -> CarInput {
    let Some(mesh) = crate::offroad::mesh() else { return CarInput { brake: 1.0, ai: true, ..Default::default() } };
    let Some(mut brain) = v.cars[i].ai.as_mut().and_then(|a| a.brain.0.take()) else { return CarInput::default() };
    let rb = v.cars[i].ai.as_ref().and_then(|a| a.rb);
    script_grid(&mut brain, to_go);
    let c = &v.cars[i];
    let st = &c.state;
    let vt = c.params().veh.clone();
    let vs = st.veh.as_ref();
    let (engine, drift_ratio, nitro_tune, sling_tune) = match vt.as_deref() {
        Some(t) => (
            RevEngine { idle: t.t.engine.s.idle_rpm, opt: t.t.engine.s.opt_rpm, max: t.t.engine.s.max_rpm, limit_min: t.t.s.rev_limit_min, limit_max: t.t.s.rev_limit_max },
            if t.sync_torque > 0.0 { t.t.gyro.drift_torque / t.sync_torque } else { 0.0 },
            [t.t.nitro.boost_exp, t.t.nitro.boost_amount, t.t.nitro.boost_time],
            [t.t.ss_turbo.boost_exp, t.t.ss_turbo.boost_amount, t.t.ss_turbo.use_time],
        ),
        None => (RevEngine::default(), 0.0, [0.0; 3], [0.0; 3]),
    };
    let wheel_count = if c.model.bike { 2 } else { 4 };
    let wheels_down = st.wheels.iter().take(4).filter(|w| w.grounded).count().min(wheel_count);
    let gravity_factor = vt.as_deref().map_or(1.4, |t| t.gravity);
    let car = BrainCar {
        pos: st.pos,
        forward: st.forward(),
        right: st.right(),
        up: st.up(),
        vel: st.vel,
        ang_vel: st.ang_vel,
        speed: st.vel.length(),
        shaft_speed: 0.0,
        wheels_down,
        wheel_count,
        gear: vs.map_or(1, |s| s.gear),
        rpm: vs.map_or(0.0, |s| s.engine_w * 9.549_296_6),
        engine,
        drift_ratio,
        // (approx.: 2 x the tuning's box - the original takes its collision bound's half extents)
        size: c.params().body_half * 2.0,
        gravity: -9.8 * gravity_factor,
        nitro_tune,
        nitro_t: vs.map_or(0.0, |s| s.nitro_level * nitro_tune[2]),
        sling_tune,
        sling_t: vs.map_or(0.0, |s| s.sst.level * sling_tune[2]),
        nitro_ready: st.nitro_shots > 0 && vs.map_or(false, |s| s.nitro_level <= 0.0 && s.nitro_rest <= 0.0),
        nitro_burning: vs.map_or(false, |s| s.nitro_level > 0.0),
        // (0x82345FE0, the car answering its pad off the grid)
        sling_ready: vs.map_or(false, |s| s.sst.can_fire(to_go.is_none())),
        sling_active: vs.map_or(false, |s| s.sst.burning),
        grid: to_go.is_some(),
    };
    // the obstacles near the line ahead (sub_822BD9D8)
    // (cars, then the fixed props the game gathered around the racers; approx.: no pedestrians - the
    // brain ignores them anyway - and the grid without the level's bounds)
    let objects: Vec<crate::brainturn::ObjectInfo> = v.cars.iter().enumerate().map(|(j, o)| crate::brainturn::ObjectInfo::from_car(j, o)).chain(v.prop_objects.iter().copied()).collect();
    let line: Vec<Vec3> = brain.aiv.points().collect();
    let sel = crate::brainturn::SelCar { pos: car.pos, forward: car.forward, vel: car.vel, speed: car.speed, half_length: car.size.z * 0.5 };
    let obstacles = crate::brainturn::select_obstacles_car(i, &objects, &line, &sel, brain.ignore, &crate::brainturn::ObjectGrid::unbounded(), &brain.aiv.tun);
    // (the chassis lists its wheels FL, RL, FR, RR at +412 .. +424; the port's order is FL, FR, RL, RR)
    let w = &v.cars[i].state.wheels;
    let turn = crate::brainturn::TurnCar {
        pos: car.pos,
        right: car.right,
        back: -car.forward,
        size: car.size,
        wheels: [w[0].world_pos, w[2].world_pos, w[1].world_pos, w[3].world_pos],
        wheel_count,
        drive: vs.map_or(0, |s| s.drive as u32),
    };
    let rubber = brain.rubber.clone();
    let out = brain.update(&car, &turn, mesh, &obstacles, rubber.as_ref().zip(rb), dt);
    // (diagnostics: MCLA_BRAIN_TRACE=<file.csv> - one line per racer and tick)
    if let Ok(path) = std::env::var("MCLA_BRAIN_TRACE") {
        use std::io::Write;
        static FILE: std::sync::OnceLock<std::sync::Mutex<Option<std::fs::File>>> = std::sync::OnceLock::new();
        let f = FILE.get_or_init(|| std::sync::Mutex::new(std::fs::File::create(&path).ok().map(|mut f| {
            let _ = writeln!(f, "car,grid,x,y,z,speed,gear,valid,aiv_thr,aiv_brk,aiv_steer,thr,brk,steer,hb,cap,gap,stuck,nitro");
            f
        })));
        if let Some(f) = f.lock().ok().as_mut().and_then(|g| g.as_mut()) {
            let _ = writeln!(f, "{i},{},{:.3},{:.3},{:.3},{:.3},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{},{:.3},{:.1},{},{}", to_go.is_some() as u8, car.pos.x, car.pos.y, car.pos.z, car.speed, car.gear, brain.last.valid as u8,
                brain.last.throttle, brain.last.brake, brain.last.steer, out.input.throttle, out.input.brake, out.input.steer, out.input.handbrake as u8, rb.map_or(1.0, |r| r.cap), rb.map_or(0.0, |r| r.gap), brain.stuck.state, out.nitro as u8);
        }
    }
    // (diagnostics: MCLA_BRAIN_DUMP=<seconds>)
    if let Some(every) = std::env::var("MCLA_BRAIN_DUMP").ok().and_then(|x| x.parse::<f32>().ok()) {
        brain.dbg_t += dt;
        if brain.dbg_t >= every {
            brain.dbg_t = 0.0;
            log::info!("brain {i}: pos ({:.0},{:.0},{:.0}) v {:.1} gear {} | goals {} cur {} next {} nav {} | aiv portals {} goals {} pts {} done {} valid {} thr {:.2} brk {:.2} st {:.2} | stuck {} man {:?} | rb {:?} | obst {} | in thr {:.2} brk {:.2} st {:.2} hb {}",
                car.pos.x, car.pos.y, car.pos.z, car.speed, car.gear, brain.plan.goals.len(), brain.cur, brain.next, brain.nav_state,
                brain.aiv.portal_count(), brain.aiv.goal_count(), brain.aiv.point_count(), brain.aiv.goals_done(), brain.last.valid, brain.last.throttle, brain.last.brake, brain.last.steer,
                brain.stuck.state, brain.man, rb.map(|r| (r.gap as i32, r.cap)), obstacles.len(), out.input.throttle, out.input.brake, out.input.steer, out.input.handbrake);
            if std::env::var("MCLA_BRAIN_SAMPLES").is_ok() {
                log::info!("brain {i}: samples {:?}", brain.aiv.samples_dbg().iter().take(40).map(|s| format!("k{:.4} t{:.1} a{:.1} c{:.1} al{:.1} v{:.1}", s.0, s.1, s.2, s.3, s.4, s.5)).collect::<Vec<_>>());
            }
            if std::env::var("MCLA_BRAIN_OPT").is_ok() {
                log::info!("brain {i}: opt {}", brain.aiv.opt_dbg());
            }
            if std::env::var("MCLA_BRAIN_PTS").is_ok() {
                log::info!("brain {i}: pts {:?}", brain.aiv.points().map(|p| format!("{:.1},{:.1}", p.x, p.z)).collect::<Vec<_>>());
            }
            if std::env::var("MCLA_BRAIN_PORTALS").is_ok() {
                log::info!("brain {i}: portals {:?}", brain.aiv.portal_ends().take(16).map(|(a, b, sa, sb)| ((a.x as i32, a.z as i32), (b.x as i32, b.z as i32), (sa.x as i32, sa.z as i32), (sb.x as i32, sb.z as i32))).collect::<Vec<_>>());
            }
            if std::env::var("MCLA_BRAIN_NEAR").is_ok() {
                for (j, o) in v.cars.iter().enumerate().filter(|(j, o)| *j != i && o.state.pos.distance(car.pos) < 60.0) {
                    log::info!("brain {i}: near car {j} {} ({:.0},{:.0}) player {} parked {} ai {} pinned {} racer {} placed {} removing {}", o.model.name, o.state.pos.x, o.state.pos.z, o.is_player, o.parked.is_some(), o.ai.is_some(), o.pinned, o.racer, o.placed, o.removing.is_some());
                }
            }
            log::info!("brain {i}: line {:?} obstacles {:?}", brain.aiv.points().step_by(4).take(12).map(|p| (p.x as i32, p.z as i32)).collect::<Vec<_>>(), obstacles.iter().map(|o| (o.pos.x as i32, o.pos.z as i32)).collect::<Vec<_>>());
        }
    }
    let c = &mut v.cars[i];
    if out.unlimited_nitro {
        c.state.nitro_shots = c.state.nitro_shots.max(1);
    }
    let mut input = out.input;
    if to_go.is_some() {
        // (on the grid the chassis takes the brain's pedals under the grid flag: CarInput::hold)
        input = CarInput { hold: true, animated: false, ..input };
    }
    input.nitro = out.nitro;
    // (flag 4: 0x822B91F0 fires the slipstream turbo through 0x823460A0 - vehsim's step)
    input.sst = out.sling;
    if let Some(p) = out.teleport {
        c.state.pos = p + Vec3::Y * 0.9;
        c.state.vel = Vec3::ZERO;
        c.state.ang_vel = Vec3::ZERO;
    }
    if let Some(a) = c.ai.as_mut() {
        // (over the line: on the brake, still steering - the port's race-end rule, CAREER_RACES.md "Race end")
        if a.finished {
            input = CarInput { throttle: 0.0, brake: 1.0, nitro: false, ..input };
        }
        a.brain.0 = Some(brain);
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    /// hw_ordered_jp_11's alt paths (work/racerai/altpaths.md): two ways through checkpoint 10 (the
    /// second a shortcut that skips it), and one finishing route per opponent after the last checkpoint.
    #[test]
    fn goals_jp_11() {
        let cps: Vec<CpDef> = (0..14).map(|k| CpDef { pos: Vec3::new(k as f32 * 100.0, 0.0, 0.0), second: Vec3::ZERO, radius: 25.0, frac: 0.0, speed: -1.0, gate: false }).collect();
        let mut paths = vec![
            AltPath { before: 10, weight: 1.0, first: 0, count: 1, mask: 255, skip: 0, shortcut: false },
            AltPath { before: 10, weight: 1.0, first: 1, count: 5, mask: 255, skip: 1, shortcut: true },
        ];
        let firsts = [6, 9, 12, 15, 19, 24, 28];
        let counts = [3, 3, 3, 4, 5, 4, 6];
        for k in 0..7 {
            paths.push(AltPath { before: 14, weight: 0.0, first: firsts[k], count: counts[k], mask: 2 << k, skip: 0, shortcut: false });
        }
        let alt: Vec<AltCp> = (0..34).map(|k| AltCp { pos: Vec3::new(0.0, 0.0, k as f32), radius: 10.0, speed: -1.0 }).collect();
        let c = Course { cps, paths, alt };
        for seed in 1..40u32 {
            let mut rng = Mwc { lo: seed * 7919, carry: 1 };
            let mut g = GoalPlan::new(1, 1, 1, false);
            g.build(&c, 0, 0, &mut rng);
            let cps_hit: Vec<usize> = g.goals.iter().filter(|x| x.is_checkpoint()).map(|x| x.checkpoint()).collect();
            let alts = g.goals.iter().filter(|x| !x.is_checkpoint()).count();
            // (either one alt point and checkpoint 10, or the shortcut's five and checkpoint 10 skipped)
            let main = cps_hit.contains(&10);
            assert_eq!(alts, (if main { 1 } else { 5 }) + 3, "seed {seed}");
            assert_eq!(cps_hit.len(), if main { 14 } else { 13 });
            // (opponent 1's finishing route: alt points 6..8 last)
            assert_eq!(g.goals.last().unwrap().pos.z, 8.0);
        }
    }

    #[test]
    fn input_rates() {
        let mut i = OpponentInput::default();
        let dt = 1.0 / 60.0;
        let p = i.update(Pedals { throttle: 1.0, brake: 0.0, handbrake: 0.0, steer: 1.0 }, false, dt);
        assert!((p.throttle - 35.0 / 60.0).abs() < 1e-6);
        // (from the middle (cur - target) x cur is 0: the fsel at 0x822B92BC takes the 4 /s rate)
        assert!((p.steer - 4.0 / 60.0).abs() < 1e-6);
        let p = i.update(Pedals { throttle: 0.0, brake: 1.0, handbrake: 0.0, steer: 0.0 }, true, dt);
        // (steer back at 4 /s halved; the throttle falls to 0 in that frame, so the brake rises at 2 /s)
        assert!((p.steer - (4.0 - 2.0) / 60.0).abs() < 1e-6);
        assert_eq!(p.throttle, 0.0);
        assert!((p.brake - 2.0 / 60.0).abs() < 1e-6);
        let p = i.update(Pedals { throttle: 0.2, brake: 1.0, handbrake: 0.0, steer: 0.0 }, false, dt);
        assert_eq!(p.brake, 0.0);
    }

    #[test]
    fn revving_switched_off_mid_rev_up() {
        // (0x822BD3E0: in the rev-up phase with revving off the throttle stays 1 until the timer is out, no down phase)
        let e = RevEngine { idle: 1000.0, opt: 6000.0, max: 8000.0, limit_min: 0.0, limit_max: 0.0 };
        let mut r = Revving::default();
        r.set_enabled(true, 0.0);
        let dt = 1.0 / 60.0;
        let mut rnd = || 0.5;
        assert_eq!(r.update(&e, 1000.0, dt, &mut rnd), 1.0);
        assert!(r.up >= 0.0);
        r.set_enabled(false, 0.25);
        let mut frames = 0;
        while r.active() {
            assert_eq!(r.update(&e, 9000.0, dt, &mut rnd), 1.0);
            frames += 1;
        }
        assert_eq!(frames, 16);
    }

    #[test]
    fn stuck_cycle() {
        let mut s = Stuck::default();
        let mut m = Manoeuvre::default();
        let dt = 0.1;
        for _ in 0..21 {
            s.update(&mut m, glam::Vec3::ZERO, 0.0, 1.0, 1.0, dt);
        }
        assert_eq!(s.state, 1);
        s.update(&mut m, glam::Vec3::ZERO, 0.0, 1.0, 1.0, dt);
        assert!(m.reverse);
        for _ in 0..11 {
            s.update(&mut m, glam::Vec3::ZERO, 0.0, 1.0, 1.0, dt);
        }
        assert_eq!(s.state, 2);
        s.update(&mut m, glam::Vec3::ZERO, 0.0, 1.0, 1.0, dt);
        assert!(m.handbrake && m.turn == 1);
        s.update(&mut m, glam::Vec3::ZERO, 0.0, 0.3, 1.0, dt);
        assert_eq!((s.state, m.turn), (0, 0));
    }
}
