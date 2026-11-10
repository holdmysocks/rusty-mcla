//! (tests only) Second batch of the island routines of the articulated body against the original run offline
//! (verify-artic2): tools/artic2_reference.py writes testdata/artic2_ref.txt from the generated code unchanged, the vector
//! unit's flush mode on as the console has it. Each test builds the port's own structs from the case's words and calls the
//! function the game runs; words compared one by one (a NaN only as a NaN). Evidence: work/verify_artic2_2026-11-10.md.

use super::*;
use crate::physics::artic_offline::*;

const REF2: &str = include_str!("../testdata/artic2_ref.txt");
const BUF: u32 = 0x3800;
const CHAIN: u32 = 0x3600;
const TABLE: u32 = 0x3700;

/// The bone -> link lookup the bone thunks begin with (the port's callers hold the link already).
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

/// Joint 0 of the case against links 0 and 1 (the single-joint cases leave the body header empty).
fn pair_body(r: &Rec) -> Articulated {
    let m = &r.mem;
    let mut body = vec![0u8; 528];
    body[0..4].copy_from_slice(&2u32.to_be_bytes());
    body[4..8].copy_from_slice(&LINK0.to_be_bytes());
    body[8..12].copy_from_slice(&(LINK0 + LSZ).to_be_bytes());
    let links = [(LINK0, m.bytes(LINK0, LSZ as usize)), (LINK0 + LSZ, m.bytes(LINK0 + LSZ, LSZ as usize))];
    Articulated::from_guest(m.bytes(COLL, 752), &body, &links, &[m.bytes(JOINT0, JSZ as usize)])
}

/// The island scratch the original leaves: the links' +0 .. +127, the joints' +304 and +336 pairs, the flags and the pivot.
fn check_scratch(a: &Articulated, r: &Rec, n: usize, nj: usize, name: &str, bad: &mut Vec<String>) {
    for i in 0..n {
        let got = words_of(&a.links[i].to_guest()[..128]);
        diff(&format!("{name} link {i}"), &got, &r.want(LINK0 + LSZ * i as u32, 32), true, bad);
    }
    for j in 0..nj {
        let g = words_of(&a.joint_to_guest(j));
        diff(&format!("{name} joint {j}"), &g[76..92], &r.want(JOINT0 + JSZ * j as u32 + 304, 16), true, bad);
    }
    diff(&format!("{name} flags"), &a.flags, &r.want(BODY + 384, n), false, bad);
    diff(&format!("{name} pivot"), &[a.pivot as u32], &r.want(BODY + 512, 1), false, bad);
}

/// F-78078 (the bone thunk) with F-67069 behind it: what an impulse at a point does to the point's velocity, three vectors.
#[test]
fn point_response_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "pr") {
        n += 1;
        let a = r.mem.articulated(4, 0);
        let got = a.point_response(mapped(&r), r.v(11));
        let got: Vec<u32> = got.iter().flat_map(|v| v.bits()).collect();
        diff(&format!("case {n}"), &got, &r.want(0x3900, 12), true, &mut bad);
    }
    report("point_response", n, &bad);
}

/// F-96841: an impulse at r with j on link k into the island scratch (the pivot walked on the way).
#[test]
fn point_impulse_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "pi") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        let (rr, j) = (r.mem.vec(BUF), r.mem.vec(BUF + 16));
        a.point_impulse(r.r(4) as usize, rr, j);
        check_scratch(&a, &r, 4, 3, &format!("case {n}"), &mut bad);
    }
    report("point_impulse", n, &bad);
}

/// F-48604 with F-10668 and the pivot walk behind it: a force pair through a link's inverse into the scratch; the new pivot.
#[test]
fn add_response_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "ar") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        let pair = [r.mem.vec(BUF), r.mem.vec(BUF + 16)];
        let got = a.add_response(r.r(4) as usize, &pair, r.r(6) as i32);
        // (the original stores the new pivot in the caller: body +512 is as it was; the port's caller does the same)
        check_scratch(&a, &r, 4, 3, &format!("case {n}"), &mut bad);
        diff(&format!("case {n} ret"), &[got as u32], &[r.ret], false, &mut bad);
    }
    report("add_response", n, &bad);
}

/// F-79699: the swing-twist joint's hard cone test: +636 the limit along n, +652 the swing rate, true when active.
#[test]
fn ball_cone_hard_matches_the_original() {
    let (mut n, mut bad, mut active) = (0, Vec::new(), 0);
    for r in records(REF2, "bc") {
        n += 1;
        let mut a = pair_body(&r);
        let on = a.ball_cone_hard(0);
        active += on as usize;
        let g = words_of(&a.joint_to_guest(0));
        diff(&format!("case {n} limit"), &g[159..160], &r.want(JOINT0 + 636, 1), true, &mut bad);
        diff(&format!("case {n} rate"), &g[163..164], &r.want(JOINT0 + 652, 1), true, &mut bad);
        diff(&format!("case {n} ret"), &[on as u32], &[r.ret & 0xFF], false, &mut bad);
    }
    assert!(active > 20 && active < n, "{active} active of {n}");
    report("ball_cone_hard", n, &bad);
}

/// F-47802: the hinge's row (kind, position, rate, limit), negated against the lower limit when +632 is -1.
#[test]
fn hinge_dof_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "hd") {
        n += 1;
        let a = pair_body(&r);
        let (k, pos, rate, lim) = a.hinge_dof(0);
        diff(&format!("case {n}"), &[k, pos.to_bits(), rate.to_bits(), lim.to_bits()], &r.want(BUF, 4), true, &mut bad);
    }
    report("hinge_dof", n, &bad);
}

/// F-50660: the velocity of the point at r of link k as the sweep has moved it (the pivot walked to the link first).
#[test]
fn point_velocity_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "pv") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        let v = a.point_velocity(r.r(4) as usize, r.mem.vec(BUF));
        diff(&format!("case {n} velocity"), &v.bits(), &r.want(0x3900, 4), true, &mut bad);
        check_scratch(&a, &r, 4, 3, &format!("case {n}"), &mut bad);
    }
    report("point_velocity", n, &bad);
}

/// A row of joint `j` as the original's callers hand it over: the joint and the row's kind (the other fields are not read).
fn dof(j: u32, kind: u32) -> LimitDof {
    LimitDof { joint: j as usize, kind, depth: 0.0, rate: 0.0 }
}

/// F-94941: the row's velocity as the sweep has moved it (the pivot walked to the joint's child, the axis through the joint's type).
#[test]
fn row_velocity_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "rv") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        let v = a.row_velocity(&dof(r.r(5), r.r(6)));
        diff(&format!("case {n} velocity"), &v.bits(), &r.want(0x3900, 4), true, &mut bad);
        check_scratch(&a, &r, 4, 3, &format!("case {n}"), &mut bad);
    }
    report("row_velocity", n, &bad);
}

/// F-17591: an impulse s along the row, minus on the parent and plus on the child, each through its own inverse.
#[test]
fn row_impulse_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "ri") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        a.row_impulse(&dof(r.r(4), r.r(5)), f32::from_bits(r.reg(14)[0]));
        check_scratch(&a, &r, 4, 3, &format!("case {n}"), &mut bad);
    }
    report("row_impulse", n, &bad);
}

/// F-55744 (with F-31346 and F-81602 inside): the pivot walked to a link; where they met.
#[test]
fn walk_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "wk") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        let met = a.walk(r.r(4) as usize, r.r(5) as i32);
        check_scratch(&a, &r, 4, 3, &format!("case {n}"), &mut bad);
        diff(&format!("case {n} met"), &[met as u32], &[r.ret], false, &mut bad);
    }
    report("walk", n, &bad);
}

/// F-31346: what waits at child c taken up its joint into the parent.
#[test]
fn scratch_up_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "su") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        a.scratch_up(r.r(4) as usize, r.r(5) as usize);
        check_scratch(&a, &r, 4, 3, &format!("case {n}"), &mut bad);
    }
    report("scratch_up", n, &bad);
}

/// F-81602: what the parent has seen that the joint's pair does not hold taken down the joint into the child.
#[test]
fn scratch_down_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "sd") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        a.scratch_down(r.r(4) as usize, r.r(5) as usize);
        check_scratch(&a, &r, 4, 3, &format!("case {n}"), &mut bad);
    }
    report("scratch_down", n, &bad);
}
