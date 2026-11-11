//! The layout item spawn F-94309 against the original's code run offline (work/traffic-detail/item/off: build.py,
//! main_item.cpp, mk.sh): the request it makes (parked with its colour, on the lane with its colour, a person) or none,
//! its submit, the item's +76 / +80 and the generator after, from [`item_spawn_choice`].

use super::*;

#[test]
fn item_spawn_matches_the_original() {
    let text = include_str!("testdata/item_spawn_ref.txt");
    let (mut cases, mut kinds) = (0, [0; 4]);
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let parts: Vec<&str> = line.split(" |").collect();
        let w: Vec<&str> = parts[0].split_whitespace().collect();
        let b = |i: usize| w[i] == "1";
        let (net, t, s6, s7, c5, c6, c8) = (b(0), b(1), b(2), b(3), b(4), b(5), b(6));
        let density = f32::from_bits(u32::from_str_radix(w[7], 16).unwrap());
        let (p, pp, p64, p164, i, i140, ip, i188, i189) = (b(8), b(9), b(10), b(11), b(12), b(13), b(14), b(15), b(16));
        let id: i32 = w[17].parse().unwrap();
        let mut rng = Mwc { lo: w[18].parse().unwrap(), carry: w[19].parse().unwrap() };
        let m972 = w[20];
        let log = parts[1];
        let res: Vec<&str> = parts[2].split_whitespace().collect();
        let tail: Vec<&str> = parts[3].split_whitespace().collect();
        let want_rng: (u32, u32) = (tail[0].parse().unwrap(), tail[1].parse().unwrap());

        // the request the original made, from its stub's line
        let want = log.split_whitespace().find_map(|e| match &e[..1] {
            "P" => Some(ItemSpawn::Parked(e[1..].split(',').next().unwrap().parse().unwrap())),
            "L" => Some(ItemSpawn::Lane(e[1..].split(',').next().unwrap().parse().unwrap())),
            "F" => Some(ItemSpawn::Person),
            _ => None,
        });
        let got = if net || !t {
            None
        } else {
            let inp = ItemSpawnIn {
                car: s6,
                person: !s6 && s7,
                cars_on: c5 && c8,
                peds_on: c6,
                density,
                path: (p && pp).then_some((p64, p164)),
                item: i.then_some((i140, ip.then_some((i188, i189)))),
            };
            item_spawn_choice(inp, &mut rng)
        };
        assert_eq!(got, want, "case {cases}: {line}");
        // the request's own arguments
        if let Some(e) = log.split_whitespace().find(|e| e.starts_with('P') || e.starts_with('L') || e.starts_with('F')) {
            let a: Vec<&str> = e[1..].split(',').collect();
            match got {
                Some(ItemSpawn::Parked(_)) => assert_eq!(&a[1..], [if p { "00032000" } else { "00000000" }, "00034000", m972, "3f800000"], "case {cases}"),
                Some(ItemSpawn::Lane(_)) => {
                    let pa = if p { "00032000" } else { "00000000" };
                    let ia = if i { "00034000" } else { "00000000" };
                    assert_eq!(&a[1..], ["00030000", "00000000", "40400000", "bf800000", "ffffffff", "00000000", "1", pa, ia], "case {cases}")
                }
                _ => assert_eq!(&a, &["00030000", "00034000", m972, "3f800000"], "case {cases}"),
            }
        }
        // submitted with a request; made when its id is not negative, the item's +80 the id and +76 cleared
        assert_eq!(log.contains(" B"), got.is_some(), "case {cases}");
        let made = got.is_some() && id >= 0;
        assert_eq!(res[0] == "1", made, "case {cases}");
        if got.is_some() && i {
            assert_eq!((res[1], res[2].parse::<i32>().unwrap()), ("0", id), "case {cases}");
        } else {
            assert_eq!((res[1], res[2]), ("85", "102"), "case {cases}");
        }
        assert_eq!((rng.lo, rng.carry), want_rng, "case {cases} generator");
        kinds[match got {
            None => 0,
            Some(ItemSpawn::Parked(_)) => 1,
            Some(ItemSpawn::Lane(_)) => 2,
            Some(ItemSpawn::Person) => 3,
        }] += 1;
        cases += 1;
    }
    assert!(cases >= 4000 && kinds.iter().all(|&k| k > 200), "{cases} {kinds:?}");
}
