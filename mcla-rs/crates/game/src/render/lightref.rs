//! Small lighting routines against the original's own code, run offline (`tools/lighting_small_reference.py`, the
//! recomp's unchanged functions on a flat guest memory; cases in `testdata/lighting_small_reference.txt`).
//! Every result must be the same bits.

use glam::Vec3;

const REF: &str = include_str!("testdata/lighting_small_reference.txt");

fn rows(tag: &str) -> impl Iterator<Item = Vec<u32>> + '_ {
    REF.lines().filter_map(move |l| {
        let mut w = l.split_whitespace();
        (w.next() == Some(tag)).then(|| w.map(|x| u32::from_str_radix(x, 16).unwrap()).collect())
    })
}

fn f(b: u32) -> f32 {
    f32::from_bits(b)
}

/// F-70220: the lamp table's half floats (6,021 values: specials, the city's range, tiny and huge ones).
#[test]
fn half_writer_as_the_original() {
    let mut n = 0;
    for r in rows("HALF") {
        assert_eq!(super::half_cut(f(r[0])) as u32, r[1], "{:e}", f(r[0]));
        n += 1;
    }
    assert!(n > 6000);
}

/// F-47404: the lamp grid's cell score (4,000 cases, lamps on and off, at the centre and up to 60 m off).
#[test]
fn cell_score_as_the_original() {
    let mut n = 0;
    for r in rows("CSCORE") {
        let got = super::cell_score(r[0] != 0, f(r[1]), (f(r[2]), f(r[3])), (f(r[4]), f(r[5])));
        assert_eq!(got.to_bits(), r[6], "{r:08x?}: {got:e} against {:e}", f(r[6]));
        n += 1;
    }
    assert!(n >= 4000);
}

/// F-60154: a light's score at a point (4,000 cases: head-lamp and lamp strengths, cones, the point on the light).
#[test]
fn light_score_as_the_original() {
    let mut n = 0;
    for r in rows("SCORE") {
        let v = |i: usize| Vec3::new(f(r[i]), f(r[i + 1]), f(r[i + 2]));
        let got = crate::figlight::light_score(r[0] != 0, v(1), v(4), f(r[7]), f(r[8]), f(r[9]), v(10));
        assert_eq!(got.to_bits(), r[13], "{r:08x?}: {got:e} against {:e}", f(r[13]));
        n += 1;
    }
    assert!(n >= 4000);
}

/// F-16058 / F-88844 / F-36981: HasTexCoord(i) / HasTangent(i) / HasBinormal(i) = mask bits i + 6, i + 14, i + 15,
/// as the port's `Fvf::has` is called.
#[test]
fn vertex_format_bits_as_the_original() {
    let mut n = 0;
    for r in rows("FVF") {
        let fvf = mcla_formats::rage::geometry::Fvf { mask: r[0], stride: 0, channels: [(mcla_formats::rage::geometry::ChanType::None, 0); 16] };
        let i = r[1] as usize;
        assert_eq!((fvf.has(i + 6) as u32, fvf.has(i + 14) as u32, fvf.has(i + 15) as u32), (r[2], r[3], r[4]), "{r:08x?}");
        n += 1;
    }
    assert!(n >= 590);
}
