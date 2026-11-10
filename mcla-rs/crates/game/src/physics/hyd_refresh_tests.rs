//! The hydraulics' wheel weights at the car's refresh (F-60656 with the wheels' own set-up F-73903 real) and the drag
//! numbers' square (F-31519 with the C library pow real), against the original's code run offline
//! (tools/hyd_refresh_reference.py). Each line of testdata/hyd_refresh_ref.txt is "<tag> <input words> -> <output words>"
//! (hex; a word that reads 82xxxxxx is written with an h in front). A NaN is compared only as a NaN.
use super::vehsim_words as vw;

fn words(s: &str) -> Vec<u32> {
    s.split_whitespace().map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap()).collect()
}

fn cases(tag: &'static str) -> impl Iterator<Item = (Vec<u32>, Vec<u32>)> {
    include_str!("../testdata/hyd_refresh_ref.txt").lines().filter_map(move |l| {
        let rest = l.strip_prefix(tag)?.strip_prefix(' ')?;
        let (i, o) = rest.split_once(" -> ")?;
        Some((words(i), words(o)))
    })
}

fn same(a: u32, b: u32) -> bool {
    a == b || (f32::from_bits(a).is_nan() && f32::from_bits(b).is_nan())
}

fn f(w: u32) -> f32 {
    f32::from_bits(w)
}

/// F-60656 at the refresh: fitted and (on, or a pattern), every wheel of the count (signed, at most the four built) is
/// weighed by its slot 1 - +432 the tune's SuspensionOffset (+80), +428 the share of ((gravity x body mass) x 9.8) - and
/// then given that share plus clamp(stiffness - 1, 0, 10) x the tune's mass, not under 1. The other wheels keep theirs.
#[test]
fn hydraulic_refresh_weights_match_the_original() {
    const TUNE: [usize; 18] = [4, 8, 12, 16, 24, 28, 56, 60, 64, 76, 80, 84, 88, 96, 100, 112, 116, 120];
    let at = |o: usize| TUNE.iter().position(|&x| x == o).unwrap();
    let (mut n, mut weighed) = (0, 0);
    for (i, o) in cases("hydinit") {
        let gate = i[0] & 0xFF != 0 && (i[1] != 0 || i[2] != 0);
        let count = if gate { (i[5] as i32).clamp(0, 4) as usize } else { 0 };
        let mut want = Vec::new();
        for k in 0..4 {
            let tune = 29 + if k < 2 { 0 } else { TUNE.len() };
            let (x, z, pre428, pre432) = (i[13 + 4 * k], i[14 + 4 * k], i[15 + 4 * k], i[16 + 4 * k]);
            if k < count {
                let share = vw::wheel_share(vw::wheel_init_weight(f(i[6]), f(i[7])), f(x), f(z), f(i[tune + at(8)]), f(i[8]), f(i[10]));
                want.push(vw::hyd_refresh_weight(share, f(i[3]), f(i[4])).to_bits());
                want.push(i[tune + at(80)]);
                weighed += 1;
            } else {
                want.extend([pre428, pre432]);
            }
        }
        want.push(count as u32);
        assert_eq!(want.len(), o.len());
        for (k, (&g, &w)) in want.iter().zip(&o).enumerate() {
            assert!(same(g, w), "{i:08x?}: word {k}: {g:08x} against {w:08x}");
        }
        n += 1;
    }
    assert_eq!(n, 3000);
    assert!(weighed > 4000, "{weighed}");
}

/// F-31519 on the rounding ties of the square (600 found by search), every HighGearMPH of the disc's vehicle tunes and
/// 1,000 random speeds: every number equal.
#[test]
fn drag_with_the_library_pow_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("drag") {
        let got = vw::drag_numbers(f(i[0]), f(i[3]), f(i[1]), f(i[2]));
        for (k, (g, &w)) in got.iter().zip(&o).enumerate() {
            assert!(same(g.to_bits(), w), "{i:08x?}: word {k}: {:08x} against {w:08x}", g.to_bits());
        }
        n += 1;
    }
    assert!(n >= 1600, "{n}");
}

/// Every HighGearMPH of the disc's vehicle tunes (base and mods), one hex word a line, for the reference tool's --mph
/// (MCLA_DATA_DIR = the extracted cache; MCLA_MPH_OUT = the file).
#[test]
#[ignore]
fn high_gear_speeds_of_the_disc() {
    let dir = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR")).join("resources/vehicle");
    let mut v = std::collections::BTreeSet::new();
    let (mut g, mut files) = (f32::INFINITY, 0);
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        for f in std::fs::read_dir(e.path()).unwrap().flatten() {
            if f.path().extension().map_or(false, |x| x == "r50") {
                let Ok(res) = crate::assets::load_resource_file(&f.path()) else { continue };
                let Some(st) = mcla_formats::rage::simtune::SimTunes::parse(&res) else { continue };
                v.insert(st.base.trans.s.high_gear_mph.to_bits());
                v.insert(st.mods.trans.s.high_gear_mph.to_bits());
                // (the wheels' set-up F-73903 weighs with the raw +124, the port's weighing with it where it is above 0.1)
                g = g.min(st.base.s.bound_gravity).min(st.mods.s.bound_gravity);
                files += 1;
            }
        }
    }
    println!("{files} tunes, {} speeds, smallest BoundGravity {g}", v.len());
    assert!(g > 0.1);
    let text: String = v.iter().map(|w| format!("{w:08x}\n")).collect();
    std::fs::write(std::env::var("MCLA_MPH_OUT").expect("MCLA_MPH_OUT"), text).unwrap();
    assert!(!v.is_empty());
}
