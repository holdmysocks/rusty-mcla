//! The simulator's soft push between nearby vehicles (F-49821, run after each Update of the
//! physics level, F-13054): every pair of vehicle bodies whose centres are under 3.2 m apart
//! is held apart by their ellipsoids - the point of each ellipsoid on the line between the
//! centres, from the body's own radii (a car 0.85 / 0.6 / 1.65, a motorcycle 0.3 / 0.5 / 0.9
//! across, up and along): when the centres are nearer than the sum of the two points' distances
//! the two bodies are pushed apart, each by half the overlap (F-12566 on each, the pending
//! push applied at the head of the next step). Listing and arithmetic: reference/vehicle/softpush_rule.md.

use super::exact::{dot3, dot_xyz, fma};
use glam::Vec3;

pub const CAR_RADII: Vec3 = Vec3::new(0.85, 0.6, 1.65);
pub const BIKE_RADII: Vec3 = Vec3::new(0.3, 0.5, 0.9);
/// The two centres must be nearer than this (0x8203271C).
pub const REACH: f32 = 3.2;

/// F-83489: the point of the ellipsoid `radii` along `dir` (the direction in the body's own
/// frame, components made positive).
pub fn ellipsoid_point(radii: Vec3, dir: Vec3) -> Vec3 {
    const EPS: f32 = 9.999_999_974_752_427e-7; // 0x82003774
    if dir.x.abs() > EPS {
        let t = 1.0 / dir.x;
        let a = dir.y * t;
        let b = dir.z * t;
        let (q, r) = (a / radii.y, b / radii.z);
        let s = fma(1.0 / radii.x, 1.0 / radii.x, fma(r, r, q * q));
        let k = 1.0 / s.sqrt();
        Vec3::new(k, k * a, k * b)
    } else if dir.y.abs() > EPS {
        let a = dir.z / dir.y;
        let (p, q) = (1.0 / radii.y, a / radii.z);
        let k = 1.0 / fma(q, q, p * p).sqrt();
        Vec3::new(0.0, k, k * a)
    } else {
        Vec3::new(0.0, 0.0, radii.z)
    }
}

/// A vehicle as the pass sees it: the body's place (+256), its rows (+208, +224, +240) and the
/// motorcycle flag (F-54299).
#[derive(Clone, Copy, Debug)]
pub struct SoftBody {
    pub at: Vec3,
    pub rows: [Vec3; 3],
    pub bike: bool,
}

fn radii(b: &SoftBody) -> Vec3 {
    if b.bike { BIKE_RADII } else { CAR_RADII }
}

/// The body's frame's share of a unit direction: the three rows' dot products, positive.
fn local(rows: &[Vec3; 3], n: Vec3) -> Vec3 {
    Vec3::new(dot3(rows[0], n).abs(), dot3(rows[1], n).abs(), dot3(rows[2], n).abs())
}

/// The pushes the pass gives, in the order it gives them: (body index, vector for F-12566).
/// The pairs are taken in list order; for each the later body is pushed by +v and then the
/// earlier by -v. `ok(i, j)` is the layer test of the pair.
pub fn pushes(bodies: &[SoftBody], ok: &dyn Fn(usize, usize) -> bool) -> Vec<(usize, Vec3)> {
    let mut out = Vec::new();
    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            if !ok(i, j) {
                continue;
            }
            let (a, b) = (&bodies[i], &bodies[j]);
            let d = b.at - a.at;
            let dist = dot_xyz(d, d).sqrt();
            let n = d * (1.0 / dist);
            if !(dist < REACH) {
                continue;
            }
            let pa = ellipsoid_point(radii(a), local(&a.rows, n));
            let pb = ellipsoid_point(radii(b), local(&b.rows, n));
            let sum = dot_xyz(pb, pb).sqrt() + dot_xyz(pa, pa).sqrt();
            if !(dist < sum) {
                continue;
            }
            let l2 = dot_xyz(d, d);
            let dir = if l2 > 9.999_999_974_752_427e-7 {
                // (frsqrte and two Newton steps in double, to single)
                d * ((1.0 / (l2 as f64).sqrt()) as f32)
            } else {
                d
            };
            let v = dir * ((sum - dist) * 0.5);
            out.push((j, v));
            out.push((i, Vec3::ZERO - v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 418 cases of the original F-83489 run offline (work/physics-gaps/ellipsoid_ref.py on the
    /// generated code): the port gives the same bits.
    #[test]
    fn ellipsoid_point_is_the_originals() {
        let text = include_str!("softpush_cases.txt");
        let mut n = 0;
        for line in text.lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let h: Vec<f32> = i.split(' ').map(|w| f32::from_bits(u32::from_str_radix(w, 16).unwrap())).collect();
            let want: Vec<u32> = o.split(' ').map(|w| u32::from_str_radix(w, 16).unwrap()).collect();
            let got = ellipsoid_point(Vec3::new(h[0], h[1], h[2]), Vec3::new(h[3], h[4], h[5]));
            assert_eq!([got.x.to_bits(), got.y.to_bits(), got.z.to_bits()], [want[0], want[1], want[2]], "case {line}");
            n += 1;
        }
        assert_eq!(n, 418);
    }

    #[test]
    fn two_cars_overlapping_are_pushed_apart_by_half_the_overlap() {
        let id = [Vec3::X, Vec3::Y, Vec3::Z];
        // side by side, 1.5 m apart: both lateral radii 0.85 -> overlap 0.2, 0.1 each
        let a = SoftBody { at: Vec3::ZERO, rows: id, bike: false };
        let b = SoftBody { at: Vec3::new(1.5, 0.0, 0.0), rows: id, bike: false };
        let p = pushes(&[a, b], &|_, _| true);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].0, 1);
        assert!((p[0].1.x - 0.1).abs() < 1e-6 && p[0].1.y == 0.0);
        assert_eq!(p[1].0, 0);
        assert!((p[1].1.x + 0.1).abs() < 1e-6);
        // 1.8 m apart: no push; 3.3 m apart: out of reach
        let far = SoftBody { at: Vec3::new(1.8, 0.0, 0.0), rows: id, bike: false };
        assert!(pushes(&[a, far], &|_, _| true).is_empty());
    }
}
