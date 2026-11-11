//! The car's suspension parts: the vehSuspension objects the car builder F-70909 makes from the arm / shock / guide
//! bones, their update F-17019 each tick, and the pose F-98654 copying each one's matrix into the model's bone
//! (docs/reference/carparts/susp_parts_rule.md).

use crate::physics::exact::fma;

/// The bone names of the twelve suspension holders, in the order of the model's table (F-13644: "%s%s0" of arm / shock /
/// guide and fl / fr / rl / rr) - the car-parts object's D+1220 holds the bones' local matrices in this order.
pub const SUSP_BONES: [&str; 12] = ["armfl0", "armfr0", "armrl0", "armrr0", "shockfl0", "shockfr0", "shockrl0", "shockrr0", "guidefl0", "guidefr0", "guiderl0", "guiderr0"];

/// The holders the car builder F-70909 makes a vehSuspension for - all but the front guides (8, 9) -, each with its wheel
/// (the port's order: front left, front right, rear left, rear right - chassis +412, +420, +416, +424) and whether the rear
/// axle is its partner (the rear shocks).
pub fn susp_slot(holder: usize) -> Option<(usize, bool)> {
    match holder {
        0..=7 => Some((holder % 4, holder == 6 || holder == 7)),
        10 | 11 => Some((holder - 8, false)),
        _ => None,
    }
}

/// A vehSuspension (vtable 0x8207E8E4 class: ctor F-18804, owned-copy set-up F-68963, Init F-66627, update F-17019): the
/// bone's local matrix M (the copy the part owns, +8) that the update reshapes from its wheel's axle displacement every
/// tick, and that the pose F-98654 copies into the car model's bone - the drawn arm, shock or guide follows its wheel.
#[derive(Clone, Debug)]
pub struct SuspPart {
    /// +96: -1 none, 0 a pitched arm (row 2's y), 1 a strut (row 2's length), 2 / 3 a slider (the place along row 2)
    pub mode: i16,
    /// +92: 1 / (L.r2 . (wheel rest - L.r3)), L the bone's global
    pub inv: f32,
    /// the wheel's rest place x (+384)
    pub rest_x: f32,
    /// +84 set: the rear axle (the mean of its two wheels' displacements)
    pub partner: bool,
    /// M, rows (row 3 the place)
    pub m: [[f32; 4]; 4],
}

impl SuspPart {
    /// F-66627 for a bone found (M not null): `local` and `global` the bone's matrices (rows), `rest` the wheel's rest
    /// place, `steer` its wheel tune's +40 (SteeringLimit). The +88 the mode-0 branch reads is never written (0 from the
    /// constructor): that branch never runs.
    pub fn new(local: [[f32; 4]; 4], global: [[f32; 4]; 4], rest: [f32; 3], steer: f32, partner: bool) -> SuspPart {
        let v = [rest[0] - global[3][0], rest[1] - global[3][1], rest[2] - global[3][2]];
        let r2 = global[2];
        let inv = 1.0 / fma(r2[0], v[0], fma(r2[1], v[1], r2[2] * v[2]));
        let near = steer > 0.0 && rest[0].abs() < 0.1;
        // (the place's xyz under the compare mask 0x82860950, all ones in the running original)
        let mode = if r2[1].abs() > 0.7 {
            if near && local[3][0] == 0.0 && local[3][1] == 0.0 && local[3][2] == 0.0 { 3 } else { 1 }
        } else if near {
            2
        } else {
            0
        };
        SuspPart { mode, inv, rest_x: rest[0], partner, m: local }
    }

    /// F-17019 on M: `d` the wheel's axle displacement (F-48387), `axle` the rear axle's two (left, right) when it is the
    /// partner. (The update's +16 = M x the chassis matrix is the place the bike gyro reads - `physics::vehsim::SuspLever`.)
    pub fn update(&mut self, d: f32, axle: (f32, f32)) {
        use crate::physics::exact::fnms;
        let m = &mut self.m;
        let mut d = d;
        if m[3][0].abs() > 0.1 && m[2][0].abs() < 0.1 {
            let s = (m[3][0] / self.rest_x).abs();
            let p = if self.partner { (axle.1 + axle.0) * 0.5 } else { 0.0 };
            d = fma(d - p, s, p);
        }
        match self.mode {
            0 => m[2][1] = self.inv * d,
            1 => {
                let f6 = fnms(self.inv, d, 1.0);
                let f2 = if f6 - 0.001 >= 0.0 { f6 } else { 0.001 };
                let r = m[2];
                let l2 = fma(r[2], r[2], fma(r[0], r[0], r[1] * r[1]));
                let e = (1.0 / f64::from(l2).sqrt()) as f32 as f64;
                let l = f64::from(l2);
                let e1 = (-(e * e).mul_add(l, -1.0)).mul_add(e * 0.5, e);
                let e2 = (-(e1 * e1).mul_add(l, -1.0)).mul_add(e1 * 0.5, e1) as f32;
                let rs = if -l2 >= 0.0 { 0.0 } else { e2 };
                let s = rs * f2;
                for x in &mut m[2] {
                    *x *= s;
                }
            }
            2 | 3 => {
                let r2 = m[2];
                for k in 0..4 {
                    m[3][k] = r2[k] * -d;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn susp_parts_match_the_original() {
        // (tools/susp_parts_reference.py: the constructor, F-66627 and six ticks of F-17019 run offline; each line partner,
        // steer, rest, local, global and per tick the wheel's and the axle's two displacements, then the mode, +92 and M
        // after each tick)
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let (mut n, mut bad) = (0, 0);
        for line in include_str!("testdata/susp_parts_ref.txt").lines() {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let t: Vec<&str> = inp.split(' ').collect();
            let i: Vec<f32> = t[1..].iter().map(|s| f(s)).collect();
            let o: Vec<&str> = out.split(' ').collect();
            let rows = |k: usize| -> [[f32; 4]; 4] { std::array::from_fn(|r| std::array::from_fn(|c| i[k + 4 * r + c])) };
            let mut p = SuspPart::new(rows(4), rows(20), [i[1], i[2], i[3]], i[0], t[0] == "1");
            let mut got = vec![format!("{}", p.mode), format!("{:08x}", p.inv.to_bits())];
            for tick in 0..6 {
                let k = 36 + 3 * tick;
                p.update(i[k], (i[k + 1], i[k + 2]));
                got.extend(p.m.iter().flatten().map(|x| format!("{:08x}", x.to_bits())));
            }
            if got != o {
                if bad < 5 {
                    eprintln!("line {n}: {inp}\n  got  {}\n  want {out}", got.join(" "));
                }
                bad += 1;
            }
            n += 1;
        }
        assert_eq!(n, 3000);
        assert_eq!(bad, 0, "{bad} of {n} differ");
    }
}
