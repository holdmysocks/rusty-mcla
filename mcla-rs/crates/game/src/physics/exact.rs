//! Pieces of the original's vehicle update written out operation for operation, in single
//! precision and in the original's order, so that a tick from the same state leaves the same
//! bits (decode notes: work/veh/gjk_exact.md part 4; listings from the static recompilation).
//!
//! Conventions of the code as it runs: the vector unit's multiply-add is NOT fused (the product
//! is rounded, then the sum); the scalar unit's `fmadds` family IS (one rounding to double, then
//! to single); a sum of three products of the vector unit is (0 + z z') + (y y' + x x').

use glam::Vec3;

/// a * b + c, fused (fmadds).
#[inline]
pub fn fma(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// a * b - c, fused (fmsubs).
#[inline]
pub fn fms(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, -(c as f64)) as f32
}

/// c - a * b, fused (fnmsubs).
#[inline]
pub fn fnms(a: f32, b: f32, c: f32) -> f32 {
    (-(a as f64).mul_add(b as f64, -(c as f64))) as f32
}

/// -(a * b + c), fused (fnmadds).
#[inline]
pub fn fnma(a: f32, b: f32, c: f32) -> f32 {
    (-(a as f64).mul_add(b as f64, c as f64)) as f32
}

/// The vector unit's sum of three products.
#[inline]
pub fn dot3(a: Vec3, b: Vec3) -> f32 {
    (0.0 + a.z * b.z) + (a.y * b.y + a.x * b.x)
}

/// The scalar unit's usual chain x x' + (y y' + z z') with the innermost product rounded.
#[inline]
pub fn dot_xyz(a: Vec3, b: Vec3) -> f32 {
    fma(a.x, b.x, fma(a.y, b.y, a.z * b.z))
}

/// The chain x x' + (z z' + y y').
#[inline]
pub fn dot_xzy(a: Vec3, b: Vec3) -> f32 {
    fma(a.x, b.x, fma(a.z, b.z, a.y * b.y))
}

pub use super::gjk::{cross_as_original as cross, recip_as_original as recip};

/// 1 / sqrt as the vector unit's estimate and one Newton step.
#[inline]
pub fn rsqrt(l2: f32) -> f32 {
    let x = 1.0 / l2.sqrt();
    (-(l2 * (x * x) - 1.0)) * (0.5 * x) + x
}

/// +1, 0 or -1 as the original's pairs of `fsel` give it.
#[inline]
pub fn sign(x: f32) -> f32 {
    if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }
}

// ---- the guest C runtime's sine and cosine (F-72580, F-63589), in double: the argument
// less a whole number of half turns in two steps, then an odd polynomial of nine terms. The
// table is at 0x827EAFD0.
const HALF_PI: f64 = f64::from_bits(0x3ff921fb54442d18);
const OVER_PI: f64 = f64::from_bits(0x3fd45f306dc9c883);
const PI_HIGH: f64 = f64::from_bits(0x400921fb54400000);
const PI_LOW: f64 = f64::from_bits(0x3de0b4611a600000);
const POLY: [f64; 8] = [
    f64::from_bits(0xbfc5555555555555),
    f64::from_bits(0x3f811111111110b0),
    f64::from_bits(0xbf2a01a01a013e1a),
    f64::from_bits(0x3ec71de3a524f063),
    f64::from_bits(0xbe5ae6454b5dc0ab),
    f64::from_bits(0x3de6123c686ad430),
    f64::from_bits(0xbd6ae420dc08499c),
    f64::from_bits(0x3ce880ff6993df95),
];

#[inline]
fn poly(r: f64) -> f64 {
    let s = r * r;
    let mut p = POLY[7].mul_add(s, POLY[6]);
    for c in [POLY[5], POLY[4], POLY[3], POLY[2], POLY[1], POLY[0], 1.0] {
        p = p.mul_add(s, c);
    }
    p * r
}

pub fn guest_cos(x: f64) -> f64 {
    let a = x.abs();
    if a == 0.0 {
        return 1.0;
    }
    let shifted = HALF_PI + a;
    let q = (OVER_PI * shifted).round_ties_even();
    let half = q - 0.5;
    let r = -(PI_HIGH.mul_add(half, -a));
    let r = -(PI_LOW.mul_add(half, -r));
    let v = poly(r);
    if shifted - 220_000_000.0 >= 0.0 {
        return f64::NAN;
    }
    if (q as i64) & 1 != 0 { -v } else { v }
}

pub fn guest_sin(x: f64) -> f64 {
    let a = x.abs();
    if a == 0.0 {
        return x;
    }
    let q = (OVER_PI * a).round_ties_even();
    let r = -(PI_HIGH.mul_add(q, -a));
    let r = -(PI_LOW.mul_add(q, -r));
    let v = poly(r);
    let v = if (q as i64) & 1 != 0 { -v } else { v };
    if a - 220_000_000.0 >= 0.0 {
        return f64::NAN;
    }
    v * if x >= 0.0 { 1.0 } else { -1.0 }
}

// ---- the guest runtime's arcsine (F-80613): a rational function of the square, about
// zero below one half and about one above it. Table at 0x827EB068.
const ASIN_P: [f64; 5] = [
    f64::from_bits(0xc03b5e55a83a0a62),
    f64::from_bits(0x404c9aa7360ad48a),
    f64::from_bits(0xc043d82ca9a6da9f),
    f64::from_bits(0x40244e1764ec3927),
    f64::from_bits(0xbfe64bbdb5e61e65),
];
const ASIN_Q: [f64; 5] = [
    f64::from_bits(0xc06486c03e2b87cc),
    f64::from_bits(0x407a124f101eb843),
    f64::from_bits(0xc077ddcefc56a848),
    f64::from_bits(0x4062de7c96591c70),
    f64::from_bits(0xc037d2e86ef9861f),
];

pub fn guest_asin(x: f64) -> f64 {
    let a = x.abs();
    let (z, t, shift) = if a > 0.5 {
        let z = (1.0 - a) * 0.5;
        (z, -(z.sqrt() * 2.0), HALF_PI)
    } else {
        if x == 0.0 {
            return x;
        }
        (a * a, a, 0.0)
    };
    let p = ASIN_P[4].mul_add(z, ASIN_P[3]);
    let q = ASIN_Q[4] + z;
    let p = p.mul_add(z, ASIN_P[2]);
    let q = q.mul_add(z, ASIN_Q[3]);
    let p = p.mul_add(z, ASIN_P[1]);
    let q = q.mul_add(z, ASIN_Q[2]);
    let p = p.mul_add(z, ASIN_P[0]);
    let q = q.mul_add(z, ASIN_Q[1]);
    let p = p * z;
    let q = q.mul_add(z, ASIN_Q[0]);
    let r = (p * t) / q + t + shift;
    if x >= 0.0 { r } else { -r }
}

/// The guest C runtime's atan2 (F-65975, table 0x827EB158), in double: the smaller of |y|, |x|
/// over the larger, folded once more about tan(pi/6), a rational of the square, and the
/// octant's angle added.
pub fn guest_atan2(y: f64, x: f64) -> f64 {
    const PI: f64 = f64::from_bits(0x400921fb54442d18);
    let r = if x == 0.0 {
        if y == 0.0 {
            if !x.is_sign_negative() {
                return y;
            }
            return if y.is_sign_negative() { -PI } else { PI };
        }
        f64::from_bits(0x3ff921fb54442d18)
    } else {
        let (ax, ay) = (x.abs(), y.abs());
        let (mut k, big, mut f) = if ay > ax { (2usize, ay, ax) } else { (0usize, ax, ay) };
        f /= big;
        if f > f64::from_bits(0x3fd126145e9ecd56) {
            let root3 = f64::from_bits(0x3ffbb67ae8584caa);
            let sum = root3 + f;
            f = root3.mul_add(f, -1.0) / sum;
            k += 1;
        }
        let z = f * f;
        let p = f64::from_bits(0xbfeacd7ad9b187bd).mul_add(z, f64::from_bits(0xc020fd3f5c8d6a63));
        let q = f64::from_bits(0x402e0c49e14ac710) + z;
        let p = p.mul_add(z, f64::from_bits(0xc034817fb9e2bccb));
        let q = q.mul_add(z, f64::from_bits(0x404dca0a320da3d7));
        let p = p.mul_add(z, f64::from_bits(0xc02b60a651061ce2));
        let q = q.mul_add(z, f64::from_bits(0x40558a12040b6da5));
        let p = p * z;
        let q = q.mul_add(z, f64::from_bits(0x4044887cbcc495a9));
        let p = p * f;
        let mut f = p / q + f;
        if k > 1 {
            f = -f;
        }
        [0.0, f64::from_bits(0x3fe0c152382d7366), f64::from_bits(0x3ff921fb54442d18), f64::from_bits(0x3ff0c152382d7366)][k] + f
    };
    let r = if x >= 0.0 { r } else { PI - r };
    if y.is_sign_negative() { -r } else { r }
}

/// F-85884 / F-12566: `v` added to the body's pending turn (body +416) or push (+400):
/// no longer than 100, and less its share along what is already pending.
pub fn pending_add(pending: Vec3, v: Vec3) -> Vec3 {
    let l2 = dot3(v, v);
    let v = if l2 > 100.0f32 * 100.0 { (v * rsqrt(l2)) * 100.0 } else { v };
    let have = dot3(pending, pending);
    let r = 1.0 / have;
    let inv = r * (1.0 - r * have) + r;
    let along = dot3(pending, v);
    let v = if along > 0.0 && inv > 0.0 { v - pending * (along * inv) } else { v };
    pending + v
}

/// F-17888: nothing at or below 0, one at or above 1, between them the sine of that share
/// of a quarter turn.
pub fn ease(x: f32) -> f32 {
    if !(x > 0.0) {
        0.0
    } else if !(x < 1.0) {
        1.0
    } else {
        guest_sin((x * f32::from_bits(0x3fc90fdb)) as f64) as f32
    }
}

/// F-89809: the steady acceleration that covers `to - from` in `time` from the speed `v`
/// (2 (d - v t) / t^2 with t = time / 2) - or, where the car would be past it sooner, the one that
/// ends at the speed `v_end` there.
pub fn stop_rate(time: f32, from: f32, v: f32, to: f32, v_end: f32) -> f32 {
    if !(time > f32::from_bits(0x3586_37bd)) {
        return 0.0;
    }
    let sum = v + v_end;
    let mut plain = true;
    if sum != 0.0 {
        let t = ((to - from) / sum) * 2.0;
        if t > 0.0 && t < time {
            plain = false;
        }
    }
    if plain {
        let h = time * 0.5;
        let d = to - from;
        (fnms(h, v, d) * 2.0) / (h * h)
    } else {
        let vv = v * v;
        let d = to - from;
        (fms(v_end, v_end, vv) * 0.5) / d
    }
}

/// F-90102: `value` moved towards `target` by rate times step, not past it.
pub fn approach(value: &mut f32, target: f32, rate: f32, step: f32) {
    if *value < target {
        *value = fma(rate, step, *value);
        if *value > target {
            *value = target;
        }
    } else if *value > target {
        *value = fnms(rate, step, *value);
        if *value < target {
            *value = target;
        }
    }
}

/// A velocity made a unit vector as the gyro does it (0 where it has no length).
pub fn unit_or_zero(v: Vec3) -> Vec3 {
    let l2 = dot3(v, v);
    let mut e = 1.0 / l2.sqrt();
    if e == f32::INFINITY {
        e = 0.0;
    }
    v * ((-(l2 * (e * e) - 1.0)) * (0.5 * e) + e)
}

/// The body's inverse inertia in the world (F-16702 through body vfunc 17): rows
/// (r0[i] (r0 d.x) + r1[i] (r1 d.y)) + r2[i] (r2 d.z).
pub fn world_inverse_inertia(rows: &[Vec3; 3], inverse: Vec3) -> [Vec3; 3] {
    let (a, b, c) = (rows[0] * inverse.x, rows[1] * inverse.y, rows[2] * inverse.z);
    let row = |i: usize| (a * rows[0][i] + b * rows[1][i]) + c * rows[2][i];
    [row(0), row(1), row(2)]
}

/// The vector unit's sum of four products, (w w' + z z') + (y y' + x x'), where the fourth
/// factor of the first vector is 1.
#[inline]
fn dot4_one(a: Vec3, b: Vec3, bw: f32) -> f32 {
    (1.0 * bw + a.z * b.z) + (a.y * b.y + a.x * b.x)
}

/// vehGyro's turn (F-81152): the torque is taken into the car's frame and back as if it
/// were a point - through the chassis's place, which costs it the last digits a coordinate
/// of some hundred metres has - and applied; and with it the force that leaves the point at
/// `offset` from the body's place where it is: -mass (inverse inertia . torque) x offset.
/// `chassis`: rows and place (chassis +304..+352); `rows`, `inverse`: the body's rows and
/// its inverse moments. Returns (torque, force).
pub fn gyro_turn(torque: Vec3, offset: Vec3, chassis: &[Vec3; 4], rows: &[Vec3; 3], inverse: Vec3, mass: f32) -> (Vec3, Vec3) {
    let [r0, r1, r2, at] = *chassis;
    let back = Vec3::ZERO - Vec3::new(dot3(r0, at), dot3(r1, at), dot3(r2, at));
    let local = Vec3::new(dot4_one(torque, r0, back.x), dot4_one(torque, r1, back.y), dot4_one(torque, r2, back.z));
    let cols = [Vec3::new(r0.x, r1.x, r2.x), Vec3::new(r0.y, r1.y, r2.y), Vec3::new(r0.z, r1.z, r2.z)];
    let applied = Vec3::new(dot4_one(local, cols[0], at.x), dot4_one(local, cols[1], at.y), dot4_one(local, cols[2], at.z));
    let m = world_inverse_inertia(rows, inverse);
    let mc = [Vec3::new(m[0].x, m[1].x, m[2].x), Vec3::new(m[0].y, m[1].y, m[2].y), Vec3::new(m[0].z, m[1].z, m[2].z)];
    let spin = Vec3::new(dot3(applied, mc[0]), dot3(applied, mc[1]), dot3(applied, mc[2]));
    // spin x offset, as two rounded products and their difference
    let zxy = |v: Vec3| Vec3::new(v.z, v.x, v.y);
    let yzx = |v: Vec3| Vec3::new(v.y, v.z, v.x);
    let first = zxy(offset) * yzx(spin);
    let c = -(yzx(offset) * zxy(spin) - first);
    (applied, c * -mass)
}

// ---- the body (class at 0x82081554): what the wheels' damping hands it, its integrator, its
// move. Rows are the body's matrix rows (body +208..+240).

/// The three blocks the damping hand-offs of a tick add up to (body +464, +528, +592).
#[derive(Clone, Copy, Debug, Default)]
pub struct Damping {
    pub k: [Vec3; 3],
    pub b: [Vec3; 3],
    pub a: [Vec3; 3],
    pub any: bool,
}

impl Damping {
    /// One hand-off (F-70848 as vehWheel::Update calls it, A-79526): a matrix
    /// strength x n n^T at the offset `r` from the body's place. `n_s` = (n x value) x factor.
    pub fn add(&mut self, n: Vec3, n_s: Vec3, r: Vec3) {
        let m = [
            Vec3::new(n.x * n_s.x, n.y * n_s.x, n.z * n_s.x),
            Vec3::new(n.y * n_s.x, n.y * n_s.y, n.z * n_s.y),
            Vec3::new(n.z * n_s.x, n.z * n_s.y, n.z * n_s.z),
        ];
        self.any = true;
        for i in 0..3 {
            self.k[i] = self.k[i] + m[i];
        }
        let mut x = [Vec3::ZERO; 3];
        for (i, e) in IDENTITY.into_iter().enumerate() {
            let c = cross(e, r);
            x[i] = (m[0] * c.x + m[1] * c.y) + m[2] * c.z;
        }
        for i in 0..3 {
            self.b[i] = self.b[i] + x[i];
        }
        for i in 0..3 {
            self.a[i] = self.a[i] + cross(r, x[i]);
        }
    }
}

#[inline]
fn columns(m: &[Vec3; 3]) -> [Vec3; 3] {
    [Vec3::new(m[0].x, m[1].x, m[2].x), Vec3::new(m[0].y, m[1].y, m[2].y), Vec3::new(m[0].z, m[1].z, m[2].z)]
}

/// The inverse of a 3x3 given by rows (F-76283): cross products over the determinant.
pub fn inverse3(m: &[Vec3; 3]) -> [Vec3; 3] {
    let (ab, bc, ca) = (cross(m[0], m[1]), cross(m[1], m[2]), cross(m[2], m[0]));
    let r = recip(dot3(ab, m[2]));
    [Vec3::new(bc.x, ca.x, ab.x) * r, Vec3::new(bc.y, ca.y, ab.y) * r, Vec3::new(bc.z, ca.z, ab.z) * r]
}

/// The body's inertia in the world (F-24854): rows ((r0[i] Ix) r0 + (r1[i] Iy) r1) + (r2[i] Iz) r2.
pub fn world_inertia(rows: &[Vec3; 3], inertia: Vec3) -> [Vec3; 3] {
    let cols = columns(rows);
    let row = |i: usize| {
        let c = cols[i] * inertia;
        (rows[0] * c.x + rows[1] * c.y) + rows[2] * c.z
    };
    [row(0), row(1), row(2)]
}

/// The turning rate that goes with an angular momentum (body vfunc 46, F-80680).
pub fn spin_of(l: Vec3, rows: &[Vec3; 3], inverse: Vec3) -> Vec3 {
    let local = Vec3::new(dot3(rows[0], l), dot3(rows[1], l), dot3(rows[2], l)) * inverse;
    let cols = columns(rows);
    Vec3::new(dot3(cols[0], local), dot3(cols[1], local), dot3(cols[2], local))
}

/// The solver of the integrator's 3x3 system where it has full rank (F-12385): the
/// cofactors' rows times the right side, over the determinant. `None` where the original
/// takes one of its other ways (a matrix all but singular).
pub fn solve3(m: &[Vec3; 3], rhs: Vec3) -> Option<Vec3> {
    let (m00, m01, m02) = (m[0].x, m[0].y, m[0].z);
    let (m10, m11, m12) = (m[1].x, m[1].y, m[1].z);
    let (m20, m21, m22) = (m[2].x, m[2].y, m[2].z);
    let mut most = 0.0f32;
    for v in [m00, m01, m02, m10, m11, m12, m20, m21, m22] {
        if most < v.abs() {
            most = v.abs();
        }
    }
    if most < f32::from_bits(0x322bcc77) {
        return None;
    }
    let small = most * f32::from_bits(0x3a1d4952);
    let c0 = Vec3::new(fms(m11, m22, m12 * m21), fnms(m10, m22, m20 * m12), fms(m10, m21, m20 * m11));
    let c1 = Vec3::new(fnms(m01, m22, m02 * m21), fms(m00, m22, m02 * m20), fnms(m00, m21, m01 * m20));
    let c2 = Vec3::new(fms(m01, m12, m02 * m11), fnms(m00, m12, m02 * m10), fms(m00, m11, m01 * m10));
    let most_of = |c: Vec3| c.x.abs().max(c.y.abs()).max(c.z.abs());
    let rows_most = [most_of(c0), most_of(c1), most_of(c2)];
    let all_most = rows_most[0].max(rows_most[1]).max(rows_most[2]);
    let det = fma(c0.y, m01, fma(m00, c0.x, c0.z * m02));
    let (cube, other) = ((small * small) * small, all_most * small);
    let floor = if cube - other >= 0.0 { cube } else { other };
    if !(det.abs() > floor) || rows_most.iter().any(|r| !(*r > small * small)) {
        return None;
    }
    Some(Vec3::new(dot3(c0, rhs), dot3(c1, rhs), dot3(c2, rhs)) * recip(det))
}

/// What the integrator leaves (F-94789 with F-20578 or F-21091): the new momenta.
/// `p`, `l` the momenta before; `force`, `torque` the tick's sums; `rows`, `inertia` the
/// body's matrix and moments as they stand when it runs (after the move).
#[allow(clippy::too_many_arguments)]
pub fn integrate(p: Vec3, l: Vec3, force: Vec3, torque: Vec3, damping: &Damping, rows: &[Vec3; 3], inertia: Vec3, mass: f32, dt: f32) -> (Vec3, Vec3) {
    integrate_with(p, l, force, torque, Vec3::ZERO, Vec3::ZERO, damping, rows, inertia, mass, dt)
}

/// The same with what the contact solver has already put into the body's impulse sums
/// (+368, +384) before the forces' share is added to them.
#[allow(clippy::too_many_arguments)]
pub fn integrate_with(p: Vec3, l: Vec3, force: Vec3, torque: Vec3, impulse: Vec3, turning: Vec3, damping: &Damping, rows: &[Vec3; 3], inertia: Vec3, mass: f32, dt: f32) -> (Vec3, Vec3) {
    let over_mass = 1.0 / mass;
    let jf = force * dt + impulse;
    let jt = torque * dt + turning;
    if !damping.any {
        let p = if jf == Vec3::ZERO { p } else { p + jf };
        let l = if jt == Vec3::ZERO { l } else { l + jt };
        return (p, l);
    }
    let share = dt * over_mass;
    let m = [damping.k[0] * share + Vec3::X, damping.k[1] * share + Vec3::Y, damping.k[2] * share + Vec3::Z];
    let n = inverse3(&m);
    let b = damping.b;
    let x: [Vec3; 3] = std::array::from_fn(|i| (n[0] * b[i].x + n[1] * b[i].y) + n[2] * b[i].z);
    let back = Vec3::ZERO - Vec3::splat(dt * share);
    let iw = world_inertia(rows, inertia);
    let s: [Vec3; 3] = std::array::from_fn(|i| {
        let g = Vec3::new(dot3(b[0], x[i]), dot3(b[1], x[i]), dot3(b[2], x[i]));
        (damping.a[i] * dt + back * g) + iw[i]
    });
    let nc = columns(&n);
    let t = jf * (Vec3::ZERO - Vec3::splat(share));
    let u = Vec3::new(dot3(nc[0], t), dot3(nc[1], t), dot3(nc[2], t));
    let rhs = Vec3::new(dot3(b[0], u), dot3(b[1], u), dot3(b[2], u)) + jt;
    let turn = solve3(&s, rhs).unwrap_or_else(|| {
        // (not the original's way for a matrix without full rank: a plain solve)
        let m = glam::DMat3::from_cols(s[0].as_dvec3(), s[1].as_dvec3(), s[2].as_dvec3()).transpose();
        if m.determinant().abs() > 1e-30 { (m.inverse() * rhs.as_dvec3()).as_vec3() } else { Vec3::ZERO }
    });
    let ic = columns(&iw);
    let l = l + Vec3::new(dot3(ic[0], turn), dot3(ic[1], turn), dot3(ic[2], turn));
    let bc = columns(&b);
    let step = turn * dt;
    let z = Vec3::new(dot3(bc[0], step), dot3(bc[1], step), dot3(bc[2], step));
    let q = (z + jf) * over_mass;
    let dv = Vec3::new(dot3(nc[0], q), dot3(nc[1], q), dot3(nc[2], q));
    (dv * mass + p, l)
}

/// The body's rows turned by `turn` = turning rate times step (F-54600): nothing below a
/// millionth of a radian.
pub fn rows_turned(rows: &[Vec3; 3], turn: Vec3) -> [Vec3; 3] {
    let l2 = dot3(turn, turn);
    if !(l2 > f32::from_bits(0x2b8cbccc)) {
        return *rows;
    }
    let mut e = 1.0 / l2.sqrt();
    if e == f32::INFINITY {
        e = 0.0;
    }
    let angle = l2 * ((-(l2 * (e * e) - 1.0)) * (0.5 * e) + e);
    turned(rows, &rotation_about_unit(turn * recip(angle), angle))
}

/// The body's rows made square again (F-84832, every 33rd move): the second made a unit
/// vector, the third the first across it, the first those two's.
pub fn rows_squared(rows: &[Vec3; 3]) -> [Vec3; 3] {
    if rows.iter().any(|r| *r == Vec3::ZERO) {
        return IDENTITY;
    }
    let unit = |v: Vec3| {
        let l2 = dot3(v, v);
        let mut e = 1.0 / l2.sqrt();
        if e == f32::INFINITY {
            e = 0.0;
        }
        v * ((-(l2 * (e * e) - 1.0)) * (0.5 * e) + e)
    };
    let up = unit(rows[1]);
    let back = unit(cross(rows[0], up));
    [cross(up, back), up, back]
}

/// The model's origin that goes with the body's place (F-18040): the place less the
/// bound's offset carried through the rows.
pub fn origin_of(at: Vec3, rows: &[Vec3; 3], offset: Vec3) -> Vec3 {
    let cols = columns(rows);
    at + (Vec3::ZERO - Vec3::new(dot3(offset, cols[0]), dot3(offset, cols[1]), dot3(offset, cols[2])))
}

pub const IDENTITY: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];

/// The rotation by `angle` about the unit vector `n` (F-65964), as three rows.
pub fn rotation_about_unit(n: Vec3, angle: f32) -> [Vec3; 3] {
    let c = guest_cos(angle as f64) as f32;
    let s = guest_sin(angle as f64) as f32;
    let t = 1.0 - c;
    let (x, y, z) = (n.x, n.y, n.z);
    [
        Vec3::new(fma(x * x, t, c), fma(x * t, y, s * z), fms(x * t, z, s * y)),
        Vec3::new(fms(x * t, y, s * z), fma(y * y, t, c), fma(t * z, y, x * s)),
        Vec3::new(fma(x * t, z, s * y), fms(t * z, y, x * s), fma(z * z, t, c)),
    ]
}

/// The rotation by `angle` about `axis` (F-10410): nothing for no angle, the plain
/// forms about a coordinate axis, else about the axis made a unit vector.
pub fn rotation_about(axis: Vec3, angle: f32) -> [Vec3; 3] {
    if angle == 0.0 {
        return IDENTITY;
    }
    if axis.x == 0.0 {
        if axis.y == 0.0 {
            // about z (F-16906)
            let (c, s) = c_s(if axis.z > 0.0 { angle } else { -angle });
            return [Vec3::new(c, s, 0.0), Vec3::new(-s, c, 0.0), Vec3::Z];
        }
        if axis.z == 0.0 {
            // about y (F-92665)
            let (c, s) = c_s(if axis.y > 0.0 { angle } else { -angle });
            return [Vec3::new(c, 0.0, -s), Vec3::Y, Vec3::new(s, 0.0, c)];
        }
    } else if axis.y == 0.0 && axis.z == 0.0 {
        // about x (F-67091)
        let (c, s) = c_s(if axis.x > 0.0 { angle } else { -angle });
        return [Vec3::X, Vec3::new(0.0, c, s), Vec3::new(0.0, -s, c)];
    }
    rotation_about_unit(axis * rsqrt(dot3(axis, axis)), angle)
}

#[inline]
fn c_s(a: f32) -> (f32, f32) {
    (guest_cos(a as f64) as f32, guest_sin(a as f64) as f32)
}

/// `m` turned by `r` (F-41945's product): each row (r0 x + r1 y) + r2 z.
pub fn turned(m: &[Vec3; 3], r: &[Vec3; 3]) -> [Vec3; 3] {
    let row = |v: Vec3| (r[0] * v.x + r[1] * v.y) + r[2] * v.z;
    [row(m[0]), row(m[1]), row(m[2])]
}

/// A row carried through the matrix `c`: c2 z + (c1 y + c0 x).
#[inline]
pub fn row_through(v: Vec3, c: &[Vec3; 4]) -> Vec3 {
    c[2] * v.z + (c[1] * v.y + c[0] * v.x)
}

/// A point carried through the matrix `c`: c2 z + (c1 y + (c0 x + place)).
#[inline]
pub fn point_through(p: Vec3, c: &[Vec3; 4]) -> Vec3 {
    c[2] * p.z + (c[1] * p.y + (c[0] * p.x + c[3]))
}

/// The wheel's steering angle (F-57353, wheel +492): `steer` the chassis's value as handed
/// to this wheel (negated for the rear ones), `side` the wheel's place across the car.
pub fn steer_angle(steer: f32, side: f32, limit: f32, offset: f32) -> f32 {
    let a = -(limit * steer);
    fnms(sign(side) * offset, a, 1.0) * a
}

/// How far a steered wheel's pivot is inboard of it (F-73903, wheel +416).
pub fn pivot_arm(side: f32, pivot_offset: f32, width: f32) -> f32 {
    if pivot_offset > 0.0 {
        side * pivot_offset
    } else if side.abs() > width {
        (sign(side) * width) * 0.25
    } else {
        0.0
    }
}

/// The wheel's matrix as the update rebuilds it (F-70434): its steering about its own up
/// through a pivot `arm` inboard, the hub at rest less `offset` (wheel +432), carried through
/// the chassis's matrix `c`. Rows axle, up, way; place. `steered`: the tuning has a
/// SteeringLimit at all. No compression in the place, no lean.
pub fn wheel_rebuilt(c: &[Vec3; 4], rest: Vec3, arm: f32, offset: f32, steered: bool, angle: f32) -> [Vec3; 4] {
    wheel_rebuilt_about(c, rest, arm, offset, steered, angle, Vec3::Y)
}

/// F-49981 (-> F-81327, F-65964): the frame a steered wheel turns in - the rotation
/// that takes straight up to the wheel's steering axis (wheel +400). Nothing where the two are
/// within 3.6 degrees (their product at least 0.998001), which is every car.
pub fn steer_frame(axis: Vec3) -> [Vec3; 3] {
    let (a, b) = (Vec3::Y, axis);
    let d = fma(a.x, b.x, fma(a.z, b.z, b.y * a.y));
    if d >= f32::from_bits(0x3f7f7cff) {
        return IDENTITY;
    }
    if d > -f32::from_bits(0x3f7f7cff) {
        let c = Vec3::new(b.z * a.y - b.y * a.z, b.x * a.z - b.z * a.x, b.y * a.x - b.x * a.y);
        let len = fma(c.z, c.z, fma(c.x, c.x, c.y * c.y)).sqrt();
        if len == 0.0 {
            return rotation_about_unit(Vec3::X, 0.0);
        }
        let r = 1.0 / len;
        let r = r * (1.0 - r * len) + r;
        let n = c * r;
        let turn = if len <= -1.0 { -std::f32::consts::FRAC_PI_2 } else if len >= 1.0 { std::f32::consts::FRAC_PI_2 } else { guest_asin(len as f64) as f32 };
        let angle = if d > 0.0 { turn } else { f32::from_bits(0x40490fdb) - turn };
        rotation_about_unit(n, angle)
    } else {
        rotation_about_unit(Vec3::X, f32::from_bits(0x40490fdb))
    }
}

/// `wheel_rebuilt` for a wheel whose steering axis is `axis` in the car's frame (F-70434:
/// the frame of F-49981, turned about its own second row by the steering angle).
pub fn wheel_rebuilt_about(c: &[Vec3; 4], rest: Vec3, arm: f32, offset: f32, steered: bool, angle: f32, axis: Vec3) -> [Vec3; 4] {
    let (l, mut p) = if steered {
        let frame = steer_frame(axis);
        let l = turned(&frame, &rotation_about(frame[1], angle));
        let p = Vec3::new(rest.x - arm, rest.y, rest.z);
        (l, l[0] * arm + p)
    } else {
        (IDENTITY, rest)
    };
    p.y -= offset;
    [row_through(l[0], c), row_through(l[1], c), row_through(l[2], c), point_through(p, c)]
}

/// The wheel's rows leaned by `angle` about `axis` (F-33566 -> F-41945).
pub fn leaned(rows: &[Vec3; 3], axis: Vec3, angle: f32) -> [Vec3; 3] {
    turned(rows, &rotation_about(axis, angle))
}

/// How far a wheel is pressed in as the axle reads it (F-48387): within the travel, less
/// the offset.
pub fn axle_disp(comp: f32, travel_extent: f32, travel_limit: f32, offset: f32) -> f32 {
    let low = -travel_extent;
    let a = if comp - low >= 0.0 { comp } else { low };
    let b = if a - travel_limit >= 0.0 { travel_limit } else { a };
    b - offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guest_runtime_sine_and_cosine_are_sound() {
        for i in -2000..2000 {
            let x = i as f64 * 0.0037;
            assert!((guest_sin(x) - x.sin()).abs() < 1e-13, "{x}");
            assert!((guest_cos(x) - x.cos()).abs() < 1e-13, "{x}");
        }
        assert_eq!(guest_cos(0.0), 1.0);
        assert_eq!(guest_sin(-0.0).to_bits(), (-0.0f64).to_bits());
    }

    /// 1,107 cases of the original asin (F-80613) run offline on its generated code
    /// (work/physics-gaps/asin_ref.py): the port gives the same bits, on both sides of 0.5.
    #[test]
    fn guest_asin_is_the_originals() {
        let text = include_str!("guest_asin_cases.txt");
        let mut bad = Vec::new();
        let mut n = 0;
        for line in text.lines() {
            let (i, o) = line.split_once(' ').unwrap();
            let x = f64::from_bits(u64::from_str_radix(i, 16).unwrap());
            let want = u64::from_str_radix(o, 16).unwrap();
            if guest_asin(x).to_bits() != want {
                bad.push((x, f64::from_bits(want), guest_asin(x)));
            }
            n += 1;
        }
        assert!(bad.is_empty(), "{} of {} differ, first {:?}", bad.len(), n, &bad[..bad.len().min(4)]);
    }

    /// 1,804 cases each of the original cosine (F-63589) and sine (F-72580) run offline on their
    /// generated code (work/physics-gaps/trig_ref.py).
    #[test]
    fn guest_sin_cos_are_the_originals() {
        for (name, text, f) in [("cos", include_str!("guest_cos_cases.txt"), guest_cos as fn(f64) -> f64), ("sin", include_str!("guest_sin_cases.txt"), guest_sin as fn(f64) -> f64)] {
            let mut bad = Vec::new();
            for line in text.lines() {
                let (i, o) = line.split_once(' ').unwrap();
                let x = f64::from_bits(u64::from_str_radix(i, 16).unwrap());
                let want = u64::from_str_radix(o, 16).unwrap();
                if f(x).to_bits() != want {
                    bad.push((x, f64::from_bits(want), f(x)));
                }
            }
            assert!(bad.is_empty(), "{name}: {} differ, first {:?}", bad.len(), &bad[..bad.len().min(4)]);
        }
    }
}
