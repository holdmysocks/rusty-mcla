//! (tests only) F-12436's arithmetic after the xml read, against the original's code run offline (tools/leaves3_reference.py).

use super::*;
use crate::physics::artic_offline::{diff, records, report};

const DATA: &str = include_str!("testdata/leaves3_ref.txt");

/// Eleven tune records of 35 words in each of 12 cases: the three scalars scaled and the BehindDistOffsets shifted exactly
/// as `RubberBand::after_load` does it; every other word of a record stays as read.
#[test]
fn the_records_after_the_read_match_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(DATA, "rb") {
        for i in 0..11u32 {
            let base = 0x1000 + 76 + 140 * i;
            let w = |k: u32| f32::from_bits(r.mem.word(base + 4 * k));
            let mut b = RubberBand {
                nitro_min_dist_from_finish: w(12),
                unlimited_nitro_min_dist_behind_player: w(14),
                home_stretch_distance: w(16),
                behind_offsets: std::array::from_fn(|k| w(27 + k as u32)),
                ..Default::default()
            };
            b.after_load();
            let mut want = r.want(base, 35);
            let got: Vec<u32> = [b.nitro_min_dist_from_finish, b.unlimited_nitro_min_dist_behind_player, b.home_stretch_distance].iter().chain(b.behind_offsets.iter()).map(|x| x.to_bits()).collect();
            let theirs: Vec<u32> = [12usize, 14, 16].iter().chain([27usize, 28, 29, 30, 31, 32, 33, 34].iter()).map(|k| want[*k]).collect();
            diff(&format!("case {n} record {i}"), &got, &theirs, true, &mut bad);
            // (the other words are as the case put them)
            for k in [12usize, 14, 16, 27, 28, 29, 30, 31, 32, 33, 34] {
                want[k] = r.mem.word(base + 4 * k as u32);
            }
            let input: Vec<u32> = (0..35).map(|k| r.mem.word(base + 4 * k)).collect();
            assert_eq!(want, input, "case {n} record {i}: a word the rule does not touch changed");
        }
        n += 1;
    }
    report("F-12436 records", n, &bad);
}
