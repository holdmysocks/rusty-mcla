//! The game's multiply-with-carry draws against the original's code run offline (`tools/rng_reference.py`, data
//! `testdata/rng_ref.txt`): the seed F-37317, RAND_INT_RANGE F-32828, RAND_FLOAT_RANGE F-47737,
//! RAND_INT_RANGE_DIFFERENT F-40328 and the particles' frame draw F-61280, word for word (the result and the
//! generator's two words after the draw). Cases the original traps on are not compared.
use super::Mwc;

fn cases(tag: &str) -> Vec<(Vec<u32>, Vec<u32>)> {
    include_str!("testdata/rng_ref.txt")
        .lines()
        .filter(|l| l.split(' ').next() == Some(tag))
        .filter(|l| !l.ends_with("trap"))
        .map(|l| {
            let (i, o) = l[tag.len() + 1..].split_once(" -> ").unwrap();
            let w = |s: &str| {
                s.split_whitespace()
                    .map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap())
                    .collect::<Vec<_>>()
            };
            (w(i), w(o))
        })
        .collect()
}

#[test]
fn seed_matches_the_original() {
    let c = cases("seed");
    assert_eq!(c.len(), 2000);
    for (i, o) in c {
        let g = Mwc::seed(i[0]);
        assert_eq!(
            [g.lo, g.carry, 0, 0],
            [o[0], o[1], o[2], o[3]],
            "seed {:08x}",
            i[0]
        );
    }
}

#[test]
fn int_range_matches_the_original() {
    let c = cases("int");
    // (2 cases trap: hi - lo + 1 wraps to 0)
    assert_eq!(c.len(), 3998);
    for (i, o) in c {
        let mut g = Mwc {
            lo: i[0],
            carry: i[1],
        };
        let r = g.int_range(i[2] as i32, i[3] as i32);
        assert_eq!(
            [r as u32, g.lo, g.carry],
            [o[0], o[1], o[2]],
            "int {:08x?}",
            i
        );
    }
}

#[test]
fn float_range_matches_the_original() {
    let c = cases("float");
    assert_eq!(c.len(), 4000);
    for (i, o) in c {
        let mut g = Mwc {
            lo: i[0],
            carry: i[1],
        };
        let r = g.float_range(f32::from_bits(i[2]), f32::from_bits(i[3]));
        let same = r.to_bits() == o[0] || (r.is_nan() && f32::from_bits(o[0]).is_nan());
        assert!(
            same && [g.lo, g.carry] == [o[1], o[2]],
            "float {:08x?} -> {:08x} {:08x} {:08x}",
            i,
            r.to_bits(),
            g.lo,
            g.carry
        );
    }
}

#[test]
fn int_range_different_matches_the_original() {
    for tag in ["diff", "ptfx"] {
        let c = cases(tag);
        assert!(c.len() > 3500, "{tag} {}", c.len());
        for (i, o) in c {
            let mut g = Mwc {
                lo: i[0],
                carry: i[1],
            };
            let r = g.int_range_different(i[2] as i32, i[3] as i32, i[4] as i32);
            assert_eq!(
                [r as u32, g.lo, g.carry],
                [o[0], o[1], o[2]],
                "{tag} {:08x?}",
                i
            );
        }
    }
}
