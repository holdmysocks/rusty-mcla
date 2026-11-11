//! (tests only) F-14839 (the two 12-node curves built from the tune) with the spline builder F-85496 behind its virtual call,
//! against the original's code run offline (tools/leaves3_reference.py): the torque curve from the tune's integer controls
//! (padded with zeros), the idle curve (padded with its last control). The builder writes four vectors a b c d to a segment; the
//! port keeps the y lane (torque) of each.

use super::*;
use crate::physics::artic_offline::{diff, records, report, Rec};

const DATA: &str = include_str!("../testdata/leaves3_ref.txt");
const E: u32 = 0x1000;
const TUNE: u32 = 0x1440;

fn fl(r: &Rec, a: u32) -> f32 {
    f32::from_bits(r.mem.word(a))
}

#[test]
fn the_curves_match_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(DATA, "ec") {
        let count = r.mem.word(TUNE + 52) as usize;
        let ctl = |at: u32| -> Vec<f32> { (0..count as u32).map(|i| r.mem.word(at + 4 * i) as i32 as f32).collect() };
        let (idle, max) = (fl(&r, E + 76), fl(&r, E + 68));
        for (name, base, curve) in [("torque", 0x4000u32, Curve::new(&ctl(TUNE + 56), idle, max, false)), ("idle", 0x4300, Curve::new(&ctl(TUNE + 100), idle, max, true))] {
            for seg in 0..12u32 {
                let mine: Vec<u32> = curve.coefficients[seg as usize].iter().map(|x| x.to_bits()).collect();
                let theirs: Vec<u32> = (0..4u32).map(|k| r.want(base + 64 * seg + 16 * k + 4, 1)[0]).collect();
                diff(&format!("case {n} {name} segment {seg}"), &mine, &theirs, true, &mut bad);
            }
        }
        n += 1;
    }
    report("F-14839 curves", n, &bad);
}
