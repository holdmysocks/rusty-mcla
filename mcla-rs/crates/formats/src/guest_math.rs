//! The guest C runtime's double-precision sine, cosine and atan2, bit for bit (verified in game::physics::exact's tests).

// ---- the guest C runtime's sine and cosine (F-72580, F-63589), in double: the argument
// less a whole number of half turns in two steps, then an odd polynomial of nine terms.
pub const HALF_PI: f64 = f64::from_bits(0x3ff921fb54442d18);
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

/// The guest C runtime's atan (F-79102), in double, on atan2's table (0x827EB158): |x| above 1 taken as 1 / |x| (above
/// 2^1022 the answer is pi/2), folded about tan(pi/12) by sqrt 3, the same rational of the square, the octant's angle
/// added; the sign by fsel on x (a NaN gives the negated value), +-0 kept.
pub fn guest_atan(x: f64) -> f64 {
    let a = x.abs();
    let r = if a > f64::from_bits(0x7fd0000000000000) {
        f64::from_bits(0x3ff921fb54442d18)
    } else {
        let (mut k, mut f) = if a > 1.0 { (2usize, 1.0 / a) } else { (0usize, a) };
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
    if a.to_bits() == 0 || x >= 0.0 {
        r
    } else {
        -r
    }
}

/// The guest C runtime's atan2 (F-65975), in double: the smaller of |y|, |x|
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
