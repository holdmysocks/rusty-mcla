//! The vinyl editor's group edit (EDIT LAYER on a marked group, the chart's Vinyl_Group_Edit): the zone's group record
//! moved by the stick (F-66271 -> F-48118), baked into the marked layers on ACCEPT (FinalizeGroupMovement, F-88223) and
//! reset by both ACCEPT and BACK (UndoGroupMovement, F-90968). Records are the original's 20-byte layer records as halves:
//! `[0]` scale x, `[1]` scale y, `[2]` pos x, `[3]` pos y, `[5]` skew, `[6]` rotation (degrees), `[7]` shape (0xFFFF empty),
//! `[8]` the mode byte; `[4]` the colour word. Rules: work/ui2/group/rule.md (docs/reference/carparts/vinyl_group_edit.md).

use mcla_formats::guest_math::{guest_cos, guest_sin};

/// A layer or group record: the halves and the colour word as the original holds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rec {
    pub h: [u16; 8],
    pub color: u32,
    pub b: [u8; 2],
}

/// The group record at rest (F-90968): scale 1.0, no offset, no rotation.
pub const IDENTITY: Rec = Rec { h: [0x3C00, 0x3C00, 0, 0, 0, 0, 0, 0], color: 0, b: [0, 0] };

/// The zones' angles (the table F-88223 reads in .rdata): Top, Left, Right, Front, Rear.
const ZONE_ANGLE: [u32; 5] = [0xbfc90fdb, 0xbfc90fdb, 0x3fc90fdb, 0, 0];
const DEG2RAD: u32 = 0x3c8efa35;
const INV127: u32 = 0x3c010204;
const EPS: u32 = 0x3c23d70a;
const P9: u32 = 0x3f666666;
/// F-56493's factors: 1000 (position) and 100 (scale).
const K1000: u32 = 0x447a0000;
const K100: u32 = 0x42c80000;

fn fb(u: u32) -> f32 {
    f32::from_bits(u)
}

/// F-38353's half decoder.
pub fn h2f(h: u16) -> f32 {
    if h == 0 {
        return 0.0;
    }
    let u = ((h as u32 & 0x8000) << 16) | ((((h as u32 >> 10) & 0x1F) + 112) << 23) | ((h as u32 & 0x3FF) << 13);
    f32::from_bits(u)
}

/// F-70220's half cut (`render::half_cut`).
pub fn f2h(x: f32) -> u16 {
    crate::render::half_cut(x)
}

/// fmsubs a*b - c: one rounding.
fn fms(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, -(c as f64)) as f32
}

/// fmadds a*b + c: one rounding.
fn fma(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

fn sin_cos(a: f32) -> (f32, f32) {
    (guest_sin(a as f64) as f32, guest_cos(a as f64) as f32)
}

/// FinalizeGroupMovement's bake (F-88223) of zone `z`'s group record `g` into its marked, non-empty layers: rotated into
/// the zone's frame, by the group's angle, scaled by the group's +0, back out; then rotation + the group's, scale x the
/// group's +0, x - the group's +4, y + the group's +6.
pub fn bake(z: usize, layers: &mut [Rec], marks: &[bool], g: &Rec) {
    let za = ZONE_ANGLE.get(z).map_or(0.0, |u| fb(*u));
    for (l, m) in layers.iter_mut().zip(marks) {
        if !*m || l.h[7] == 0xFFFF {
            continue;
        }
        let (px, py) = (h2f(l.h[2]), h2f(l.h[3]));
        let (s, c) = sin_cos(-za);
        let (r1, r2) = (fms(c, px, py * s), fma(py, c, s * px));
        let th = h2f(g.h[6]) * fb(DEG2RAD);
        let (s, c) = sin_cos(th);
        let (r1, r2) = (fms(c, r1, r2 * s), fma(r2, c, s * r1));
        let gs = h2f(g.h[0]);
        let (r1, r2) = (gs * r1, gs * r2);
        let (s, c) = sin_cos(za);
        let (x, y) = (fms(c, r1, s * r2), fma(c, r2, s * r1));
        l.h[2] = f2h(x);
        l.h[3] = f2h(y);
        l.h[6] = f2h(h2f(g.h[6]) + h2f(l.h[6]));
        l.h[0] = f2h(h2f(g.h[0]) * h2f(l.h[0]));
        l.h[1] = f2h(h2f(g.h[0]) * h2f(l.h[1]));
        l.h[2] = f2h(h2f(l.h[2]) - h2f(g.h[2]));
        l.h[3] = f2h(h2f(g.h[3]) + h2f(l.h[3]));
    }
}

/// UndoGroupMovement / the end of FinalizeGroupMovement (F-90968): scale 1.0, offset and rotation 0; the rest kept.
pub fn reset(g: &mut Rec) {
    g.h[0] = 0x3C00;
    g.h[1] = 0x3C00;
    g.h[2] = 0;
    g.h[3] = 0;
    g.h[6] = 0;
}

fn clampsel(x: f32, lo: f32, hi: f32) -> f32 {
    let x = if x - lo >= 0.0 { x } else { lo };
    if x - hi >= 0.0 {
        hi
    } else {
        x
    }
}

/// F-43389's wrap to -360 .. 360 (by fmod 720).
fn wrap360(x: f32) -> f32 {
    if x < -360.0 {
        return 360.0 - ((-360.0 - x) as f64 % 720.0) as f32;
    }
    if x > 360.0 {
        return ((x - 360.0) as f64 % 720.0) as f32 - 360.0;
    }
    x
}

/// One axis of the edit step F-48118 on the record `r` (the group's when `group`), `mode` the chart's attribute (1
/// position, 2 scale, 3 rotate, 6 skew, 7 the mode byte), `axis` 1 for the stick's x.
pub fn step(r: &mut Rec, z: usize, mode: u32, group: bool, axis: bool, v: f32) {
    let zf = z == 3 || z == 4;
    let v = if group {
        if axis && !zf {
            -v
        } else {
            v
        }
    } else if !axis && mode != 2 && zf {
        -v
    } else {
        v
    };
    let t = (-1200.0f64).mul_add(v.abs() as f64, 1300.0) as f32;
    let t = t.max(100.0).min(600.0);
    let d = v / t;
    let sign: i32 = if v > 0.0 { 1 } else { -1 };
    match mode {
        1 => {
            let k = if zf { !axis } else { axis };
            let i = if k { 2 } else { 3 };
            r.h[i] = f2h(clampsel(h2f(r.h[i]) + d, -2.0, 2.0));
        }
        2 => {
            let i = if axis || group { 0 } else { 1 };
            r.h[i] = f2h(clampsel(h2f(r.h[i]) + d, -5.0, 5.0));
        }
        3 if axis => {
            let w = if v > fb(P9) { v * 2.0 } else { v };
            r.h[6] = f2h(wrap360(h2f(r.h[6]) + w));
        }
        6 if axis => r.h[5] = f2h(clampsel(h2f(r.h[5]) + d, -1.0, 1.0)),
        7 if axis => {
            let x = r.b[0] as i32 + sign;
            r.b[0] = if x >= 0 { (x % 3) as u8 } else { 2 };
        }
        _ => {}
    }
}

/// A stick byte through F-93983's 0.25 dead zone (F-66271's read).
pub fn stick(b: u8) -> f32 {
    let x = ((b as f32 - 127.5) * fb(INV127)).clamp(-1.0, 1.0);
    if x > 0.25 {
        (x - 0.25) / 0.75
    } else if x < -0.25 {
        (x + 0.25) / 0.75
    } else {
        0.0
    }
}

/// ChangeLayerAttribute (F-66271), one UI update with the stick's bytes: nothing while the livery is being drawn (`busy`),
/// else the x axis, then the y axis turned over. True when a step was taken (the UI_MENU_NAV_UP sound).
pub fn change(r: &mut Rec, z: usize, mode: u32, group: bool, busy: bool, bx: u8, by: u8) -> bool {
    if busy {
        return false;
    }
    let (x, y) = (stick(bx), stick(by));
    let e = fb(EPS);
    let mut moved = false;
    if x > e || x < -e {
        step(r, z, mode, group, true, x);
        moved = true;
    }
    if y > e || y < -e {
        step(r, z, mode, group, false, -y);
        moved = true;
    }
    moved
}

/// The port's layer as the original's record (its halves cut from the port's values).
pub fn rec_of(l: &crate::vinyl::Layer) -> Rec {
    Rec { h: [f2h(l.scale[0]), f2h(l.scale[1]), f2h(l.pos[0]), f2h(l.pos[1]), 0, f2h(l.skew), f2h(l.rot), l.shape], color: 0, b: [0, 0] }
}

/// The record's position, scale and rotation back into the port's layer.
pub fn apply(l: &mut crate::vinyl::Layer, r: &Rec) {
    l.scale = [h2f(r.h[0]), h2f(r.h[1])];
    l.pos = [h2f(r.h[2]), h2f(r.h[3])];
    l.rot = h2f(r.h[6]);
}

/// The marked layers as drawn while the group is edited (F-61384 -> F-83876, rule.md section 2): in exact arithmetic the
/// bake, without the half rounding; the port draws the baked values.
pub fn preview(z: usize, layers: &[crate::vinyl::Layer], marks: &[bool], g: &Rec) -> Vec<crate::vinyl::Layer> {
    let mut recs: Vec<Rec> = layers.iter().map(rec_of).collect();
    bake(z, &mut recs, marks, g);
    layers.iter().zip(&recs).zip(marks).map(|((l, r), m)| {
        let mut l = l.clone();
        if *m {
            apply(&mut l, r);
        }
        l
    }).collect()
}


/// The integer wrap F-58674 of `x` into lo ..= hi: lo when the two are equal, `x` when hi < lo, else lo + (x - lo) rem n
/// (n = hi - lo + 1, the remainder with x's sign) - below lo from hi + 1, so lo - n gives hi + 1.
pub fn wrap_int(x: i32, lo: i32, hi: i32) -> i32 {
    if hi == lo {
        return lo;
    }
    if hi < lo {
        return x;
    }
    let n = hi.wrapping_sub(lo).wrapping_add(1);
    let base = if x < lo { hi.wrapping_add(1) } else { lo };
    x.wrapping_sub(lo).wrapping_rem(n).wrapping_add(base)
}

/// One figure of `VElayer_dimension`: a number (type 4) or an int (type 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dim {
    F(f32),
    I(i32),
}

/// The read-out F-56493 of the record the step edits, for the tabs a group has (`mode` 1 .. 3): position +4 x 1000 and
/// +6 x 1000, scale +0 x 100 and (group ? +0 : +2) x 100 (f32 products, as numbers), rotation trunc(+14) wrapped into
/// 0 ..= 359 (an int; only element 0).
pub fn readout(r: &Rec, mode: u32, group: bool) -> Vec<Dim> {
    let k1000 = fb(K1000);
    let k100 = fb(K100);
    match mode {
        1 => vec![Dim::F(h2f(r.h[2]) * k1000), Dim::F(h2f(r.h[3]) * k1000)],
        2 => vec![Dim::F(h2f(r.h[0]) * k100), Dim::F(h2f(r.h[if group { 0 } else { 1 }]) * k100)],
        3 => vec![Dim::I(wrap_int(h2f(r.h[6]) as i32, 0, 359))],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-58674 as its asm reads (divw's remainder with the dividend's sign, hi + 1 as the base below lo).
    #[test]
    fn wrap_int_as_the_original() {
        let cases = [(-1, 359), (-360, 360), (360, 0), (359, 359), (0, 0), (-200, 160), (-361, 359), (719, 359)];
        for (x, want) in cases {
            assert_eq!(wrap_int(x, 0, 359), want, "{x}");
        }
        assert_eq!([wrap_int(3, 0, 2), wrap_int(-1, 0, 2), wrap_int(5, 2, 2), wrap_int(5, 3, 1)], [0, 2, 2, 5]);
        assert_eq!(readout(&IDENTITY, 2, true), vec![Dim::F(100.0), Dim::F(100.0)]);
        assert_eq!(readout(&Rec { h: [0, 0, 0, 0, 0, 0, 0xD9A0, 0], ..IDENTITY }, 3, true), vec![Dim::I(wrap_int(-180, 0, 359))]);
    }

    /// A record from its 20 bytes in hex (+0 +2 +4 +6 halves, +8 colour, +12 skew, +14 rotation, +16 shape, +18 / +19 bytes).
    fn rec(hx: &str) -> Rec {
        let b: Vec<u8> = (0..20).map(|i| u8::from_str_radix(&hx[2 * i..2 * i + 2], 16).unwrap()).collect();
        let w = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
        Rec { h: [w(0), w(2), w(4), w(6), 0, w(12), w(14), w(16)], color: u32::from_be_bytes([b[8], b[9], b[10], b[11]]), b: [b[18], b[19]] }
    }

    /// F-88223 (the bake) and F-66271 / F-48118 (the stick step) against the original's code run whole offline
    /// (tools/vinyl_group_reference.py -> testdata/vinyl_group_ref.txt): 3,000 bakes of random zones, layers and marks with
    /// the five records' reset after, and 3,000 edit sessions (18,228 frames of stick bytes in every mode, group or layer,
    /// busy or not). Every half and byte equal.
    #[test]
    fn group_edit_matches_the_original() {
        let (mut bakes, mut frames) = (0, 0);
        let mut st: Option<(usize, u32, bool, bool, Rec, Rec)> = None;
        for line in include_str!("testdata/vinyl_group_ref.txt").lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            match t.first().copied() {
                Some("B") => {
                    let z: usize = t[1][1..].parse().unwrap();
                    let n: usize = t[2][1..].parse().unwrap();
                    let marks: Vec<bool> = t[3][1..].chars().map(|c| c == '1').collect();
                    let g = rec(&t[4][1..]);
                    let mut layers: Vec<Rec> = (0..n).map(|i| rec(&t[5 + i][1..])).collect();
                    bake(z, &mut layers, &marks, &g);
                    for i in 0..n {
                        assert_eq!(layers[i], rec(&t[6 + n + i][1..]), "{line}");
                    }
                    let mut r = rec(&t[6 + 2 * n][1..41]);
                    reset(&mut r);
                    assert_eq!(r, rec(&t[6 + 2 * n][1..41]), "{line}");
                    bakes += 1;
                }
                Some("E") => {
                    let z: usize = t[1][1..].parse().unwrap();
                    let cur: usize = t[2][1..].parse().unwrap();
                    let marks: Vec<bool> = t[4][1..].chars().map(|c| c == '1').collect();
                    let group = marks.iter().filter(|m| **m).count() > 1 && marks[cur];
                    st = Some((z, t[5][1..].parse().unwrap(), group, &t[6][1..] == "1", rec(&t[7][1..]), rec(&t[8][1..])));
                }
                Some(f) if f.starts_with('f') => {
                    let (z, mode, group, busy, g, l) = st.as_mut().unwrap();
                    let (bx, by) = f[1..].split_once(',').unwrap();
                    let r = if *group { &mut *g } else { &mut *l };
                    change(r, *z, *mode, *group, *busy, bx.parse().unwrap(), by.parse().unwrap());
                    assert_eq!(*g, rec(&t[2][1..]), "{line}");
                    assert_eq!(*l, rec(&t[3][1..]), "{line}");
                    frames += 1;
                }
                _ => {}
            }
        }
        assert_eq!((bakes, frames), (3000, 18228));
    }
}
