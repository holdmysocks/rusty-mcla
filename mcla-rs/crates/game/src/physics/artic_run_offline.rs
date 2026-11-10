//! (tests only) The contact iterator's answer against the original run offline: tools/artic_reference.py writes
//! testdata/artic_run_ref.txt from the generated code unchanged. Evidence: docs/reference/vehicle/offline_leaf_checks.md.

use super::*;
use crate::physics::artic_offline::*;

const REF: &str = include_str!("../testdata/artic_run_ref.txt");
const BUF: u32 = 0x3800;

/// F-78981: the iterator's record (+0 the row list, +4 the run's instance, +8 the index); the answer is the other side of
/// the current row. The port picks it by the list the run is in (`as_a`: the list sorted by A); the cases are the rows that
/// belong to the run (A or B is the run's instance), 0 the city.
#[test]
fn other_instance_matches_the_original() {
    let (mut n, mut bad, mut self_rows) = (0, Vec::new(), 0);
    for r in records(REF, "ol") {
        let m = &r.mem;
        let list = m.word(BUF);
        let run = m.word(BUF + 4);
        let entry = m.word(list + 4 * m.word(BUF + 8));
        let pair = m.word(entry + 160);
        let (a, b) = (m.word(pair), m.word(pair + 4));
        let side = |x: u32| if x == 0 { None } else { Some(x as i64) };
        let w = WalkPair { a: side(a), b: side(b), key_a: 0, key_b: 0, rows: 1 };
        for as_a in [true, false] {
            // the row is in this run's list: the run's instance is its A side (the A list) or its B side (the B list)
            if (if as_a { a } else { b }) != run {
                continue;
            }
            n += 1;
            self_rows += (a == b) as usize;
            diff(&format!("case {n} as_a {as_a}"), &[other_level(&w, as_a) as u32], &[r.ret], false, &mut bad);
        }
    }
    assert!(n > 500 && self_rows > 50, "{n} rows, {self_rows} self contacts");
    report("other_level", n, &bad);
}
