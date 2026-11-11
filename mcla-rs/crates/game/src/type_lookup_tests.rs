//! The type lookup F-54839 with its name compare F-40755 against the original's code run offline
//! (work/traffic-detail/tylook/off: build.py, main_tylook.cpp, mk.sh): lists of 0 - 6 names with case variants, the bytes
//! beside A - Z and Z - a, two high bytes, a name one byte shorter or longer; the index found or none, from
//! [`type_by_name`].

use super::*;

fn bytes(h: &str) -> Vec<u8> {
    if h == "-" {
        return Vec::new();
    }
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn type_lookup_matches_the_original() {
    let (mut cases, mut hits) = (0, 0);
    for line in include_str!("testdata/type_lookup_ref.txt").lines().filter(|l| !l.starts_with('#')) {
        let (l, want) = line.split_once(" | ").unwrap();
        let want: i32 = want.trim().parse().unwrap();
        let w: Vec<&str> = l.split_whitespace().collect();
        let n: usize = w[0].parse().unwrap();
        let names: Vec<Vec<u8>> = w[1..1 + n].iter().map(|h| bytes(h)).collect();
        let q = bytes(w[1 + n]);
        let got = type_by_name(names.iter().map(|v| v.as_slice()), &q).map_or(-1, |i| i as i32);
        assert_eq!(got, want, "case {cases}: {line}");
        hits += (want >= 0) as i32;
        cases += 1;
    }
    assert!(cases >= 4000 && hits > 1500, "{cases} {hits}");
}
