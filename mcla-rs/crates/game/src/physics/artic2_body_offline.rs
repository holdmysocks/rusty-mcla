//! (tests only) Second batch of the articulated body's small routines against the original run offline (verify-artic2):
//! tools/artic2_reference.py writes testdata/artic2_ref.txt from the generated code unchanged, the vector unit's flush mode on
//! as the console has it. Each test builds the port's own structs from the case's words and calls the function the game runs;
//! words compared one by one (a NaN only as a NaN). Evidence: work/verify_artic2_2026-11-10.md.

use super::*;
use crate::physics::artic_offline::*;

const REF2: &str = include_str!("../testdata/artic2_ref.txt");
const BUF: u32 = 0x3800;
/// The world's gravity vector, an image global the original's slot reads (the harness puts the case's words there).
const GRAV: u32 = 0x8200_0000 + 0x81_2800;

/// A joint block of the case, links by index 0 (the cases of the maps leave the link words empty).
fn lone_joint(r: &Rec) -> Joint {
    Joint::from_guest(r.mem.bytes(JOINT0, JSZ as usize), |_| 0)
}

fn hinge_of(j: &Joint) -> &Hinge {
    match &j.kind {
        JointKind::Hinge(h) => h,
        _ => unreachable!(),
    }
}

fn flat(m: &[V4; 9]) -> Vec<u32> {
    m.iter().flat_map(|v| v.bits()).collect()
}

/// F-91185 (with F-64271 in the collider's slot 41): the root's velocity change d is added to every link's +608; +640 = +624
/// and +656 = +608 + c x w. Only the links' words are the original routine's; the port's mirror (F-64271) is not compared.
#[test]
fn set_velocity_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "sv") {
        n += 1;
        let mut a = r.mem.articulated(4, 0);
        a.set_velocity(r.mem.vec(BUF));
        for i in 0..4u32 {
            let g = words_of(&a.links[i as usize].to_guest());
            diff(&format!("case {n} link {i}"), &g[152..168], &r.want(LINK0 + LSZ * i + 608, 16), true, &mut bad);
        }
    }
    report("set_velocity", n, &bad);
}

/// F-52649 (with the hinge F-72940 and the swing-twist F-70063 behind it): every child put back on its parent through its joint,
/// parents first, its motion refreshed. Compared: every link's rows and position (+416 .. +479) and velocities (+608 .. +671).
#[test]
fn reseat_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "rt") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        a.reseat();
        for i in 0..4u32 {
            let g = words_of(&a.links[i as usize].to_guest());
            diff(&format!("case {n} link {i} +416"), &g[104..120], &r.want(LINK0 + LSZ * i + 416, 16), true, &mut bad);
            diff(&format!("case {n} link {i} +608"), &g[152..168], &r.want(LINK0 + LSZ * i + 608, 16), true, &mut bad);
        }
    }
    report("reseat", n, &bad);
}

/// F-39401: every link's impulse through its own inverse, taken up the joints to the root and down again; the impulses cleared.
#[test]
fn propagate_impulses_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "pg") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        a.propagate_impulses();
        for i in 0..4u32 {
            let g = words_of(&a.links[i as usize].to_guest());
            diff(&format!("case {n} link {i} +704"), &g[176..184], &r.want(LINK0 + LSZ * i + 704, 8), true, &mut bad);
            diff(&format!("case {n} link {i} +912"), &g[228..244], &r.want(LINK0 + LSZ * i + 912, 16), true, &mut bad);
        }
        for j in 0..3u32 {
            let g = words_of(&a.joint_to_guest(j as usize));
            diff(&format!("case {n} joint {j}"), &g[76..92], &r.want(JOINT0 + JSZ * j + 304, 16), true, &mut bad);
        }
    }
    report("propagate_impulses", n, &bad);
}

/// F-60999: the articulated bias forces (per link, backward through the joints, forward again).
#[test]
fn bias_forces_match_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "bf") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        a.bias_forces();
        for i in 0..4u32 {
            let g = words_of(&a.links[i as usize].to_guest());
            diff(&format!("case {n} link {i}"), &g[220..228], &r.want(LINK0 + LSZ * i + 880, 8), true, &mut bad);
        }
        for j in 0..3u32 {
            let g = words_of(&a.joint_to_guest(j as usize));
            diff(&format!("case {n} joint {j}"), &g[92..108], &r.want(JOINT0 + JSZ * j + 368, 16), true, &mut bad);
        }
    }
    report("bias_forces", n, &bad);
}

/// F-90112 (with F-60999, F-91555 and F-39401 inside): velocities made relative to the root's, bias, forces and impulses
/// committed, the root's velocity added back. Compared: the links' +608 .. +735 and +880 .. +975, the joints' +304 .. +415.
#[test]
fn forces_to_velocities_match_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "fv") {
        n += 1;
        let mut a = r.mem.articulated(4, 3);
        a.forces_to_velocities(f32::from_bits(r.reg(11)[0]));
        for i in 0..4u32 {
            let g = words_of(&a.links[i as usize].to_guest());
            diff(&format!("case {n} link {i} +608"), &g[152..184], &r.want(LINK0 + LSZ * i + 608, 32), true, &mut bad);
            diff(&format!("case {n} link {i} +880"), &g[220..244], &r.want(LINK0 + LSZ * i + 880, 24), true, &mut bad);
        }
        for j in 0..3u32 {
            let g = words_of(&a.joint_to_guest(j as usize));
            diff(&format!("case {n} joint {j}"), &g[76..104], &r.want(JOINT0 + JSZ * j + 304, 28), true, &mut bad);
        }
    }
    report("forces_to_velocities", n, &bad);
}

/// F-58091: a . response(a), the joint's inverse inertia about the axis a (returned in f1).
#[test]
fn axis_inertia_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "ax") {
        n += 1;
        let a = r.mem.articulated(4, 3);
        let j = ((r.r(3) - JOINT0) / JSZ) as usize;
        let got = a.axis_inertia(j, r.mem.vec(BUF));
        diff(&format!("case {n}"), &[got.to_bits()], &r.want(0x3900, 1), true, &mut bad);
    }
    report("axis_inertia", n, &bad);
}

/// F-11734: the 6x6 block matrix inverse (nine vectors in, nine out).
#[test]
fn block_inverse_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "bi") {
        n += 1;
        let m: [V4; 9] = std::array::from_fn(|i| r.mem.vec(0x3A00 + 16 * i as u32));
        diff(&format!("case {n}"), &flat(&block_inverse(&m)), &r.want(0x3B00, 36), true, &mut bad);
    }
    report("block_inverse", n, &bad);
}

/// F-38496: the rotation rows for an angle about a unit axis (all three rows, every lane).
#[test]
fn turn_rows_match_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "tr") {
        n += 1;
        let rows = turn_rows(r.v(11), f32::from_bits(r.reg(12)[0]));
        let got: Vec<u32> = rows.iter().flat_map(|v| v.bits()).collect();
        diff(&format!("case {n}"), &got, &r.want(0x3900, 12), true, &mut bad);
    }
    report("turn_rows", n, &bad);
}

/// F-21563 -> F-69646: gravity times the body's factor on every link's mass, force and torque about the common origin.
#[test]
fn gravity_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "gr") {
        n += 1;
        let mut a = r.mem.articulated(4, 0);
        a.gravity(r.mem.vec(GRAV));
        for i in 0..4u32 {
            let g = words_of(&a.links[i as usize].to_guest());
            diff(&format!("case {n} link {i}"), &g[168..176], &r.want(LINK0 + LSZ * i + 672, 8), true, &mut bad);
        }
    }
    report("gravity", n, &bad);
}

/// F-53726: a link's spatial inertia about the common origin (three blocks of three vectors).
#[test]
fn spatial_inertia_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "si") {
        n += 1;
        let l = Link::from_guest(r.mem.bytes(LINK0, LSZ as usize));
        diff(&format!("case {n}"), &flat(&spatial_inertia(&l)), &r.want(LINK0 + 128, 36), true, &mut bad);
    }
    report("spatial_inertia", n, &bad);
}

/// F-25250: the hinge's rest inertia = the child's articulated inertia minus (U U^T / d) (the rebuild's `rest_ia`).
#[test]
fn minus_outer_child_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "m1") {
        n += 1;
        let j = lone_joint(&r);
        let h = hinge_of(&j);
        diff(&format!("case {n}"), &flat(&minus_outer(&j.child_ia, &h.u, h.inv_d)), &r.want(BUF, 36), true, &mut bad);
    }
    report("minus_outer (F-25250)", n, &bad);
}

/// F-93538: the same with the rest-pose pair (U', 1/d' at +776) taken off `rest_ia` (the rebuild's second use).
#[test]
fn minus_outer_rest_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(REF2, "m2") {
        n += 1;
        let j = lone_joint(&r);
        let h = hinge_of(&j);
        diff(&format!("case {n}"), &flat(&minus_outer(&j.rest_ia, &h.u_rest, h.inv_d_rest)), &r.want(BUF, 36), true, &mut bad);
    }
    report("minus_outer (F-93538)", n, &bad);
}
