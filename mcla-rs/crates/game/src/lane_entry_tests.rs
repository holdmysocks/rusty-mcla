//! An ambient car's slot 10 (F-93055: its lane entries this frame) against the original's code run offline
//! (tools/lane_entry_reference.py): 4,000 cases of the record, the active test, the proxy and trailer bytes, the velocity
//! around 6 mph (and 0, -0, denormal, NaN, inf), +500, the behaviour, its rail and the rail's +4 word; the calls the
//! original makes on the record (A slot 11, R slot 14, none) from [`lane_entry_rule`].

use super::*;

#[test]
fn lane_entry_rule_matches_the_original() {
    let mut n = [0; 3];
    for line in include_str!("testdata/lane_entry_ref.txt").lines() {
        let (inp, want) = line.split_once(" -> ").unwrap();
        let w: Vec<&str> = inp.split_whitespace().collect();
        let int = |i: usize| w[i].parse::<u32>().unwrap();
        let f = |i: usize| f32::from_bits(u32::from_str_radix(w[i], 16).unwrap());
        let (rec, active, proxy, trailer, b500, beh, rail, flags) = (int(0) != 0, int(1) != 0, int(2) != 0, int(3) != 0, int(4), int(5) != 0, int(6) != 0, int(7));
        let (x, y, z) = (f(9), f(10), f(11));
        let v2 = z.mul_add(z, x.mul_add(x, y * y));
        // (F-96472: a behaviour with a rail whose +4 & 0x1800 is 0)
        let on_lane = beh && rail && flags & 0x1800 == 0;
        let got = if !rec {
            "-"
        } else {
            match lane_entry_rule(active, proxy, v2, trailer, b500 & 0x10 != 0, beh, on_lane) {
                Some(true) => "A",
                Some(false) => "R",
                None => "-",
            }
        };
        assert_eq!(got, want, "{line}");
        n[["A", "R", "-"].iter().position(|k| *k == want).unwrap()] += 1;
    }
    assert!(n[0] > 2000 && n[1] > 300 && n[2] > 1000, "{n:?}");
}
