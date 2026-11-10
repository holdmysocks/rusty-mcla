//! Small routines of the loose-object simulation compared with the original (tools/loose_small_reference.py): each
//! runs the generated code unchanged on flat guest memory; every line of testdata/loose_small_ref.txt is
//! "<tag> <input words> -> <output words>" (hex; a word that reads 82xxxxxx is written with an h in front). A NaN is
//! compared only as a NaN (the harness's default NaN is not the console's).
use super::*;
use crate::physics::loose_words::{self as lw, Words};
use glam::Vec3;
use mcla_formats::rage::fragphys::FragGroup;

fn words(s: &str) -> Vec<u32> {
    s.split_whitespace().map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap()).collect()
}

fn cases(tag: &'static str) -> impl Iterator<Item = (Vec<u32>, Vec<u32>)> {
    include_str!("../testdata/loose_small_ref.txt").lines().filter_map(move |l| {
        let rest = l.strip_prefix(tag)?.strip_prefix(' ')?;
        let (i, o) = rest.split_once(" -> ")?;
        Some((words(i), words(o)))
    })
}

fn same(a: u32, b: u32) -> bool {
    a == b || (f32::from_bits(a).is_nan() && f32::from_bits(b).is_nan())
}

fn all_same(got: &[u32], want: &[u32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}");
    for (k, (&g, &w)) in got.iter().zip(want).enumerate() {
        assert!(same(g, w), "{what}: word {k}: {g:08x} against {w:08x}");
    }
}

/// The guest's words from address 0 to 0x3000.
struct Mem(Vec<u32>);

impl Mem {
    fn new() -> Mem {
        Mem(vec![0; 0xC00])
    }
}

impl Words for Mem {
    fn get(&self, a: u32) -> u32 {
        self.0[(a / 4) as usize]
    }
    fn set(&mut self, a: u32, v: u32) {
        self.0[(a / 4) as usize] = v;
    }
}

/// The list sections' node codes: 0 null, 1 .. 6 the node at 0x1100 + 0x40 (k - 1), 7 the header at 0x1000.
fn a(k: u32) -> u32 {
    match k {
        0 => 0,
        7 => 0x1000,
        k => 0x1100 + 0x40 * (k - 1),
    }
}

/// F-19844: the other side of a contact.
#[test]
fn other_side_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("other") {
        assert_eq!(lw::other_side(i[1], i[2], i[0]), o[0]);
        n += 1;
    }
    assert_eq!(n, 2000);
}

/// F-59304 (append), F-68557 (unlink), F-51875 (a node's parent, with F-69979 in front of the root list).
#[test]
fn list_operations_match_the_original() {
    let list_state = |i: &[u32]| {
        let mut m = Mem::new();
        m.set(0x1000, a(i[0]));
        m.set(0x1004, a(i[1]));
        m.set(0x1008, i[2]);
        for k in 0..6 {
            m.set(0x1100 + 0x40 * k, a(i[3 + 2 * k as usize]));
            m.set(0x1104 + 0x40 * k, a(i[4 + 2 * k as usize]));
        }
        m
    };
    let dump = |m: &Mem| {
        let mut o = vec![m.get(0x1000), m.get(0x1004), m.get(0x1008)];
        for k in 0..6 {
            o.push(m.get(0x1100 + 0x40 * k));
            o.push(m.get(0x1104 + 0x40 * k));
        }
        o
    };
    let mut n = [0; 3];
    for (i, o) in cases("append") {
        let mut m = list_state(&i);
        lw::list_append(&mut m, 0x1000, a(i[15]));
        all_same(&dump(&m), &o, "append");
        n[0] += 1;
    }
    for (i, o) in cases("unlink") {
        let mut m = list_state(&i);
        let next = lw::list_unlink(&mut m, 0x1000, a(i[15]));
        let mut got = dump(&m);
        got.push(next);
        all_same(&got, &o, "unlink");
        n[1] += 1;
    }
    for (i, o) in cases("move") {
        let mut m = Mem::new();
        m.set(0x1000, a(i[0]));
        m.set(0x1004, a(i[1]));
        m.set(0x1008, i[2]);
        for k in 0..6u32 {
            let (b, f) = (0x1100 + 0x40 * k, 3 + 6 * k as usize);
            m.set(b, a(i[f]));
            m.set(b + 4, a(i[f + 1]));
            m.set(b + 16, a(i[f + 2]));
            m.set(b + 20, a(i[f + 3]));
            m.set(b + 24, i[f + 4]);
            m.set(b + 28, a(i[f + 5]));
        }
        lw::list_move(&mut m, 0x1000, a(i[39]), a(i[40]));
        let mut got = vec![m.get(0x1000), m.get(0x1004), m.get(0x1008)];
        for k in 0..6u32 {
            let b = 0x1100 + 0x40 * k;
            got.extend([m.get(b), m.get(b + 4), m.get(b + 16), m.get(b + 20), m.get(b + 24), m.get(b + 28)]);
        }
        all_same(&got, &o, "move");
        n[2] += 1;
    }
    assert_eq!(n, [2000, 2000, 2500]);
}

/// F-45377: a group and its sub-groups in a bit array, one sub-tree left out.
#[test]
fn subtree_marks_match_the_original() {
    let mut n = 0;
    for (i, o) in cases("subtree") {
        let groups = (0..i[2] as usize).map(|k| FragGroup { first_group: i[5 + 2 * k] as u8, groups: i[6 + 2 * k] as u8, ..Default::default() }).collect();
        let t = FragType { groups, ..Default::default() };
        let mut bits = [i[3], i[4]];
        mark_subtree(&t, i[0] as usize, i[1], &mut bits);
        all_same(&bits, &o, "subtree");
        n += 1;
    }
    assert_eq!(n, 2000);
}

/// F-21340 / F-17349: a manifold side's matrix (the instance's, carried by the part's where the bound is a composite;
/// the identity without an instance). The three rows' xyz.
#[test]
fn side_matrices_match_the_original() {
    let mut n = 0;
    for tag in ["matrixA", "matrixB"] {
        for (i, o) in cases(tag) {
            let rows = |w: &[u32]| [0, 1, 2, 3].map(|r| Vec3::new(f32::from_bits(w[4 * r]), f32::from_bits(w[4 * r + 1]), f32::from_bits(w[4 * r + 2])));
            let inst = rows(&i[3..19]);
            let idx = i[2] as usize;
            let part = rows(&i[19 + 16 * idx..35 + 16 * idx]);
            let got = lw::side_matrix((i[0] != 0).then_some(&inst), (i[1] == 12).then_some(&part));
            let want = rows(&o);
            for r in 0..4 {
                for (g, w) in got[r].to_array().iter().zip(want[r].to_array()) {
                    assert!(same(g.to_bits(), w.to_bits()), "{tag} row {r}: {g:?} against {w:?}: {i:08x?}");
                }
            }
            n += 1;
        }
    }
    assert_eq!(n, 2400);
}

fn f(w: u32) -> f32 {
    f32::from_bits(w)
}

fn v3(w: &[u32]) -> Vec3 {
    Vec3::new(f(w[0]), f(w[1]), f(w[2]))
}

fn v3_words(v: Vec3) -> [u32; 3] {
    [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
}

/// A body at rest with the given velocities.
fn body_of(v: Vec3, w: Vec3) -> PBody {
    let mut b = PBody::new(&[Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO], Vec3::ZERO, &Entry::default(), BODY_MAX_SPEED, BODY_MAX_SPIN);
    (b.v, b.w) = (v, w);
    b
}

/// The case words of "terms" and "still" (see the tool) as a body.
fn still_body(i: &[u32]) -> PBody {
    let mut b = body_of(v3(&i[2..5]), v3(&i[5..8]));
    b.force = v3(&i[8..11]);
    b.torque = v3(&i[11..14]);
    b.jf = v3(&i[14..17]);
    b.jt = v3(&i[17..20]);
    b.push = v3(&i[20..23]);
    b.turn = v3(&i[23..26]);
    b.rows = [v3(&i[26..29]), v3(&i[29..32]), v3(&i[32..35])];
    b.inv_inertia = v3(&i[35..38]);
    b.inv_mass = f(i[38]);
    b
}

/// F-66163: the five still-test terms (the fifth is 0 for a pool body), through the real world inverse inertia of the
/// body's table.
#[test]
fn still_terms_match_the_original() {
    let mut n = 0;
    for (i, o) in cases("terms") {
        let b = still_body(&i);
        let t = still_terms(&b, f(i[0]), f(i[1]));
        all_same(&t.map(|x| x.to_bits()), &o[..4], "terms");
        assert_eq!(o[4], 0);
        n += 1;
    }
    assert_eq!(n, 2500);
}

/// F-61306: the verdict against the object's tolerances (and, with the usual ones, the port's own `still`).
#[test]
fn still_verdict_matches_the_original() {
    let (mut n, mut usual) = (0, 0);
    for (i, o) in cases("still") {
        let b = still_body(&i);
        let t = still_terms(&b, f(i[0]), f(i[1]));
        let got = still_with(t, 0.0, [f(i[39]), f(i[40]), f(i[41])]) as u32;
        assert_eq!(got, o[0], "{i:08x?}");
        if (i[39], i[40], i[41]) == (0x3d4ccccd, 0x3d4ccccd, 0x3e4ccccd) {
            assert_eq!(still(&b, f(i[0]), f(i[1])) as u32, o[0]);
            usual += 1;
        }
        n += 1;
    }
    assert_eq!(n, 2500);
    assert!(usual >= 1250, "{usual}");
}

/// F-43494: the sleep object's update (still?, state, counters, limits) - the counters and the state, and whether the
/// awake slot is called (the port sets state 1 itself).
#[test]
fn sleep_update_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("sleep") {
        let mut s = Sleep { state: i[1] as u8, a: i[2], b: i[3] };
        let called = s.update_with(i[0] & 0xFF != 0, i[4], i[5]);
        assert_eq!(called as u32, o[3], "{i:08x?}");
        let state = if called { 1 } else { o[0] };
        assert_eq!((s.state as u32, s.a, s.b), (state, o[1], o[2]), "{i:08x?}");
        if called {
            assert_eq!(o[0], i[1]);
        }
        n += 1;
    }
    assert_eq!(n, 4000);
}

/// F-21091: the pending impulses into the momenta and velocities (the real slots 45 and 46 of the table).
#[test]
fn pending_impulses_match_the_original() {
    let mut n = 0;
    for (i, o) in cases("endtick") {
        let mut b = body_of(v3(&i[12..15]), v3(&i[15..18]));
        (b.p, b.l, b.jf, b.jt) = (v3(&i[0..3]), v3(&i[3..6]), v3(&i[6..9]), v3(&i[9..12]));
        b.rows = [v3(&i[18..21]), v3(&i[21..24]), v3(&i[24..27])];
        b.inv_inertia = v3(&i[27..30]);
        (b.inv_mass, b.max_speed, b.max_spin) = (f(i[30]), f(i[31]), f(i[32]));
        b.take_impulses();
        let got: Vec<u32> = [b.p, b.l, b.jf, b.jt, b.v, b.w].into_iter().flat_map(v3_words).collect();
        all_same(&got, &o, &format!("endtick {i:08x?}"));
        n += 1;
    }
    assert_eq!(n, 2500);
}

fn mem_with(words: &[u32]) -> Mem {
    let mut m = Mem::new();
    for (k, &x) in words.iter().enumerate() {
        m.set(0x1000 + 4 * k as u32, x);
    }
    m
}

/// F-33655 / F-31401: the body's and the constraint's defaults, every word of the object (the table pointers are the
/// class's, which the port replaces by types: taken from the reference).
#[test]
fn constructors_match_the_original() {
    let (mut nb, mut nc) = (0, 0);
    for (i, o) in cases("ctor") {
        let mut m = mem_with(&i);
        lw::body_defaults(&mut m, 0x1000, o[0]);
        all_same(&(0..172).map(|k| m.get(0x1000 + 4 * k)).collect::<Vec<_>>(), &o[..172], "ctor");
        assert_eq!((o[172], o[173]), (1, 0x1000));
        nb += 1;
    }
    for (i, o) in cases("constraint") {
        let mut m = mem_with(&i);
        lw::constraint_defaults(&mut m, 0x1000, o[0], ROPE_DEPTH);
        all_same(&(0..84).map(|k| m.get(0x1000 + 4 * k)).collect::<Vec<_>>(), &o, "constraint");
        nc += 1;
    }
    assert_eq!((nb, nc), (1000, 1000));
}

/// F-50834: does an island rest (with F-82734, the manager's flag of a collider).
#[test]
fn island_rest_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("islands") {
        let k = |c: u32| match c {
            4 => 0x2C00,
            5 => 0,
            c => 0x2000 + 672 * c,
        };
        let mut m = Mem::new();
        m.set(0x1004, 0x1100);
        m.set(0x100C, 0x1200);
        m.set(0x1014, i[22]);
        m.set(0x1034, i[26]);
        m.set(0x1800 + 24, i[0]);
        m.set(0x1800 + 32, 0x2000);
        m.set(0x1800 + 36, 0x1580);
        for b in 0..8 {
            m.set_u8(0x1580 + b, i[1 + b as usize] as u8);
        }
        for j in 0..4u32 {
            m.set(0x1600 + 0x40 * j + 24, i[9 + 2 * j as usize]);
            m.set_u8(0x1600 + 0x40 * j + 40, i[10 + 2 * j as usize] as u8);
        }
        let rest = |r: u32| if r == 4 { 0 } else { 0x1600 + 0x40 * r };
        for c in 0..4u32 {
            m.set(0x2000 + 672 * c + 24, rest(i[17 + c as usize]));
        }
        m.set(0x2C00 + 24, rest(i[21]));
        for c in 0..3u32 {
            m.set(0x1200 + 4 * c, k(i[23 + c as usize]));
            m.set(0x1100 + 4 * c, 0x1300 + 0xC0 * c);
            m.set(0x1300 + 0xC0 * c + 160, 0x1300 + 0xC0 * c + 176);
            m.set(0x1300 + 0xC0 * c + 184, k(i[27 + 2 * c as usize]));
            m.set(0x1300 + 0xC0 * c + 188, k(i[28 + 2 * c as usize]));
        }
        assert_eq!(lw::island_rests(&m, 0x1000, 0x1800) as u32, o[0], "{i:08x?}");
        n += 1;
    }
    assert_eq!(n, 2500);
}

/// F-14324: the share of an impact through the groups, with the tuning object absent, 1 and 1, or other numbers.
#[test]
fn group_spread_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("spread") {
        let count = i[3] as usize;
        let groups = (0..count)
            .map(|g| {
                let b = 10 + count + 5 * g;
                FragGroup { up: f(i[b]), down: f(i[b + 1]), first_group: i[b + 2] as u8, parent: i[b + 3] as u8, groups: i[b + 4] as u8, ..Default::default() }
            })
            .collect();
        let t = FragType { groups, ..Default::default() };
        let absent = |g: usize| (if g < 32 { i[5] >> g } else { i[6] >> (g - 32) }) & 1 != 0;
        let present = |g: usize| i[4] == 0 || !absent(g);
        let mut gi: Vec<f32> = (0..count).map(|g| f(i[10 + g])).collect();
        let scales = (i[7] != 0).then(|| (f(i[8]), f(i[9])));
        spread(&t, &present, &mut gi, f(i[0]), i[1] as usize, i[2] as i32, scales);
        all_same(&gi.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), &o[..count], &format!("spread {i:08x?}"));
        n += 1;
    }
    assert_eq!(n, 2500);
}

/// F-49276: an entry's mass with the damaged forms, and (all healthy, or none with a damaged form) the port's own sum.
#[test]
fn entry_mass_with_health_matches_the_original() {
    let (mut n, mut plain) = (0, 0);
    for (i, o) in cases("mass") {
        let count = i[0] as usize;
        let absent = i[1] as u64 | (i[2] as u64) << 32;
        let health: Vec<f32> = (0..16).map(|g| f(i[3 + g])).collect();
        let kids: Vec<(f32, f32, u8, bool)> = (0..count).map(|c| (f(i[19 + 4 * c]), f(i[20 + 4 * c]), i[21 + 4 * c] as u8, i[22 + 4 * c] != 0)).collect();
        let got = lw::entry_mass_health(&kids, absent, &health);
        assert!(same(got.to_bits(), o[0]), "{i:08x?}");
        let healthy = kids.iter().all(|&(_, _, g, has)| !has || health[g as usize] > 0.0);
        if healthy {
            let children = kids.iter().map(|&(m, d, g, _)| mcla_formats::rage::fragphys::FragChild { mass: m, damaged_mass: d, group: g, ..Default::default() }).collect();
            let t = FragType { children, ..Default::default() };
            assert!(same(entry_mass(&t, absent).to_bits(), o[0]), "{i:08x?}");
            plain += 1;
        }
        n += 1;
    }
    assert_eq!(n, 1500);
    assert!(plain > 300, "{plain}");
}

/// F-43430: a contact's refresh - the impulse cleared, and (kind 0) the friction and elasticity from the two materials
/// and the limit from the two instances; any other kind leaves the three alone.
#[test]
fn contact_refresh_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("contact") {
        assert_eq!(o[..4], [0; 4]);
        if i[0] & 0xFF != 0 {
            assert_eq!(o[4..7], [0xA5A5_A5A5; 3]);
        } else {
            let mat = |k: u32| (f(i[3 + 2 * k as usize]), f(i[4 + 2 * k as usize]));
            let (fr, el) = contact_terms(mat(i[1]), mat(i[2]));
            let limit = lw::contact_limit((i[13] != 0).then(|| f(i[11])), (i[14] != 0).then(|| f(i[12])));
            all_same(&[fr.to_bits(), el.to_bits(), limit.to_bits()], &o[4..7], &format!("contact {i:08x?}"));
        }
        assert_eq!(o[7], 0);
        n += 1;
    }
    assert_eq!(n, 2500);
}

/// F-99875: the volume-weighted centre of a type's parts.
#[test]
fn volume_centre_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("centre") {
        let count = i[0] as usize;
        let parts = (0..count).filter(|k| i[2 + 23 * k] != 0).map(|k| {
            let b = 2 + 23 * k;
            let m = [0, 1, 2, 3].map(|r| v3(&i[b + 7 + 4 * r..]));
            (f(i[b + 6]), part_place(&m, (i[b + 1] & 0xFF != 0).then(|| v3(&i[b + 2..]))))
        });
        let c = volume_centre(parts);
        all_same(&v3_words(c), &o[..3], &format!("centre {i:08x?}"));
        assert_eq!(o[4], 1);
        n += 1;
    }
    assert_eq!(n, 2500);
}

/// F-66303: a body's damping (the archetype asks for it): the force and the torque it adds to the sums, through the
/// real slots 32 and 33 of the body's table.
#[test]
fn damping_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("damp") {
        let rows = [v3(&i[1..4]), v3(&i[4..7]), v3(&i[7..10])];
        let d: [[f32; 3]; 6] = std::array::from_fn(|k| [f(i[20 + 3 * k]), f(i[21 + 3 * k]), f(i[22 + 3 * k])]);
        let (df, dtq) = damping_forces(&rows, v3(&i[10..13]), v3(&i[13..16]), f(i[16]), v3(&i[17..20]), &d, f(i[0]));
        // (the sums start from the case's, the vectors added lane by lane)
        let want = (v3(&i[38..41]) + df, v3(&i[41..44]) + dtq);
        let got: Vec<u32> = [want.0, want.1].into_iter().flat_map(v3_words).collect();
        all_same(&got, &o, &format!("damp {i:08x?}"));
        n += 1;
    }
    assert_eq!(n, 3000);
}
