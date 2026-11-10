//! The shape and polygon helpers of the GJK code compared with the original (tools/leaves_reference.py): each runs the
//! generated code unchanged on flat guest memory; every line of testdata/leaves_ref.txt is "<tag> <input words> -> <output
//! words>" (hex; a word that reads 82xxxxxx is written with an h in front).
use super::{edge_bisector, polygon_normal};
use glam::Vec3;

fn words(s: &str) -> Vec<u32> {
    s.split_whitespace().map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap()).collect()
}

fn cases(tag: &'static str) -> impl Iterator<Item = (Vec<u32>, Vec<u32>)> {
    include_str!("../testdata/leaves_ref.txt").lines().filter_map(move |l| {
        let rest = l.strip_prefix(tag)?.strip_prefix(' ')?;
        let (i, o) = rest.split_once(" -> ")?;
        Some((words(i), words(o)))
    })
}

fn f(w: u32) -> f32 {
    f32::from_bits(w)
}

fn same(a: u32, b: u32) -> bool {
    a == b || (f(a).is_nan() && f(b).is_nan())
}

fn v3(w: &[u32], at: usize) -> Vec3 {
    Vec3::new(f(w[at]), f(w[at + 1]), f(w[at + 2]))
}

fn expect(got: Vec3, o: &[u32], at: usize, what: &str) {
    for (k, g) in [got.x, got.y, got.z].into_iter().enumerate() {
        assert!(same(g.to_bits(), o[at + k]), "{what}: lane {k}: {:08x} against {:08x}", g.to_bits(), o[at + k]);
    }
}

/// F-23797: the direction between a polygon's normal and its neighbour's across an edge. The case words: the edge index,
/// the bit its answer is set in, the neighbour's u16 (0xFFFF none), the mesh's polygon count, the polygon's normal (4),
/// the edge's two ends (4 each), the neighbour's area, its three vertex indices (the records at +8, +6, +4), the eight
/// vertices (4 words each) the fetch answers from. The answer is the flag (1 << bit when the polygons meet in a ridge),
/// the direction (the sum made unit in a ridge, the polygon's own normal otherwise) and the second vector (the neighbour's
/// normal in a ridge, the polygon's own otherwise). A neighbour past the mesh's count is the constant (0, 0, 1).
#[test]
fn edge_bisector_matches_the_original() {
    let (mut none, mut past, mut ridge, mut flat) = (0, 0, 0, 0);
    let mut bad = [0u32; 3];
    for (i, o) in cases("edgebis") {
        let what = format!("{i:08x?}");
        let (bit, nbr, count, area) = (i[1], i[2], i[3], f(i[16]));
        let (face, from, to) = (v3(&i, 4), v3(&i, 8), v3(&i, 12));
        let vertex = |k: usize| v3(&i, 20 + 4 * (i[k] as usize & 7));
        let (flag, b, m);
        if nbr == 0xFFFF {
            none += 1;
            (flag, b, m) = (0, face, face);
        } else {
            let other = if nbr as usize >= count as usize {
                past += 1;
                Vec3::new(0.0, 0.0, 1.0)
            } else {
                // (the record's vertices at +4, +6, +8 are the triangle's first, second and third corner)
                polygon_normal(&[vertex(19), vertex(18), vertex(17)], area)
            };
            let (dir, is_ridge) = edge_bisector(face, other, from, to);
            if is_ridge {
                ridge += 1;
                (flag, b, m) = (1u32 << bit, dir, other);
            } else {
                flat += 1;
                (flag, b, m) = (1u32 << bit, dir, face);
            }
        }
        let ok = flag == o[0] && [b.x, b.y, b.z].iter().enumerate().all(|(k, g)| same(g.to_bits(), o[1 + k])) && [m.x, m.y, m.z].iter().enumerate().all(|(k, g)| same(g.to_bits(), o[5 + k]));
        if !ok {
            let kind = if nbr == 0xFFFF { 0 } else if nbr as usize >= count as usize { 1 } else { 2 };
            bad[kind] += 1;
            if bad[kind] == 1 {
                eprintln!("first mismatch kind {kind}: {what}
 got flag {flag} {b:?} {m:?}
 want {:08x?}", &o);
            }
        }
    }
    assert_eq!(bad, [0, 0, 0], "mismatches by kind (none, past the count, in range)");
    assert!(none >= 100 && past >= 100 && ridge >= 100 && flat >= 100, "{none} {past} {ridge} {flat}");
}
