//! Several cars in one island, as the original solves them (GitHub #39, plan step 3): a recomp recording of two or more
//! vehicles in contact with no thrown rider (recorder `MCLA_CAR_SNAP`, work/ragdoll/carcar/export_cars.py, fixture
//! format RGCAR001 below), and the island's velocity solve with every rigid body and every enabled row, word for word.
//!
//! The original (F-93826 -> F-38310 -> F-96487, ragdoll_self_contacts.md section 17) builds one island over every pair
//! of the level (union-find over the cache's pairs, F-68442) and solves each island once (F-68693) with every body's
//! velocity as the move left it and its sums +336 / +352; the solve's impulses go into each body's accumulators +368 /
//! +384, which the integrator F-94789 reads. The recording's kind 4 snapshot is taken at the tick's first F-94789 entry
//! (every island solved, no body integrated yet), its kind 3 at the tick before's end.
//!
//! The loader (`multicar_ticks`) hands the same data as plain structs - the cars' bodies, each car's rows with the city
//! (and with objects that have no body) and the rows between two bodies, all in the recorded A list's order - so that
//! the live `LooseState::solve_island` with several cars is fed from it (`two_car_live_island_is_the_originals`).
#![allow(dead_code)]

use crate::physics::exact::fma;
use crate::physics::solver;
use glam::Vec3;

/// The step of the recorded solve (the rigid slot 10's dt, as v1 of the rider's records: 1/60 s).
pub(crate) const DT: f32 = f32::from_bits(0x3c88_8889);
/// The instance classes of the vehicles (inst +0): an mcCar (player, AI racers, cops) and an ambient car.
pub(crate) const MCCAR: u32 = 0x8204_5F44;
pub(crate) const AMBIENT: u32 = 0x820C_4314;

/// A rigid body of a recorded tick: its level index, its instance's class, its collider (176 big-endian words) at the
/// tick before's end (kind 3) and after the tick's velocity solve (kind 4), and its instance's matrix at both.
pub(crate) struct RecBody {
    pub level: u32,
    pub class: u32,
    pub col0: Vec<u32>,
    pub col1: Vec<u32>,
    pub matrix0: Vec<u32>,
    pub matrix1: Vec<u32>,
    /// (RGCAR002) its collider and its instance's matrix after the push solve (kind 5: every island's push solve F-56590
    /// done, no push applied yet)
    pub col5: Option<Vec<u32>>,
    pub matrix5: Option<Vec<u32>>,
}

/// An enabled row of a recorded island, in the A list's order: the rigid body of each side (None: the city, a fixed or
/// inactive object), the parts and levels of both sides (None: no level object), whether the record is the kind 4
/// snapshot's (a row in no PAIR record), the 176-byte record after the solve, and the warm start (the last earlier
/// tick's record at the same address: its +0, +128, +16).
pub(crate) struct RecRow {
    pub bodies: [Option<usize>; 2],
    pub parts: [i32; 2],
    pub levels: [Option<u32>; 2],
    pub snap: bool,
    pub w: [u32; 44],
    pub prev_tick: u32,
    pub prev: [u32; 12],
    /// (RGCAR002) the record's +0 .. +15 after the push solve (the push multipliers, w the depth), and per side A, B its
    /// whole bound's type (u32::MAX: no instance) and margin as F-49764 reads it (bits)
    pub push: Option<([u32; 4], [(u32, u32); 2])>,
}

pub(crate) struct RecTick {
    pub tick: u32,
    /// isl +40 (bits) and isl +52 (the enabled contacts)
    pub bounce: u32,
    pub n: u32,
    pub rows: Vec<RecRow>,
    pub bodies: Vec<RecBody>,
}

/// Reads an RGCAR001 fixture (export_cars.py): per tick u32 words tick, isl +40, isl +52, the row count, per row 64
/// words (body A, body B, part A, part B, from, level A, level B, the 44-word record, the warm start's tick and 12
/// words), the body count, per body 386 words (level, class, collider at kind 3 and kind 4, matrix at both).
/// RGCAR002 (work/ragdoll/carpush/export_cars2.py) adds per tick, after the bodies, per row 8 words (the record's +0 ..
/// +15 at the push snapshot kind 5; per side the whole bound's type and margin) and per body 192 words (its collider and
/// its instance's matrix at kind 5).
pub(crate) fn rec_ticks(b: &[u8]) -> Vec<RecTick> {
    let v2 = match &b[..8] {
        b"RGCAR001" => false,
        b"RGCAR002" => true,
        m => panic!("not an RGCAR fixture: {m:?}"),
    };
    let w: Vec<u32> = b[8..].chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let (ticks, mut i) = (w[0] as usize, 1);
    let opt = |x: u32| if x == u32::MAX { None } else { Some(x) };
    let mut out = Vec::new();
    for _ in 0..ticks {
        let len = w[i] as usize;
        let t = &w[i + 1..i + 1 + len];
        i += 1 + len;
        let mut k = 4;
        let mut rows = Vec::new();
        for _ in 0..t[3] {
            rows.push(RecRow {
                bodies: [opt(t[k]).map(|x| x as usize), opt(t[k + 1]).map(|x| x as usize)],
                parts: [t[k + 2] as i32, t[k + 3] as i32],
                snap: t[k + 4] != 0,
                levels: [opt(t[k + 5]), opt(t[k + 6])],
                w: t[k + 7..k + 51].try_into().unwrap(),
                prev_tick: t[k + 51],
                prev: t[k + 52..k + 64].try_into().unwrap(),
                push: None,
            });
            k += 64;
        }
        let nb = t[k] as usize;
        k += 1;
        let mut bodies = Vec::new();
        for _ in 0..nb {
            bodies.push(RecBody { level: t[k], class: t[k + 1], col0: t[k + 2..k + 178].to_vec(), col1: t[k + 178..k + 354].to_vec(), matrix0: t[k + 354..k + 370].to_vec(), matrix1: t[k + 370..k + 386].to_vec(), col5: None, matrix5: None });
            k += 386;
        }
        if v2 {
            for r in rows.iter_mut() {
                r.push = Some((t[k..k + 4].try_into().unwrap(), [(t[k + 4], t[k + 5]), (t[k + 6], t[k + 7])]));
                k += 8;
            }
            for b in bodies.iter_mut() {
                b.col5 = Some(t[k..k + 176].to_vec());
                b.matrix5 = Some(t[k + 176..k + 192].to_vec());
                k += 192;
            }
        }
        assert_eq!(k, len);
        out.push(RecTick { tick: t[0], bounce: t[1], n: t[2], rows, bodies });
    }
    out
}

fn wf(w: &[u32], o: usize) -> f32 {
    f32::from_bits(w[o / 4])
}
fn wv(w: &[u32], o: usize) -> Vec3 {
    Vec3::new(wf(w, o), wf(w, o + 4), wf(w, o + 8))
}
fn words3(w: &[u32], o: usize) -> [u32; 3] {
    [w[o / 4], w[o / 4 + 1], w[o / 4 + 2]]
}
fn vec_words(got: Vec3, want: [u32; 3]) -> bool {
    [got.x.to_bits(), got.y.to_bits(), got.z.to_bits()] == want
}

/// A rigid body as the island's solve takes it: from its collider after the solve (the solve writes none of these -
/// place +256, velocity +272, angular velocity +288, inverse mass +196, inverse moments +176, the sums +336 / +352)
/// and its instance's rows.
pub(crate) fn rec_body(b: &RecBody) -> solver::Body {
    let (w, m) = (&b.col1, &b.matrix1);
    solver::Body { at: wv(w, 256), rows: [wv(m, 0), wv(m, 16), wv(m, 32)], vel: wv(w, 272), ang: wv(w, 288), inv_mass: wf(w, 196), inv_moments: wv(w, 176), force: wv(w, 336), torque: wv(w, 352) }
}

/// A row as the solver takes it: both sides with their bodies and world points (+48 / +64), the record's depth,
/// friction, elasticity, normal, target and kind; the warm start (+128 kept, +0 pushed) the last earlier tick's record at
/// the same address when that is the tick before and A's local point moved less than the original's refresh distance
/// (as artic_island.rs `isl2_touches`), else none.
pub(crate) fn rec_touch(t: &RecTick, r: &RecRow) -> solver::Touch {
    let f = |o: usize| f32::from_bits(r.w[o / 4]);
    let v = |o: usize| Vec3::new(f(o), f(o + 4), f(o + 8));
    // (each side's point in the world +48 / +64 and in its part's frame +16 / +32 - the push solve's thickness takes the latter)
    let side = |o: usize, s: usize| solver::Side { body: r.bodies[s], point: v(o), local: v(o - 32), ..Default::default() };
    let pw = |i: usize| f32::from_bits(r.prev[i]);
    let d = v(16) - Vec3::new(pw(8), pw(9), pw(10));
    let near = fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)) < f32::from_bits(0x39d1_b717);
    let prev = |i: usize| if r.prev_tick + 1 != t.tick || !near { Vec3::ZERO } else { Vec3::new(pw(i), pw(i + 1), pw(i + 2)) };
    solver::Touch {
        depth: f(12),
        friction: f(28),
        elasticity: f(44),
        a: side(48, 0),
        b: side(64, 1),
        normal: v(80),
        back: v(96),
        target: v(144),
        kind: (r.w[41] >> 24) as u8,
        jointed: None,
        kept: prev(4),
        pushed: prev(0),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------------------------
// The loader for the live path: per tick the cars' bodies, each car's own rows (with the city or an object without a
// body) and the rows between two bodies, all in the recorded A list's order

/// A body of the island: its level, its class (`MCCAR`, `AMBIENT`, or another object's), the body as the solve takes
/// it, its instance's matrix (rows and place) after the move, and the recorded share of the solve: what +368 (impulse)
/// and +384 (turning) hold after it (bits).
pub(crate) struct MultiBody {
    pub level: u32,
    pub class: u32,
    pub body: solver::Body,
    pub matrix: [Vec3; 4],
    pub share: [[u32; 3]; 2],
    /// what +400 (push) and +416 (turn) hold as the push solve begins (kind 4, bits), and (RGCAR002) after it with the
    /// deep byte +658
    pub push_start: [[u32; 3]; 2],
    pub push_end: Option<([[u32; 3]; 2], bool)>,
    /// its instance's address (collider +16): the island's A / B lists are sorted by it
    pub inst: u32,
}

/// What a row joins: one body and something without one (the city, a fixed or inactive object; `body_is_a`: the body is
/// the record's A), or two bodies (A, B of the record).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowJoin {
    Own { body: usize, body_is_a: bool },
    Pair { a: usize, b: usize },
}

/// An enabled row: its place in the recorded A list (`order`), what it joins, the levels and parts of both sides, the row
/// as the solver takes it (warm start included), and the recorded results after the solve: kept multipliers +128 and
/// impulse +112 (bits).
pub(crate) struct MultiRow {
    pub order: usize,
    pub join: RowJoin,
    pub levels: [Option<u32>; 2],
    pub parts: [i32; 2],
    pub touch: solver::Touch,
    pub kept: [u32; 3],
    pub impulse: [u32; 3],
    /// (RGCAR002) the push multipliers +0 after the push solve (bits), and per side its whole bound's type (u32::MAX: no
    /// instance) and margin
    pub pushed: Option<[u32; 3]>,
    pub bound: Option<[(u32, f32); 2]>,
}

impl MultiRow {
    /// The row in the one-body form (solver::Contact, the body A) - for a body's own row where the body is the record's
    /// A, as the car's city rows of `LooseState::solve_island` (`IslRow::Bike`) take it; None otherwise.
    pub fn contact(&self) -> Option<solver::Contact> {
        let x = &self.touch;
        match self.join {
            RowJoin::Own { body_is_a: true, .. } => Some(solver::Contact { depth: x.depth, friction: x.friction, elasticity: x.elasticity, local: Vec3::ZERO, on_body: x.a.point, on_world: x.b.point, normal: x.normal, pushed: x.pushed, kept: x.kept, target: x.target, impulse: Vec3::ZERO }),
            _ => None,
        }
    }
}

pub(crate) struct MultiTick {
    pub tick: u32,
    /// isl +40 (bits)
    pub bounce: u32,
    pub dt: f32,
    pub bodies: Vec<MultiBody>,
    pub rows: Vec<MultiRow>,
}

impl MultiTick {
    /// The bodies that are vehicles (an mcCar or an ambient car).
    pub fn cars(&self) -> Vec<usize> {
        (0..self.bodies.len()).filter(|&i| matches!(self.bodies[i].class, MCCAR | AMBIENT)).collect()
    }
    /// A body's own rows (with the city or an object without a body), in the A list's order.
    pub fn own_rows(&self, body: usize) -> Vec<&MultiRow> {
        self.rows.iter().filter(|r| matches!(r.join, RowJoin::Own { body: b, .. } if b == body)).collect()
    }
    /// The rows between two bodies, in the A list's order.
    pub fn pair_rows(&self) -> Vec<&MultiRow> {
        self.rows.iter().filter(|r| matches!(r.join, RowJoin::Pair { .. })).collect()
    }
}

/// Every tick of an RGCAR001 fixture as `MultiTick`s.
pub(crate) fn multicar_ticks(fixture: &[u8]) -> Vec<MultiTick> {
    rec_ticks(fixture)
        .iter()
        .map(|t| {
            let bodies = t
                .bodies
                .iter()
                .map(|b| {
                    let m = &b.matrix1;
                    let push_end = b.col5.as_ref().map(|c| ([words3(c, 400), words3(c, 416)], (c[658 / 4] >> 8) & 0xff != 0));
                    MultiBody { level: b.level, class: b.class, body: rec_body(b), matrix: [wv(m, 0), wv(m, 16), wv(m, 32), wv(m, 48)], share: [words3(&b.col1, 368), words3(&b.col1, 384)], push_start: [words3(&b.col1, 400), words3(&b.col1, 416)], push_end, inst: b.col1[4] & !15 }
                })
                .collect();
            let rows = t
                .rows
                .iter()
                .enumerate()
                .map(|(order, r)| {
                    let join = match r.bodies {
                        [Some(a), Some(b)] => RowJoin::Pair { a, b },
                        [Some(a), None] => RowJoin::Own { body: a, body_is_a: true },
                        [None, Some(b)] => RowJoin::Own { body: b, body_is_a: false },
                        [None, None] => panic!("tick {}: row {order} has no body", t.tick),
                    };
                    let pushed = r.push.map(|p| [p.0[0], p.0[1], p.0[2]]);
                    let bound = r.push.map(|p| p.1.map(|(ty, m)| (ty, f32::from_bits(m))));
                    MultiRow { order, join, levels: r.levels, parts: r.parts, touch: rec_touch(t, r), kept: words3(&r.w, 128), impulse: words3(&r.w, 112), pushed, bound }
                })
                .collect();
            MultiTick { tick: t.tick, bounce: t.bounce, dt: DT, bodies, rows }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The island's velocity solve (solver::island_solve over every rigid body and every enabled row in the recorded A
    /// order), the rows' multipliers and impulses (island_impulses) and the bodies' shares (island_apply): isl +40, every
    /// row's +128 / +112 and every body's +368 / +384 word for word; on `open` ticks only signed zeros of an impulse may
    /// differ. Answers the report and the ticks checked.
    fn multicar_island(fixture: &[u8], open: &[u32]) -> (Vec<String>, usize) {
        let mut report = Vec::new();
        let ticks = rec_ticks(fixture);
        for t in &ticks {
            let bodies: Vec<solver::Body> = t.bodies.iter().map(rec_body).collect();
            let mut touches: Vec<solver::Touch> = t.rows.iter().map(|r| rec_touch(t, r)).collect();
            assert_eq!(touches.len(), t.n as usize);
            let solved = solver::island_solve(&bodies, &touches, DT, 1.0 / DT);
            let mut bad = Vec::new();
            if solved.bounce.to_bits() != t.bounce {
                bad.push(format!("bounce {:08x} want {:08x}", solved.bounce.to_bits(), t.bounce));
            }
            solver::island_impulses(&solved, &mut touches);
            for (k, (x, r)) in touches.iter().zip(t.rows.iter()).enumerate() {
                let what = format!("row {k} (levels {:x?}, bodies {:?}, parts {:?})", r.levels, r.bodies, r.parts);
                if !vec_words(x.kept, words3(&r.w, 128)) {
                    bad.push(format!("{what} kept {:?} want {:08x?}", x.kept, words3(&r.w, 128)));
                }
                if !vec_words(x.impulse, words3(&r.w, 112)) {
                    let want = words3(&r.w, 112);
                    // (equal as numbers: only the sign of a zero differs)
                    let zero_sign = x.impulse == Vec3::new(f32::from_bits(want[0]), f32::from_bits(want[1]), f32::from_bits(want[2]));
                    bad.push(format!("{what} impulse {:?} want {want:08x?}{}", x.impulse, if zero_sign { " (signed zero)" } else { "" }));
                }
            }
            let given = solver::island_apply(&bodies, &touches);
            for (bi, b) in t.bodies.iter().enumerate() {
                for (name, o, got) in [("impulse +368", 368, given[bi].0), ("turning +384", 384, given[bi].1)] {
                    if wv(&b.col0, o) != Vec3::ZERO || !vec_words(got, words3(&b.col1, o)) {
                        bad.push(format!("body {bi} (level {:#x}, class {:08x}) {name}: island {got:?}, recorded {:?} -> {:?}", b.level, b.class, wv(&b.col0, o), wv(&b.col1, o)));
                    }
                }
            }
            let pairs = t.rows.iter().filter(|r| r.bodies[0].is_some() && r.bodies[1].is_some()).count();
            eprintln!("multi-car island, tick {}: {} bodies ({:08x?}), {} rows ({} between two bodies, {} from the snapshot), {} results differ", t.tick, t.bodies.len(), t.bodies.iter().map(|b| b.class).collect::<Vec<_>>(), touches.len(), pairs, t.rows.iter().filter(|r| r.snap).count(), bad.len());
            if open.contains(&t.tick) {
                // (an open tick: only signed zeros of the impulse may differ, see OPEN_ZERO)
                if bad.is_empty() {
                    report.push(format!("tick {}: exact now - take it off the open list", t.tick));
                } else if !bad.iter().all(|b| b.ends_with("(signed zero)")) {
                    report.push(format!("open tick {}: {} differences, first {:#?}", t.tick, bad.len(), &bad[..bad.len().min(16)]));
                }
            } else if !bad.is_empty() {
                report.push(format!("tick {}: {} differences, first {:#?}", t.tick, bad.len(), &bad[..bad.len().min(16)]));
            }
        }
        (report, ticks.len())
    }

    /// No open ticks. (ce.0 86 was one: row 0's impulse +112 (0, -0, 0) with its multipliers all +0 - its 3 x 3 has
    /// denormal entries, and the vector unit flushes them, so the sign of the zero comes from the flushed cofactor;
    /// `solver::rigid_impulse` now makes the impulse in the unit's own operations, as F-66979.)
    const OPEN_ZERO: [u32; 0] = [];

    /// cd.0 110 - 125: the player's R8 (mcCar 0x5732) driven with full throttle into racer 53's car (an AI mcCar,
    /// 0x5743) nose to nose: one island of the two cars, 2 - 3 enabled rows between them (their hull and wheel rows with
    /// the city are in the island's lists, disabled), every tick word for word.
    #[test]
    fn multicar_island_velocity_solve_is_the_originals() {
        let (report, n) = multicar_island(include_bytes!("testdata/carcar_cd0.bin"), &[]);
        assert_eq!(n, 16);
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    /// ce.0 69 - 92: the player's R8 (mcCar 0x572a) let go at 25 m/s nose to nose into racer 53's car (AI mcCar 0x5763),
    /// then full throttle: one island of the two cars, 1 - 5 enabled rows between them, and on 70 - 75 the player's hull
    /// row with the city (the car A) in the same solve (the game's level step builds it: vehicles.rs `step_cars`). Every tick word for word.
    #[test]
    fn two_cars_and_city_island_velocity_solve_is_the_originals() {
        let (report, n) = multicar_island(include_bytes!("testdata/carcar_ce0.bin"), &OPEN_ZERO);
        assert_eq!(n, 24);
        let city = multicar_ticks(include_bytes!("testdata/carcar_ce0.bin")).iter().filter(|t| (0..t.bodies.len()).any(|b| !t.own_rows(b).is_empty())).count();
        assert_eq!(city, 6);
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    /// cc.4 141: an ambient car (0x56d4) pushed into racer 7's car (mcCar 0x571a): one enabled row between them.
    #[test]
    fn ambient_car_island_velocity_solve_is_the_originals() {
        let (report, n) = multicar_island(include_bytes!("testdata/carcar_cc4.bin"), &[]);
        assert_eq!(n, 1);
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    /// The loader hands every row once, each in its recorded place, and every car has a body.
    #[test]
    fn multicar_loader_is_consistent() {
        for t in multicar_ticks(include_bytes!("testdata/carcar_ce0.bin")).into_iter().chain(multicar_ticks(include_bytes!("testdata/carcar_cd0.bin"))).chain(multicar_ticks(include_bytes!("testdata/carcar_cc4.bin"))) {
            assert!(t.cars().len() >= 2, "tick {}: {} cars", t.tick, t.cars().len());
            let own: usize = (0..t.bodies.len()).map(|b| t.own_rows(b).len()).sum();
            assert_eq!(own + t.pair_rows().len(), t.rows.len());
            assert!(t.rows.iter().enumerate().all(|(k, r)| r.order == k));
        }
    }

    /// The live path of an island of two cars (vehicles.rs `step_cars` -> loose.rs): the car on the record's A side of
    /// the pair (`a_lent`; else the one on its B side) is the lent car - its part (the pair's rows as it names them, the car its own side, the other car an
    /// object) taken out of its level (`take_car`) and lent into the other car's (`lend_extra`, with its contacts with the
    /// city and its solver body), the other car the stepping one (the island's `Car`, its contacts with the city its own
    /// rows); then `islands_again`, `car_island` and `solve_island` as the stepping car's step runs them. Answers per
    /// fixture the ticks checked and the report: isl +40, every row's +128 / +112 in the solve's order against the
    /// recorded A list, the stepping car's +368 / +384 (`IslandOut`) and the lent car's (`return_extras`), word for word.
    ///
    /// `push`: then also the island's push solve as the stepping car's step runs it (`push_island`, the stepping car's
    /// push and turn as its +400 / +416 held at kind 4, the lent car's turn through `lend_extra`): every row's push
    /// multipliers +0, both cars' +400 / +416 and deep byte +658 after it (kind 5, `return_extras` for the lent car), and
    /// each side's whole bound as the thickness takes it (type 6 - 12: margin +112 about the origin, no core), word for
    /// word; ticks without the push snapshot (RGCAR001) are not counted.
    fn live_two_cars(fixture: &[u8], a_lent: bool, push: bool) -> (usize, Vec<String>) {
        use crate::physics::loose::{CarDyn, CarIn, ExtraCar, LooseState, LooseWorld, Mani, Obj, PBody, Slot2, VehObj, Who};
        use crate::physics::solver::Pushed;
        let vb = |w: [u32; 3]| Vec3::new(f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2]));
        let shape: &'static crate::physics::hull::HullShape = Box::leak(Box::new(crate::physics::hull::HullShape::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z], &[[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]])));
        let mut report = Vec::new();
        let mut n = 0;
        for t in multicar_ticks(fixture) {
            if t.bodies.len() != 2 || t.cars().len() != 2 || (push && t.rows.iter().any(|r| r.pushed.is_none())) {
                continue;
            }
            let Some(&RowJoin::Pair { a, b }) = t.pair_rows().first().map(|r| &r.join) else { continue };
            let (extra, lead) = if a_lent { (a, b) } else { (b, a) };
            if t.rows.iter().any(|r| matches!(r.join, RowJoin::Own { body_is_a: false, .. })) {
                report.push(format!("tick {}: a row with a car on its B side and no body on A - not fed", t.tick));
                continue;
            }
            n += 1;
            let obj = |k: usize, addr: i64| {
                let b = &t.bodies[k];
                let sb = b.body;
                let body = PBody { at: sb.at, rows: sb.rows, v: sb.vel, w: sb.ang, p: Vec3::ZERO, l: Vec3::ZERO, force: sb.force, torque: sb.torque, jf: Vec3::ZERO, jt: Vec3::ZERO, push: Vec3::ZERO, turn: Vec3::ZERO, last: b.matrix, prev: b.matrix, last_set: true, deep: false, deep_count: 0, mass: 0.0, inv_mass: sb.inv_mass, inertia: Vec3::ZERO, inv_inertia: sb.inv_moments, max_speed: f32::MAX, max_spin: f32::MAX, moves: 0 };
                let veh = VehObj { shape, shape2: None, class: 0, gravity: 1.0, cg: Vec3::ZERO, index: b.level, here: true, inflict: 1.0, rider_touch: None, pair: true };
                Obj { ty: 0, inst: None, matrix: b.matrix, entry: None, hit: false, body: Some(body), level: b.level as i64, addr, veh: Some(veh), sleep: crate::physics::loose::Sleep::reset(), dormant: false, amb: None }
            };
            // (the lent car's level: the stepping car object 0, the lent car object 1; its part the pair's manifolds -
            // one per pair of parts, its own side the car)
            let mut ls = LooseState::default();
            // (the stepping car's instance is at 0 - `car_place` with no hints - the lent car's below or above it as
            // their instances' addresses are: the island's lists are sorted by them)
            ls.objs.push(obj(lead, 0));
            ls.objs.push(obj(extra, if t.bodies[extra].inst < t.bodies[lead].inst { -1 } else { 1 }));
            let who = |k: usize| if k == extra { Who::Car } else { Who::Obj(0) };
            for r in t.pair_rows() {
                let RowJoin::Pair { a, b } = r.join else { unreachable!() };
                let (wa, wb, pa, pb) = (who(a), who(b), r.parts[0] as u16, r.parts[1] as u16);
                let k = match ls.manis.iter().position(|m| m.a == wa && m.b == wb && m.part_a == pa && m.part_b == pb) {
                    Some(k) => k,
                    None => {
                        ls.manis.push(Mani { a: wa, b: wb, part_a: pa, part_b: pb, cell: 0, stale: false, visited: true, island: 0, order: 0, slots: Vec::new(), rope: false, owner: 0 });
                        ls.manis.len() - 1
                    }
                };
                let surface = crate::physics::collision::Surface::Road;
                ls.manis[k].slots.push(Slot2 { t: r.touch, age: 1, enabled: true, element: 0, surface, surface_a: surface });
            }
            let part = ls.take_car();
            let own = |k: usize| -> (Vec<solver::Contact>, Vec<u8>) { (t.own_rows(k).iter().map(|r| r.contact().unwrap()).collect(), t.own_rows(k).iter().map(|r| r.parts[0] as u8).collect()) };
            let (ecity, epart) = own(extra);
            // (its push solve's turn as it begins: what its +416 holds; its +400 is cleared by its own slot 14, F-32811)
            let mut e = ExtraCar::new(t.bodies[extra].body, Vec3::ZERO, Vec3::ZERO, vb(t.bodies[extra].push_start[1]), ecity, epart);
            e.uid = t.bodies[extra].level;
            e.obj = 1;
            ls.lend_extra(e, part, Some(0));
            ls.lead = Some(0);
            // (the stepping car's step)
            let (city, city_part) = own(lead);
            ls.islands_again(!city.is_empty());
            let ids = ls.car_island();
            let mut car = t.bodies[lead].body;
            let world = LooseWorld::default();
            let m4 = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO];
            let car_in = CarIn { hull: shape, hull_cg: Vec3::ZERO, hull_margin: 0.04, now: m4, last: m4, speed: 0.0, bound: (Vec3::ZERO, Vec3::ZERO), wheels: Vec::new(), vel: Vec3::ZERO, touch: (0.0, 0.0), wiped: false };
            let (mut p, mut l) = (Vec3::ZERO, Vec3::ZERO);
            let mut dynm = CarDyn { p: &mut p, l: &mut l, mass: 0.0, inverse: Vec3::ZERO, rows: car.rows };
            let city_rows = vec![true; city.len()];
            let mut out = ls.solve_island(&world, &car_in, &mut car, &mut dynm, &city, &city_rows, &city_part, &ids, t.dt, None);
            let mut bad = Vec::new();
            let mut lead_pushed = None;
            if push {
                if t.bodies[extra].push_start[0] != [0; 3] {
                    bad.push(format!("lent car: +400 {:08x?} as the push begins (lend_extra starts it from zero)", t.bodies[extra].push_start[0]));
                }
                // (each side's whole bound as F-49764 reads it: types 6 - 12 (a composite 12, the city's grid) the margin
                // +112 about the bound's origin)
                for (k, (x, r)) in out.touches.iter().zip(t.rows.iter()).enumerate() {
                    for (name, side, (ty, margin)) in [("A", &x.a, r.bound.unwrap()[0]), ("B", &x.b, r.bound.unwrap()[1])] {
                        // (0xfffffffe: the bound's page is in no snapshot - the city's grid; not checked)
                        if ty == 0xffff_fffe {
                            continue;
                        }
                        if !(6..=12).contains(&ty) || side.margin.to_bits() != margin.to_bits() || side.core != Vec3::ZERO {
                            bad.push(format!("row {k} ({:?}) side {name}: bound type {ty} margin {margin:e}, port margin {:e} core {:?}", r.join, side.margin, side.core));
                        }
                    }
                }
                let start = Pushed { push: vb(t.bodies[lead].push_start[0]), turn: vb(t.bodies[lead].push_start[1]), deep: false };
                lead_pushed = Some(ls.push_island(&mut out, &car, start, t.dt, None));
                for (k, (x, r)) in out.touches.iter().zip(t.rows.iter()).enumerate() {
                    if !vec_words(x.pushed, r.pushed.unwrap()) {
                        bad.push(format!("row {k} ({:?}) pushed {:?} want {:08x?}", r.join, x.pushed, r.pushed.unwrap()));
                    }
                }
            }
            ls.lead = None;
            if out.solved.bounce.to_bits() != t.bounce {
                bad.push(format!("bounce {:08x} want {:08x}", out.solved.bounce.to_bits(), t.bounce));
            }
            if out.touches.len() != t.rows.len() {
                bad.push(format!("{} rows, recorded {}", out.touches.len(), t.rows.len()));
            }
            for (k, (x, r)) in out.touches.iter().zip(t.rows.iter()).enumerate() {
                if !vec_words(x.kept, r.kept) {
                    bad.push(format!("row {k} ({:?}) kept {:?} want {:08x?}", r.join, x.kept, r.kept));
                }
                if !vec_words(x.impulse, r.impulse) {
                    bad.push(format!("row {k} ({:?}) impulse {:?} want {:08x?}", r.join, x.impulse, r.impulse));
                }
            }
            let back = ls.return_extras(Some(0));
            let lent = back.iter().find(|(e, _)| e.obj == 1).map(|(e, _)| (e.impulse, e.turning));
            if push {
                let lent_pushed = back.iter().find(|(e, _)| e.obj == 1).map(|(e, _)| e.pushed);
                for (who, k, got) in [("stepping car", lead, lead_pushed), ("lent car", extra, lent_pushed)] {
                    let (Some(got), Some((want, deep))) = (got, t.bodies[k].push_end) else {
                        bad.push(format!("{who}: no push"));
                        continue;
                    };
                    for (name, g, w) in [("push +400", got.push, want[0]), ("turn +416", got.turn, want[1])] {
                        if !vec_words(g, w) {
                            bad.push(format!("{who} {name}: island {g:?}, recorded {w:08x?} (from {:08x?})", if name.starts_with("push") { t.bodies[k].push_start[0] } else { t.bodies[k].push_start[1] }));
                        }
                    }
                    if got.deep != deep {
                        bad.push(format!("{who} deep +658: island {}, recorded {deep}", got.deep));
                    }
                }
            }
            for (who, k, got) in [("stepping car", lead, Some((out.impulse, out.turning))), ("lent car", extra, lent)] {
                let Some(got) = got else {
                    bad.push(format!("{who}: no answer"));
                    continue;
                };
                for (name, g, want) in [("impulse +368", got.0, t.bodies[k].share[0]), ("turning +384", got.1, t.bodies[k].share[1])] {
                    if !vec_words(g, want) {
                        bad.push(format!("{who} {name}: island {g:?}, recorded {want:08x?}"));
                    }
                }
            }
            eprintln!("live two-car island{}, tick {}: {} rows ({} of the stepping car with the city, {} of the lent car), {} results differ", if push { " (with the push)" } else { "" }, t.tick, t.rows.len(), city.len(), t.own_rows(extra).len(), bad.len());
            if !bad.is_empty() {
                report.push(format!("tick {}: {} differences, first {:#?}", t.tick, bad.len(), &bad[..bad.len().min(12)]));
            }
        }
        (n, report)
    }

    /// carpush_pg (recorder kind 5, work/ragdoll/carpush/export_cars2.py, RGCAR002): pg.0 29 - 39, the player's R8 (mcCar,
    /// level 0x5723) teleported against racer 53's standing car (AI mcCar 0x5733), 1 - 7 rows between them, on 36 - 38
    /// also the racer's row with the city (the racer then the pair's B and the city row's A); pg.1 49 - 60, the R8
    /// put 8 m in front of the racer's nose and driven into it, 1 - 4 rows. The push solve through the game's path
    /// (`live_two_cars` with the push), with either car lent: every row's push multipliers +0 and both cars' +400 / +416 /
    /// +658 after the push solve (kind 5 at the first slot 14 entry, F-32811), word for word on all 23 ticks; and the
    /// velocity solve of the same ticks. (No tick has a contact deeper than the cars are thick: +658 stays clear, so
    /// the thickness branch's direct push is not exercised.)
    #[test]
    fn two_car_live_island_push_is_the_originals() {
        let fx = &include_bytes!("testdata/carpush_pg.bin")[..];
        for a_lent in [true, false] {
            let (n, report) = live_two_cars(fx, a_lent, true);
            assert!(report.is_empty(), "carpush_pg (A lent {a_lent}): {}", report.join("\n"));
            assert_eq!(n, 23, "carpush_pg");
        }
        // (and the velocity solve of the same ticks)
        for a_lent in [true, false] {
            let (_, report) = live_two_cars(fx, a_lent, false);
            assert!(report.is_empty(), "carpush_pg velocity (A lent {a_lent}): {}", report.join("\n"));
        }
    }

    #[test]
    fn two_car_live_island_is_the_originals() {
        for (name, fx, want) in [("cd.0", &include_bytes!("testdata/carcar_cd0.bin")[..], 16), ("ce.0", &include_bytes!("testdata/carcar_ce0.bin")[..], 24), ("cc.4", &include_bytes!("testdata/carcar_cc4.bin")[..], 1)] {
            for a_lent in [true, false] {
                let (n, report) = live_two_cars(fx, a_lent, false);
                assert!(report.is_empty(), "{name} (A lent {a_lent}): {}", report.join("\n"));
                assert_eq!(n, want, "{name}");
            }
        }
    }
}
