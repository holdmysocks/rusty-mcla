//! The pad's rumble, as the original's car feedback object makes it (mcFeedback, one per player car at
//! mcCar +24; its base vehFeedback holds the two motors). Rule and recordings:
//! docs/reference/controls/rumble_effects.md, work/rumble/.
//!
//! - An effect (F-61844, PlayEffect) queues one pattern per motor from the table F-60005: a list of levels
//!   and a list of priorities, one sample each. Motor 0 is the pad's left (low-frequency) motor, motor 1
//!   the right (high-frequency) one.
//! - The pattern player (F-51065) takes, per motor, one sample of every running pattern: the highest
//!   priority wins, among equal priorities the larger level; a pattern ends after its last sample.
//! - The motors' update (F-87448) steps a motor only when more than 1/30 s (real time) has passed since
//!   its level last changed, and sets the new level only when it differs (F-20043 -> the pad's actuator
//!   F-77279: u16 = level x 65535, sent with the next pad poll, F-31331 -> XInputSetState F-16749).
//! - mcFeedback's own update (F-25579), once a car tick: gear changes (4), sliding / melted tyres (0),
//!   a hard suspension hit (5), the ground's rumble type at speed (8 / 9 / 10).
//!
//! Every other effect comes from the car's own events (`CarRumble::play`): hits, scrapes, landing,
//! wipeout, wreck, nitrous, slipstream turbo, Agro, the posts' deflection.

/// One effect of the table F-60005: (samples, levels of motor 1 (right), levels of motor 0 (left),
/// priorities). Motor 1 of ids 2, 4, 5, 6, 8, 9, 10 is read from a buffer the game fills at run time
/// (0x8288DE80): it held zeros all through the recordings and no writer of it was found.
struct Effect {
    right: &'static [f32],
    left: &'static [f32],
    prio: &'static [i32],
}

const P1: [i32; 10] = [1; 10];
const P2: [i32; 5] = [2; 5];
/// Effect 3 reads the shared priority list (eleven 2s) past its end: the next words are the levels of
/// effect 0's right motor (0, 0.75 = 0x3F400000, 0, 0) taken as integers - sample 12 always wins.
const P3: [i32; 15] = [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0x3F40_0000, 0, 0];

const EFFECTS: [Effect; 11] = [
    // 0: tyres sliding (a wheel's slide over 0.9) or melted (mcCarTireFric +24 over 0.5)
    Effect { right: &[0.0, 0.75, 0.0, 0.0], left: &[0.25, 0.0, 0.25, 0.0], prio: &P1 },
    // 1: a hit, a landing, a wipeout, a post's deflection
    Effect { right: &[0.0, 0.0, 1.0, 0.0, 0.0], left: &[1.0, 0.8, 0.0, 0.0, 0.0], prio: &P2 },
    // 2: a scrape
    Effect { right: &[0.0, 0.0], left: &[0.4, 0.2], prio: &P1 },
    // 3: nitrous / slipstream turbo fired, an Agro strike
    Effect {
        right: &[0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        left: &[1.0, 1.0, 0.9, 0.0, 0.0, 1.0, 1.0, 1.0, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3],
        prio: &P3,
    },
    // 4: a gear change
    Effect { right: &[0.0], left: &[0.75], prio: &P1 },
    // 5: a hard suspension hit (a wheel's compression rate over 5 m/s, or bottomed)
    Effect { right: &[0.0], left: &[0.5], prio: &P1 },
    // 6: (no caller found)
    Effect { right: &[0.0, 0.0], left: &[0.6, 0.3], prio: &P1 },
    // 7: the car is wrecked
    Effect { right: &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], left: &[0.8, 1.0, 0.8, 0.0, 0.0, 0.0, 0.0, 0.8, 1.0, 0.8], prio: &P1 },
    // 8: nitrous / turbo burning (every tick), or ground of rumble type 0 at speed
    Effect { right: &[0.0; 5], left: &[0.3; 5], prio: &P1 },
    // 9: ground of rumble type 1 at speed
    Effect { right: &[0.0; 5], left: &[0.6, 0.0, 0.0, 0.4, 0.0], prio: &P1 },
    // 10: ground of rumble type 2 at speed
    Effect { right: &[0.0; 5], left: &[1.0, 0.0, 0.0, 0.0, 0.0], prio: &P1 },
];

/// The motors' hold: vehFeedback +48, set to 1/30 by mcFeedback's constructor (F-86699 -> F-21161).
const HOLD: f32 = 1.0 / 30.0;

#[derive(Clone, Copy, Debug)]
struct Slot {
    effect: u8,
    /// motor 1 reads `right`, motor 0 `left`
    cursor: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct Motor {
    slots: [Option<Slot>; 8],
    active: u32,
    level: f32,
    /// when the level was last set (seconds of the caller's clock); none: long ago
    stamp: Option<f64>,
}

/// What the car's tick leaves for the feedback object (mcFeedback's own update, F-25579, reads it from the car).
#[derive(Clone, Copy, Debug, Default)]
pub struct Feel {
    /// the transmission's gear (any numbering: only its changes count)
    pub gear: i32,
    /// mcCarTireFric +24, the tyres' melt
    pub melt: f32,
    /// chassis +224, the speed (m/s)
    pub speed: f32,
    /// per wheel: slide (+520), compression rate (+504), bottomed (+530), contact record (+529),
    /// roughness height (+436), the ground's rumble type (the material's `effect`, record +56; -1 none)
    pub wheels: [(f32, f32, bool, bool, f32, i8); 4],
    pub wheel_count: usize,
}

/// One player car's feedback object (mcFeedback + vehFeedback).
#[derive(Clone, Debug)]
pub struct Feedback {
    motors: [Motor; 2],
    /// the effect each of motor 0's slots plays (mcFeedback +320)
    ids: [i32; 8],
    /// byte +8 (SetEnabled F-68736: also the pad's own enable, +45)
    pub enabled: bool,
    /// +352: the gear the last update saw
    base: i32,
    /// the levels last sent to the pad, and when (the port resends a held level, SDL's rumble times out)
    sent: Option<(u16, u16, f64)>,
}

impl Default for Feedback {
    fn default() -> Self {
        // (F-86699: no effect in any slot, the gear baseline 2)
        Feedback { motors: [Motor::default(); 2], ids: [-1; 8], enabled: false, base: 2, sent: None }
    }
}

impl Feedback {
    /// vt#9 F-63089 on a new car: the motors stopped (F-11560) and the gear taken as the baseline.
    pub fn init(&mut self, gear: i32, now: f64) {
        self.reset(true, now);
        self.base = gear;
    }

    /// F-11560: every pattern dropped; with `stop` both motors set to 0 at once.
    pub fn reset(&mut self, stop: bool, now: f64) {
        for m in self.motors.iter_mut() {
            m.slots = [None; 8];
            m.active = 0;
        }
        if stop {
            for m in self.motors.iter_mut() {
                m.level = 0.0;
                m.stamp = Some(now);
            }
        }
    }

    /// F-62455: the pattern into the motor's first free slot; false when all eight run.
    fn queue(&mut self, motor: usize, effect: u8) -> bool {
        let m = &mut self.motors[motor];
        match m.slots.iter().position(|s| s.is_none()) {
            Some(i) => {
                m.slots[i] = Some(Slot { effect, cursor: 0 });
                m.active += 1;
                true
            }
            None => false,
        }
    }

    /// PlayEffect F-61844 (the gates outside: the object enabled - checked here -, no replay playing, a pad).
    /// An effect that still runs is not started again; a hard suspension hit (5) also waits for 0, 1 and 2.
    pub fn play(&mut self, id: u8) -> bool {
        if !self.enabled || id as usize >= EFFECTS.len() {
            return false;
        }
        let mut first = None;
        for i in 0..8 {
            if self.motors[0].slots[i].is_none() {
                self.ids[i] = -1;
                if first.is_none() {
                    first = Some(i);
                }
            }
            let v = self.ids[i];
            if v == id as i32 || (id == 5 && (0..=2).contains(&v)) {
                return false;
            }
        }
        if let Some(i) = first {
            self.ids[i] = id as i32;
        }
        let a = self.queue(1, id);
        let b = self.queue(0, id);
        a && b
    }

    /// F-51065: one sample of every running pattern of the motor.
    fn step(&mut self, motor: usize) -> f32 {
        let mut best = -1i32;
        let mut level = 0.0f32;
        let m = &mut self.motors[motor];
        for s in m.slots.iter_mut() {
            let Some(slot) = s.as_mut() else { continue };
            let e = &EFFECTS[slot.effect as usize];
            let curve = if motor == 1 { e.right } else { e.left };
            let cur = slot.cursor;
            slot.cursor += 1;
            if cur < curve.len() {
                let (f, p) = (curve[cur], e.prio[cur]);
                if p > best {
                    best = p;
                    level = f;
                } else if p == best && f > level {
                    level = f;
                }
            }
            if slot.cursor == curve.len() {
                *s = None;
                m.active -= 1;
            }
        }
        level
    }

    /// The motors' update F-87448 at `now` (seconds, a monotonic real-time clock, as the original's time base).
    pub fn motors(&mut self, now: f64) {
        for i in 0..2 {
            let held = self.motors[i].stamp.map_or(false, |s| !(((now - s) as f32) > HOLD));
            if held {
                continue;
            }
            if self.motors[i].active > 0 {
                let v = self.step(i);
                if v != self.motors[i].level {
                    self.motors[i].level = v;
                    self.motors[i].stamp = Some(now);
                }
            } else if self.motors[i].level > 0.0 {
                self.motors[i].level = 0.0;
                self.motors[i].stamp = Some(now);
            }
        }
    }

    /// The effects mcFeedback's update F-25579 asks for, in order, from the car's state and the gear it
    /// saw last (`base`, updated).
    pub fn asks(f: &Feel, base: &mut i32) -> Vec<u8> {
        let mut out = Vec::new();
        if f.gear != *base {
            *base = f.gear;
            out.push(4);
        }
        let w = &f.wheels[..f.wheel_count.min(4)];
        // (mcCarTireFric +24 over 0.5, or a wheel's slide over 0.9)
        if f.melt > 0.5 || w.iter().any(|w| w.0 > 0.9) {
            out.push(0);
        }
        // (a wheel bottomed, else the largest compression rate over 5)
        if w.iter().any(|w| w.2) || w.iter().fold(0.0f32, |m, w| if w.1 > m { w.1 } else { m }) > 5.0 {
            out.push(5);
        }
        if f.speed > 10.0 {
            // (wheels with a contact record, a roughness that is not 0 and a material: its rumble type)
            let has = |t: i8| w.iter().any(|w| w.3 && w.4 != 0.0 && w.5 == t);
            for (t, id) in [(0, 8), (1, 9), (2, 10)] {
                if has(t) {
                    out.push(id);
                }
            }
        }
        out
    }

    /// mcFeedback's update F-25579, once a car tick: the effects the car's state asks for, then the motors.
    pub fn update(&mut self, f: &Feel, now: f64) -> Vec<u8> {
        let mut asked = Vec::new();
        if self.enabled {
            let mut base = self.base;
            asked = Self::asks(f, &mut base);
            self.base = base;
            for id in asked.iter() {
                self.play(*id);
            }
        }
        self.motors(now);
        asked
    }

    /// The pad's two motors (left / low frequency, right / high frequency) as the original sends them
    /// (F-77279: the level x 65535 truncated; F-31331: 0 while the pad's enable is off).
    pub fn output(&self) -> (u16, u16) {
        if !self.enabled {
            return (0, 0);
        }
        let u = |l: f32| (l.clamp(0.0, 1.0) * 65535.0) as u16;
        (u(self.motors[0].level), u(self.motors[1].level))
    }

    /// What to hand SDL this frame: Some((low, high)) when the levels changed, or a held non-zero
    /// level is due again (SDL_RumbleGamepad runs for a duration; the original's XInput holds a level
    /// until it changes).
    pub fn to_send(&mut self, now: f64) -> Option<(u16, u16)> {
        let out = self.output();
        let due = match self.sent {
            None => true,
            Some((l, r, t)) => (l, r) != out || (out != (0, 0) && now - t > RESEND),
        };
        if due {
            self.sent = Some((out.0, out.1, now));
            Some(out)
        } else {
            None
        }
    }

    /// Forget what was sent (a new pad): the next frame sends again.
    pub fn resend(&mut self) {
        self.sent = None;
    }

    pub fn levels(&self) -> (f32, f32) {
        (self.motors[0].level, self.motors[1].level)
    }
}

/// How long a sent level is asked to run, and when a held one is sent again (the port's: SDL's rumble
/// runs for a duration, XInput's does not).
pub const RUMBLE_MS: u32 = 1500;
const RESEND: f64 = 1.0;

/// The car's side: what its tick asked of the feedback object (PlayEffect calls in order) and the
/// landing flag (mcCar +473, F-53665).
#[derive(Clone, Debug, Default)]
pub struct CarRumble {
    pub events: Vec<u8>,
    /// mcCar byte +473: set when the car comes down after more than 0.5 s in the air, cleared once no
    /// wheel is bottomed
    pub landing: bool,
    /// chassis +224 as the tick started (what the feedback update reads)
    pub speed: f32,
    /// the feedback object has been set up for this car (F-63089)
    pub init: bool,
}

impl CarRumble {
    pub fn play(&mut self, id: u8) {
        // (the port's: a car nobody drains keeps at most a few)
        if self.events.len() < 32 {
            self.events.push(id);
        }
    }
}

/// The game's side: the player car's feedback object, the game state's switch (game.SetForceFeedback,
/// F-84376) and the pad output, with the debug log `MCLA_RUMBLE_LOG=<file>`.
pub struct PadRumble {
    pub fb: Feedback,
    /// the ground's rumble type by surface number
    types: std::collections::HashMap<u8, i8>,
    t0: std::time::Instant,
    /// game.SetForceFeedback's last state (None before the first frame)
    ff: Option<bool>,
    log: Option<std::io::BufWriter<std::fs::File>>,
    ticks: u64,
    console: bool,
}

impl PadRumble {
    pub fn new(data_dir: &std::path::Path) -> PadRumble {
        let log = std::env::var_os("MCLA_RUMBLE_LOG").and_then(|p| std::fs::File::create(p).ok()).map(std::io::BufWriter::new);
        PadRumble { fb: Feedback::default(), types: surface_types(data_dir), t0: std::time::Instant::now(), ff: None, log, ticks: 0, console: std::env::var("MCLA_RUMBLE_CLOCK").map_or(false, |v| v == "console") }
    }

    /// The clock of the motors' hold: real time, as the original's time base. (Debug, MCLA_RUMBLE_CLOCK=console:
    /// the console's - two ticks at the start of each 1001/30000 s frame, the second 5 ms later, as the recorded
    /// original runs them - for comparing headless runs that go faster than real time.)
    pub fn now(&self) -> f64 {
        if self.console {
            (self.ticks / 2) as f64 * (1001.0 / 30000.0) + (self.ticks % 2) as f64 * 0.005
        } else {
            self.t0.elapsed().as_secs_f64()
        }
    }

    fn line(&mut self, s: String) {
        if let Some(f) = self.log.as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{s}");
        }
    }

    /// One gameplay tick of the player's car (after its step, the props' pass and the abilities' strikes):
    /// the PlayEffect calls its tick asked for, then mcFeedback's update (F-25579). `pad`: the player has a
    /// pad (the original's PlayEffect needs the player's pad record; the keyboard has none). `replay`: a
    /// replay plays (PlayEffect refused, vehFeedback's option byte 0x828CCFC4 +76 bit 0x40).
    pub fn tick(&mut self, st: &mut crate::physics::CarState, pad: bool, replay: bool) {
        // (the debug log runs as with a pad)
        let pad = pad || self.log.is_some();
        self.ticks += 1;
        let now = self.now();
        let events = std::mem::take(&mut st.rumble.events);
        let Some(vs) = st.veh.as_deref() else { return };
        // (a new car: its feedback object's init, F-63089 - the gear it has is the baseline)
        if !st.rumble.init {
            st.rumble.init = true;
            self.fb.init(vs.gear, now);
        }
        for id in events {
            if pad && !replay {
                let ok = self.fb.play(id);
                self.line(format!("P k={} t={:.1} id={} ok={}", self.ticks, now * 1000.0, id, ok as u8));
            }
        }
        let mut f = Feel { gear: vs.gear, melt: vs.melt, speed: st.rumble.speed, wheel_count: if st.wheels.len() >= 4 { 4 } else { st.wheels.len() }, ..Default::default() };
        for (i, w) in vs.wheels.iter().enumerate().take(4) {
            let ty = w.ground.and_then(|g| self.types.get(&g).copied()).unwrap_or(-1);
            f.wheels[i] = (w.slide, w.comp_rate, w.bottomed, w.record, w.bump, ty);
        }
        let before = self.fb.levels();
        let asked = if pad && !replay {
            self.fb.update(&f, now)
        } else {
            self.fb.motors(now);
            Vec::new()
        };
        for id in asked {
            self.line(format!("P k={} t={:.1} id={} upd=1", self.ticks, now * 1000.0, id));
        }
        let after = self.fb.levels();
        if self.log.is_some() {
            self.line(format!("U k={} t={:.1} gear={} spd={:.4} melt={:.4} w={:?}", self.ticks, now * 1000.0, f.gear, f.speed, f.melt, f.wheels));
            for m in 0..2 {
                let (b, a) = if m == 0 { (before.0, after.0) } else { (before.1, after.1) };
                if a != b {
                    self.line(format!("A k={} t={:.1} slot={} lvl={}", self.ticks, now * 1000.0, m, a));
                }
            }
        }
    }

    /// Once a frame: game.SetForceFeedback as the game state has it (`ff`: off in the garage but on its
    /// test drive, off at a hangout), the vibration option the original's enable reads (LocalOptions
    /// +1188, F-53501), the stop while the game is held (`held`: the pause and the other stops of the
    /// game's tick - F-54667 on entry, F-77444 every frame), and the levels to the pad.
    pub fn frame(&mut self, pad: Option<&crate::pads::Pad>, ff: bool, vibration: bool, held: bool) {
        let now = self.now();
        if self.ff != Some(ff) {
            if ff {
                // (F-53501: the pad's device and the enable from the option)
                self.fb.enabled = vibration;
            } else {
                // (F-65753: disabled, the patterns dropped, the motors stopped)
                self.fb.enabled = false;
                self.fb.reset(true, now);
            }
            self.line(format!("E t={:.1} ff={} on={}", now * 1000.0, ff as u8, self.fb.enabled as u8));
            self.ff = Some(ff);
        }
        if held {
            self.fb.reset(true, now);
        }
        if pad.is_none() && self.log.is_none() {
            self.fb.resend();
            return;
        }
        if let Some((l, r)) = self.fb.to_send(now) {
            if let Some(pad) = pad {
                pad.rumble(l, r, if (l, r) == (0, 0) { 0 } else { RUMBLE_MS });
            }
            self.line(format!("X k={} t={:.1} L={} R={}", self.ticks, now * 1000.0, l, r));
        }
    }
}

/// The ground's rumble types: the `effect` field of each material's .mtl (record +56), by the port's
/// surface number (`collision::Surface as u8`).
pub fn surface_types(data_dir: &std::path::Path) -> std::collections::HashMap<u8, i8> {
    let mut out = std::collections::HashMap::new();
    let dir = data_dir.join("tune/physics/materials");
    for s in crate::physics::collision::Surface::ALL {
        let name = s.material();
        let txt = [dir.join(format!("0x{:08x}", crate::mc4::joaat(&format!("{name}.mtl")))), dir.join(format!("0x{:08x}", crate::mc4::joaat(name)))]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok());
        let Some(txt) = txt else { continue };
        if let Some(v) = txt.lines().find_map(|l| l.trim().strip_prefix("effect:").and_then(|v| v.trim().parse::<i32>().ok())) {
            out.insert(s as u8, v as i8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded original (work/rumble/cap r1 + s1, the player's feedback object): every PlayEffect
    /// with its gates passed (P id), the motors' hold test as the original timed it (T motor pass), the
    /// resets (R stop) and every SetActuator call (A motor level). Replayed through the rule, the levels
    /// set must be the recorded ones, call for call.
    #[test]
    fn replays_the_recorded_motor_levels() {
        let txt = include_str!("testdata/rumble_rec.txt");
        let mut fb = Feedback { enabled: true, ..Default::default() };
        let (mut want, mut got) = (Vec::new(), Vec::new());
        for l in txt.lines() {
            let t: Vec<&str> = l.split_whitespace().collect();
            match t.first().copied() {
                Some("P") => {
                    fb.play(t[1].parse().unwrap());
                }
                Some("T") => {
                    let m: usize = t[1].parse().unwrap();
                    if t[2] == "1" {
                        let mo = fb.motors[m];
                        if mo.active > 0 {
                            let v = fb.step(m);
                            if v != fb.motors[m].level {
                                fb.motors[m].level = v;
                                got.push((m, v));
                            }
                        } else if mo.level > 0.0 {
                            fb.motors[m].level = 0.0;
                            got.push((m, 0.0));
                        }
                    }
                }
                Some("R") => {
                    fb.reset(t[1] == "1", 0.0);
                    if t[1] == "1" {
                        got.push((0, 0.0));
                        got.push((1, 0.0));
                    }
                }
                Some("A") => want.push((t[1].parse::<usize>().unwrap(), t[2].parse::<f32>().unwrap())),
                _ => {}
            }
        }
        assert!(want.len() > 1000, "{}", want.len());
        assert_eq!(got, want);
    }

    /// The recorded original's mcFeedback updates (work/rumble/cap r1 + s1): the car's state it read (gear
    /// baseline, gear, melt, speed, per wheel slide / compression rate / bottomed / record / roughness /
    /// rumble type) and the effects it asked for (PlayEffect calls by their return address).
    #[test]
    fn update_asks_the_recorded_effects() {
        let txt = include_str!("testdata/rumble_upd.txt");
        let mut n = 0;
        for l in txt.lines() {
            let (st, ids) = l.split_once('|').unwrap();
            let t: Vec<&str> = st.split_whitespace().collect();
            let mut f = Feel { gear: t[2].parse().unwrap(), melt: t[3].parse().unwrap(), speed: t[4].parse().unwrap(), wheel_count: 4, ..Default::default() };
            for i in 0..4 {
                let w: Vec<&str> = t[5 + i].split('/').collect();
                f.wheels[i] = (w[0].parse().unwrap(), w[1].parse().unwrap(), w[2] == "1", w[3] == "1", w[4].parse().unwrap(), w[5].parse().unwrap());
            }
            let mut base: i32 = t[1].parse().unwrap();
            let want: Vec<u8> = ids.split_whitespace().map(|x| x.parse().unwrap()).collect();
            assert_eq!(Feedback::asks(&f, &mut base), want, "{l}");
            n += 1;
        }
        assert!(n > 300);
    }

    #[test]
    fn hold_is_one_thirtieth_of_real_time() {
        let mut fb = Feedback { enabled: true, ..Default::default() };
        fb.play(4);
        fb.motors(10.0);
        assert_eq!(fb.levels(), (0.75, 0.0));
        assert_eq!(fb.output(), (49151, 0));
        // (held: 1/30 s has not passed)
        fb.motors(10.0 + 1.0 / 30.0);
        assert_eq!(fb.levels(), (0.75, 0.0));
        fb.motors(10.0 + 0.034);
        assert_eq!(fb.levels(), (0.0, 0.0));
    }

    #[test]
    fn gear_change_and_surface_types_ask_their_effects() {
        let mut fb = Feedback { enabled: true, ..Default::default() };
        fb.init(2, 0.0);
        let mut f = Feel { gear: 3, speed: 12.0, wheel_count: 4, ..Default::default() };
        f.wheels[1] = (0.0, 0.0, false, true, 0.001, 1);
        fb.update(&f, 1.0);
        assert_eq!(fb.ids[..2], [4, 9]);
        // (a running effect is not started again; 5 waits for 0 / 1 / 2)
        assert!(!fb.play(9));
        fb.play(1);
        assert!(!fb.play(5));
    }
}
