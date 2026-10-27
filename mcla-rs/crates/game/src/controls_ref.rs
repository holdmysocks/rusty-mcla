//! Controls rows compared with the original's own code (recompiled functions run offline by
//! `tools/recomp_closure.py`'s shim; the recorded outputs are `testdata/controls_ref.txt`, made by
//! `work/verify-small/CONTROLS_OPTIONS/drv.py`). Evidence: docs/reference/controls/verify_small_evidence.md.
use crate::mc4::{is_analog_slot, stick_float, PadState, SlotMap};

const REF: &str = include_str!("../testdata/controls_ref.txt");

fn lines(tag: &'static str) -> impl Iterator<Item = Vec<&'static str>> {
    REF.lines().filter(move |l| l.starts_with(tag)).map(|l| l.split_whitespace().skip(1).collect())
}

/// F-73799 / F-43402 (the right stick's X and Y bytes as floats): all 256 byte values, bit for bit.
#[test]
fn stick_floats_match_the_original() {
    let mut n = 0;
    for l in lines("S ") {
        let b: u8 = l[0].parse().unwrap();
        for w in &l[1..3] {
            assert_eq!(stick_float(b).to_bits(), u32::from_str_radix(w, 16).unwrap(), "byte {b}");
        }
        n += 1;
    }
    assert_eq!(n, 256);
}

/// F-56931: which slots are the analog actions (slots -3 .. 79).
#[test]
fn analog_slot_predicate_matches_the_original() {
    let mut n = 0;
    for l in lines("P ") {
        let s: i32 = l[0].parse().unwrap();
        let want = l[1] == "1";
        assert_eq!(s >= 0 && is_analog_slot(s as usize), want, "slot {s}");
        n += 1;
    }
    assert_eq!(n, 83);
}

/// F-33017's neighbour in the mapper update: the four stick switches (keys 4..7 left, 12..15 right) with
/// the original's stick floats (above) and its thresholds (-0.8 / 0.8 on the main axis, 0.57 on the other, the
/// compares as the instructions read them: `fcmpu` + `bge`/`ble`). Every pair of bytes.
#[test]
fn stick_switches_follow_the_original_floats() {
    let f: Vec<f32> = (0..=255u8).map(stick_float).collect();
    let mut n = 0;
    for x in 0..=255usize {
        for y in 0..=255usize {
            // left stick: bytes 0 (x) and 1 (y); right: 2 (x) and 3 (y)
            let p = PadState { buttons: 0, triggers: [0; 2], sticks: [x as u8, y as u8, x as u8, y as u8] };
            let other = |v: f32| v.abs() < 0.57;
            // (key, expected): up: y below -0.8; down: y above 0.8; left: x below -0.8; right: x above 0.8
            let want = [
                (4, f[y] < -0.8 && other(f[x])),
                (5, f[y] > 0.8 && other(f[x])),
                (6, f[x] < -0.8 && other(f[y])),
                (7, f[x] > 0.8 && other(f[y])),
            ];
            for (k, w) in want {
                assert_eq!(p.byte(9, k) == 255, w, "left key {k} x {x} y {y}");
                assert_eq!(p.byte(9, k + 8) == 255, w, "right key {} x {x} y {y}", k + 8);
                n += 2;
            }
        }
    }
    assert_eq!(n, 256 * 256 * 8);
}

/// The compare of two mappings (74 x {device, key}): differs at any slot / word -> true. The port's
/// `work != saved` is the same equality on `SlotMap`.
#[test]
fn mapping_compare_matches_the_original() {
    let mut n = 0;
    for l in lines("C ") {
        let t: i32 = l[0].parse().unwrap();
        let want = l[1] == "1";
        let base = |i: usize| ((i as u32 * 7) % 12, (i as u32 * 5) % 40);
        let mut a: SlotMap = [(0, 0); 74];
        let mut b: SlotMap = [(0, 0); 74];
        for i in 0..74 {
            a[i] = base(i);
            b[i] = base(i);
        }
        if t > 0 {
            let k = (t - 1) as usize;
            let (i, word, w) = (k % 74, (k / 74) % 2, (k / 148) % 2);
            let m = (1 + t as u32 % 5) | if t % 7 == 0 { 0x8000_0000 } else { 0 };
            let e = if w == 0 { &mut a[i] } else { &mut b[i] };
            if word == 0 { e.0 ^= m } else { e.1 ^= m }
        }
        assert_eq!(a != b, want, "case {t}");
        n += 1;
    }
    assert_eq!(n, 297);
}
