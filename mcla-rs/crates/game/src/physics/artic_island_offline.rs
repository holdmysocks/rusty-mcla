//! (tests only) The island routines of the articulated body against the original run offline: tools/artic_reference.py
//! writes testdata/artic_island_ref.txt from the generated code unchanged; each test runs the port on the same memory
//! and compares word for word (a NaN only as a NaN). Evidence: docs/reference/vehicle/offline_leaf_checks.md.

use super::*;
use crate::physics::artic_offline::*;

const REF: &str = include_str!("../testdata/artic_island_ref.txt");
const BUF: u32 = 0x3800;
const CHAIN: u32 = 0x3600;
const TABLE: u32 = 0x3700;

/// The bone -> link lookup both F-78078 and F-41014 begin with: the bone inside the collider's bone count (+146 of the
/// object three pointers down from +16), the table (+716) entry not negative and below the body's link count; else 0.
/// The port's callers hold the link already.
fn mapped(r: &Rec) -> usize {
    let m = &r.mem;
    let nb = (m.word(CHAIN + 0x80 + 144) & 0xFFFF) as i32;
    let bone = r.r(4) as i32;
    if bone < 0 || bone >= nb {
        return 0;
    }
    let t = m.word(TABLE + 4 * bone as u32) as i32;
    if t < 0 || t >= m.word(BODY) as i32 {
        return 0;
    }
    t as usize
}

fn link_head(a: &Articulated, i: usize) -> Vec<u32> {
    words_of(&a.links[i].to_guest()[..128])
}

/// F-41014 (the thunk that moves v2, v3 to v1, v2 and calls slot 34) with F-19400 behind it.
#[test]
fn apply_at_point_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF, "am") {
        n += 1;
        let mut a = r.mem.articulated(4, 0);
        a.apply_at_point(mapped(&r), r.v(12), r.v(13));
        for i in 0..4 {
            let got: Vec<u32> = a.links[i].impulse.bits().into_iter().chain(a.links[i].ang_impulse.bits()).collect();
            diff(&format!("case {n} link {i}"), &got, &r.want(LINK0 + LSZ * i as u32 + 704, 8), true, &mut bad);
        }
    }
    report("apply_at_point", n, &bad);
}

/// F-18964: a contact on a part marks the hinge above its link.
#[test]
fn contact_on_part_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF, "cp") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        let nb = (r.mem.word(CHAIN + 0x80 + 144) & 0xFFFF) as usize;
        let map: Vec<i32> = (0..nb).map(|i| r.mem.word(TABLE + 4 * i as u32) as i32).collect();
        a.contact_on_part(&map, r.r(4) as i32);
        for j in 0..3 {
            let got = words_of(&a.joint_to_guest(j))[133];
            diff(&format!("case {n} joint {j}"), &[got], &r.want(JOINT0 + JSZ * j as u32 + 532, 1), false, &mut bad);
        }
    }
    report("contact_on_part", n, &bad);
}

