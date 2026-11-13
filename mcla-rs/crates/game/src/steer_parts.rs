//! The car-parts object's eased steering and speedo values (object D at the vehicle's +84, its update F-98654): F-69494
//! moves two pairs each frame through the slew F-54584 - the speedo / steering wheel pair (+1120 / +1124) and the
//! steering pair (+1160 / +1164) - towards targets made of the racer's input values (F-80793 copies them in: +1168,
//! +1172, +1176) and the car's speed. +1164 is what a motorcycle's front wheel is drawn turned by (F-47822's bike
//! branch, wheel_pose.rs) and the rider's steering input (+1164 / +1136). Each step in the original's own operations.
use crate::physics::exact::fma;

/// The fields of D that F-69494 reads and writes. Defaults: the constructor's (F-26362).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteerParts {
    /// +1108, +1112 (the speedo pair's scale and its rate and ease; a driver of a car sets them, F-28290)
    pub needle_scale: f32,
    pub needle_rate: f32,
    /// +1120 / +1124: the speedo pair's slewed and eased values
    pub needle_mid: f32,
    pub needle: f32,
    /// +1136: the steering's scale (F-52244 rescales the pair with it: a rider's gatherer each frame, drivers.rs `rider_parts`)
    pub scale: f32,
    /// +1140 / +1144: the steering pair's slew and ease rates
    pub rate: f32,
    pub ease: f32,
    /// +1148 / +1152: while +1148 is above 0 both pairs are pulled to 0, +1148 and +1152 then their rates
    pub hold_ease: f32,
    pub hold_rate: f32,
    /// +1156: the speedo pair's second scale
    pub needle_scale2: f32,
    /// +1160 / +1164: the steering pair's slewed and eased values
    pub mid: f32,
    pub out: f32,
    /// +1168, +1172, +1176: the racer's values (F-80793: the racer's virtual +168, +176, +180)
    pub gate: f32,
    pub value: f32,
    pub divisor: f32,
    /// not D's: the player pad's +36 that feeds +1168, kept here between frames (vehicles.rs)
    pub pad_x: f32,
}

impl Default for SteerParts {
    fn default() -> Self {
        SteerParts {
            needle_scale: 1.0,
            needle_rate: 10.0,
            needle_mid: 0.0,
            needle: 0.0,
            scale: 1.0,
            rate: 3.0,
            ease: 7.0,
            hold_ease: 0.0,
            hold_rate: 0.0,
            needle_scale2: 1.0,
            mid: 0.0,
            out: 0.0,
            gate: 0.0,
            value: 0.0,
            divisor: 1.0,
            pad_x: 0.0,
        }
    }
}

/// F-54584: `mid` slews to `target` at `rate` a second (clamped at it, and set to it at once when it lies between
/// `mid` and `out`), then `out` eases to `mid` by (mid - out) x dt x `ease` unless within 1e-4. While D's +1148 is
/// above 0 the target is 0, `mid` 0 at once, and D's +1148 / +1152 are the ease and rate.
fn slew(d: &SteerParts, mut target: f32, mut rate: f32, mut ease: f32, mid: &mut f32, out: &mut f32, dt: f32) {
    if d.hold_ease > 0.0 {
        *mid = 0.0;
        target = 0.0;
        ease = d.hold_ease;
        rate = d.hold_rate;
    }
    let m = *mid;
    if (target - *out) * (target - m) < 0.0 {
        *mid = target;
    } else if m < target {
        *mid = fma(dt, rate, m);
        if *mid > target {
            *mid = target;
        }
    } else if m > target {
        *mid = -fma(dt, rate, -m);
        if *mid < target {
            *mid = target;
        }
    }
    let o = *out;
    if (o - *mid).abs() > f32::from_bits(0x38d1b717) {
        *out = fma((*mid - o) * dt, ease, o);
    }
}

impl SteerParts {
    /// F-69494, once a frame: `speed` = the chassis's +224 (0 without one), `garage` = the showroom's interior flag
    /// (byte 0x82908D5D: both targets 0), `dt` = the frame's step.
    pub fn step(&mut self, speed: f32, garage: bool, dt: f32) {
        // (the full share above 0.567 m/s with the gate value set, else half)
        let full = self.gate != 0.0 && speed > f32::from_bits(0x3f112a95);
        let share = if self.divisor > 0.0 {
            if full { self.value / self.divisor } else { self.value / self.divisor * 0.5 }
        } else {
            0.0
        };
        let target = if garage { 0.0 } else { self.value / self.divisor * self.needle_scale2 * self.needle_scale };
        let (mut m, mut o) = (self.needle_mid, self.needle);
        slew(self, target, self.needle_rate, self.needle_rate, &mut m, &mut o, dt);
        (self.needle_mid, self.needle) = (m, o);
        let target = if garage { 0.0 } else { self.scale * share };
        let (mut m, mut o) = (self.mid, self.out);
        slew(self, target, self.rate, self.ease, &mut m, &mut o, dt);
        (self.mid, self.out) = (m, o);
    }

    /// F-52244: a new steering scale (+1136), the steering pair (+1160 / +1164) rescaled by new / old. A rider's gatherer
    /// (F-28290's rider branch, drivers.rs `rider_parts`) calls it each frame with the driver's +408 x context +64.
    pub fn rescale(&mut self, s: f32) {
        let q = s / self.scale;
        self.scale = s;
        self.mid *= q;
        self.out *= q;
    }
}

/// The turns F-98654 gives the bones it holds, each frame after the skeleton's locals are reset (each bone's local
/// matrix turned in place: F-83846 about its z - rows 0 and 1 - and F-52786 about its x - rows 1 and 2 - by the guest's
/// cos / sin of the angle, the rows becoming c row + s next and c next - s row: in glam the local times the turn).
impl SteerParts {
    /// D +1216, the base skeleton's `steer` bone (a motorcycle's bars and fork): about z by -(+1164 kept within
    /// +-0.5 x +1136 by two fsel)
    pub fn steer_bone_angle(&self) -> f32 {
        let h = self.scale * 0.5;
        let lo = -h;
        let x = if self.out - lo >= 0.0 { self.out } else { lo };
        let x = if x - h >= 0.0 { h } else { x };
        -x
    }

    /// D +1200, the bone the fitted steering wheel (slot 52 `steer_whl`) hangs on: about z by
    /// +1128 - (1 - +1132) x +1124 (fnmsubs); +1128 and +1132 are 0 from the constructor and every repair (F-75221) and
    /// nothing else writes them, so this is -+1124
    pub fn steer_wheel_angle(&self) -> f32 {
        wheel_turn(self.needle, 0.0, 0.0)
    }

    /// The repair F-75221 zeroes both pairs (+1120 / +1124, +1160 / +1164).
    pub fn repair(&mut self) {
        (self.needle_mid, self.needle, self.mid, self.out) = (0.0, 0.0, 0.0, 0.0);
    }
}

/// D +1208, the fitted blower's own bone (`blower<n>`): about z by the engine's shake angle (engine +40, kept by the
/// engine update F-55648: `VehState::engine_shake`) x 0.5, only while the engine's word +46 is not 0.
pub fn blower_angle(shake: f32) -> f32 {
    shake * 0.5
}

/// The fitted steering wheel's angle from +1124, +1128 and +1132: -((1 - +1132) x +1124 - +1128), fused (fnmsubs).
pub fn wheel_turn(d1124: f32, d1128: f32, d1132: f32) -> f32 {
    -fma(1.0 - d1132, d1124, -d1128)
}

/// D +1880, the blower valve's slew (built by the constructor F-26362: rate 9, the rest 0): each frame its target is
/// the chassis' throttle (+136) and F-38786 moves it there; the valve bone (D +1212, `blowervalve<n>` of the fitted
/// blower) turns about x by the value x pi/2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Valve {
    /// +0 the rate, +4 the target, +8 the value, +12 / +16 what the repair puts back into +4 / +8
    pub rate: f32,
    pub target: f32,
    pub value: f32,
    pub start: [f32; 2],
}

impl Default for Valve {
    fn default() -> Self {
        Valve { rate: 9.0, target: 0.0, value: 0.0, start: [0.0; 2] }
    }
}

impl Valve {
    /// F-38786 with the target set: nothing within 0.01 (0x3C23D70A) of it, else a step of rate x dt (the frame
    /// step, image word 0x827D7508), no more than the distance, towards it.
    pub fn step(&mut self, target: f32, dt: f32) {
        self.target = target;
        let d = self.value - self.target;
        let step = self.rate * dt;
        // (fcmpu branches: a NaN distance goes on, takes the distance and is added)
        if d.abs() <= f32::from_bits(0x3c23_d70a) {
            return;
        }
        let step = if step <= d.abs() { step } else { d.abs() };
        self.value = if d >= 0.0 { self.value - step } else { self.value + step };
    }

    /// The valve bone's angle about x: the value x pi/2 (0x3FC90FDB).
    pub fn angle(&self) -> f32 {
        self.value * f32::from_bits(0x3fc9_0fdb)
    }

    /// The repair F-75221: +4 = +12, +8 = +16.
    pub fn repair(&mut self) {
        (self.target, self.value) = (self.start[0], self.start[1]);
    }
}

/// A bone's local turned by F-83846 (about z) or F-52786 (about x) by the angle, as a glam matrix to multiply the local
/// by on the right: the guest's cos and sin of the angle as doubles, rounded to single.
pub fn turn(about_x: bool, a: f32) -> glam::Mat4 {
    let c = crate::physics::exact::guest_cos(f64::from(a)) as f32;
    let s = crate::physics::exact::guest_sin(f64::from(a)) as f32;
    if about_x {
        glam::Mat4::from_cols(glam::Vec4::X, glam::Vec4::new(0.0, c, s, 0.0), glam::Vec4::new(0.0, -s, c, 0.0), glam::Vec4::W)
    } else {
        glam::Mat4::from_cols(glam::Vec4::new(c, s, 0.0, 0.0), glam::Vec4::new(-s, c, 0.0, 0.0), glam::Vec4::Z, glam::Vec4::W)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-52244 run offline on the original's code (tools/steer_rescale_reference.py, 2,000 cases with 0, -0, inf and NaN
    /// among them): the scale +1136 and the pair +1160 / +1164 after equal `rescale`'s bit for bit.
    #[test]
    fn rescale_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/steer_rescale_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<f32> = i.split(' ').map(|t| f32::from_bits(u32::from_str_radix(t, 16).unwrap())).collect();
            let e: Vec<u32> = o.split(' ').map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let mut p = SteerParts { scale: w[0], mid: w[1], out: w[2], ..SteerParts::default() };
            p.rescale(w[3]);
            assert_eq!([p.scale.to_bits(), p.mid.to_bits(), p.out.to_bits()].as_slice(), e.as_slice(), "{line}");
            n += 1;
        }
        assert_eq!(n, 2000);
    }

    #[test]
    fn steer_parts_match_the_original() {
        // (tools/steer_parts_reference.py: F-69494 with F-54584 run offline on sequences of frames; each line D's fields
        // before, the speed, the garage byte and the step, then the four values after)
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let (mut n, mut bad) = (0, 0);
        for line in include_str!("testdata/steer_parts_ref.txt").lines() {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let i: Vec<f32> = inp.split(' ').map(f).collect();
            let o: Vec<u32> = out.split(' ').map(|s| u32::from_str_radix(s, 16).unwrap()).collect();
            let mut d = SteerParts {
                needle_scale: i[0], needle_rate: i[1], needle_mid: i[2], needle: i[3], scale: i[4], rate: i[5], ease: i[6],
                hold_ease: i[7], hold_rate: i[8], needle_scale2: i[9], mid: i[10], out: i[11], gate: i[12], value: i[13], divisor: i[14], pad_x: 0.0,
            };
            d.step(i[15], i[16] != 0.0, i[17]);
            let got = [d.needle_mid, d.needle, d.mid, d.out].map(f32::to_bits);
            if got[..] != o[..] {
                if bad < 5 {
                    eprintln!("line {n}: {inp} -> got {got:08x?} want {o:08x?}");
                }
                bad += 1;
            }
            n += 1;
        }
        assert!(n >= 10_000);
        assert_eq!(bad, 0, "{bad} of {n} frames differ");
    }

    /// The bone turns of the car-parts update F-98654 against the original run offline (tools/part_turns_reference.py,
    /// testdata/part_turns_ref.txt: 4,000 cases of the steering values with +-0.5 and an ulp past it, -0, NaN, the
    /// flaps' angle, reversed and still bytes, the shake, the throttle, the valve's rate / target / value, dt, which
    /// bones exist, the model byte +122 and the engine's word): every call - its bone, x or z, the angle - and the
    /// valve's target and value equal (a NaN as a NaN: the x86 recomp's NaN sign is not the console's).
    #[test]
    fn part_turns_match_the_original() {
        use crate::damageparts::Flap;
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let same = |a: f32, b: u32| a.to_bits() == b || (a.is_nan() && f32::from_bits(b).is_nan());
        let mut n = 0;
        for line in include_str!("testdata/part_turns_ref.txt").lines() {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let t: Vec<&str> = inp.split(' ').collect();
            let c: Vec<f32> = t[..13].iter().map(|x| f(x)).collect();
            let b: Vec<bool> = t[13..].iter().map(|x| *x == "1").collect();
            let (calls, valve) = out.split_once("= ").unwrap();
            let want: Vec<&str> = calls.split_whitespace().collect();
            let mut got: Vec<(&str, u32, f32)> = Vec::new();
            let flap = |angle: f32, rev: bool, still: bool| Flap { reversed: rev, locked: still, angle, ..Flap::new(0.2, 1) };
            // (the trunk bone D +1204 by the flap D +1664's drawn angle; the hood D +1360 by minus D +1456's, unless the
            // model's +122)
            if b[1] {
                got.push(("x", 0x6100, flap(c[5], b[5], b[6]).drawn()));
            }
            if !b[9] {
                got.push(("x", 0x1550, -flap(c[6], b[7], b[8]).drawn()));
            }
            if b[2] && b[10] {
                got.push(("z", 0x6200, blower_angle(c[7])));
            }
            let mut v = Valve { rate: c[9], target: c[10], value: c[11], start: [0.0; 2] };
            if b[3] {
                v.step(c[8], c[12]);
                got.push(("x", 0x6300, v.angle()));
            }
            let d = SteerParts { scale: c[3], out: c[4], needle: c[0], ..SteerParts::default() };
            if b[4] {
                got.push(("z", 0x6400, d.steer_bone_angle()));
            }
            if b[0] {
                got.push(("z", 0x6000, wheel_turn(c[0], c[1], c[2])));
                if c[1].to_bits() == 0 && c[2].to_bits() == 0 {
                    assert_eq!(wheel_turn(c[0], c[1], c[2]).to_bits(), d.steer_wheel_angle().to_bits());
                }
            }
            assert_eq!(want.len(), 3 * got.len(), "case {n}: {line}");
            for (k, (axis, bone, a)) in got.iter().enumerate() {
                assert_eq!(want[3 * k], *axis, "case {n}: {line}");
                assert_eq!(u32::from_str_radix(want[3 * k + 1], 16).unwrap(), *bone, "case {n}: {line}");
                assert!(same(*a, u32::from_str_radix(want[3 * k + 2], 16).unwrap()), "case {n} call {k}: {a:e} ({:08x}): {line}", a.to_bits());
            }
            let vw: Vec<u32> = valve.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            assert!(same(v.target, vw[0]) && same(v.value, vw[1]), "case {n} valve: {line}");
            n += 1;
        }
        assert_eq!(n, 4000);
    }
}
