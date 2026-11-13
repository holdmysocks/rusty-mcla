//! The livery's DXT5 compression, as the original's livery job does it. F-80492 starts the job (busy byte +1696); its worker
//! F-93625 runs the body F-42143, which for each mip untiles the painted target (F-29807) and hands it to the XDK's surface copy
//! F-60873: linear A8R8G8B8 -> DXT5 (0x1A200054), alpha threshold 0, flags 1. The copy reads every 4 x 4 block as floats - each
//! byte times f32(1 / 255), r g b a - and encodes it with F-45145 (premultiplied path off: its global is 0):
//!
//! - colour (F-39916, D3DX's BC1 encoder): Floyd-Steinberg error diffusion (7/16, 3/16, 5/16, 1/16) while rounding to 5:6:5,
//!   luminance weights (0.2970, 1, 0.1008), the end points by F-11276 (the box's diagonal, then up to 8 Newton steps), packed by
//!   F-85341, four steps {0, 2, 3, 1}, the texels diffused again while they are matched;
//! - alpha (F-71349): the same diffusion while rounding to 8 bits, 6 steps (0 and 255 kept) when the block holds 0 or 1, else 8,
//!   the end points by F-18905 (up to 8 Newton steps), its step tables read from the executable as they are (the 8-step pD table
//!   ends in 8/7 there).
//!
//! The arithmetic is the generated code's: every single-precision operation is done in double and rounded (fmadds is one fused
//! double multiply-add), the step lerps are vector operations (multiply, then add, denormals flushed). Bit for bit as the
//! original's code run offline (tools/livery_bc3_reference.py, test `livery_bc3::tests::blocks_match_the_original`). The block
//! is written in the PC's order (alpha end points, 6 alpha index bytes, the two 5:6:5 colours and the 32 index bits, little
//! endian); the original stores the same block byte-swapped per 16 bits for the Xbox's GPU.

#[inline]
fn add(a: f32, b: f32) -> f32 {
    (a as f64 + b as f64) as f32
}
#[inline]
fn sub(a: f32, b: f32) -> f32 {
    (a as f64 - b as f64) as f32
}
#[inline]
fn mul(a: f32, b: f32) -> f32 {
    (a as f64 * b as f64) as f32
}
#[inline]
fn div(a: f32, b: f32) -> f32 {
    (a as f64 / b as f64) as f32
}
#[inline]
fn madd(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}
/// fctiwz as the recomp runs it: NaN -> 0x80000000, at or over INT_MAX -> INT_MAX, else truncation (under INT_MIN -> INT_MIN).
#[inline]
fn ctiwz(v: f32) -> i32 {
    if v.is_nan() {
        i32::MIN
    } else if v as f64 >= i32::MAX as f64 {
        i32::MAX
    } else {
        v as i32
    }
}
#[inline]
fn ftz(x: f32) -> f32 {
    if x.is_subnormal() {
        0.0f32.copysign(x)
    } else {
        x
    }
}
/// vsubfp / vmaddfp (the recomp: a single multiply and a single add, denormals flushed in and out) on the three colour lanes.
fn vlerp(s0: [f32; 3], s1: [f32; 3], t: f32) -> [f32; 3] {
    let t = ftz(t);
    std::array::from_fn(|k| {
        let d = ftz(ftz(s1[k]) - ftz(s0[k]));
        ftz(ftz(d * t) + ftz(s0[k]))
    })
}

const fn f(bits: u32) -> f32 {
    f32::from_bits(bits)
}
const LUM: [f32; 3] = [f(0x3e981530), 1.0, f(0x3dce6734)];
const LUM_INV: [f32; 3] = [f(0x4057762e), 1.0, f(0x411ec1dd)];
const R31: f32 = f(0x3d042108);
const R63: f32 = f(0x3c820821);
const R255: f32 = f(0x3b808081);
const THIRD: f32 = f(0x3eaaaaab);
const TWO_THIRDS: f32 = f(0x3f2aaaab);
const FIFTH: f32 = f(0x3e4ccccd);
const SEVENTH: f32 = f(0x3e124925);
const E4096: f32 = f(0x39800000);
const E65536: f32 = f(0x37800000);
const E64: f32 = f(0x3c800000);
const E256: f32 = f(0x3b800000);
const W716: f32 = 0.4375;
const W516: f32 = 0.3125;
const W316: f32 = 0.1875;
const W116: f32 = 0.0625;

/// The executable's words from the 8-step alpha pD table on (one table after another, as they lie there), so that an index
/// one or two before a table reads what the original reads.
const TAB: [u32; 65] = [
    0x00000000, 0x3e124925, 0x3e924925, 0x3edb6db7, 0x3f124925, 0x3f36db6e, 0x3f5b6db7, 0x3f924925, // pD8
    0x3f800000, 0x3f5b6db7, 0x3f36db6e, 0x3f124925, 0x3edb6db7, 0x3e924925, 0x3e124925, 0x00000000, // pC8
    0x00000000, 0x3e4ccccd, 0x3ecccccd, 0x3f19999a, 0x3f4ccccd, 0x3f800000, // pD6
    0x3f800000, 0x3f4ccccd, 0x3f19999a, 0x3ecccccd, 0x3e4ccccd, 0x00000000, // pC6
    0x39800000, // 1/4096
    0x00000000, 0x3eaaaaab, 0x3f2aaaab, 0x3f800000, // pD4
    0x3f800000, 0x3f2aaaab, 0x3eaaaaab, 0x00000000, // pC4
    0x00000000, 0x3f000000, 0x3f800000, // pD3
    0x3f800000, 0x3f000000, 0x00000000, // pC3
    0, 2, 3, 1, // the colour order, 4 steps
    0, 2, 1, // ... 3 steps
    0x3ee00000, // 7/16
    0, 2, 3, 4, 5, 6, 7, 1, // the alpha order, 8 steps
    0, 2, 3, 4, 5, 1, // ... 6 steps
];
const PD8: usize = 0;
const PC8: usize = 8;
const PD6: usize = 16;
const PC6: usize = 22;
const PD4: usize = 29;
const PC4: usize = 33;
const PD3: usize = 37;
const PC3: usize = 40;
const STEPS4: usize = 43;
const STEPS3: usize = 47;
const STEPS8: usize = 51;
const STEPS6: usize = 59;

fn tab_f(base: usize, i: i32) -> f32 {
    TAB.get((base as i32 + i) as usize).map_or(0.0, |&w| f32::from_bits(w))
}

/// F-85341: a colour (clamped to 0 .. 1) to 5:6:5, each channel trunc(c x 31 (63) + 0.5) fused.
pub fn encode565(c: [f32; 3]) -> u16 {
    let cl = |v: f32| if v < 0.0 { 0.0 } else if v > 1.0 { 1.0 } else { v };
    let (r, g, b) = (cl(c[0]), cl(c[1]), cl(c[2]));
    let g = ctiwz(madd(g, 63.0, 0.5)) as u32;
    let b = ctiwz(madd(b, 31.0, 0.5)) as u32;
    let r = ctiwz(madd(r, 31.0, 0.5)) as u32;
    ((((r << 6) | g) << 5 | b) & 0xffff) as u16
}

/// F-11276: the two end points for `steps` (3 or 4) colour steps of `pts` (weighted colours).
fn optimize_rgb(pts: &[[f32; 4]; 16], steps: usize) -> ([f32; 3], [f32; 3]) {
    let (pc, pd) = if steps == 3 { (PC3, PD3) } else { (PC4, PD4) };
    // (the program's stack words -304 .. -212: its step array at -272, a scratch word pair at -288 - read when a step
    // index comes out negative)
    let mut fr = [0u32; 24];
    let at = |off: i32| ((off + 304) / 4) as usize;
    fr[at(-288)] = LUM[0].to_bits();
    fr[at(-284)] = LUM[1].to_bits();
    fr[at(-280)] = LUM[2].to_bits();
    fr[at(-276)] = 1.0f32.to_bits();
    let (mut xr, mut xg, mut xb) = (LUM[0], LUM[1], LUM[2]);
    let (mut yr, mut yg, mut yb) = (0.0f32, 0.0f32, 0.0f32);
    for p in pts {
        let (r, g, b) = (p[0], p[1], p[2]);
        if r < xr {
            xr = r;
        }
        if g < xg {
            xg = g;
        }
        if b < xb {
            xb = b;
        }
        if r > yr {
            yr = r;
        }
        if g > yg {
            yg = g;
        }
        if b > yb {
            yb = b;
        }
    }
    let abg = sub(yg, xg);
    let abb = sub(yb, xb);
    let abr = sub(yr, xr);
    let fab = madd(abr, abr, madd(abb, abb, mul(abg, abg)));
    if fab < f32::MIN_POSITIVE {
        return ([xr, xg, xb], [yr, yg, yb]);
    }
    let mr = mul(add(yr, xr), 0.5);
    let mg = mul(add(yg, xg), 0.5);
    let mb = mul(add(yb, xb), 0.5);
    let inv = div(1.0, fab);
    let (dr, dg, db) = (mul(inv, abr), mul(inv, abg), mul(inv, abb));
    let (mut f0, mut f1, mut f2, mut f3) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for p in pts {
        let pg = mul(sub(p[1], mg), dg);
        let pb = mul(sub(p[2], mb), db);
        let pr = mul(sub(p[0], mr), dr);
        let bg = add(pb, pg);
        let rmg = sub(pr, pg);
        let gr = add(pg, pr);
        let a0 = add(bg, pr);
        let a2 = add(rmg, pb);
        let a1 = sub(gr, pb);
        let a3 = sub(rmg, pb);
        f0 = madd(a0, a0, f0);
        f2 = madd(a2, a2, f2);
        f1 = madd(a1, a1, f1);
        f3 = madd(a3, a3, f3);
    }
    fr[at(-276)] = f3.to_bits();
    fr[at(-284)] = f1.to_bits();
    fr[at(-280)] = f2.to_bits();
    fr[at(-288)] = f0.to_bits();
    let dirs = [f0, f1, f2, f3];
    let (mut best, mut imax) = (f0, 0usize);
    for (i, &d) in dirs.iter().enumerate().skip(1) {
        if d > best {
            best = d;
            imax = i;
        }
    }
    if imax & 2 != 0 {
        std::mem::swap(&mut xg, &mut yg);
    }
    if imax & 1 != 0 {
        std::mem::swap(&mut xb, &mut yb);
    }
    fr[at(-304)] = E4096.to_bits();
    if fab < E4096 {
        return ([xr, xg, xb], [yr, yg, yb]);
    }
    fr[at(-288)] = 0;
    fr[at(-284)] = (steps - 1) as u32;
    let fsteps = (steps - 1) as f32;
    for _ in 0..8 {
        for i in 0..steps {
            let c = tab_f(pc, i as i32);
            let d = tab_f(pd, i as i32);
            let o = -272 + 16 * i as i32;
            fr[at(o)] = madd(d, yr, mul(c, xr)).to_bits();
            fr[at(o + 4)] = madd(d, yg, mul(c, xg)).to_bits();
            fr[at(o + 8)] = madd(d, yb, mul(c, xb)).to_bits();
        }
        let dg0 = sub(yg, xg);
        let db0 = sub(yb, xb);
        let dr0 = sub(yr, xr);
        let flen = madd(dr0, dr0, madd(db0, db0, mul(dg0, dg0)));
        if flen < E4096 {
            break;
        }
        let s = div(fsteps, flen);
        let (dr, dg, db) = (mul(s, dr0), mul(s, dg0), mul(s, db0));
        let (mut d2x, mut d2y) = (0.0f32, 0.0f32);
        let (mut dxr, mut dxg, mut dxb, mut dyr, mut dyg, mut dyb) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for p in pts {
            let dot = madd(sub(p[0], xr), dr, madd(sub(p[2], xb), db, mul(sub(p[1], xg), dg)));
            let istep: i32 = if dot < fsteps {
                let v = ctiwz(add(dot, 0.5));
                // (stfd: the conversion's 64 bits, sign-extended as the recomp holds them)
                fr[at(-288)] = ((v as i64) >> 32) as u32;
                fr[at(-284)] = v as u32;
                v
            } else {
                (steps - 1) as i32
            };
            let o = -272 + 16 * istep;
            let rd = |off: i32| {
                let k = off + 304;
                if (0..96).contains(&k) {
                    f32::from_bits(fr[(k / 4) as usize])
                } else {
                    0.0
                }
            };
            let sr = rd(o);
            let sg = rd(o + 4);
            let sb = rd(o + 8);
            let c = tab_f(pc, istep);
            let d = tab_f(pd, istep);
            let er = sub(sr, p[0]);
            let c8 = mul(c, 0.125);
            let d8 = mul(d, 0.125);
            let eg = sub(sg, p[1]);
            let eb = sub(sb, p[2]);
            d2x = madd(c, c8, d2x);
            d2y = madd(d, d8, d2y);
            dxr = madd(c8, er, dxr);
            dxg = madd(c8, eg, dxg);
            dxb = madd(c8, eb, dxb);
            dyr = madd(d8, er, dyr);
            dyg = madd(d8, eg, dyg);
            dyb = madd(d8, eb, dyb);
        }
        if !(d2x <= 0.0) {
            let k = div(-1.0, d2x);
            xr = madd(k, dxr, xr);
            xg = madd(k, dxg, xg);
            xb = madd(k, dxb, xb);
        }
        if !(d2y <= 0.0) {
            let k = div(-1.0, d2y);
            yr = madd(k, dyr, yr);
            yg = madd(k, dyg, yg);
            yb = madd(k, dyb, yb);
        }
        let small = |v: f32| mul(v, v) < E65536;
        if small(dxr) && small(dxg) && small(dxb) && small(dyr) && small(dyg) && small(dyb) {
            break;
        }
    }
    ([xr, xg, xb], [yr, yg, yb])
}

/// The Floyd-Steinberg spread of one texel's error `d` over the block (F-39916 / F-71349: 7/16 right, then on the next row
/// 3/16 left, 5/16 below, 1/16 right).
fn spread<const N: usize>(err: &mut [[f32; N]; 16], i: usize, d: [f32; N]) {
    if i & 3 != 3 {
        for k in 0..N {
            err[i + 1][k] = madd(d[k], W716, err[i + 1][k]);
        }
    }
    if i < 12 {
        if i & 3 != 0 {
            for k in 0..N {
                err[i + 3][k] = madd(d[k], W316, err[i + 3][k]);
            }
        }
        for k in 0..N {
            err[i + 4][k] = madd(d[k], W516, err[i + 4][k]);
        }
        if i & 3 != 3 {
            for k in 0..N {
                err[i + 5][k] = madd(d[k], W116, err[i + 5][k]);
            }
        }
    }
}

/// F-39916 with four steps, dithered (flags 1): the colour half of a block -> (colour 0, colour 1, the 32 index bits).
fn encode_bc1(px: &[[f32; 4]; 16]) -> (u16, u16, u32) {
    let steps = 4usize;
    let mut err = [[0.0f32; 3]; 16];
    let mut col = [[0.0f32; 4]; 16];
    for i in 0..16 {
        let r = add(err[i][0], px[i][0]);
        let g = add(px[i][1], err[i][1]);
        let b = add(err[i][2], px[i][2]);
        let qr = ctiwz(madd(r, 31.0, 0.5)) as f32;
        let qg = ctiwz(madd(g, 63.0, 0.5)) as f32;
        let qb = ctiwz(madd(b, 31.0, 0.5)) as f32;
        let cb = mul(qb, R31);
        let cr = mul(qr, R31);
        let cg = mul(qg, R63);
        spread(&mut err, i, [sub(r, cr), sub(g, cg), sub(b, cb)]);
        col[i] = [mul(cr, LUM[0]), mul(cg, LUM[1]), mul(cb, LUM[2]), 1.0];
    }
    let (a, b) = optimize_rgb(&col, steps);
    let c = [mul(LUM_INV[0], a[0]), mul(a[1], LUM_INV[1]), mul(a[2], LUM_INV[2])];
    let d = [mul(LUM_INV[0], b[0]), mul(b[1], LUM_INV[1]), mul(b[2], LUM_INV[2])];
    let wa = encode565(c);
    let wb = encode565(d);
    if steps == 4 && wa == wb {
        return (wa, wb, 0);
    }
    let dec = |w: u16| {
        let w = w as u32;
        [mul(LUM[0], mul(((w >> 11) & 31) as f32, R31)), mul(mul(((w >> 5) & 63) as f32, R63), LUM[1]), mul(mul((w & 31) as f32, R31), LUM[2])]
    };
    let ca = dec(wa);
    let cb = dec(wb);
    let (c0, c1, s0, s1) = if (steps == 3) == (wa <= wb) { (wa, wb, ca, cb) } else { (wb, wa, cb, ca) };
    let s2 = vlerp(s0, s1, THIRD);
    let s3 = vlerp(s0, s1, TWO_THIRDS);
    let st = [s0, s1, s2, s3];
    let fsteps = (steps - 1) as f32;
    let dr0 = sub(s1[0], s0[0]);
    let db0 = sub(s1[2], s0[2]);
    let dg0 = sub(s1[1], s0[1]);
    let scale = if wa != wb { div(fsteps, madd(dr0, dr0, madd(db0, db0, mul(dg0, dg0)))) } else { 0.0 };
    let (dr, dg, db) = (mul(scale, dr0), mul(scale, dg0), mul(scale, db0));
    let mut err = [[0.0f32; 3]; 16];
    let mut dw = 0u32;
    for i in 0..16 {
        let mut r = mul(px[i][0], LUM[0]);
        let mut g = mul(px[i][1], LUM[1]);
        let mut b = mul(px[i][2], LUM[2]);
        r = add(err[i][0], r);
        b = add(err[i][2], b);
        g = add(err[i][1], g);
        let dot = madd(sub(r, s0[0]), dr, madd(sub(b, s0[2]), db, mul(sub(g, s0[1]), dg)));
        let istep = if !(dot > 0.0) {
            0usize
        } else if !(dot < fsteps) {
            1
        } else {
            TAB[STEPS4 + ctiwz(add(dot, 0.5)) as usize] as usize
        };
        dw = ((istep as u32) << 30) | (dw >> 2);
        let a = col[i][3];
        spread(&mut err, i, [mul(sub(r, st[istep][0]), a), mul(sub(g, st[istep][1]), a), mul(sub(b, st[istep][2]), a)]);
    }
    let _ = STEPS3;
    (c0, c1, dw)
}

/// F-18905: the two alpha end points for `steps` (6 or 8).
fn optimize_alpha(pts: &[f32; 16], steps: usize) -> (f32, f32) {
    let (pc, pd) = if steps == 6 { (PC6, PD6) } else { (PC8, PD8) };
    let (mut x, mut y) = (1.0f32, 0.0f32);
    if steps == 8 {
        for &a in pts {
            if a < x {
                x = a;
            }
            if a > y {
                y = a;
            }
        }
    } else {
        for &a in pts {
            if a < x && a > 0.0 {
                x = a;
            }
            if a > y && a < 1.0 {
                y = a;
            }
        }
        if x == y {
            y = 1.0;
        }
    }
    let fsteps = (steps - 1) as f32;
    let mut st = [0.0f32; 8];
    for _ in 0..8 {
        let len = sub(y, x);
        if len < E256 {
            break;
        }
        let scale = div(fsteps, len);
        for i in 0..steps {
            st[i] = madd(tab_f(pd, i as i32), y, mul(tab_f(pc, i as i32), x));
        }
        if steps == 6 {
            st[6] = 0.0;
            st[7] = 1.0;
        }
        let (mut dx, mut dy, mut d2x, mut d2y) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for &a in pts {
            let dot = mul(sub(a, x), scale);
            let istep = if !(dot > 0.0) {
                if steps == 6 && !(a > mul(x, 0.5)) {
                    continue;
                }
                0usize
            } else if !(dot < fsteps) {
                if steps == 6 && !(a < mul(add(y, 1.0), 0.5)) {
                    continue;
                }
                steps - 1
            } else {
                ctiwz(add(dot, 0.5)) as u32 as usize
            };
            if istep >= steps {
                continue;
            }
            let c = tab_f(pc, istep as i32);
            let d = tab_f(pd, istep as i32);
            d2x = madd(c, c, d2x);
            let e = sub(a, st[istep]);
            d2y = madd(d, d, d2y);
            dx = madd(c, e, dx);
            dy = madd(d, e, dy);
        }
        if !(d2x <= 0.0) {
            x = sub(x, div(dx, d2x));
        }
        if !(d2y <= 0.0) {
            y = sub(y, div(dy, d2y));
        }
        if x > y {
            std::mem::swap(&mut x, &mut y);
        }
        if mul(dx, dx) < E64 && mul(dy, dy) < E64 {
            break;
        }
    }
    let cl = |v: f32| if v < 0.0 { 0.0 } else if v > 1.0 { 1.0 } else { v };
    (cl(x), cl(y))
}

/// F-71349, dithered (flags 1): the alpha half of a block -> its 8 bytes in the PC's order.
fn encode_alpha(px: &[[f32; 4]; 16]) -> [u8; 8] {
    let mut err = [[0.0f32; 1]; 16];
    let mut fa = [0.0f32; 16];
    let (mut amin, mut amax) = (px[0][3], px[0][3]);
    for i in 0..16 {
        let a = add(err[i][0], px[i][3]);
        let q = mul(ctiwz(madd(a, 255.0, 0.5)) as f32, R255);
        fa[i] = q;
        if q < amin {
            amin = q;
        } else if q > amax {
            amax = q;
        }
        spread(&mut err, i, [sub(a, q)]);
    }
    let mut out = [0u8; 8];
    if amin == 1.0 {
        out[0] = 255;
        out[1] = 255;
        return out;
    }
    let steps = if amin == 0.0 || amax == 1.0 { 6 } else { 8 };
    let (fa_a, fa_b) = optimize_alpha(&fa, steps);
    let ba = (ctiwz(madd(fa_a, 255.0, 0.5)) as u32 & 255) as u8;
    let bb = (ctiwz(madd(fa_b, 255.0, 0.5)) as u32 & 255) as u8;
    let fa_a = mul(ba as f32, R255);
    let fa_b = mul(bb as f32, R255);
    if steps == 8 && ba == bb {
        out[0] = ba;
        out[1] = bb;
        return out;
    }
    let mut st = [0.0f32; 8];
    let order;
    if steps == 6 {
        out[0] = ba;
        out[1] = bb;
        st[0] = fa_a;
        st[1] = fa_b;
        for i in 1..5u32 {
            st[i as usize + 1] = mul(madd((5 - i) as f32, st[0], mul(i as f32, st[1])), FIFTH);
        }
        st[6] = 0.0;
        st[7] = 1.0;
        order = STEPS6;
    } else {
        out[0] = bb;
        out[1] = ba;
        st[1] = fa_a;
        st[0] = fa_b;
        for i in 1..7u32 {
            st[i as usize + 1] = mul(madd((7 - i) as f32, st[0], mul(i as f32, st[1])), SEVENTH);
        }
        order = STEPS8;
    }
    let fsteps = (steps - 1) as f32;
    let scale = if st[0] == st[1] { 0.0 } else { div(fsteps, sub(st[1], st[0])) };
    let mut err = [[0.0f32; 1]; 16];
    for set in 0..2 {
        let mut dw = 0u32;
        for i in set * 8..set * 8 + 8 {
            let a = add(err[i][0], px[i][3]);
            let dot = mul(sub(a, st[0]), scale);
            let istep = if !(dot > 0.0) {
                if steps == 6 && !(a > mul(st[0], 0.5)) { 6 } else { 0 }
            } else if !(dot < fsteps) {
                if steps == 6 && !(a < mul(add(st[1], 1.0), 0.5)) { 7 } else { 1 }
            } else {
                TAB[order + ctiwz(add(dot, 0.5)) as usize] as usize
            };
            dw = ((istep as u32) << 21) | (dw >> 3);
            spread(&mut err, i, [sub(a, st[istep])]);
        }
        out[2 + set * 3] = dw as u8;
        out[3 + set * 3] = (dw >> 8) as u8;
        out[4 + set * 3] = (dw >> 16) as u8;
    }
    out
}

/// F-45145 on one block of texel bytes (r g b a, row by row): its 16 BC3 bytes in the PC's order.
pub fn encode_block(texels: &[[u8; 4]; 16]) -> [u8; 16] {
    let px: [[f32; 4]; 16] = std::array::from_fn(|i| std::array::from_fn(|k| mul(texels[i][k] as f32, R255)));
    let a = encode_alpha(&px);
    let (c0, c1, dw) = encode_bc1(&px);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a);
    out[8..10].copy_from_slice(&c0.to_le_bytes());
    out[10..12].copy_from_slice(&c1.to_le_bytes());
    out[12..].copy_from_slice(&dw.to_le_bytes());
    out
}

/// A whole RGBA8 image (w, h multiples of 4) to BC3, blocks row by row - what the job leaves in the car's texture.
pub fn encode_image(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
    let mut out = vec![0u8; bw * bh * 16];
    use rayon::prelude::*;
    out.par_chunks_mut(bw * 16).enumerate().for_each(|(by, row)| {
        for bx in 0..bw {
            let t: [[u8; 4]; 16] = std::array::from_fn(|i| {
                let (x, y) = ((bx * 4 + i % 4).min(w - 1), (by * 4 + i / 4).min(h - 1));
                let o = (y * w + x) * 4;
                [rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]]
            });
            row[bx * 16..bx * 16 + 16].copy_from_slice(&encode_block(&t));
        }
    });
    out
}

/// The livery's mip chain as the original's work target gets it before the job compresses every level (F-42143): the
/// pass end F-52621 unlocks the target with the resolve flags' make-mips byte (+24) set, and the target's mip maker F-27026
/// draws each level from the one above with rage_fastmipmap's PSFastMipMap (loaded by F-58842): four bilinear taps at
/// +-0.5 of the destination texel x 0.25, which is a centred 4 x 4 box of the level above (texels 2i - 1 .. 2i + 2,
/// clamped at the edge; measured on the original's env map, render/env_mip.wgsl), written to the 8-bit target (rounded to
/// nearest). Levels down to 4 x 4: 9 for the 1024 livery, 8 for the editor's 512. Level 0 is `rgba` itself.
pub fn mip_chain(rgba: &[u8], w: usize, h: usize) -> Vec<(Vec<u8>, usize, usize)> {
    use rayon::prelude::*;
    let mut out = vec![(rgba.to_vec(), w, h)];
    while out.last().map_or(false, |l| l.1 > 4 && l.2 > 4) {
        let (src, sw, sh) = out.last().unwrap();
        let (sw, sh) = (*sw, *sh);
        let (dw, dh) = (sw / 2, sh / 2);
        let mut dst = vec![0u8; dw * dh * 4];
        dst.par_chunks_mut(dw * 4).enumerate().for_each(|(y, row)| {
            for x in 0..dw {
                let mut acc = [0u32; 4];
                for dy in -1i32..3 {
                    let sy = (2 * y as i32 + dy).clamp(0, sh as i32 - 1) as usize;
                    for dx in -1i32..3 {
                        let sx = (2 * x as i32 + dx).clamp(0, sw as i32 - 1) as usize;
                        let o = (sy * sw + sx) * 4;
                        for c in 0..4 {
                            acc[c] += src[o + c] as u32;
                        }
                    }
                }
                for c in 0..4 {
                    row[x * 4 + c] = ((acc[c] + 8) / 16) as u8;
                }
            }
        });
        out.push((dst, dw, dh));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tools/livery_bc3_reference.py: the original's surface copy F-60873 run offline on 15 images of 32 x 32 (noise,
    /// gradients, flat blocks, soft-edged stripes, greys, extremes, transparent fields; 960 blocks): every byte as the port's.
    /// The generator's constructor F-88260: the 1024 work target has 9 levels, the 512 one 8 (both down to 4 x 4); a flat
    /// image stays flat, a 4 x 4 box of the level above.
    #[test]
    fn mip_chain_levels() {
        for (n, want) in [(1024usize, 9usize), (512, 8)] {
            let img: Vec<u8> = (0..n * n).flat_map(|_| [10u8, 200, 30, 255]).collect();
            let c = mip_chain(&img, n, n);
            assert_eq!(c.len(), want);
            assert_eq!((c[want - 1].1, c[want - 1].2), (4, 4));
            assert!(c.iter().all(|(l, _, _)| l.chunks(4).all(|p| p == [10, 200, 30, 255])));
        }
        // texel (1, 1) of level 1 of a 8 x 8 ramp: the mean of source texels 1 .. 4 in x and y
        let img: Vec<u8> = (0..64).flat_map(|i| [(i % 8 * 16) as u8, (i / 8 * 16) as u8, 0, 255]).collect();
        let c = mip_chain(&img, 8, 8);
        assert_eq!(&c[1].0[(4 + 1) * 4..(4 + 1) * 4 + 2], &[40, 40]);
    }

    #[test]
    fn blocks_match_the_original() {
        let text = include_str!("testdata/livery_bc3_ref.txt");
        let mut lines = text.lines();
        let (mut images, mut blocks, mut bad) = (0, 0, 0);
        while let Some(head) = lines.next() {
            let h: Vec<usize> = head.split_whitespace().skip(1).map(|v| v.parse().unwrap()).collect();
            let (w, ht) = (h[0], h[1]);
            // (one line per image row, its ARGB words unbroken)
            let px: Vec<u32> = (0..ht)
                .flat_map(|_| {
                    let row = lines.next().unwrap();
                    (0..w).map(|x| u32::from_str_radix(&row[x * 8..x * 8 + 8], 16).unwrap()).collect::<Vec<_>>()
                })
                .collect();
            let rgba: Vec<u8> = px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]).collect();
            let got = encode_image(&rgba, w, ht);
            for k in 0..(w / 4) * (ht / 4) {
                let want = lines.next().unwrap();
                let mine: String = got[k * 16..k * 16 + 16].iter().map(|b| format!("{b:02x}")).collect();
                if mine != want {
                    if bad < 8 {
                        eprintln!("image {images} block {k}: port {mine} original {want}");
                    }
                    bad += 1;
                }
                blocks += 1;
            }
            images += 1;
        }
        assert_eq!(images, 15);
        assert_eq!(bad, 0, "{bad} of {blocks} blocks differ");
    }
}
