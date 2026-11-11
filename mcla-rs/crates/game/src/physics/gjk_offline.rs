//! (tests only) The wheel bound's three leaves against the original's code run offline (tools/leaves3_reference.py): random
//! shapes, factors and edge cases, memory laid out as the original holds the objects.

use super::*;
use crate::physics::artic_offline::{diff, records, report, Rec};

const DATA: &str = include_str!("../testdata/leaves3_ref.txt");
const V: u32 = 0x1100;
const EDG: u32 = 0x1400;
const FACE: u32 = 0x1600;

fn fl(r: &Rec, a: u32) -> f32 {
    f32::from_bits(r.mem.word(a))
}
fn v3(r: &Rec, a: u32) -> Vec3 {
    Vec3::new(fl(r, a), fl(r, a + 4), fl(r, a + 8))
}
fn bits(v: Vec3) -> Vec<u32> {
    vec![v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
}
fn corners(r: &Rec) -> Vec<Vec3> {
    (0..8).map(|i| v3(r, V + 16 * i)).collect()
}
fn edge(r: &Rec, a: u32) -> Edge {
    Edge { centre: v3(r, a), normal: v3(r, a + 16), radius: fl(r, a + 32), ends: [r.mem.word(a + 36) as usize, r.mem.word(a + 40) as usize] }
}

/// F-90069 (the vertex loops; polygon count 0 - the polygon part is not in the wheel's bound; a type 10 bound's box is not
/// the wheel's): 200 cases, factors above 0, 0, -0 and below; an axis whose factor is not above 0 stays.
#[test]
fn corners_scale_as_the_original_does() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(DATA, "gv") {
        let nv = r.mem.word(0x1000 + 200) as usize;
        let mut c = corners(&r);
        let (sx, sy) = (f32::from_bits(r.r(14)), f32::from_bits(r.r(15)));
        assert_eq!(r.r(15), r.r(16));
        scale_corners(&mut c[..nv], sx, sy);
        for (i, p) in c[..nv].iter().enumerate() {
            diff(&format!("case {n} corner {i}"), &bits(*p), &r.want(V + 16 * i as u32, 3), true, &mut bad);
        }
        n += 1;
    }
    report("F-90069 corners", n, &bad);
}

/// F-17852: 600 cases (corners already scaled, a centre, an old normal, a radius, the ends; some on a line).
#[test]
fn edges_update_as_the_original_does() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(DATA, "ge") {
        let mut e = edge(&r, 0x1300);
        let (sx, sy) = (f32::from_bits(r.r(14)), f32::from_bits(r.r(15)));
        edge_update(&mut e, &corners(&r), sx, sy);
        diff(&format!("case {n} centre"), &bits(e.centre), &r.want(0x1300, 3), true, &mut bad);
        diff(&format!("case {n} normal"), &bits(e.normal), &r.want(0x1310, 3), true, &mut bad);
        diff(&format!("case {n} radius"), &[e.radius.to_bits()], &r.want(0x1320, 1), true, &mut bad);
        n += 1;
    }
    report("F-17852 edges", n, &bad);
}

/// F-22892 (F-95825 inside): 600 cases, caps and bands, the edge count 1 .. 5.
#[test]
fn faces_update_as_the_original_does() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(DATA, "gf") {
        let es: Vec<Edge> = (0..6).map(|k| edge(&r, EDG + 48 * k)).collect();
        let (w60, w64) = (r.mem.word(FACE + 60), r.mem.word(FACE + 64));
        let mut f = Face {
            cap: (r.mem.word(FACE + 80) >> 8) & 0xFF != 0,
            corner: (r.mem.word(FACE + 4) >> 16) as usize,
            centre: v3(&r, FACE + 16),
            normal: Vec3::ZERO,
            outer: fl(&r, FACE + 48),
            inner: fl(&r, FACE + 52),
            least: fl(&r, FACE + 56),
            edges: [(w60 >> 16) as usize, (w60 & 0xFFFF) as usize, (w64 >> 16) as usize, 0],
            count: r.mem.word(FACE + 76) as usize,
        };
        let (sx, sy) = (f32::from_bits(r.r(14)), f32::from_bits(r.r(15)));
        face_update(&mut f, &es, &corners(&r), sx, sy);
        diff(&format!("case {n} centre"), &bits(f.centre), &r.want(FACE + 16, 3), true, &mut bad);
        diff(&format!("case {n} outer inner least"), &[f.outer.to_bits(), f.inner.to_bits(), f.least.to_bits()], &r.want(FACE + 48, 3), true, &mut bad);
        n += 1;
    }
    report("F-22892 faces", n, &bad);
}
