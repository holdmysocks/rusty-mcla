//! (tests only) F-82909's spring arithmetic against the original's code run offline (tools/leaves3_reference.py).

use super::*;
use crate::physics::artic_offline::{diff, records, report};

const DATA: &str = include_str!("testdata/leaves3_ref.txt");

/// 500 cases (half in the shipped files' range, half wide, some tune values 0 and -0 so the quotients are infinite or NaN):
/// the four spring numbers and the two centre-of-gravity products.
#[test]
fn the_ambient_springs_match_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(DATA, "am") {
        let f = |a: u32| f32::from_bits(r.mem.word(a));
        let (mass, gravity) = (f(0x3A00), f(0x3A04));
        let t = |k: u32| f(0x1000 + 196 + 4 * k);
        let (cog_sim, cog_off) = (t(0), t(1));
        let y = f(0x1300 + 48 + 4);
        let got: Vec<u32> = [cog_off * y, cog_sim * y]
            .into_iter()
            .chain(amb_springs(mass, gravity, t(3), t(4), t(5), t(6), t(7), t(8)))
            .map(|x| x.to_bits())
            .collect();
        diff(&format!("case {n}"), &got, &r.want(0x1000 + 236, 6), true, &mut bad);
        n += 1;
    }
    report("F-82909 springs", n, &bad);
}
