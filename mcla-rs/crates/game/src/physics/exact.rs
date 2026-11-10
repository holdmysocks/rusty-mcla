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

// The guest C runtime's sine, cosine and atan2 live in mcla_formats::guest_math (the navsys uses them too).
pub use mcla_formats::guest_math::{guest_atan2, guest_cos, guest_sin};
const HALF_PI: f64 = f64::from_bits(0x3ff921fb54442d18);

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

/// The guest C runtime's arccosine (F-11618, the worker of F-68873 with its mode set to 1): the same rational function
/// of the square as the arcsine (F-80613, table 0x827EB068), the result taken from 0, pi/2 or pi by the
/// side of 0.5 and the sign of the argument.
pub fn guest_acos(x: f64) -> f64 {
    const PI: f64 = f64::from_bits(0x400921fb54442d18);
    let a = x.abs();
    let (z, t, big) = if a > 0.5 {
        let z = (1.0 - a) * 0.5;
        (z, -(z.sqrt() * 2.0), true)
    } else {
        (a * a, a, false)
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
    let r = (p * t) / q + t;
    let (lo, hi) = if big { (0.0, PI) } else { (HALF_PI, HALF_PI) };
    if x >= 0.0 { lo - r } else { hi + r }
}


/// F-85884 / F-12566: `v` added to the body's pending turn (body +416) or push (+400):
/// no longer than 100, and less its share along what is already pending. The two are one
/// routine on two offsets (their code differs only in the `addi` of the offset), the push's
/// being `solver::direct`.
pub fn pending_add(pending: Vec3, v: Vec3) -> Vec3 {
    super::solver::direct(pending, v)
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
    gyro_turn_with(torque, offset, chassis, &world_inverse_inertia(rows, inverse), mass)
}

/// F-81152 with the body's world inverse inertia `m` (its vtable slot 17's three rows) given.
pub fn gyro_turn_with(torque: Vec3, offset: Vec3, chassis: &[Vec3; 4], m: &[Vec3; 3], mass: f32) -> (Vec3, Vec3) {
    let [r0, r1, r2, at] = *chassis;
    let back = Vec3::ZERO - Vec3::new(dot3(r0, at), dot3(r1, at), dot3(r2, at));
    let local = Vec3::new(dot4_one(torque, r0, back.x), dot4_one(torque, r1, back.y), dot4_one(torque, r2, back.z));
    let cols = [Vec3::new(r0.x, r1.x, r2.x), Vec3::new(r0.y, r1.y, r2.y), Vec3::new(r0.z, r1.z, r2.z)];
    let applied = Vec3::new(dot4_one(local, cols[0], at.x), dot4_one(local, cols[1], at.y), dot4_one(local, cols[2], at.z));
    let mc = [Vec3::new(m[0].x, m[1].x, m[2].x), Vec3::new(m[0].y, m[1].y, m[2].y), Vec3::new(m[0].z, m[1].z, m[2].z)];
    let spin = Vec3::new(dot3(applied, mc[0]), dot3(applied, mc[1]), dot3(applied, mc[2]));
    // spin x offset, as two rounded products and their difference
    let zxy = |v: Vec3| Vec3::new(v.z, v.x, v.y);
    let yzx = |v: Vec3| Vec3::new(v.y, v.z, v.x);
    let first = zxy(offset) * yzx(spin);
    let c = -(yzx(offset) * zxy(spin) - first);
    (applied, c * -mass)
}

/// F-81152 with a ground normal `n`: of the force its part that points along `n` is taken off where there is one
/// (the dot product, fused, over zero): -(n lift - force).
pub fn off_ground(force: Vec3, n: Vec3) -> Vec3 {
    let lift = fma(force.x, n.x, fma(n.y, force.y, n.z * force.z));
    if lift > 0.0 {
        -(n * lift - force)
    } else {
        force
    }
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

/// The share of the way from the medium to the maximum damage a car has taken, to the power 0.3 (F-10364: the
/// damage points' place between the tune's MedDamage and MaxDamage by two `fsel`s - a NaN gives 0 - then the C
/// library's pow, which single-precision `powf` matches bit for bit on every recorded case).
pub fn damage_share(points: f32, med: f32, max: f32) -> f32 {
    let x = (points - med) / (max - med);
    let x = if x >= 0.0 { x } else { 0.0 };
    let x = if x - 1.0 >= 0.0 { 1.0 } else { x };
    x.powf(0.3)
}

/// The inverse of a frame (F-30106; the 4x4 wrapper F-43153): three rows `m` and a place `t` (a row-vector
/// frame, the place in the fourth row), by cofactors in the scalar unit's fused operations. `None` where the
/// original answers 0 (and leaves its output untouched): |det| not above 3.6e-7 times the largest |element|
/// of the 3x3. The result rows 0-2 are the 3x3 inverse, row 3 carries the place back.
pub fn inverse_frame(m: &[Vec3; 3], t: Vec3) -> Option<[Vec3; 4]> {
    let (a, b, c) = (m[0], m[1], m[2]);
    let cx = fms(b.y, c.z, b.z * c.y);
    let cy = fms(b.x, c.z, c.x * b.z);
    let cz = fms(b.x, c.y, c.x * b.y);
    let det = fma(a.z, cz, fms(a.x, cx, a.y * cy));
    let big = |v: Vec3| {
        let (p, q, r) = (v.x.abs(), v.y.abs(), v.z.abs());
        let pq = if p - q >= 0.0 { p } else { q };
        if pq - r >= 0.0 { pq } else { r }
    };
    let (r0, r1, r2) = (big(a), big(b), big(c));
    let r01 = if r0 - r1 >= 0.0 { r0 } else { r1 };
    let all = if r01 - r2 >= 0.0 { r01 } else { r2 };
    if !(det.abs() > all * f32::from_bits(0x34c1_4606)) {
        return None;
    }
    let inv = 1.0 / det;
    let (o0, o16, o32) = (inv * cx, -(inv * cy), inv * cz);
    let o48 = fnma(o0, t.x, fma(t.y, o16, o32 * t.z));
    let o4 = -(fms(a.y, c.z, c.y * a.z) * inv);
    let o20 = inv * fms(c.z, a.x, a.z * c.x);
    let o36 = -(inv * fms(c.y, a.x, a.y * c.x));
    let o52 = fnma(o36, t.z, fma(t.y, o20, t.x * o4));
    let o8 = fms(b.z, a.y, b.y * a.z) * inv;
    let o24 = -(inv * fms(b.z, a.x, b.x * a.z));
    let o40 = inv * fms(b.y, a.x, b.x * a.y);
    let o56 = fnma(o40, t.z, fma(t.y, o24, t.x * o8));
    Some([Vec3::new(o0, o4, o8), Vec3::new(o16, o20, o24), Vec3::new(o32, o36, o40), Vec3::new(o48, o52, o56)])
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
        // the cross as F-81327 forms it, -(b.yzx a.zxy - b.zxy a.yzx) (vnmsubfp): the sign of a zero lane is its
        let c = Vec3::new(-(b.y * a.z - b.z * a.y), -(b.z * a.x - b.x * a.z), -(b.x * a.y - b.y * a.x));
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
        // (anti-parallel) half a turn about unit(e x a), e the first unit axis with |a_e| < 1 / sqrt 3 (0x3f13cd3a):
        // for a = up that is x, and the cross (formed as above) is (-0, -0, -1)
        let e = if a.x.abs() < f32::from_bits(0x3f13cd3a) { Vec3::X } else if a.y.abs() < f32::from_bits(0x3f13cd3a) { Vec3::Y } else { Vec3::Z };
        let c = Vec3::new(-(e.y * a.z - e.z * a.y), -(e.z * a.x - e.x * a.z), -(e.x * a.y - e.y * a.x));
        rotation_about_unit(c * rsqrt((c.x * c.x + c.y * c.y) + c.z * c.z), f32::from_bits(0x40490fdb))
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
    wheel_comp(comp, travel_extent, travel_limit) - offset
}

/// F-92300: the compression +496 held in [-TravelExtent, TravelLimit] by two fsel (a NaN gives -TravelExtent, and
/// limits the wrong way round give TravelLimit, where `f32::clamp` would panic).
pub fn wheel_comp(comp: f32, travel_extent: f32, travel_limit: f32) -> f32 {
    let low = -travel_extent;
    let a = if comp - low >= 0.0 { comp } else { low };
    if a - travel_limit >= 0.0 { travel_limit } else { a }
}

#[cfg(test)]
mod tests {
    /// F-89809 run offline (tools/stop_rate_reference.py): 6,000 cases with zeros, equal ends, opposite speeds, NaN
    /// and infinities - every word equal (a NaN as a NaN: the harness's x86 default NaN is not the console's).
    /// F-92300 and F-48387 run offline (tools/wheel_comp_reference.py): 5,000 wheels, limits the wrong way round, NaN
    /// and infinities included - every word equal (a NaN as a NaN).
    #[test]
    fn wheel_comp_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let nan = |x: u32| if f32::from_bits(x).is_nan() { 0x7fc0_0000 } else { x };
        let mut n = 0;
        for line in include_str!("../testdata/wheel_comp_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let c: Vec<f32> = case.split(' ').map(|x| f32::from_bits(h(x))).collect();
            let w: Vec<u32> = want.split(' ').map(h).collect();
            let got = [super::wheel_comp(c[0], c[1], c[2]).to_bits(), super::axle_disp(c[0], c[1], c[2], c[3]).to_bits()];
            assert_eq!(got.map(nan), [nan(w[0]), nan(w[1])], "{line}");
            n += 1;
        }
        assert_eq!(n, 5000);
    }

    /// A box's moments F-30573 and the world inverse inertia F-16702 run offline (tools/inertia_reference.py): 3,003
    /// boxes and 3,000 bodies (rotations, some a little off square, and the identity) - every word equal.
    #[test]
    fn inertia_matches_the_original() {
        use glam::Vec3;
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let mut n = 0;
        for line in include_str!("../testdata/inertia_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let mut w = case.split(' ');
            let kind = w.next().unwrap();
            let c: Vec<f32> = w.map(|x| f32::from_bits(h(x))).collect();
            let want: Vec<u32> = want.split(' ').map(h).collect();
            let got: Vec<u32> = if kind == "box" {
                crate::physics::loose::box_inertia(c[0], Vec3::new(c[1], c[2], c[3])).iter().map(|x| x.to_bits()).collect()
            } else {
                let rows = [Vec3::new(c[0], c[1], c[2]), Vec3::new(c[3], c[4], c[5]), Vec3::new(c[6], c[7], c[8])];
                super::world_inverse_inertia(&rows, Vec3::new(c[9], c[10], c[11])).iter().flat_map(|r| r.to_array()).map(|x| x.to_bits()).collect()
            };
            assert_eq!(got, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 8003);
    }

    #[test]
    fn stop_rate_matches_the_original() {
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let nan = |x: u32| if f32::from_bits(x).is_nan() { 0x7fc0_0000 } else { x };
        let mut n = 0;
        for line in include_str!("../testdata/stop_rate_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let c: Vec<f32> = case.split(' ').map(|x| f32::from_bits(h(x))).collect();
            let got = super::stop_rate(c[0], c[1], c[2], c[3], c[4]).to_bits();
            assert_eq!(nan(got), nan(h(want)), "{line}: {got:08x}");
            n += 1;
        }
        assert_eq!(n, 6000);
    }

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

    /// The script native's arccosine F-62232 (through the C runtime's F-68873 -> F-11618) run offline
    /// (tools/acos_reference.py): 8,000 floats (random, 0.5 and +-1 with ulps either side, +-0, tiny, NaN, the
    /// infinities; NaN compared as NaN, the harness's sign bit being x86's) - the stored float equal to `guest_acos(x) as f32`.
    #[test]
    fn guest_acos_is_the_originals() {
        let mut n = 0;
        for line in include_str!("testdata/acos_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let x = f32::from_bits(u32::from_str_radix(i, 16).unwrap());
            let canon = |w: u32| if f32::from_bits(w).is_nan() { 0x7fc0_0000 } else { w };
            let got = canon((guest_acos(x as f64) as f32).to_bits());
            assert_eq!(got, canon(u32::from_str_radix(o, 16).unwrap()), "{line}");
            n += 1;
        }
        assert_eq!(n, 8000);
    }

    /// The wheel's lean F-41945 (with F-10410) and steering frame F-49981 (with F-81327) run offline
    /// (tools/wheel_frames_reference.py): 6,000 leans (random rows, the signed coordinate axes, non-unit axes, angles 0,
    /// +-pi, random and tiny) and 4,000 axes (random, either side of the 3.6 degree limit, straight up and down, the
    /// coordinate axes) - every word of the rows equal to `leaned` / `steer_frame`, NaN compared as NaN.
    #[test]
    fn wheel_frames_are_the_originals() {
        use glam::Vec3;
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let canon = |w: u32| if f32::from_bits(w).is_nan() { 0x7fc0_0000 } else { w };
        let (mut nl, mut nf) = (0, 0);
        for line in include_str!("testdata/wheel_frames_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let (kind, case) = case.split_once(' ').unwrap();
            let c: Vec<f32> = case.split(' ').map(|x| f32::from_bits(h(x))).collect();
            let v = |i: usize| Vec3::new(c[i], c[i + 1], c[i + 2]);
            let rows = if kind == "L" {
                nl += 1;
                leaned(&[v(0), v(4), v(8)], v(12), c[16])
            } else {
                nf += 1;
                steer_frame(v(0))
            };
            let got: Vec<u32> = rows.iter().flat_map(|r| r.to_array()).map(|x| canon(x.to_bits())).collect();
            let want: Vec<u32> = want.split_whitespace().map(h).map(canon).collect();
            assert_eq!(got, want, "{line}");
        }
        assert_eq!((nl, nf), (6000, 4000));
    }

    #[test]
    fn guest_atan2_is_the_originals() {
        let mut bad = Vec::new();
        let mut n = 0;
        for line in include_str!("guest_atan2_cases.txt").lines() {
            let mut w = line.split(' ').map(|h| u64::from_str_radix(h, 16).unwrap());
            let (y, x, want) = (f64::from_bits(w.next().unwrap()), f64::from_bits(w.next().unwrap()), w.next().unwrap());
            n += 1;
            if guest_atan2(y, x).to_bits() != want {
                bad.push((y, x, f64::from_bits(want), guest_atan2(y, x)));
            }
        }
        assert!(bad.is_empty(), "{} of {n} differ, first {:?}", bad.len(), &bad[..bad.len().min(4)]);
    }
}

#[cfg(test)]
mod gyro_tests {
    use super::*;

    /// vehGyro's turn F-81152 run whole offline (tools/gyro_turn_reference.py): the torque taken through the chassis frame
    /// and applied (body slot 33's vector), the force (slot 128's) with the ground normal's part taken off - 6,000 cases
    /// (frames not square, places some hundred metres out, NaN, infinities, no normal and one). The body's slot 17 hands the
    /// case's three rows as the world inverse inertia.
    #[test]
    fn gyro_turn_matches_the_original() {
        let w = |s: &str| u32::from_str_radix(s.trim_start_matches('h'), 16).unwrap();
        let (mut n, mut with_normal) = (0, 0);
        for line in include_str!("../testdata/gyro_turn_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let i: Vec<&str> = i.split_whitespace().collect();
            let o: Vec<u32> = o.split_whitespace().map(w).collect();
            let f = |k: usize| f32::from_bits(w(i[k]));
            let v = |k: usize| Vec3::new(f(k), f(k + 1), f(k + 2));
            let chassis = [v(6), v(9), v(12), v(15)];
            let m = [v(18), v(21), v(24)];
            let (applied, force) = gyro_turn_with(v(0), v(3), &chassis, &m, f(27));
            let force = if i[28] == "none" { force } else { with_normal += 1; off_ground(force, v(28)) };
            let got = [applied.x, applied.y, applied.z, force.x, force.y, force.z];
            for k in 0..6 {
                let same = got[k].to_bits() == o[k] || (got[k].is_nan() && f32::from_bits(o[k]).is_nan());
                assert!(same, "word {k}: {:08x} against {:08x}: {line}", got[k].to_bits(), o[k]);
            }
            n += 1;
        }
        assert_eq!(n, 6000);
        assert!(with_normal > 3000);
    }
}
