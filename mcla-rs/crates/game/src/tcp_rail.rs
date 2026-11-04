//! The original's arc-line-arc rail `mcRailAmbientTCP` operation for operation: an arc about c1 from the
//! start P, the tangent line A -> B, an arc about c2 into the end T. The builder F-85722 with its solver
//! F-13911, the point F-56929 and tangent F-28568 (the vtable thunks F-88723 / F-31489 return both),
//! the phase advance F-70353, the length F-62306, the largest single turn F-39283, the total turn F-65534.
//! Record (+offsets in the original): A +0, B +16, a1 +32, a2 +36 (signed turns), c1 +48, c2 +64, R +80,
//! P +96, T +112, H0 +128, H1 +144, flags +160 (0x80 start sense, 0x40 end sense, 0x20 valid).
//! Vector-unit steps flush denormals in and out (the SDK's flush mode), their multiply-add is two
//! roundings and the three-lane dot is (0 + z z) + (y y + x x); scalar `fmadds` is fused.
//! Decode and evidence: docs/reference/people/tcp_rail_rule.md (tools/tcp_rail_reference.py).
#![allow(dead_code)] // (the traffic rails move onto it next)
use crate::physics::exact::{fma, fms, guest_asin, guest_cos, guest_sin};
use glam::Vec3;

const TWO_PI: f32 = 6.2831855;
const HALF_PI: f32 = 1.5707964;

/// Denormals in and out of the vector unit are zero.
#[inline]
fn ftz(x: f32) -> f32 {
    if x != 0.0 && x.is_subnormal() { 0.0f32.copysign(x) } else { x }
}

fn vmap(a: Vec3, b: Vec3, f: impl Fn(f32, f32) -> f32) -> Vec3 {
    Vec3::new(ftz(f(ftz(a.x), ftz(b.x))), ftz(f(ftz(a.y), ftz(b.y))), ftz(f(ftz(a.z), ftz(b.z))))
}

fn vadd(a: Vec3, b: Vec3) -> Vec3 {
    vmap(a, b, |x, y| x + y)
}

fn vsub(a: Vec3, b: Vec3) -> Vec3 {
    vmap(a, b, |x, y| x - y)
}

/// vmaddfp a * splat(k) + c: the product rounded, then the sum.
fn vmadd(a: Vec3, k: f32, c: Vec3) -> Vec3 {
    vadd(vmap(a, Vec3::splat(k), |x, y| x * y), c)
}

/// vmsum3fp of a with itself.
fn vdot(a: Vec3) -> f32 {
    let p = vmap(a, a, |x, y| x * y);
    ftz(ftz(0.0 + p.z) + ftz(p.y + p.x))
}

/// The normalisation's scale: vrsqrtefp (1 / sqrt), an infinite estimate replaced by the zero vector,
/// one Newton step -(x y y - 1) (0.5 y) + y.
fn rsq(x: f32) -> f32 {
    let x = ftz(x);
    let y0 = ftz(1.0 / ftz(x.sqrt()));
    let y = if y0 == f32::INFINITY { 0.0 } else { y0 };
    let e = -ftz(ftz(x * ftz(y * y)) - 1.0);
    ftz(ftz(e * ftz(0.5 * y)) + y)
}

/// F-57591: rotation about y.
fn rot_y(v: Vec3, a: f32) -> Vec3 {
    let s = guest_sin(a as f64) as f32;
    let c = guest_cos(a as f64) as f32;
    Vec3::new(fma(v.x, c, v.z * s), v.y, fms(v.z, c, v.x * s))
}

/// The turn's sign by the pair of `fsel`: 1, 0 (for +-0) or -1.
fn sgn(a: f32) -> f32 {
    let f = if -a >= 0.0 { 0.0 } else { 1.0 };
    if a >= 0.0 { f } else { -1.0 }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TcpRail {
    pub a: Vec3,
    pub b: Vec3,
    pub a1: f32,
    pub a2: f32,
    pub c1: Vec3,
    pub c2: Vec3,
    pub r: f32,
    pub p: Vec3,
    pub t: Vec3,
    pub h0: Vec3,
    pub h1: Vec3,
    pub flags: u8,
}

/// F-85722 with its solver F-13911: the rail from `p` heading `h0` to `t` heading `h1`, turning radius
/// `r`, the start / end circle on the heading's left (x z: (-h.z, h.x)) when the sense is set, else on
/// its right. An invalid rail (flags 0x20 clear: T inside the start circle, or opposite senses with
/// |c2 - c1| <= 2R) keeps what the solver wrote before it gave up.
pub(crate) fn build(p: Vec3, h0: Vec3, t: Vec3, h1: Vec3, r: f32, s0: bool, s1: bool) -> TcpRail {
    let mut q = TcpRail { p, t, h0, h1, r, flags: (s0 as u8) << 7 | (s1 as u8) << 6, ..Default::default() };
    let ok = solve(&mut q);
    q.flags |= (ok as u8) << 5;
    q
}

fn solve(q: &mut TcpRail) -> bool {
    let (p, t, h0, h1, r) = (q.p, q.t, q.h0, q.h1, q.r);
    let (s0, s1) = (q.flags & 0x80 != 0, q.flags & 0x40 != 0);
    // the circles' sides, made unit; flipped for a clear sense
    let (e0, e1) = (Vec3::new(-h0.z, 0.0, h0.x), Vec3::new(-h1.z, 0.0, h1.x));
    let k1 = rsq(vdot(e1));
    let k0 = rsq(vdot(e0));
    let mut n1 = vmap(e1, Vec3::splat(k1), |x, y| x * y);
    let mut n0 = vmap(e0, Vec3::splat(k0), |x, y| x * y);
    if !s0 {
        n0 = vsub(Vec3::ZERO, n0);
    }
    if !s1 {
        n1 = vsub(Vec3::ZERO, n1);
    }
    q.c1 = vmadd(n0, r, Vec3::new(p.x, 0.0, p.z));
    q.c2 = vmadd(n1, r, Vec3::new(t.x, 0.0, t.z));
    let (c1, c2) = (q.c1, q.c2);
    let (dz, dx) = (c1.z - t.z, c1.x - t.x);
    if fma(dx, dx, fma(dz, dz, c1.y * c1.y)) < r * r {
        return false;
    }
    let d = vsub(c2, c1);
    let (m1, m0) = (vsub(Vec3::ZERO, n1), vsub(Vec3::ZERO, n0));
    let u = vmap(d, Vec3::splat(rsq(vdot(d))), |x, y| x * y);
    // the cross product's y (-(h.z u.x - h.x u.z)) and the heading's side of the line
    let cross = |h: Vec3| -ftz(ftz(ftz(h.z) * ftz(u.x)) - ftz(ftz(h.x) * ftz(u.z))) > 0.0;
    let along = |h: Vec3| fma(u.x, h.x, fma(h.y, u.y, h.z * u.z)) < 0.0;
    let f1 = crate::aiv::ang3(m0, u);
    q.a1 = if along(h0) {
        if s0 { (f1 + HALF_PI) - TWO_PI } else { (TWO_PI - f1) - HALF_PI }
    } else if s0 ^ cross(h0) {
        let f0 = HALF_PI - f1;
        if s0 { f0 - TWO_PI } else { TWO_PI - f0 }
    } else {
        let f0 = f1 - HALF_PI;
        if s0 { -f0 } else { f0 }
    };
    let f1 = crate::aiv::ang3(m1, u);
    q.a2 = if along(h1) {
        let f0 = f1 + HALF_PI;
        if s1 { -f0 } else { f0 }
    } else if s1 ^ cross(h1) {
        let f0 = HALF_PI - f1;
        if s1 { -f0 } else { f0 }
    } else {
        let f13 = f1 - HALF_PI;
        if s1 { f13 - TWO_PI } else { TWO_PI - f13 }
    };
    if s0 != s1 {
        // crossed line: both turns grow by asin(2R / |c2 - c1|), wrapped once each way
        let w = vsub(c2, c1);
        let f0 = r * 2.0;
        let l = fma(w.x, w.x, fma(w.y, w.y, w.z * w.z)).sqrt();
        if !(f0 < l) {
            return false;
        }
        let k = guest_asin((f0 / l) as f64) as f32;
        let wrap = |a: f32| {
            let mut a = if a < 0.0 { a - k } else { a + k };
            if a > TWO_PI {
                a -= TWO_PI;
            }
            if a < -TWO_PI {
                a += TWO_PI;
            }
            a
        };
        q.a1 = wrap(q.a1);
        q.a2 = wrap(q.a2);
    }
    q.a = vmadd(rot_y(m0, q.a1), r, c1);
    q.b = vmadd(rot_y(m1, -q.a2), r, c2);
    q.c2.y = t.y;
    q.c1.y = p.y;
    q.a.y = p.y;
    q.b.y = t.y;
    true
}

impl TcpRail {
    pub(crate) fn valid(&self) -> bool {
        self.flags & 0x20 != 0
    }

    /// |A - B| as the scalar unit has it (dy dy, then z, then x fused).
    fn line(&self) -> f32 {
        let (dy, dz, dx) = (self.a.y - self.b.y, self.a.z - self.b.z, self.a.x - self.b.x);
        fma(dx, dx, fma(dz, dz, dy * dy)).sqrt()
    }

    /// F-62306: |a1| R + |A - B| + |a2| R (fused).
    pub(crate) fn len(&self) -> f32 {
        fma(self.a2.abs(), self.r, fma(self.a1.abs(), self.r, self.line()))
    }

    /// F-70353: the phase `step` metres on.
    pub(crate) fn advance(&self, phase: f32, step: f32) -> f32 {
        step / self.len() + phase
    }

    /// F-39283: the larger of |a1|, |a2|; 0 for an invalid rail.
    pub(crate) fn max_turn(&self) -> f32 {
        if !self.valid() {
            return 0.0;
        }
        let (a1, a2) = (self.a1.abs(), self.a2.abs());
        if a1 - a2 >= 0.0 { a1 } else { a2 }
    }

    /// F-65534: |a1| + |a2|.
    pub(crate) fn total_turn(&self) -> f32 {
        self.a1.abs() + self.a2.abs()
    }

    /// The distance along the rail at `phase` (clamped to 0 - 1 by `fsel`) and the two segment ends
    /// (|a1 R|, |a1 R| + line) and the line's length, as F-56929 and F-28568 compute them (products rounded, not fused).
    fn at(&self, phase: f32) -> (f32, f32, f32, f32) {
        let f0 = if phase >= 0.0 { phase } else { 0.0 };
        let frac = if f0 - 1.0 >= 0.0 { 1.0 } else { f0 };
        let (r1, r2, line) = ((self.a1 * self.r).abs(), (self.a2 * self.r).abs(), self.line());
        (((r2 + line) + r1) * frac, r1, line + r1, line)
    }

    /// F-56929: the point at `phase`.
    pub(crate) fn point(&self, phase: f32) -> Vec3 {
        let (s, e1, e2, line) = self.at(phase);
        if s < e1 {
            let v = rot_y(vsub(self.p, self.c1), sgn(self.a1) * (s / self.r));
            vadd(self.c1, v)
        } else if s < e2 {
            vmadd(vsub(self.b, self.a), (s - e1) / line, self.a)
        } else {
            let v = rot_y(vsub(self.b, self.c2), sgn(self.a2) * ((s - e2) / self.r));
            vadd(self.c2, v)
        }
    }

    /// F-28568: the derivative at `phase`: on an arc the radial vector turned a quarter turn more
    /// (length R), on the line B - A (not unit).
    pub(crate) fn tangent(&self, phase: f32) -> Vec3 {
        let (s, e1, e2, _) = self.at(phase);
        if s < e1 {
            let v = rot_y(vsub(self.p, self.c1), sgn(self.a1) * (s / self.r));
            rot_y(v, sgn(self.a1) * HALF_PI)
        } else if s < e2 {
            vsub(self.b, self.a)
        } else {
            let v = rot_y(vsub(self.b, self.c2), sgn(self.a2) * ((s - e2) / self.r));
            rot_y(v, sgn(self.a2) * HALF_PI)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The builder, both thunks' point and tangent, the advance, the length and the turns against the original's code
    /// run offline (tools/tcp_rail_reference.py -> testdata/tcp_rail_ref.txt): 2,000 rails (inside / on the start
    /// circle, |c2 - c1| about 2R, coincident circles, equal / opposite senses, heights, odd headings), five phases a
    /// valid rail (0, -0, 1 and a step either side, out of range, the segment ends -2 .. +2 float steps) and two
    /// advances. Every word equal (xyz lanes; the w lanes hold stack leftovers in the original).
    #[test]
    fn tcp_rail_matches_the_original() {
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let v = |w: &[&str]| Vec3::new(h(w[0]), h(w[1]), h(w[2]));
        let x = |f: f32| format!("{:08x}", f.to_bits());
        let xv = |v: Vec3| format!("{} {} {}", x(v.x), x(v.y), x(v.z));
        let (mut n, mut valid, mut evals) = (0, 0, 0);
        for line in include_str!("testdata/tcp_rail_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let o: Vec<&str> = o.split_whitespace().collect();
            let q = build(v(&w[0..]), v(&w[3..]), v(&w[6..]), v(&w[9..]), h(w[12]), w[13] == "1", w[14] == "1");
            let got = format!(
                "{} {} {} {} {} {} {} {:02x} {} {} {}",
                q.valid() as u8, xv(q.a), xv(q.b), xv(q.c1), xv(q.c2), x(q.a1), x(q.a2), q.flags, x(q.len()), x(q.max_turn()), x(q.total_turn())
            );
            assert_eq!(got, o[..19].join(" "), "{line}");
            if q.valid() {
                valid += 1;
                let mut k = 19;
                let mut ph = [0.0; 5];
                for p in ph.iter_mut() {
                    *p = h(o[k]);
                    assert_eq!(format!("{} {}", xv(q.point(*p)), xv(q.tangent(*p))), o[k + 1..k + 7].join(" "), "{line}");
                    k += 7;
                    evals += 1;
                }
                assert_eq!(x(q.advance(ph[0], h(w[20]))), o[k], "{line}");
                assert_eq!(x(q.advance(ph[3], h(w[21]))), o[k + 1], "{line}");
                assert_eq!(o.len(), k + 2);
            } else {
                assert_eq!(o.len(), 19);
            }
            n += 1;
        }
        assert_eq!((n, valid, evals), (2000, 1255, 6275));
    }
}
