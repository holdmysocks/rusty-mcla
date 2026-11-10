//! Small routines of the vehicle simulation compared with the original (tools/vehsim_small_reference.py): each runs the
//! generated code unchanged on flat guest memory; every line of testdata/vehsim_small_ref.txt is
//! "<tag> <input words> -> <output words>" (hex; a word that reads 82xxxxxx is written with an h in front). A NaN is
//! compared only as a NaN (the harness's default NaN is not the console's).
use super::loose_words::Words;
use super::vehsim_words as vw;
use mcla_formats::rage::simtune::EngineScalars;

fn words(s: &str) -> Vec<u32> {
    s.split_whitespace().map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap()).collect()
}

fn cases(tag: &'static str) -> impl Iterator<Item = (Vec<u32>, Vec<u32>)> {
    include_str!("../testdata/vehsim_small_ref.txt").lines().filter_map(move |l| {
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

fn f(w: u32) -> f32 {
    f32::from_bits(w)
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// F-86064: the idle torque without an idle curve (the curve branch is taken, and counted, when the count and the flag
/// are both set).
#[test]
fn idle_torque_matches_the_original() {
    let (mut n, mut curve) = (0, 0);
    for (i, o) in cases("idle") {
        if i[4] != 0 && i[5] != 0 {
            assert_eq!(o[1], 1);
            curve += 1;
            continue;
        }
        assert_eq!(o[1], 0);
        let got = vw::idle_fallback(f(i[1]), f(i[2]), f(i[3]), f(i[0]));
        all_same(&[got.to_bits()], &o[..1], &format!("{i:08x?}"));
        n += 1;
    }
    assert!(n >= 1000 && curve >= 100, "{n} {curve}");
}

/// F-48349: the torque for a pedal position, the torque source and the idle torque (real F-86064) given.
#[test]
fn pedal_torque_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("pedal") {
        let (w, pedal) = (f(i[0]), f(i[1]));
        let curve = i[11] != 0 && i[12] != 0;
        let torque = f(if curve { i[6] } else { i[7] });
        let got = vw::pedal_torque(pedal, f(i[2]), f(i[3]), f(i[4]), f(i[5]), || torque, || vw::idle_fallback(f(i[10]), f(i[8]), f(i[9]), w));
        all_same(&[got.to_bits()], &o[..1], &format!("{i:08x?}"));
        if pedal > 0.5 {
            assert_eq!((o[1], o[2]), (if curve { 1 } else { 2 }, i[0]), "{i:08x?}");
        } else {
            assert_eq!(o[1], 0);
        }
        n += 1;
    }
    assert_eq!(n, 3000);
}

/// F-31519: the aero's drag numbers. The original squares the top speed with the C library's pow (a logarithm and
/// exponential in double precision); the port runs the same pow (`caraudio::pow_guest`), so the cases where the square
/// lies exactly half way between two floats agree too (more of those in hyd_refresh_tests).
#[test]
fn drag_init_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("drag") {
        let got = vw::drag_numbers(f(i[0]), f(i[3]), f(i[1]), f(i[2]));
        all_same(&bits(&got), &o, &format!("{i:08x?}"));
        n += 1;
    }
    assert_eq!(n, 3000);
}

/// F-75615: lying on the ground.
#[test]
fn lying_on_ground_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("lying") {
        let want = i[2] != 0 && vw::lying_on_ground(f(i[0]), f(i[1]));
        assert_eq!(o[0], want as u32, "{i:08x?}");
        n += 1;
    }
    assert_eq!(n, 2000);
}

/// F-60656 and F-10823: the hydraulics' loads for every wheel (the wheel setter, F-51861, a stub that records its
/// argument) and the wheel tunes.
#[test]
fn hydraulic_loads_and_tunes_match_the_original() {
    let (mut n, mut ran) = (0, 0);
    for tag in ["hydload", "hydtune"] {
        for (i, o) in cases(tag) {
            let weights: Vec<f32> = i[6..6 + (i[5] as usize).min(4)].iter().map(|&x| f(x)).collect();
            let want = vw::hyd_wheel_loads(i[0] & 0xFF != 0, i[1] != 0, i[2] != 0, f(i[3]), f(i[4]), &weights);
            assert_eq!((o[0], o[1]), (want.len() as u32, want.len() as u32), "{tag} {i:08x?}");
            let mut got = bits(&want);
            got.resize(4, 0);
            all_same(&got, &o[2..6], &format!("{tag} {i:08x?}"));
            if tag == "hydtune" {
                let t = vw::hyd_wheel_tune(f(i[10]), f(i[11]));
                let t = bits(&t);
                all_same(&t, &o[6..10], "front tune");
                all_same(&t, &o[10..14], "back tune");
            }
            ran += !want.is_empty() as usize;
            n += 1;
        }
    }
    assert_eq!(n, 6000);
    assert!(ran > 1000, "{ran}");
}

/// The guest's words from address 0 to 0x3000.
struct Mem(Vec<u32>);

impl Words for Mem {
    fn get(&self, a: u32) -> u32 {
        self.0[(a / 4) as usize]
    }
    fn set(&mut self, a: u32, v: u32) {
        self.0[(a / 4) as usize] = v;
    }
}

/// F-42631: vehStuck's constructor leaves the other words; VehState starts with the same values.
#[test]
fn stuck_ctor_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("stuck") {
        let mut m = Mem(vec![0; 0xC00]);
        for (k, &x) in i.iter().enumerate() {
            m.set(0x1000 + 4 * k as u32, x);
        }
        super::vehsim_words::stuck_ctor(&mut m, 0x1000);
        let got: Vec<u32> = (0..20).map(|k| m.get(0x1000 + 4 * k)).collect();
        assert_eq!(got, o);
        n += 1;
    }
    assert_eq!(n, 1000);
    let vs = super::vehsim::VehState::default();
    assert_eq!((vs.stuck, vs.stuck_at, vs.stuck_timer.to_bits()), (0, glam::Vec3::ZERO, 0));
}

fn v4(i: &[u32]) -> [f32; 4] {
    [f(i[0]), f(i[1]), f(i[2]), f(i[3])]
}

/// F-96311: a force at a point.
#[test]
fn force_at_a_point_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("fpoint") {
        let rows = [v4(&i[12..16]), v4(&i[16..20]), v4(&i[20..24]), v4(&i[24..28])];
        let (force, torque) = vw::force_at_point(&rows, v4(&i[8..12]), v4(&i[0..4]), v4(&i[4..8]));
        all_same(&bits(&force), &o[0..4], &format!("force {i:08x?}"));
        all_same(&bits(&torque), &o[4..8], &format!("torque {i:08x?}"));
        assert_eq!(o[8], 2);
        n += 1;
    }
    assert_eq!(n, 3000);
}

/// F-29993: the body's pending turn and push.
#[test]
fn turn_and_push_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("thelper") {
        let (first, second) = vw::turn_and_push(v4(&i[0..4]), v4(&i[4..8]));
        all_same(&bits(&first), &o[0..4], &format!("slot 28 {i:08x?}"));
        all_same(&bits(&second), &o[4..8], &format!("slot 27 {i:08x?}"));
        assert_eq!(&o[8..12], &i[8..12], "the constant vector");
        assert_eq!((o[12], o[13], o[14]), ((0x8286u32 << 16) | 0x08D0, 0, 2));
        n += 1;
    }
    assert_eq!(n, 3000);
}

/// F-61274: a suspension lever's reciprocals.
#[test]
fn lever_init_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("lever") {
        let mut b = [0.0f32; 16];
        for k in 0..16 {
            b[k] = f(i[1 + k]);
        }
        let (a100, a104) = vw::lever_reciprocals((i[0] != 0).then_some(&b), v4(&i[17..21]));
        all_same(&[a100.to_bits(), a104.to_bits()], &o, &format!("{i:08x?}"));
        n += 1;
    }
    assert_eq!(n, 3000);
}

/// F-23005: the engine's init after its constructor (F-48129) on tunes with and without control values.
#[test]
fn engine_init_matches_the_original() {
    let (mut n, mut curves) = (0, 0);
    for (i, o) in cases("init") {
        let e = EngineScalars { max_hp: f(i[0]), idle_rpm: f(i[1]), opt_rpm: f(i[2]), max_rpm: f(i[3]), ..Default::default() };
        let count = i[6] as usize;
        let torque: Vec<f32> = i[7..7 + count].iter().map(|&x| x as i32 as f32).collect();
        let [max, best, idle, c84, c88, t] = vw::engine_init(&e, &torque);
        all_same(&bits(&[max, best, idle, c84, c88, t]), &o[..6], &format!("{i:08x?}"));
        assert_eq!(o[6], 0, "+28 is put back");
        assert_eq!(o[7], if count > 0 { 12 } else { 0 });
        curves += (count > 0) as usize;
        n += 1;
    }
    assert_eq!(n, 2500);
    assert!(curves > 1000);
}

/// F-68237: the car's centre of mass settles against fixed walls (the contact list, the instances and the level table
/// are in guest memory; the two list helpers are the real ones).
#[test]
fn settle_cg_matches_the_original() {
    let (mut n, mut moved) = (0, 0);
    for (i, o) in cases("settle") {
        let count = i[0] as usize;
        let touches: Vec<vw::SettleTouch> = (0..count).map(|k| {
            let b = 8 + 8 * k;
            vw::SettleTouch { enabled: i[b] & 0x80 != 0, fixed: i[b + 2] == 2 && i[b + 3] & 3 == 2, normal_y: f(i[b + 5]) }
        }).collect();
        let (cg, reset) = vw::settle_cg(v4(&i[1..5]), f(i[5]), f(i[6]), &touches, i[7] & 0x10 != 0);
        all_same(&bits(&cg), &o[..4], &format!("{i:08x?}"));
        assert_eq!(o[4], if reset { 0 } else { 0xA5A5A5A5 }, "{i:08x?}");
        moved += (cg[1].to_bits() != i[2]) as usize;
        n += 1;
    }
    assert_eq!(n, 3000);
    assert!(moved > 300, "{moved}");
}

/// F-32386: the thrown rider's term of the damage object's frame update (the harness finds it by searching the one
/// constant it is compared with; the sentinel is a NaN or a term not under the largest float).
#[test]
fn rider_term_matches_the_original() {
    let (mut n, mut terms) = (0, 0);
    for (i, o) in cases("rider") {
        let found = i[0] != 0 && i[1] != 0 && f(i[2]) > -75.0 && f(i[2]) < 400.0;
        let term = if found { super::vehsim::rider_term(glam::Vec3::new(f(i[3]), f(i[4]), f(i[5]))) } else { 0.0 };
        let want = if term.is_nan() || !(term < f32::MAX) { 0xFFFF_FFFF } else { term.to_bits() };
        assert_eq!(o[0], want, "{i:08x?}");
        terms += found as usize;
        n += 1;
    }
    assert_eq!(n, 3000);
    assert!(terms > 1000, "{terms}");
}

/// F-72737: the speeds a car is set down with (the gear for the speed is a stub, the gear setter real).
#[test]
fn set_down_speeds_match_the_original() {
    let (mut n, mut engines) = (0, 0);
    for (i, o) in cases("setdown") {
        let fwd = vw::set_down_forward([f(i[3]), f(i[4]), f(i[5])], [f(i[0]), f(i[1]), f(i[2])]);
        let mut shaft_speeds = [None; 2];
        for s in 0..2 {
            // (the output: shaft 0's +48, +52 and its three wheels' +552, then shaft 1's)
            let base = if s == 0 { 0 } else { 5 };
            if i[7 + s] == 0 {
                assert_eq!(&o[base..base + 5], &[0xA5A5A5A5; 5][..],"absent shaft {s} {i:08x?}");
                continue;
            }
            let count = i[9 + s] as usize;
            let radius = (count > 0).then(|| f(i[11 + 3 * s]));
            let v = vw::set_down_shaft(fwd, radius);
            shaft_speeds[s] = Some(v);
            let wheels = |k: usize| if k < count { v.to_bits() } else { 0xA5A5A5A5 };
            all_same(&[v.to_bits(), 1.0f32.to_bits(), wheels(0), wheels(1), wheels(2)], &o[base..base + 5], &format!("shaft {s} {i:08x?}"));
        }
        // the body: slot 41 once, slot 43 once with the case's argument
        assert_eq!((o[13], o[14], o[15]), (1, i[6], 1), "{i:08x?}");
        if let (Some(v), true, true) = (shaft_speeds[0], i[17] != 0, i[18] != 0) {
            assert_eq!((o[11], o[12]), (fwd.to_bits(), 1), "gear stub {i:08x?}");
            let ratio = f(i[25 + (o[16] as i32 + 1) as usize]);
            let e = vw::set_down_engine(ratio, v, f(i[20]), f(i[19]));
            all_same(&[e.to_bits()], &[o[10]], &format!("engine {i:08x?}"));
            engines += 1;
        } else {
            assert_eq!((o[10], o[12]), (0xA5A5A5A5, 0), "{i:08x?}");
        }
        n += 1;
    }
    assert_eq!(n, 3000);
    assert!(engines > 1000, "{engines}");
}
