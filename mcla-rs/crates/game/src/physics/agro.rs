//! The special-ability contact hook (F-46500, mcCarInstance slot 23, pre-solve, once per step) and the Agro strike
//! (F-70024): the Agro user's car, touching an ambient car, sets that car's velocity to its own, throws it sideways
//! and up and forward (the tune's Force in the Agro car's own axes) with a spin about the vertical; the user's side
//! is a camera jolt, the stat gAGH and a burst of aerodynamic damping. Every other contact of the user's car arms
//! the gauge's strike (F-85219 vehicle, F-25896 prop, F-74524 anything else on a wall) while no ability runs.
//! The rules are `work/cameras2/r5_rules.md` 1 - 4; the hook's walk of the contacts is `loose.rs`
//! (`LooseState::agro_contacts`) and `vehsim.rs` (the city's), this file holds the numbers.
//!
//! Not ported (approx.): rumble (D5); the struck mcCar's damage and wipeout and the instance-flagged branch (archetype
//! type flags 0x0C: which instances carry them is not traced - a racer or police car is therefore never struck);
//! the Agro-crash sounds; the zeroing of the contact's last impulse (probably no effect, the solve rewrites it).

use crate::vehicles::parked::Mwc;
use glam::Vec3;

/// The agro file's `specialAbility` fields the strike reads (the copy at ability +624, F-58558).
#[derive(Clone, Copy, Debug)]
pub struct Tune {
    /// Force (T +64): the push on the struck car, in the Agro car's right / up / back axes (m/s x 30)
    pub force: Vec3,
    /// StrikeTorque (T +80) and StrikeTorqueVariant (T +96): the spin (rad/s) and its random range
    pub strike_torque: Vec3,
    pub variant: Vec3,
    /// ShakeTime / ShakeScale (T +112 / +116): the Agro user's camera jolt
    pub shake_time: f32,
    pub shake_scale: f32,
    /// PlayerAccel (T +120): an acceleration, along the car's back axis negated, the user's car gets
    pub player_accel: f32,
    /// SpeedAndForceScale (T +124) and ThresholdSpeed (T +48)
    pub speed_force_scale: f32,
    pub threshold_speed: f32,
    /// DelayBetweenAgroStrikes (T +128): a struck car is struck again after this many seconds
    pub delay: f32,
    /// RandomSide (T +133), AgroFeedback (T +134, the rumble: D5), ScaleAgroBasedOnSpeed (T +132)
    pub random_side: bool,
    pub feedback: bool,
    pub scale_by_speed: bool,
}

impl Tune {
    /// What the shipped agro file says (the fallback when the file cannot be read).
    pub const FILE: Tune = Tune {
        force: Vec3::new(400.0, 50.0, -500.0),
        strike_torque: Vec3::new(0.0, 8.0, 0.0),
        variant: Vec3::new(0.0, 2.0, 0.0),
        shake_time: 0.5,
        shake_scale: 1.0,
        player_accel: 0.0,
        speed_force_scale: 0.0,
        threshold_speed: 20.0,
        delay: 1.0,
        random_side: true,
        feedback: true,
        scale_by_speed: false,
    };

    /// The fields out of a tune file's text (`Name v` or `Name x y z`).
    pub fn read(text: &str, mut t: Tune) -> Tune {
        for l in text.lines() {
            let mut it = l.split_whitespace();
            let Some(k) = it.next() else { continue };
            let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
            let one = v.first().copied();
            let three = (v.len() >= 3).then(|| Vec3::new(v[0], v[1], v[2]));
            match k {
                "Force" => t.force = three.unwrap_or(t.force),
                "StrikeTorque" => t.strike_torque = three.unwrap_or(t.strike_torque),
                "StrikeTorqueVariant" => t.variant = three.unwrap_or(t.variant),
                "ShakeTime" => t.shake_time = one.unwrap_or(t.shake_time),
                "ShakeScale" => t.shake_scale = one.unwrap_or(t.shake_scale),
                "PlayerAccel" => t.player_accel = one.unwrap_or(t.player_accel),
                "SpeedAndForceScale" => t.speed_force_scale = one.unwrap_or(t.speed_force_scale),
                "ThresholdSpeed" => t.threshold_speed = one.unwrap_or(t.threshold_speed),
                "DelayBetweenAgroStrikes" => t.delay = one.unwrap_or(t.delay),
                "RandomSide" => t.random_side = one.map_or(t.random_side, |x| x != 0.0),
                "AgroFeedback" => t.feedback = one.map_or(t.feedback, |x| x != 0.0),
                "ScaleAgroBasedOnSpeed" => t.scale_by_speed = one.map_or(t.scale_by_speed, |x| x != 0.0),
                _ => {}
            }
        }
        t
    }
}

/// What a strike gives the struck car (F-70024): the push `R . F` (a velocity change x 30) and the spin change Q
/// (rad/s, world axes), and the camera jolt's amplitude.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strike {
    pub push: Vec3,
    pub spin: Vec3,
    pub amp: f32,
}

/// The velocity change of a push, per second of the file's Force: `R . F` is taken over this many (x 1 / 30, with the
/// mass: the impulse; F-70024 multiplies by `mass x 0.033333335`).
pub const PUSH_SCALE: f32 = 0.033_333_335;

/// F-70024: the Force and torque of one strike. `rows` the Agro car's body rows (right, up, back), `speed` its chassis
/// speed (+224), `struck_yaw` the struck body's turning rate about the vertical (+292, the flagged branch's side).
/// `flagged`: the instance carries archetype type flags 0x0C (never, here: see the file's head).
/// The draws are the shared MWC's, in the order x1, x2, x3 and (RandomSide, unflagged) x4; a 23-bit float each.
pub fn strike(t: &Tune, flagged: bool, struck_yaw: f32, rows: &[Vec3; 3], speed: f32, rng: &mut Mwc) -> Strike {
    let mut f = t.force;
    let mut q = Vec3::ZERO;
    if flagged {
        q = t.strike_torque;
        q.y *= if struck_yaw >= 0.0 { 1.0 } else { -1.0 };
    }
    let v = t.variant;
    let x = [(rng.next() & 0x7F_FFFF) as f32, (rng.next() & 0x7F_FFFF) as f32, (rng.next() & 0x7F_FFFF) as f32];
    // (fmadds( fmuls(fsubs(V, -V), x), 2^-23, -V ) + Q: each component U[-V, V))
    let k = 1.0 / 8_388_608.0;
    let draw = |v: f32, x: f32, q: f32| ((v - -v) * x).mul_add(k, -v) + q;
    q = Vec3::new(draw(v.x, x[0], q.x), draw(v.y, x[1], q.y), draw(v.z, x[2], q.z));
    let mut s = speed - t.threshold_speed;
    if !(s >= 1.0) {
        s = 1.0;
    }
    f.z = (-t.speed_force_scale).mul_add(s, f.z);
    if flagged {
        f.z = 0.0;
    }
    let mut amp = t.shake_scale;
    if t.scale_by_speed {
        // (F-91892: 0 at rest, 1 from ThresholdSpeed up, linear between)
        let k = if speed <= 0.0 { 0.0 } else if speed >= t.threshold_speed { 1.0 } else { speed / t.threshold_speed };
        amp *= k;
        q *= k;
        f *= k;
    }
    if !flagged && t.random_side {
        let r = rng.next();
        if (r & 0x7FFF_FFFF) & 1 != 0 {
            q.y = -q.y;
            f.x = -f.x;
        }
    }
    Strike { push: rows[0] * f.x + rows[1] * f.y + rows[2] * f.z, spin: q, amp }
}

/// The cooldown ring (F-41549): 64 {struck body, time} entries and the index it starts from.
#[derive(Clone, Copy, Debug)]
pub struct Ring {
    e: [(u32, f32); 64],
    head: usize,
}

impl Default for Ring {
    fn default() -> Ring {
        Ring { e: [(0, 0.0); 64], head: 0 }
    }
}

impl Ring {
    /// True: `key` was struck within `delay` seconds (inclusive) of `now` - no strike. False: it is recorded now.
    /// (approx.: the head after a ring of 65 live entries, `r5_rules.md` 3, "I on head" - not reachable in play.)
    pub fn inside(&mut self, now: f32, key: u32, delay: f32) -> bool {
        let mut i = self.head;
        for _ in 0..65 {
            let e = &mut self.e[i];
            if e.0 == 0 {
                *e = (key, now);
                return false;
            }
            let d = now - e.1;
            if d < 0.0 || d > delay {
                e.0 = 0;
                self.head = (i + 1) % 64;
            } else if e.0 == key {
                return true;
            }
            i = (i + 1) % 64;
        }
        self.e[self.head] = (key, now);
        self.head = (i + 1) % 64;
        false
    }
}

/// The class of the other instance of a contact, as the hook sorts it (F-46500).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// an mcCar (8) or ambient traffic (6)
    Vehicle,
    /// a prop (4, 5, 9)
    Prop,
    /// anything else: the city, a torn-off part
    Other,
}

/// What the hook leaves for the game to take once a frame.
#[derive(Clone, Debug, Default)]
pub struct Out {
    /// camera jolts (ShakeTime, amplitude) of the strikes
    pub jolts: Vec<(f32, f32)>,
    /// strikes counted (the stat gAGH)
    pub strikes: u32,
    /// the gauge's strike last armed: 0 vehicle, 1 prop, 2 city
    pub gauge: Option<usize>,
}

/// The hook of the car whose driver owns the ability: the ability's state as the game sets it each frame, the
/// cooldown and what the contacts left.
#[derive(Clone, Debug, Default)]
pub struct Hook {
    /// the car is the ability owner's (the human player's)
    pub armed: bool,
    /// Agro runs (ability +48 == 3 and +52 == 1)
    pub agro_run: bool,
    /// any ability runs (the gauge's strikes are not armed), or the car's crash flags (chassis +592) are set
    pub running: bool,
    pub crashed: bool,
    /// the agro tune (T)
    pub tune: Option<Tune>,
    /// the unpaused clock the cooldown reads
    pub now: f32,
    pub ring: Ring,
    pub out: Out,
    /// aero_damp = 3.0 asked for (a car, not a bike; every Agro contact with a struck body)
    pub damp: bool,
    /// force the user's car is given along its back axis (PlayerAccel x mass), summed over the step's strikes
    pub accel: f32,
}

impl Hook {
    /// The gauge's strike for a contact with `class`; `n_y` the contact normal's height, signed into the car.
    pub fn arm(&mut self, class: Class, n_y: f32) {
        if self.agro_run || self.running || self.crashed {
            return;
        }
        match class {
            Class::Vehicle => self.out.gauge = Some(0),
            Class::Prop => self.out.gauge = Some(1),
            Class::Other if n_y < 0.5 => self.out.gauge = Some(2),
            Class::Other => {}
        }
    }

    /// The user's side of a strike that passed the cooldown (F-70024): the jolt, the count, the force.
    pub fn struck(&mut self, amp: f32) {
        let Some(t) = self.tune else { return };
        self.out.jolts.push((t.shake_time, amp));
        self.out.strikes += 1;
        self.accel += t.player_accel;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];

    /// The draws a seed gives, by the MWC's own arithmetic (a = 0x5CDCFAA7).
    fn draws(lo: u32, hi: u32, n: usize) -> Vec<u32> {
        let (mut lo, mut hi) = (lo as u64, hi as u64);
        (0..n)
            .map(|_| {
                let t = 0x5CDC_FAA7u64 * lo + hi;
                lo = t & 0xFFFF_FFFF;
                hi = t >> 32;
                lo as u32
            })
            .collect()
    }

    #[test]
    fn strike_of_the_shipped_file() {
        let t = Tune::FILE;
        for seed in [(1u32, 0u32), (0x2545_F491, 1), (12345, 7), (0xFFFF_FFFF, 3)] {
            let d = draws(seed.0, seed.1, 4);
            let s = strike(&t, false, 0.0, &ROWS, 30.0, &mut Mwc { lo: seed.0, carry: seed.1 });
            // Q = U[-V, V) per axis, from the first three draws; x and z have V = 0
            let u = |v: f32, r: u32| ((v + v) * (r & 0x7F_FFFF) as f32).mul_add(1.0 / 8_388_608.0, -v);
            let flip = (d[3] & 0x7FFF_FFFF) & 1 != 0;
            let q = Vec3::new(u(0.0, d[0]), u(2.0, d[1]), u(0.0, d[2]));
            assert_eq!(s.spin, if flip { Vec3::new(q.x, -q.y, q.z) } else { q });
            assert!(s.spin.y >= -2.0 && s.spin.y < 2.0);
            // the push: Force in the car's own axes, the sideways part to the other side with the spin
            assert_eq!(s.push, Vec3::new(if flip { -400.0 } else { 400.0 }, 50.0, -500.0));
            assert_eq!(s.amp, 1.0);
            // the shipped numbers: 13.33 m/s sideways, 1.667 up, 16.67 forward
            let dv = s.push * PUSH_SCALE;
            assert!((dv.x.abs() - 13.333).abs() < 1e-3 && (dv.y - 1.6667).abs() < 1e-3 && (dv.z + 16.667).abs() < 1e-3);
        }
    }

    #[test]
    fn strike_uses_the_cars_axes_and_the_speed_scale() {
        let t = Tune { random_side: false, speed_force_scale: 2.0, ..Tune::FILE };
        // the car turned a quarter: right = -z, back = +x
        let rows = [-Vec3::Z, Vec3::Y, Vec3::X];
        let s = strike(&t, false, 0.0, &rows, 25.0, &mut Mwc { lo: 5, carry: 1 });
        // F.z = -500 - 2 x (25 - 20) = -510
        assert_eq!(s.push, -Vec3::Z * 400.0 + Vec3::Y * 50.0 + Vec3::X * -510.0);
        // below ThresholdSpeed + 1 the excess is 1
        let s = strike(&t, false, 0.0, &ROWS, 3.0, &mut Mwc { lo: 5, carry: 1 });
        assert_eq!(s.push.z, -502.0);
        // ScaleAgroBasedOnSpeed: half of ThresholdSpeed -> half of everything, shake included
        let t = Tune { random_side: false, scale_by_speed: true, ..Tune::FILE };
        let s = strike(&t, false, 0.0, &ROWS, 10.0, &mut Mwc { lo: 5, carry: 1 });
        assert_eq!((s.push, s.amp), (Vec3::new(200.0, 25.0, -250.0), 0.5));
    }

    #[test]
    fn strike_draws_in_order() {
        // three draws without RandomSide, four with it
        let mut a = Mwc { lo: 77, carry: 5 };
        strike(&Tune { random_side: false, ..Tune::FILE }, false, 0.0, &ROWS, 0.0, &mut a);
        let mut b = Mwc { lo: 77, carry: 5 };
        strike(&Tune::FILE, false, 0.0, &ROWS, 0.0, &mut b);
        let mut c = Mwc { lo: 77, carry: 5 };
        for _ in 0..3 {
            c.next();
        }
        assert_eq!((a.lo, a.carry), (c.lo, c.carry));
        c.next();
        assert_eq!((b.lo, b.carry), (c.lo, c.carry));
    }

    #[test]
    fn cooldown_ring() {
        let mut r = Ring::default();
        assert!(!r.inside(10.0, 7, 1.0));
        // inside the window, the end of it included
        assert!(r.inside(10.5, 7, 1.0));
        assert!(r.inside(11.0, 7, 1.0));
        // another body is its own
        assert!(!r.inside(10.5, 9, 1.0));
        assert!(r.inside(10.6, 9, 1.0));
        // past the window the entry is dropped and the body is struck again
        assert!(!r.inside(11.01, 7, 1.0));
        assert!(r.inside(11.5, 7, 1.0));
        // a clock gone back drops the entry too
        assert!(!r.inside(5.0, 7, 1.0));
    }

    #[test]
    fn gauge_arming() {
        let mut h = Hook { armed: true, ..Default::default() };
        h.arm(Class::Prop, 0.0);
        assert_eq!(h.out.gauge, Some(1));
        h.arm(Class::Other, 1.0);
        assert_eq!(h.out.gauge, Some(1));
        h.arm(Class::Other, 0.4);
        assert_eq!(h.out.gauge, Some(2));
        h.arm(Class::Vehicle, 1.0);
        assert_eq!(h.out.gauge, Some(0));
        h.out.gauge = None;
        h.running = true;
        h.arm(Class::Vehicle, 0.0);
        assert_eq!(h.out.gauge, None);
        h.running = false;
        h.crashed = true;
        h.arm(Class::Vehicle, 0.0);
        assert_eq!(h.out.gauge, None);
        h.crashed = false;
        h.agro_run = true;
        h.arm(Class::Vehicle, 0.0);
        assert_eq!(h.out.gauge, None);
    }
}
