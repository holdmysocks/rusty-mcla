//! The point-constraint row routines of the contact solver compared with the original (tools/leaves_reference.py): each
//! runs the generated code unchanged on flat guest memory; every line of testdata/leaves_ref.txt is
//! "<tag> <input words> -> <output words>" (hex; a word that reads 82xxxxxx is written with an h in front).
//!
//! The row of the original is 272 bytes: +0 the multipliers, +16 the bias, +32 the basis by columns (3 vectors), +80 and
//! +84 the two sides' states, +96 the K block (its inverse once inverted), +144 and +192 the arms by columns (A, B), byte
//! +240 the angular-only flag. A state is +0 velocity, +16 angular velocity, +32 and +80 the mass and inertia rows. The
//! case is: neg (4), bias (4), basis (12), K (12), arms A (12), arms B (12), flag, state A (32), state B (32), the global
//! the step adds its square to (4), the step's operand (4).
use super::{block_inverse, columns, k_block, Row, Row2, State};
use glam::Vec3;

fn words(s: &str) -> Vec<u32> {
    s.split_whitespace().map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap()).collect()
}

fn cases(tag: &'static str) -> impl Iterator<Item = (Vec<u32>, Vec<u32>)> {
    include_str!("../testdata/leaves_ref.txt").lines().filter_map(move |l| {
        let rest = l.strip_prefix(tag)?.strip_prefix(' ')?;
        let (i, o) = rest.split_once(" -> ")?;
        Some((words(i), words(o)))
    })
}

fn f(w: u32) -> f32 {
    f32::from_bits(w)
}

fn same(a: u32, b: u32) -> bool {
    a == b || (f(a).is_nan() && f(b).is_nan())
}

/// The first three words of a vector at word `at`.
fn v3(w: &[u32], at: usize) -> Vec3 {
    Vec3::new(f(w[at]), f(w[at + 1]), f(w[at + 2]))
}

/// Three vectors (12 words) as three Vec3.
fn m3(w: &[u32], at: usize) -> [Vec3; 3] {
    [v3(w, at), v3(w, at + 4), v3(w, at + 8)]
}

/// A state as the port holds it: the stored rows are `columns` of its `mcol` / `icol`.
fn state_of(w: &[u32], at: usize) -> State {
    State { vel: v3(w, at), ang: v3(w, at + 4), mcol: columns(&m3(w, at + 8)), icol: columns(&m3(w, at + 20)) }
}

fn expect(got: Vec3, o: &[u32], at: usize, what: &str) {
    for (k, g) in [got.x, got.y, got.z].into_iter().enumerate() {
        assert!(same(g.to_bits(), o[at + k]), "{what}: lane {k}: {:08x} against {:08x}", g.to_bits(), o[at + k]);
    }
}

fn blocks(i: &[u32]) -> ([Vec3; 3], [Vec3; 3], [Vec3; 3], [Vec3; 3], [Vec3; 3], [Vec3; 3]) {
    // basis by columns (+32) and its rows; the arms by columns (+144, +192) and their rows
    let (bcol, jcol, jcol_b) = (m3(i, 8), m3(i, 32), m3(i, 44));
    (columns(&bcol), bcol, columns(&jcol), jcol, columns(&jcol_b), jcol_b)
}

/// F-72162: the K block of a point row, with and without the angular-only byte.
#[test]
fn k_block_matches_the_original() {
    let (mut n, mut flagged) = (0, 0);
    for (i, o) in cases("kblock") {
        let (basis, bcol, arm, jcol, arm_b, jcol_b) = blocks(&i);
        let (a, b) = (state_of(&i, 57), state_of(&i, 89));
        let flag = i[56] != 0;
        flagged += flag as i32;
        let k = k_block(&basis, &bcol, &arm, &jcol, &arm_b, &jcol_b, &a, &b, true, flag);
        for r in 0..3 {
            expect(k[r], &o, 4 * r, &format!("{i:08x?} row {r}"));
        }
        n += 1;
    }
    assert!(n >= 1000 && flagged >= 100, "{n} {flagged}");
}

/// F-52431: the K block, then inverted in place (F-68939).
#[test]
fn k_block_inverse_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("kinv") {
        let (basis, bcol, arm, jcol, arm_b, jcol_b) = blocks(&i);
        let (a, b) = (state_of(&i, 57), state_of(&i, 89));
        let k = k_block(&basis, &bcol, &arm, &jcol, &arm_b, &jcol_b, &a, &b, true, i[56] != 0);
        let inv = block_inverse(&k);
        for r in 0..3 {
            expect(inv[r], &o, 4 * r, &format!("{i:08x?} row {r}"));
        }
        n += 1;
    }
    assert!(n >= 1000);
}

/// The row as the port holds it, with K's inverse rows from the case (the port keeps them by columns).
fn row_of(i: &[u32]) -> Row2 {
    let (basis, bcol, arm, jcol, arm_b, jcol_b) = blocks(i);
    Row2 {
        row: Row { neg: v3(i, 0), bias: v3(i, 4), basis, bcol, arm, jcol, kcol: columns(&m3(i, 20)), c: Vec3::X, mu: 0.0 },
        arm_b,
        jcol_b,
        sa: 1,
        sb: 2,
        kind: 1,
    }
}

fn states_of(i: &[u32]) -> Vec<State> {
    vec![State { vel: Vec3::ZERO, ang: Vec3::ZERO, mcol: [Vec3::ZERO; 3], icol: [Vec3::ZERO; 3] }, state_of(i, 57), state_of(i, 89)]
}

fn check_states(states: &[State], o: &[u32], what: &str) {
    expect(states[1].vel, o, 16, what);
    expect(states[1].ang, o, 20, what);
    expect(states[2].vel, o, 24, what);
    expect(states[2].ang, o, 28, what);
}

/// The global the step adds its square to: the vector multiply-add `step x step + global`, two roundings.
fn square_into(step: Vec3, g: Vec3) -> Vec3 {
    (step * step) + g
}

/// F-70556: the step. The multipliers' step is minus K^-1 times the operand; A gets it, B loses it (`give`).
#[test]
fn step_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("step") {
        let what = format!("{i:08x?}");
        let row = row_of(&i);
        let mut states = states_of(&i);
        let u = v3(&i, 125);
        let r = &row.row;
        let ku = Vec3::new(super::dot3(r.kcol[0], u), super::dot3(r.kcol[1], u), super::dot3(r.kcol[2], u));
        let step = Vec3::ZERO - ku;
        row.give(&mut states, step);
        expect(step, &o, 36, &what);
        check_states(&states, &o, &what);
        expect(square_into(step, v3(&i, 121)), &o, 32, &what);
        n += 1;
    }
    assert!(n >= 1000);
}

/// F-52995: the relative velocity of the two points and the step that takes it away; the multipliers lose the step. With
/// the angular-only byte the operand is A's angular velocity minus B's, less the bias.
#[test]
fn relative_velocity_matches_the_original() {
    let (mut n, mut flagged) = (0, 0);
    for (i, o) in cases("relvel") {
        let what = format!("{i:08x?}");
        let mut row = row_of(&i);
        let mut states = states_of(&i);
        let g = v3(&i, 121);
        let sq = if i[56] == 0 {
            // (Row2::visit for a point row: both ways, no relaxation, no cone; it gives step x step back)
            row.visit(&mut states, 1.0)
        } else {
            flagged += 1;
            let r = &row.row;
            let u = (states[1].ang - states[2].ang) - r.bias;
            let ku = Vec3::new(super::dot3(r.kcol[0], u), super::dot3(r.kcol[1], u), super::dot3(r.kcol[2], u));
            let s = Vec3::ZERO - ku;
            row.give(&mut states, s);
            row.row.neg = row.row.neg - s;
            s * s
        };
        expect(sq + g, &o, 32, &what);
        expect(row.row.neg, &o, 0, &what);
        check_states(&states, &o, &what);
        n += 1;
    }
    assert!(n >= 1000 && flagged >= 100, "{n} {flagged}");
}

/// F-90205: the row multipliers' reset. The port builds its rows with the multipliers zero, so this is the original's
/// walk written out: the list's +32 takes its +36, its +16 / +24 / +20 / +4 are cleared, and for each row the iterator
/// gives (its type in the list's +16) the states of a type above 6 (+80) and above 12 (+84) are told to reset and the
/// multipliers (first vector) are cleared unless the type is 1, 7 or 13; it returns the count. The iterator is a stub
/// that writes the type of the row it returns.
#[test]
fn multiplier_reset_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("rowzero") {
        let rows = i[0] as usize;
        let types = &i[1..7];
        // the list: words 0..8 prefilled, word 8 (+32) takes word 9 (+36, zero); +4, +16, +20, +24 cleared
        let mut list: Vec<u32> = i[7..15].to_vec();
        list.push(0);
        let mut told = vec![];
        for k in [1, 4, 5, 6] {
            list[k] = 0;
        }
        let mut row_words: Vec<[u32; 4]> = (0..6).map(|r| [i[15 + 6 * r], i[16 + 6 * r], i[17 + 6 * r], i[18 + 6 * r]]).collect();
        for r in 0..rows {
            let t = types[r];
            list[4] = t;
            if (t as i32) > 6 {
                told.push(i[15 + 6 * r + 4]);
            }
            if (t as i32) > 12 {
                told.push(i[15 + 6 * r + 5]);
            }
            if !matches!(t, 1 | 7 | 13) {
                row_words[r] = [0; 4];
            }
        }
        let what = format!("{i:08x?}");
        for k in 0..9 {
            assert_eq!(o[k], list[k], "{what}: list word {k}");
        }
        for r in 0..6 {
            for k in 0..4 {
                assert_eq!(o[9 + 4 * r + k], row_words[r][k], "{what}: row {r} word {k}");
            }
        }
        for (k, w) in told.iter().enumerate().take(6) {
            assert_eq!(o[33 + k], *w, "{what}: told {k}");
        }
        assert_eq!(o[39] as usize, told.len(), "{what}");
        assert_eq!(o[40] as usize, rows, "{what}: the count");
        n += 1;
    }
    assert!(n >= 1000);
}
