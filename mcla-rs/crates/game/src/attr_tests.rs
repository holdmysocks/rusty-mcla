//! The tune attributes' readers against the original's code run offline (`tools/attr_reference.py`, data
//! `testdata/attr_ref.txt`: the disc's attribute texts and made-up numbers): the float F-79880 (atof F-53123 rounded to
//! single), the int F-79822 (strtol F-53090 / the base-16 conversion F-63823) and the bool F-83747, bit for bit.
use super::{attr_bool, attr_f32, attr_int};

fn cases(tag: &str) -> Vec<(String, Vec<u32>)> {
    include_str!("testdata/attr_ref.txt")
        .lines()
        .filter(|l| l.split(' ').next() == Some(tag))
        .map(|l| {
            let (i, o) = l[tag.len() + 1..].split_once(" -> ").unwrap();
            let bytes: Vec<u8> = if i == "-" {
                Vec::new()
            } else {
                (0..i.len())
                    .step_by(2)
                    .map(|k| u8::from_str_radix(&i[k..k + 2], 16).unwrap())
                    .collect()
            };
            let s: String = bytes.iter().map(|&b| b as char).collect();
            (
                s,
                o.split_whitespace()
                    .map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap())
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn attr_float_matches_the_original() {
    let c = cases("float");
    assert!(c.len() > 4000);
    let bad: Vec<_> = c
        .iter()
        .filter(|(s, o)| attr_f32(s).to_bits() != o[0])
        .map(|(s, o)| format!("{s:?} {:08x} {:08x}", attr_f32(s).to_bits(), o[0]))
        .collect();
    assert!(
        bad.is_empty(),
        "{} of {}: {:?}",
        bad.len(),
        c.len(),
        &bad[..bad.len().min(12)]
    );
}

#[test]
fn attr_int_matches_the_original() {
    let c = cases("int");
    assert!(c.len() > 4000);
    let bad: Vec<_> = c
        .iter()
        .filter(|(s, o)| attr_int(s) as u32 != o[0] || o[1] != 1)
        .map(|(s, o)| format!("{s:?} {:08x} {:08x?}", attr_int(s), o))
        .collect();
    assert!(
        bad.is_empty(),
        "{} of {}: {:?}",
        bad.len(),
        c.len(),
        &bad[..bad.len().min(12)]
    );
}

#[test]
fn attr_bool_matches_the_original() {
    let c = cases("bool");
    assert!(c.len() > 4000);
    for (s, o) in c {
        assert_eq!(attr_bool(&s) as u32, o[0], "{s:?}");
    }
}
