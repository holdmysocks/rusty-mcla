//! Small map routines against the original's own generated code run offline (tools/physics_math_reference.py, recomp_dispatch;
//! recorded output in physics/testdata/physics_math_ref.txt). Evidence: docs/reference/minimap/verify_dispatch_evidence.md.
use super::*;

fn w(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap()
}
fn f(s: &str) -> f32 {
    f32::from_bits(w(s))
}

fn lines(op: &'static str) -> impl Iterator<Item = (Vec<&'static str>, Vec<&'static str>)> {
    include_str!("physics/testdata/physics_math_ref.txt").lines().filter_map(move |l| {
        let (a, o) = l.split_once(" -> ")?;
        let mut it = a.split(' ');
        if it.next()? != op {
            return None;
        }
        Some((it.collect(), o.split(' ').collect()))
    })
}

#[test]
fn heading_matches_the_original() {
    let (mut n, mut bad) = (0, 0);
    for (i, o) in lines("HEAD") {
        let h = heading(Vec3::new(f(i[0]), f(i[1]), f(i[2])));
        n += 1;
        if h.to_bits() != w(o[0]) {
            bad += 1;
            if bad < 4 {
                eprintln!("heading {i:?}: port {:08x} original {}", h.to_bits(), o[0]);
            }
        }
    }
    eprintln!("heading: {bad} of {n} differ");
    assert_eq!(bad, 0);
}

#[test]
fn roll_matches_the_original() {
    let (mut n, mut bad) = (0, 0);
    for (i, o) in lines("ROLL") {
        let v: Vec<f32> = i.iter().map(|s| f(s)).collect();
        let r = CamRows { r0: Vec3::new(v[0], v[1], v[2]), r1: Vec3::new(v[3], v[4], v[5]), r2: Vec3::Z, eye: Vec3::ZERO }.roll(v[6]);
        let got: Vec<u32> = [r.r0.x, r.r0.y, r.r0.z, r.r1.x, r.r1.y, r.r1.z].iter().map(|x| x.to_bits()).collect();
        let want: Vec<u32> = o.iter().map(|s| w(s)).collect();
        n += 1;
        if got != want {
            bad += 1;
            if bad < 4 {
                eprintln!("roll {i:?}: port {got:08x?} original {want:08x?}");
            }
        }
    }
    eprintln!("roll: {bad} of {n} differ");
    assert_eq!(bad, 0);
}
