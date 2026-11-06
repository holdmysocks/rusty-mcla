//! The Roar / EMP dust at the ability start (F-77604 -> F-85331): eight slots, built once (F-78479 / F-93019), each
//! taken by a Roar or an EMP as it goes off. The slot starts the rmptfx effect `roar_dust` or `emp_dust` (MC4.r107) at the
//! car with the car's velocity (F-17154), writes the two colour globals, and while young adds a light 17.8 m above the car
//! (F-98199, F-76537 -> F-31349, the per-frame light list). Run offline against the original's code:
//! `tools/pu_dust_reference.py` (testdata/pu_dust_ref.txt).
//!
//! Not here: the slot's physics behaviour (F-58802, the push of the event record F-77604 builds - the port's Roar / EMP keep
//! their own push in `ability.rs`), the matrix the slot copies into that behaviour each tick, and the shockwave model the
//! constructor loads (F-81760, `fx_shockwave_roar_2`: no draw of it was found).

use crate::physics::exact::fma;
use glam::Vec3;

pub const SLOTS: usize = 8;
/// F-77604's f2 and f1 into the slot (+72: the age's rate a second; +76: read by nothing found).
pub const START_RATE: f32 = 3.0;
pub const START_P76: f32 = 0.4;
/// F-93019: what a slot holds when built (+72, +76, +84 its life, +88 age 0).
pub const CTOR_RATE: f32 = 4.0;
pub const CTOR_P76: f32 = 0.125;
pub const LIFE: f32 = 5.0;
/// F-98199: a slot is free again once its age passes life x 4.
pub const LIFE_SCALE: f32 = 4.0;
/// F-76537 (0x827E1B68): the light is added while the age is at most 0.8, at the owner + (0, 17.8, 0) (0x827E1BA0).
pub const LIGHT_AGE: f32 = 0.8;
pub const LIGHT_UP: Vec3 = Vec3::new(0.0, 17.8, 0.0);
/// The start's light flag (r7 of F-85331: the ability start passes 8, slot +80 bit 3).
pub const LIGHT_FLAG: u32 = 8;
/// The ability start's flags (F-77604: r7 = 8).
pub const ABILITY_FLAGS: u32 = 8;
/// The clock object's +8 (0x827D7508): the step every slot update adds rate x step of.
pub const STEP: f32 = 1.0 / 60.0;
/// The colour globals 0x827E1B80 / 0x827E1B90 as the image holds them before any start, and what F-17154 writes.
/// (+B: the light's colour and, in w, its strength; A: no reader was found.)
pub const COL_A0: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
pub const COL_B0: [f32; 4] = [1.335, 2.681, 7.256, 736.0];
pub const ROAR_A: [f32; 4] = [2.45, 1.49, 0.89, 4.0];
pub const ROAR_B: [f32; 4] = [3.5, 1.0, 0.0, 736.0];
pub const EMP_A: [f32; 4] = [1.215, 1.443, 2.198, 4.0];
pub const EMP_B: [f32; 4] = [1.335, 2.681, 7.256, 736.0];

/// One slot (112 bytes in the original).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    /// +64: the car it follows (the start's r8; None also once that car is removed, F-93284)
    pub owner: Option<usize>,
    /// +72, +76, +80, +84, +88
    pub rate: f32,
    pub p76: f32,
    pub flags: u32,
    pub life: f32,
    pub age: f32,
    /// +92: in use
    pub used: bool,
}

impl Slot {
    /// F-93019.
    pub const fn constructed() -> Slot {
        Slot { owner: None, rate: CTOR_RATE, p76: CTOR_P76, flags: 0, life: LIFE, age: 0.0, used: false }
    }
}

/// What a start asks of the game: the slot taken and, when it has an owner, the effect to start at the owner's place
/// with the owner's velocity (F-17154 -> F-78885, F-11877, +416, F-80289).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Start {
    pub slot: usize,
    pub effect: Option<&'static str>,
}

/// A light the slots add this tick (F-31349: an omni record of the per-frame list).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub pos: Vec3,
    pub color: [f32; 3],
    pub strength: f32,
}

#[derive(Clone, Debug)]
pub struct Dust {
    /// the eight slot pointers (0x8287E150..): None where the allocation failed
    pub slots: [Option<Slot>; SLOTS],
    /// 0x827E1B80, 0x827E1B90: global, so the last start colours every slot's light
    pub col_a: [f32; 4],
    pub col_b: [f32; 4],
}

impl Default for Dust {
    fn default() -> Dust {
        Dust::new()
    }
}

impl Dust {
    /// F-78479: eight slots, each built by F-93019.
    pub fn new() -> Dust {
        Dust { slots: [Some(Slot::constructed()); SLOTS], col_a: COL_A0, col_b: COL_B0 }
    }

    /// F-85331 with F-17154: the first slot not in use (none: nothing happens). `word` is the event record's first word
    /// (F-77604: 1 for Roar, 2 for EMP; 2 picks emp_dust and its colours, anything else roar_dust).
    pub fn start(&mut self, word: u32, owner: Option<usize>, flags: u32) -> Option<Start> {
        let k = self.slots.iter().position(|s| matches!(s, Some(s) if !s.used))?;
        let s = self.slots[k].as_mut()?;
        s.flags = flags;
        s.used = true;
        s.age = 0.0;
        s.p76 = START_P76;
        s.rate = START_RATE;
        s.owner = owner;
        // (F-17154: only with an owner; the colours are written whether or not the effect could be made)
        let effect = owner.map(|_| {
            let (name, a, b) = if word == 2 { ("emp_dust", EMP_A, EMP_B) } else { ("roar_dust", ROAR_A, ROAR_B) };
            self.col_a = a;
            self.col_b = b;
            name
        });
        Some(Start { slot: k, effect })
    }

    /// F-22666 -> F-98199 -> F-76537, once a step of `dt` (the clock object's +8, [`STEP`]): age += rate x dt (fused), the
    /// slot is free when the age passes life x 4, else (light flag) a light while age <= 0.8 of strength
    /// B.w (1 - sat(age / 0.8 - 0.5)) at the owner + (0, 17.8, 0). `pos_of`: an owner's matrix translation.
    pub fn tick(&mut self, dt: f32, pos_of: &dyn Fn(usize) -> Option<Vec3>) -> Vec<Light> {
        let mut out = Vec::new();
        for s in self.slots.iter_mut().flatten() {
            if !s.used {
                continue;
            }
            s.age = fma(s.rate, dt, s.age);
            if s.age > s.life * LIFE_SCALE {
                s.used = false;
                continue;
            }
            if s.flags & LIGHT_FLAG == 0 || s.age > LIGHT_AGE {
                continue;
            }
            let x = s.age / LIGHT_AGE - 0.5;
            let x = if x >= 0.0 { x } else { 0.0 };
            let x = if x - 1.0 >= 0.0 { 1.0 } else { x };
            let w = self.col_b[3];
            let strength = fma(x, -w, w);
            if let Some(p) = s.owner.and_then(pos_of) {
                out.push(Light { pos: p + LIGHT_UP, color: [self.col_b[0], self.col_b[1], self.col_b[2]], strength });
            }
        }
        out
    }

    /// F-84687: every slot free (F-11314 does this, then sweeps the behaviour pool the port does not have).
    pub fn reset(&mut self) {
        for s in self.slots.iter_mut().flatten() {
            s.used = false;
        }
    }

    /// F-93284: a car is removed - the slots in use that follow it follow nothing.
    pub fn forget(&mut self, owner: usize) {
        for s in self.slots.iter_mut().flatten() {
            if s.used && s.owner == Some(owner) {
                s.owner = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> u32 {
        u32::from_str_radix(s, 16).unwrap()
    }
    fn f(s: &str) -> f32 {
        f32::from_bits(w(s))
    }
    fn owner(s: &str) -> Option<usize> {
        match s {
            "0" => None,
            "1" => Some(0),
            "2" => Some(1),
            _ => panic!("owner {s}"),
        }
    }

    /// The full slot words of a ST line (owner:rate:+76:flags:life:age:behaviour:used, '-' a null slot) and the colours.
    fn check_full(d: &Dust, line: &str, at: usize) {
        let t: Vec<&str> = line.split(' ').collect();
        assert_eq!(t[0], "ST", "line {at}");
        for k in 0..SLOTS {
            let e = t[1 + k];
            match d.slots[k] {
                None => assert_eq!(e, "-", "line {at} slot {k}"),
                Some(s) => {
                    let v: Vec<&str> = e.split(':').collect();
                    assert_eq!(s.owner, owner(v[0]), "line {at} slot {k} owner");
                    assert_eq!(s.rate.to_bits(), w(v[1]), "line {at} slot {k} rate");
                    assert_eq!(s.p76.to_bits(), w(v[2]), "line {at} slot {k} +76");
                    assert_eq!(s.flags, w(v[3]), "line {at} slot {k} flags");
                    assert_eq!(s.life.to_bits(), w(v[4]), "line {at} slot {k} life");
                    assert_eq!(s.age.to_bits(), w(v[5]), "line {at} slot {k} age");
                    assert_eq!(s.used, v[7] == "1", "line {at} slot {k} used");
                }
            }
        }
        assert_eq!(t[9], "A");
        assert_eq!(t[14], "B");
        for i in 0..4 {
            assert_eq!(d.col_a[i].to_bits(), w(t[10 + i]), "line {at} colour A");
            assert_eq!(d.col_b[i].to_bits(), w(t[15 + i]), "line {at} colour B");
        }
    }

    fn check_short(d: &Dust, line: &str, at: usize) {
        let t: Vec<&str> = line.split(' ').collect();
        assert_eq!(t[0], "SA", "line {at}");
        for k in 0..SLOTS {
            let e = t[1 + k];
            match d.slots[k] {
                None => assert_eq!(e, "-", "line {at} slot {k}"),
                Some(s) => {
                    let v: Vec<&str> = e.split(':').collect();
                    assert_eq!(s.owner, owner(v[0]), "line {at} slot {k} owner");
                    assert_eq!(s.age.to_bits(), w(v[1]), "line {at} slot {k} age");
                    assert_eq!(s.used, v[2] == "1", "line {at} slot {k} used");
                }
            }
        }
    }

    fn v3(t: &[&str]) -> Vec3 {
        Vec3::new(f(t[0]), f(t[1]), f(t[2]))
    }

    /// The original's slot code run offline (tools/pu_dust_reference.py): the constructor's values, every start (slot
    /// pick, fields, effect name, its place and velocity, the colour globals), every tick (age, free, lights: place,
    /// colour, strength), reset and forget - every word equal.
    #[test]
    fn slots_match_the_original() {
        // (words split as 82xx_xxxx are joined again; the effect names keep their '_')
        let text: Vec<String> = include_str!("testdata/pu_dust_ref.txt").lines().map(|l| if l.starts_with("EFF ") { l.to_string() } else { l.replace('_', "") }).collect();
        let lines: Vec<&str> = text.iter().map(|l| l.as_str()).collect();
        let mut i = 0;
        let mut d = Dust::new();
        let (mut starts, mut names, mut effects, mut ticks, mut lights, mut freed, mut cases) = (0, 0, 0, 0, 0, 0, 0);
        while i < lines.len() {
            let l = lines[i];
            let t: Vec<&str> = l.split(' ').collect();
            match t[0] {
                "#" => {}
                "CASE" => {
                    cases += 1;
                    i += 1;
                    let st: Vec<&str> = lines[i].split(' ').collect();
                    d = Dust::new();
                    for k in 0..SLOTS {
                        if st[1 + k] == "-" {
                            d.slots[k] = None;
                        }
                    }
                    check_full(&d, lines[i], i);
                }
                "START" => {
                    starts += 1;
                    let (word, own, flags) = (t[1].parse().unwrap(), owner(t[2]), t[3].parse().unwrap());
                    let made = t[4] == "1";
                    let vel = v3(&t[5..8]);
                    let pos = [v3(&t[8..11]), v3(&t[11..14])];
                    let r = d.start(word, own, flags);
                    i += 1;
                    if let Some(name) = r.and_then(|r| r.effect) {
                        assert_eq!(lines[i], format!("EFF {name}"), "line {i}");
                        names += 1;
                        i += 1;
                        if made {
                            effects += 1;
                            // the effect at the owner's matrix translation (w 1), its +416 the owner's velocity (w 0)
                            let p = pos[own.unwrap()];
                            let e: Vec<&str> = lines[i].split(' ').collect();
                            assert_eq!(e[0], "POS");
                            assert_eq!([w(e[1]), w(e[2]), w(e[3]), w(e[4])], [p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), 1f32.to_bits()], "line {i}");
                            assert_eq!(e[5], "VEL");
                            assert_eq!([w(e[6]), w(e[7]), w(e[8]), w(e[9])], [vel.x.to_bits(), vel.y.to_bits(), vel.z.to_bits(), 0], "line {i}");
                            i += 1;
                            assert_eq!(lines[i], "RUN", "line {i}");
                            i += 1;
                        }
                    } else {
                        assert!(!lines[i].starts_with("EFF"), "line {i}");
                    }
                    assert_eq!(lines[i], format!("R {}", r.map_or(-1, |r| r.slot as i32)), "line {i}");
                    i += 1;
                    check_full(&d, lines[i], i);
                }
                "TICK" => {
                    ticks += 1;
                    let dt = f(t[1]);
                    let pos = [v3(&t[2..5]), v3(&t[5..8])];
                    let used: Vec<bool> = d.slots.iter().map(|s| s.is_some_and(|s| s.used)).collect();
                    let got = d.tick(dt, &|o| Some(pos[o]));
                    freed += used.iter().zip(d.slots.iter()).filter(|(u, s)| **u && !s.is_some_and(|s| s.used)).count();
                    for g in &got {
                        i += 1;
                        lights += 1;
                        let e: Vec<&str> = lines[i].split(' ').collect();
                        assert_eq!(e[0], "L", "line {i}");
                        let want: Vec<u32> = e[1..].iter().map(|x| w(x)).collect();
                        let have = [g.pos.x.to_bits(), g.pos.y.to_bits(), g.pos.z.to_bits(), 1f32.to_bits(), 1f32.to_bits(), 0, 0,
                            g.color[0].to_bits(), g.color[1].to_bits(), g.color[2].to_bits(), g.strength.to_bits()];
                        assert_eq!(want, have, "line {i}");
                    }
                    i += 1;
                    check_short(&d, lines[i], i);
                }
                "RESET" => {
                    d.reset();
                    i += 1;
                    check_full(&d, lines[i], i);
                }
                "FORGET" => {
                    d.forget(owner(t[1]).unwrap());
                    i += 1;
                    check_short(&d, lines[i], i);
                }
                _ => panic!("line {i}: {l}"),
            }
            i += 1;
        }
        assert_eq!((cases, starts, names, effects, ticks, lights), (40, 430, 256, 224, 6456, 2381));
        assert_eq!(freed, 19, "slots freed by age");
    }

    #[test]
    fn steps_of_the_original() {
        // (the step is the clock object's +8 as the image holds it)
        assert_eq!(STEP.to_bits(), 0x3c88_8889);
        // a slot lives 401 steps of 3 / 60 (the age passes 20 = 5 x 4 at the 401st); its light 15 (the 16th step's age, summed in
        // floats, is just above 0.8)
        let mut d = Dust::new();
        d.start(1, Some(0), ABILITY_FLAGS).unwrap();
        let (mut n, mut lit) = (0, 0);
        while d.slots[0].unwrap().used {
            lit += d.tick(STEP, &|_| Some(Vec3::ZERO)).len();
            n += 1;
        }
        assert_eq!((n, lit), (401, 15));
    }
}
