//! (tests only) The articulated body's small routines against the original run offline: tools/artic_reference.py writes
//! testdata/artic_ref.txt from the generated code unchanged; each test runs the port on the same memory and compares word
//! for word (a NaN only as a NaN). Evidence: docs/reference/vehicle/offline_leaf_checks.md.

use super::*;
use crate::physics::artic_offline::*;

const REF: &str = include_str!("../testdata/artic_ref.txt");
const BUF: u32 = 0x3800;

/// F-63136: the body's part of the reset (links' +672 .. +735, +880 .. +911, +976 .. +1007; joints' +368 .. +399). The
/// port's `reset` does the rest of the chain too (velocities, the collider's accumulators); only the words this routine
/// writes are compared.
#[test]
fn reset_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF, "rs") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        a.reset();
        for i in 0..4u32 {
            let g = words_of(&a.links[i as usize].to_guest());
            let at = LINK0 + LSZ * i;
            diff(&format!("case {n} link {i} +672"), &g[168..184], &r.want(at + 672, 16), false, &mut bad);
            diff(&format!("case {n} link {i} +880"), &g[220..228], &r.want(at + 880, 8), false, &mut bad);
            diff(&format!("case {n} link {i} +976"), &g[244..252], &r.want(at + 976, 8), false, &mut bad);
        }
        for j in 0..3u32 {
            let g = words_of(&a.joint_to_guest(j as usize));
            diff(&format!("case {n} joint {j}"), &g[92..100], &r.want(JOINT0 + JSZ * j + 368, 8), false, &mut bad);
        }
    }
    report("reset (the body's share)", n, &bad);
}

