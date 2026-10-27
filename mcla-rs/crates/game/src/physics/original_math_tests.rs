//! Small physics routines against the original's own generated code run offline (tools/physics_math_reference.py,
//! recomp_dispatch; recorded output in testdata/physics_math_ref.txt). Evidence: docs/reference/vehicle/verify_dispatch_evidence.md.
use super::exact::*;
use glam::Vec3;

fn w(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap()
}
fn f(s: &str) -> f32 {
    f32::from_bits(w(s))
}
fn rows_bits(r: &[Vec3; 3]) -> Vec<u32> {
    r.iter().flat_map(|v| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]).collect()
}

fn lines(op: &'static str) -> impl Iterator<Item = (Vec<&'static str>, Vec<&'static str>)> {
    include_str!("testdata/physics_math_ref.txt").lines().filter_map(move |l| {
        let (a, o) = l.split_once(" -> ")?;
        let mut it = a.split(' ');
        if it.next()? != op {
            return None;
        }
        Some((it.collect(), o.split(' ').collect()))
    })
}

#[test]
fn rotation_about_x_and_z_match_the_original() {
    for (op, axis) in [("ROTX", Vec3::X), ("ROTZ", Vec3::Z)] {
        let mut n = 0;
        for (i, o) in lines(op) {
            let ang = f(i[0]);
            let want: Vec<u32> = o.iter().map(|s| w(s)).collect();
            assert_eq!(rows_bits(&rotation_about(axis, ang)), want, "{op} {ang}");
            n += 1;
        }
        assert!(n >= 300, "{op}: {n} cases");
    }
}

#[test]
fn turned_matches_the_original() {
    let mut n = 0;
    for (i, o) in lines("TURN") {
        let v: Vec<f32> = i.iter().map(|s| f(s)).collect();
        let m = [Vec3::new(v[0], v[1], v[2]), Vec3::new(v[4], v[5], v[6]), Vec3::new(v[8], v[9], v[10])];
        let (ax, an) = (Vec3::new(v[12], v[13], v[14]), v[15]);
        let got = turned(&m, &rotation_about_unit(ax, an));
        let want: Vec<u32> = o.iter().map(|s| w(s)).collect();
        assert_eq!(rows_bits(&got), want, "case {n}");
        n += 1;
    }
    assert!(n >= 700, "{n}");
}

#[test]
fn frame_inverse_matches_the_original() {
    let (mut n, mut some, mut none) = (0, 0, 0);
    for op in ["INV", "INVW"] {
        for (i, o) in lines(op) {
            let v: Vec<f32> = i.iter().map(|s| f(s)).collect();
            let m = [Vec3::new(v[0], v[1], v[2]), Vec3::new(v[4], v[5], v[6]), Vec3::new(v[8], v[9], v[10])];
            let t = Vec3::new(v[12], v[13], v[14]);
            let got = inverse_frame(&m, t);
            let ok = w(o[0]);
            n += 1;
            match got {
                None => {
                    // (the plain routine answers 0; the wrapper, working in place, has nothing to say and leaves the rows as they were)
                    if op == "INV" {
                        assert_eq!(ok, 0, "case {n}: the original inverts, the port does not");
                    } else {
                        let same: Vec<u32> = i.iter().map(|s| w(s)).collect();
                        let out: Vec<u32> = o[1..].iter().map(|s| w(s)).collect();
                        assert_eq!(same, out, "case {n}: the wrapper changed what the port refuses");
                    }
                    none += 1;
                }
                Some(r) => {
                    assert_eq!(ok, 1, "case {n}: the port inverts, the original does not");
                    if op == "INVW" {
                        let same: Vec<u32> = i.iter().map(|s| w(s)).collect();
                        let out: Vec<u32> = o[1..].iter().map(|s| w(s)).collect();
                        assert_ne!(same, out, "case {n}: the wrapper left its rows");
                    }
                    let want: Vec<u32> = [0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14].iter().map(|k| w(o[1 + k])).collect();
                    let g: Vec<u32> = r.iter().flat_map(|v| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]).collect();
                    assert_eq!(g, want, "case {n}");
                    some += 1;
                }
            }
        }
    }
    assert!(some > 1000 && none > 100, "{some} inverted, {none} refused, {n} cases");
}

#[test]
fn sss_factor_matches_the_original() {
    let mut n = 0;
    for (i, o) in lines("SSS") {
        let mut s = mcla_formats::rage::simtune::SimScalars::default();
        s.sss_threshold_min = f(i[0]);
        s.sss_threshold_max = f(i[1]);
        s.sss_value = f(i[2]);
        let got = super::vehsim::sss_factor(&s, f(i[3]));
        assert_eq!(got.to_bits(), w(o[0]), "case {n} {i:?}");
        n += 1;
    }
    assert!(n >= 1400, "{n}");
}

#[test]
fn perlin_noise_matches_the_original() {
    let mut n = 0;
    for (i, o) in lines("PERL") {
        let x = f(i[0]);
        assert_eq!(crate::damageparts::perlin(x).to_bits(), w(o[0]), "x = {x}");
        n += 1;
    }
    assert!(n >= 2900, "{n}");
}

/// The damage share (F-10364): the original's two `fsel`s and the C library's pow against `exact::damage_share`.
#[test]
fn damage_share_matches_the_original() {
    let mut n = 0;
    for (i, o) in lines("DMG") {
        let (mx, med, dm) = (f(i[0]), f(i[1]), f(i[2]));
        assert_eq!(damage_share(dm, med, mx).to_bits(), w(o[0]), "case {n}");
        n += 1;
    }
    assert!(n >= 1900, "{n}");
}

/// The player input's speed factor (F-40620).
#[test]
fn input_speed_factor_matches_the_original() {
    let mut n = 0;
    for (i, o) in lines("SSP") {
        let mut t = mcla_formats::rage::simtune::InputTune::default();
        t.sss_threshold = f(i[0]);
        t.sss_value = f(i[1]);
        assert_eq!(super::pad::speed_factor(&t, f(i[2])).to_bits(), w(o[0]), "case {n} {i:?}");
        n += 1;
    }
    assert!(n >= 1400, "{n}");
}
