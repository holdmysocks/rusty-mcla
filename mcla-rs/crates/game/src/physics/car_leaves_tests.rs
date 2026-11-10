//! The car damage object's routines compared with the original (tools/leaves_reference.py): each runs the generated
//! code unchanged on flat guest memory; every line of testdata/leaves_ref.txt is "<tag> <input words> -> <output words>"
//! (hex; a word that reads 82xxxxxx is written with an h in front).
use super::{add_damage_points, damage_record};

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

/// F-15289: AddDamage. The case words: the shield byte of the damage owner, the global byte that suspends damage, the
/// argument (the hit's factor), the object's +12 (blow factor), +40 (damage points), +28, +36, the tune's MaxDamage
/// (+24), WipeoutThreshold (+36) and FatalBlow (+52), whether the owner has its +128 object, and what the three
/// stubbed callees answer. The callees F-76352 and F-74220 receive the 28 byte record: the hit, the new points, then the
/// bytes (wipe, reached the maximum, 0, 0). The object's +40 takes the new points when the last callee answers yes.
#[test]
fn add_damage_matches_the_original() {
    let (mut n, mut early, mut told, mut reached, mut wiped, mut nan) = (0, 0, 0, 0, 0, 0);
    for (i, o) in cases("dmgadd") {
        let what = format!("{i:08x?}");
        let (shielded, suspended) = (i[0] != 0, i[1] != 0);
        if shielded || suspended {
            early += 1;
            assert_eq!((o[0], o[1], o[2]), (i[4], i[5], 0), "{what}: nothing happens");
            continue;
        }
        let (max, wipeout, fatal) = (f(i[7]), f(i[8]), f(i[9]));
        let (hit, new, is_max, wipe) = damage_record(f(i[4]), f(i[3]), f(i[2]), max, fatal, wipeout);
        let record = [hit.to_bits(), new.to_bits(), ((wipe as u32) << 24) | ((is_max as u32) << 16)];
        // the callee that got the record
        let at = if o[2] & 1 != 0 { 3 } else if o[2] & 2 != 0 { 11 } else { usize::MAX };
        if at != usize::MAX {
            told += 1;
            for k in 0..3 {
                assert!(same(record[k], o[at + k]), "{what}: record word {k}: {:08x} against {:08x}", record[k], o[at + k]);
            }
        }
        // +40 takes the new points when the closing virtual call answers yes
        let wants = if i[13] != 0 { new.to_bits() } else { i[4] };
        assert!(same(wants, o[0]) || at == usize::MAX, "{what}: points {wants:08x} against {:08x}", o[0]);
        reached += is_max as i32;
        wiped += wipe as i32;
        nan += (new.is_nan() || hit.is_nan()) as i32;
        // the port's points: the original's rule on the points (the damage fraction times the maximum in the port)
        assert!(add_damage_points(f(i[4]), hit, max, fatal).to_bits() == new.to_bits() || new.is_nan());
        n += 1;
    }
    assert!(n + early >= 1000 && n >= 900 && early >= 100 && told >= 500 && reached >= 100 && wiped >= 50, "{n} {early} {told} {reached} {wiped} {nan}");
}
