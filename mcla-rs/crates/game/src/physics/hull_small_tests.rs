//! The small hull and contact-record routines compared with the original (tools/hull_small_reference.py, tools/
//! hull_wrappers_reference.py): each runs the generated code unchanged on flat guest memory; every line is
//! "<tag> <input words> -> <output words>" (hex; a word that reads 82xxxxxx is written with an h in front).
//! A NaN is compared only as a NaN (the harness's default NaN is not the console's).
use super::*;
use glam::Vec3;

fn words(s: &str) -> Vec<u32> {
    s.split_whitespace().map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap()).collect()
}

fn cases(tag: &'static str) -> impl Iterator<Item = (Vec<u32>, Vec<u32>)> {
    include_str!("../testdata/hull_small_ref.txt").lines().filter_map(move |l| {
        let rest = l.strip_prefix(tag)?.strip_prefix(' ')?;
        let (i, o) = rest.split_once(" -> ")?;
        Some((words(i), words(o)))
    })
}

fn same(a: u32, b: u32) -> bool {
    a == b || (f32::from_bits(a).is_nan() && f32::from_bits(b).is_nan())
}

fn all_same(got: &[u32], want: &[u32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}");
    for (k, (&g, &w)) in got.iter().zip(want).enumerate() {
        assert!(same(g, w), "{what}: word {k}: {g:08x} against {w:08x}");
    }
}

fn vec4(w: &[u32]) -> [f32; 4] {
    [f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2]), f32::from_bits(w[3])]
}

/// F-67839: the centroid setter. The vector lands at +48 and byte +5 says whether x, y or z is not zero.
#[test]
fn centroid_flag_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("centroid") {
        let v = Vec3::new(f32::from_bits(i[0]), f32::from_bits(i[1]), f32::from_bits(i[2]));
        // (nothing but byte +5 and the vector changes)
        let mut want = i[4..20].to_vec();
        want[1] = (want[1] & 0xFF00_FFFF) | ((has_centroid(v) as u32) << 16);
        want[12..16].copy_from_slice(&i[..4]);
        all_same(&o, &want, "centroid");
        n += 1;
    }
    assert_eq!(n, 4000);
}

/// F-51639, F-34223, F-86309: reset, construct and copy a contact record.
#[test]
fn contact_records_match_the_original() {
    let (mut nr, mut nc, mut nk) = (0, 0, 0);
    for (i, o) in cases("reset") {
        let mut r: record::Record = i[..44].try_into().unwrap();
        record::reset(&mut r);
        let mut want = r.to_vec();
        want.extend_from_slice(&i[44..48]);
        all_same(&o, &want, "reset");
        nr += 1;
    }
    for (i, o) in cases("ctor") {
        let v: [[f32; 4]; 6] = std::array::from_fn(|k| vec4(&i[4 * k..4 * k + 4]));
        let mut r: record::Record = i[26..26 + 44].try_into().unwrap();
        record::new(&mut r, v, (i[24], i[25]));
        let mut want = r.to_vec();
        want.extend_from_slice(&i[26 + 44..26 + 48]);
        all_same(&o, &want, "ctor");
        nc += 1;
    }
    for (i, o) in cases("copy") {
        let mut dst: record::Record = i[..44].try_into().unwrap();
        let src: record::Record = i[48..48 + 44].try_into().unwrap();
        record::copy(&mut dst, &src);
        let mut want = dst.to_vec();
        want.extend_from_slice(&i[44..48]);
        all_same(&o, &want, "copy");
        nk += 1;
    }
    assert_eq!((nr, nc, nk), (2000, 4000, 3000));
}

/// F-15933: the detector's constructor stores its first direction (0, 0, 1, 1) at +16 and the four arguments at +32 ..
/// +44 (r7, r6, r4, r5); the vtable word is the same every time.
#[test]
fn detector_constructor_matches_the_original() {
    let mut n = 0;
    let mut vtable = None;
    for (i, o) in cases("detector") {
        assert_eq!(*vtable.get_or_insert(o[0]), o[0]);
        let d = super::super::gjk::FIRST_DIRECTION;
        let mut want = o.clone();
        want[1..4].copy_from_slice(&i[5..8]);
        want[4..8].copy_from_slice(&[d.x.to_bits(), d.y.to_bits(), d.z.to_bits(), 1.0f32.to_bits()]);
        want[8..12].copy_from_slice(&[i[3], i[2], i[0], i[1]]);
        want[12..16].copy_from_slice(&i[4 + 12..4 + 16]);
        assert_eq!(o, want);
        n += 1;
    }
    assert_eq!(n, 1500);
}

/// F-12574: mcCarFluid's in-bit is bit 0 of +12 (`Fluid::inside`).
#[test]
fn in_water_bit_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("water") {
        let f = super::super::fluid::Fluid { inside: i[0] & 1 != 0, ..Default::default() };
        assert_eq!(f.inside as u32, o[0], "{:08x}", i[0]);
        n += 1;
    }
    assert_eq!(n, 300);
}

/// F-61340: the damage multiplier against a vehicle or anything else.
#[test]
fn blow_factor_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("blow") {
        let (own12, own16) = (f32::from_bits(i[0]), f32::from_bits(i[1]));
        let against_vehicle = i[2] >= 2 && i[3] & 0xC != 0;
        let got = super::super::loose::blow_for(against_vehicle.then(|| f32::from_bits(i[4])), [own12, own16]);
        assert!(same(got.to_bits(), o[0]), "blow {i:x?}: {:08x} against {:08x}", got.to_bits(), o[0]);
        n += 1;
    }
    assert_eq!(n, 3000);
}

/// F-17693: the polyhedron's material id of a polygon.
#[test]
fn polygon_material_matches_the_original() {
    use mcla_formats::rage::bound::polygon_material_of;
    let mut n = 0;
    let mut nonzero = 0;
    for (i, o) in cases("material") {
        let table = &i[4..12];
        let bytes: Vec<u8> = i[12..12 + 32].iter().map(|&b| b as u8).collect();
        let (polygons, default, count, index) = (i[0] as i32, i[1] as i32, i[2] as u8, i[3] as i32);
        let got = polygon_material_of(index, polygons, &bytes[..polygons as usize], default, count, |m| table[m]);
        assert_eq!(got, o[0], "{i:x?}");
        nonzero += (got != 0) as u32;
        n += 1;
    }
    assert_eq!(n, 4000);
    assert!(nonzero > 500);
}

/// F-26668 (shape A) / F-11678 (shape B): the element of a simplex's corners by the bound's type, with the dedupe F-37693
/// and the triangle F-48088 run along (tools/hull_wrappers_reference.py): every car's hull as filed and moved a little,
/// 0 .. 5 corners with repeats, bound types 4 / 13 / 0 / 5 / 12, directions random and along a face - every answer equal.
#[test]
fn simplex_element_matches_the_original() {
    let h = |x: &str| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap();
    let mut hull: Option<HullShape> = None;
    let mut n = 0;
    let mut by_kind = [0u32; 14];
    for line in include_str!("../testdata/hull_wrappers_ref.txt").lines() {
        if let Some(rest) = line.strip_prefix("H ") {
            let parts: Vec<&str> = rest.split(" | ").collect();
            let vs: Vec<u32> = parts[1].split(' ').map(h).collect();
            let ps: Vec<u32> = parts[2].split(' ').map(h).collect();
            let verts: Vec<Vec3> = vs.chunks(4).map(|c| Vec3::new(f32::from_bits(c[0]), f32::from_bits(c[1]), f32::from_bits(c[2]))).collect();
            let tris: Vec<[u16; 3]> = ps.chunks(4).map(|c| [(c[1] >> 16) as u16, c[1] as u16, (c[2] >> 16) as u16]).collect();
            let mut s = HullShape::new(verts, &tris);
            s.areas = ps.chunks(4).map(|c| f32::from_bits(c[0])).collect();
            s.beside = ps.chunks(4).map(|c| [c[2] as u16, (c[3] >> 16) as u16, c[3] as u16]).collect();
            hull = Some(s);
            continue;
        }
        let (case, want) = line.strip_prefix("C ").unwrap().split_once(" -> ").unwrap();
        let parts: Vec<&str> = case.split(" | ").collect();
        let head: Vec<u32> = parts[0].split(' ').map(h).collect();
        let a: Vec<u32> = parts[1].split(' ').filter(|x| !x.is_empty()).map(h).collect();
        let b: Vec<u32> = parts[2].split(' ').filter(|x| !x.is_empty()).map(h).collect();
        let d: Vec<f32> = parts[3].split(' ').map(|x| f32::from_bits(h(x))).collect();
        let dir = Vec3::new(d[0], d[1], d[2]);
        let want: Vec<u32> = want.split(' ').map(|x| x.parse().unwrap()).collect();
        let s = hull.as_mut().unwrap();
        s.mat_count = Some(head[2] as u8);
        let (kind, own) = (head[0] as u8, head[1] as u16);
        assert_eq!(simplex_element(kind, own, Some(s), &a, dir) as u32, want[0], "A {line}");
        assert_eq!(simplex_element(kind, own, Some(s), &b, dir) as u32, want[1], "B {line}");
        by_kind[kind as usize] += 1;
        n += 1;
    }
    assert_eq!(n, 7800);
    assert!(by_kind[4] > 3000 && by_kind[13] > 500);
}
