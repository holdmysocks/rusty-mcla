//! (tests only) The ragdoll build's constructors and the rest-frame copy against the original run offline:
//! tools/artic_reference.py writes testdata/artic_build_ref.txt from the generated code unchanged (the library memset
//! replaced by a byte fill); the constructors run on a block of the heap fill, and the words they write are compared with
//! the port's blocks. Evidence: docs/reference/vehicle/offline_leaf_checks.md.

use super::*;
use crate::physics::artic_offline::*;

const REF: &str = include_str!("../testdata/artic_build_ref.txt");
const FILL_WORD: u32 = FILL as u32 * 0x0101_0101;

fn words(b: &[u8]) -> Vec<u32> {
    words_of(b)
}

/// F-15442: the base joint constructor, a whole block (a hinge's and a swing-twist's size).
#[test]
fn joint_base_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF, "jb") {
        n += 1;
        let size = r.outs[0].1.len() * 4;
        diff(&format!("size {size}"), &words(&joint_base(size)), &r.want(JOINT0, size / 4), false, &mut bad);
    }
    assert_eq!(n, 2);
    report("joint_base", n, &bad);
}

/// F-60898: the link constructor zeroes 22 vectors and two words and leaves the rest of the block alone; the port's
/// block holds a zero at each of those words (it sets the other fields after).
#[test]
fn link_constructor_matches_the_original() {
    let r = records(REF, "lb").next().unwrap();
    let want = r.want(LINK0, 252);
    let touched: Vec<usize> = (0..252).filter(|&i| want[i] != FILL_WORD).collect();
    assert_eq!(touched.len(), 22 * 4 + 2);
    assert!(touched.iter().all(|&i| want[i] == 0), "the constructor writes zeros only");
    for root in [false, true] {
        let l = Link { child: 0, root, mass: 70.0, moments: [1.0, 2.0, 3.0, 0.0], rest: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.5, 0.25, 0.0]] };
        let got = words(&link_block(&l));
        for &i in &touched {
            // (a zero of either sign: the rest-frame copy after the constructor writes -0 at +656, as the original does)
            assert_eq!(got[i] << 1, 0, "root {root}: word {i} (+{})", 4 * i);
        }
    }
}

