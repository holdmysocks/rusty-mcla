//! Small car-part functions against the original's own code (tools/carparts_small_reference.py: the unchanged
//! generated functions run natively on a flat guest memory; the recorded results are in carparts_ref/).
//! Evidence: docs/reference/carparts/verify_small_evidence.md.

use crate::damagemap::CarDamage;
use crate::damageparts::Flap;
use glam::Vec3;

fn f(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

/// F-76270 (drawn angle) and F-15658 (last frame's angle): 207 cases over the reversed / locked bytes and
/// special angles (0, -0, inf, denormal).
#[test]
fn flap_drawn_angles_match_the_original() {
    let mut n = 0;
    for l in include_str!("carparts_ref/flap_drawn.txt").lines() {
        let t: Vec<&str> = l.split_whitespace().collect();
        let mut fl = Flap::new(0.2, 1);
        fl.reversed = t[0] != "0";
        fl.locked = t[1] != "0";
        fl.angle = f(t[2]);
        fl.prev_angle = f(t[3]);
        assert_eq!(fl.drawn().to_bits(), f(t[4]).to_bits(), "drawn: {l}");
        assert_eq!(fl.drawn_prev().to_bits(), f(t[5]).to_bits(), "drawn_prev: {l}");
        n += 1;
    }
    assert_eq!(n, 207);
}

/// F-63094 (a hit on a deform piece): 3000 cases, distances on, just under and just over the radius.
#[test]
fn piece_hit_matches_the_original() {
    let mut n = 0;
    for l in include_str!("carparts_ref/pieces_hit.txt").lines() {
        let t: Vec<&str> = l.split_whitespace().collect();
        let kind: u8 = t[0].parse().unwrap();
        let amount = f(t[1]);
        let v: Vec<f32> = t[2..13].iter().map(|s| f(s)).collect();
        let mut d = CarDamage::default();
        let p = &mut d.pieces[0];
        p.local = Vec3::new(v[3], v[4], v[5]);
        p.radius = v[6];
        p.hits = v[7];
        p.scrapes = v[8];
        p.hit_min = v[9];
        p.scrape_min = v[10];
        d.pieces_hit(Vec3::new(v[0], v[1], v[2]), amount, kind);
        assert_eq!(d.pieces[0].hits.to_bits(), f(t[13]).to_bits(), "hit sum: {l}");
        assert_eq!(d.pieces[0].scrapes.to_bits(), f(t[14]).to_bits(), "scrape sum: {l}");
        n += 1;
    }
    assert_eq!(n, 3000);
}

/// F-38353 (the vinyl record's half floats): all 65536 values, including the original's quirks (denormals are
/// not denormal, 0x8000 is -2^-15, the exponent 31 is an ordinary exponent).
#[test]
fn vinyl_half_matches_the_original() {
    let bin = include_bytes!("carparts_ref/half_original.bin");
    for h in 0..=65535u32 {
        let want = u32::from_be_bytes(bin[h as usize * 4..h as usize * 4 + 4].try_into().unwrap());
        let got = crate::vinyl::half(&(h as u16).to_be_bytes());
        assert_eq!(got.to_bits(), want, "half {h:#06x}");
    }
}

/// F-50602 and F-74993 (the damage map's reset and repair requests) as the original's code writes them
/// (tools/carparts_setters_reference.py, output in work/verify-small/carparts/setters_out.txt: reset writes
/// +2150 = 1 and +2154 = 0 and nothing else; repair writes +2149 = 1 and nothing else).
#[test]
fn damage_map_requests_match_the_original() {
    let mut d = CarDamage::default();
    d.stamped = true;
    d.reset();
    assert!(d.reset && !d.stamped, "reset: +2150 = 1, +2154 = 0");
    let mut d = CarDamage::default();
    d.stamped = true;
    d.repair();
    assert!(d.repair && !d.reset && d.stamped, "repair: +2149 = 1 only (the flag the map's next draw takes)");
}
