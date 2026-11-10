//! (tests only) Second batch of the ragdoll build's routines against the original run offline (verify-artic2):
//! tools/artic2_reference.py writes testdata/artic2_ref.txt from the generated code unchanged, the vector unit's flush mode on
//! as the console has it. Each test builds the port's block from the case's words and calls the function the build runs.
//! Evidence: work/verify_artic2_2026-11-10.md.

use super::*;
use crate::physics::artic_offline::*;

const REF2: &str = include_str!("../testdata/artic2_ref.txt");
const FILL_WORD: u32 = FILL as u32 * 0x0101_0101;

/// F-77258: the hinge constructor on a block of the heap fill (type, links, mode, the zeroed +16 .. +431, limits +-FLT_MAX,
/// angle and rate 0, the drive's defaults 200 / 10 / 0 / 0 / +-1e8). The port's `joint_block` runs the constructor's part and then
/// the build's own writes; for a tune joint that holds the constructor's defaults, every word the constructor wrote is the same
/// (the words it leaves alone, the frames from +432, are the build's).
#[test]
fn hinge_constructor_matches_the_original() {
    let r = records(REF2, "hj").next().unwrap();
    let want = r.want(JOINT0, HINGE_SIZE / 4);
    let max = f32::MAX;
    let j = Joint {
        ty: 0,
        parent: 0,
        k: 0.0,
        mode: 0,
        lim: [-max, max, -max, max, 0.0, 0.0],
        b632: 0,
        b633: 0,
        hd: [0x4348_0000, 0x4120_0000, 0, 0, 0x4cbe_bc20, 0xccbe_bc20].map(f32::from_bits),
        bv: [[0.0; 3]; 6],
        vw: [None; 6],
        fr: [R4([0.0; 4]); 10],
    };
    let addrs = GuestAddrs { links: vec![0], ..Default::default() };
    let got = words_of(&joint_block(&j, &addrs, 0));
    let touched: Vec<usize> = (0..want.len()).filter(|&i| want[i] != FILL_WORD).collect();
    assert!(touched.len() > 14 && touched.contains(&152) && touched.contains(&173), "{} words written", touched.len());
    let mut bad = Vec::new();
    for &i in &touched {
        diff(&format!("word {i} (+{})", 4 * i), &[got[i]], &[want[i]], false, &mut bad);
    }
    report("hinge constructor", touched.len(), &bad);
}

/// F-33001: the rest frame becomes the current frame: +416 = the rest rows and position (+512), +608 = +576, +624 = +640 = +592,
/// +656 = v + c x w. The link constructor (F-60898) has zeroed +576 and +592 in the cases, as the port assumes.
#[test]
fn rest_to_current_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "rc") {
        n += 1;
        let mut b = r.mem.bytes(LINK0, LSZ as usize).to_vec();
        let rest: [V4; 4] = std::array::from_fn(|i| std::array::from_fn(|k| f32::from_bits(r.mem.word(LINK0 + 512 + 16 * i as u32 + 4 * k as u32))));
        rest_to_current(&mut b, &rest);
        let w = words_of(&b);
        diff(&format!("case {n} +416"), &w[104..120], &r.want(LINK0 + 416, 16), true, &mut bad);
        diff(&format!("case {n} +608"), &w[152..168], &r.want(LINK0 + 608, 16), true, &mut bad);
    }
    report("rest_to_current", n, &bad);
}
