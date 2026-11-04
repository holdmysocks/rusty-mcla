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
    /// +1136: the steering's scale (F-52244 rescales the pair with it; 1 in every frame of play, see `rider_rates`)
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

    /// What F-28290 sets on a motorcycle's driver each frame (the branch of a driver without the steering wheel byte
    /// +370): the rates from the driver tune (mcAnimMachineConstantData +56 / +60, its constructor's 55 / 100; no
    /// data file sets it). The tune's +52 / +64 term rescales +1136 (F-52244) only while the driver's +408, moved to
    /// +52 = 25 at 9 a second, lies inside (0.101, 0.999) - never after the first frame - so +1136 stays 1. The tune's
    /// flag word +12 (0x3E000000) has neither 0x4 nor 0x8: +1148 / +1152 = 0 / 10.
    pub fn rider_rates(&mut self) {
        self.rate = 55.0;
        self.ease = 100.0;
        self.hold_ease = 0.0;
        self.hold_rate = 10.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
