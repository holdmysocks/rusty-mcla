//! The slipstream turbo's object routines compared with the original (tools/leaves_reference.py): each runs the generated
//! code unchanged on flat guest memory; every line of testdata/leaves_ref.txt is "<tag> <input words> -> <output words>"
//! (hex; a word that reads 82xxxxxx is written with an h in front). The object is the original's 80 bytes by words; the
//! port's `SsTurbo` holds only the fields that play reads, so these tests map the words onto it and back.
use super::{SsTurbo, SsTurboTune};
use glam::Vec3;

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

/// Equal as words; any NaN as any NaN (the harness's default NaN is not the console's).
fn same(a: u32, b: u32) -> bool {
    a == b || (f(a).is_nan() && f(b).is_nan())
}

/// The object's words (offset / 4) onto the port's fields. Byte fields lie in the high byte of their word (+40 burning,
/// +41 full: the second byte of that word).
fn from_words(w: &[u32]) -> SsTurbo {
    SsTurbo {
        level: f(w[9]),
        burning: w[10] >> 24 != 0,
        full: (w[10] >> 16) & 0xFF != 0,
        stay: f(w[11]),
        fov: f(w[5]),
        dist: f(w[6]),
        blur: f(w[7]),
        strength: f(w[16]),
        drafter: w[17] as i32,
        leader: w[18] as i32,
        ..Default::default()
    }
}

/// The port's fields against the object's words: +20 fov, +24 dist, +28 blur, +36 level, +40/+41 burning / full, +44 stay,
/// +64 strength, +68 drafter, +72 leader.
fn check_fields(s: &SsTurbo, o: &[u32], what: &str) {
    let pairs = [(5, s.fov.to_bits()), (6, s.dist.to_bits()), (7, s.blur.to_bits()), (9, s.level.to_bits()), (11, s.stay.to_bits()), (16, s.strength.to_bits()),
                 (17, s.drafter as u32), (18, s.leader as u32)];
    for (k, g) in pairs {
        assert!(same(g, o[k]), "{what}: word {k}: {g:08x} against {:08x}", o[k]);
    }
    assert_eq!((o[10] >> 24 != 0, (o[10] >> 16) & 0xFF != 0), (s.burning, s.full), "{what}: flags {:08x}", o[10]);
}

/// F-99345: the defaults. Every word not set is the prefill (+60 and the bytes +33..35, +42, +43, +49..51, +77..79 of
/// the words around the byte fields); the fields the port holds are its `Default`.
#[test]
fn defaults_match_the_original() {
    let mut n = 0;
    for (i, o) in cases("sstdef") {
        let d = SsTurbo::default();
        check_fields(&d, &o, &format!("{i:08x?}"));
        // the words the port has no field for: the vtable, +4 / +8 tune and car (0), +12 owner (-1), +16 (10000.0),
        // +52 (0.5), +56 (0.78), +48 byte and +76 byte; +60 is not written
        assert_eq!((o[0], o[1], o[2], o[3], o[4], o[13], o[14]), (0x8204_70e0, 0, 0, 0xFFFF_FFFF, 0x461c_4000, 0x3f00_0000, 0x3f47_ae14), "{i:08x?}");
        assert_eq!(o[15], i[15], "+60 untouched");
        assert_eq!(o[12] >> 24, 0, "+48");
        assert_eq!(o[19] >> 24, 1, "+76");
        assert_eq!(o[8] >> 24, 0, "+32");
        // (a byte store leaves the rest of its word: +41 .. +43, +33 .. +35, +49 .. +51, +77 .. +79)
        assert_eq!(o[10] & 0x0000_FFFF, i[10] & 0x0000_FFFF);
        assert_eq!(o[8] & 0x00FF_FFFF, i[8] & 0x00FF_FFFF);
        assert_eq!(o[12] & 0x00FF_FFFF, i[12] & 0x00FF_FFFF);
        assert_eq!(o[19] & 0x00FF_FFFF, i[19] & 0x00FF_FFFF);
        n += 1;
    }
    assert_eq!(n, 1000);
}

/// F-21020: the reset; the port's `reset_all` on the fields the prefill gives it.
#[test]
fn reset_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("sstreset") {
        let mut s = from_words(&i);
        s.reset_all();
        check_fields(&s, &o, &format!("{i:08x?}"));
        assert_eq!(o[8] >> 24, 0, "+32");
        // (everything else is the prefill)
        for k in [0, 1, 2, 3, 4, 12, 13, 14, 15, 19] {
            assert_eq!(o[k], i[k], "word {k}");
        }
        n += 1;
    }
    assert_eq!(n, 1000);
}

/// F-68670: fire. The state words are +16 (0), +36 (1.0) and +40 (1); the camera shake, the pad's rumble and the
/// player's stats go through callees the harness stubs, so only the object is compared.
#[test]
fn fire_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("sstfire") {
        let mut s = from_words(&i[..20]);
        s.fire();
        let mut want = i[..20].to_vec();
        want[4] = 0;
        want[9] = 0x3f80_0000;
        want[10] = (1 << 24) | (want[10] & 0x00FF_FFFF);
        for k in 0..20 {
            // +8 (the car) and +12 (the owner) are set by the harness
            if k != 2 && k != 3 {
                assert_eq!(o[k], want[k], "word {k}: {:08x?}", i);
            }
        }
        assert_eq!(s.level.to_bits(), o[9]);
        assert_eq!(s.burning, o[10] >> 24 != 0);
        assert_eq!(s.fires, 1);
        n += 1;
    }
    assert_eq!(n, 1000);
}

fn tune_of(i: &[u32]) -> SsTurboTune {
    SsTurboTune {
        boost_exp: f(i[12]), boost_amount: f(i[13]), use_time: f(i[14]), regen_time: f(i[15]), falloff_time: f(i[16]), charged_falloff_time: f(i[17]),
        stay_full_time: f(i[18]), lose_charge_speed: f(i[19]), fov_out_time: f(i[20]), fov_in_time: f(i[21]), fov: f(i[22]), dist: f(i[23]),
        blur_off_time: f(i[24]), blur_start_amount: f(i[25]), ..Default::default()
    }
}

/// F-60935: the update, its burning half (`burn` and the push along the back row) and its charging half (`charge`). The
/// case words, in order: burning, full, level, stay, fov, dist, blur, strength, drafter, leader, lock bits, dt, the
/// tune's exp, amount, use, regen, falloff, charged falloff, stay full, lose speed, fov out, fov in, fov, dist, blur
/// off, blur start, the chassis' speed, throttle, wheels, wheels with a contact, up y, air time, back row (4), the
/// nitrous test (0), found, strength, index, the drafter still drafting, manager count, the manager's entries (3
/// bits each: present, car, turbo), the player index, the trail, the player flag. The search F-69277 is a stub
/// answering from the case; the car with a bit-set `drafting` found by the chain through the manager's list is what
/// `drafter_drafting` is for the port.
#[test]
fn update_matches_the_original() {
    let (mut burning, mut charging, mut locked, mut drafters) = (0, 0, 0, 0);
    for (i, o) in cases("sstupd") {
        let what = format!("{i:08x?}");
        let t = tune_of(&i);
        let mut s = from_words(&[0, 0, 0, 0, 0, i[4], i[5], i[6], 0, i[2], (i[0] << 24) | (i[1] << 16), i[3], 0, 0, 0, 0, i[7], i[8], i[9], 0]);
        s.trail_target = f(i[44]);
        let dt = f(i[11]);
        if i[0] != 0 {
            burning += 1;
            let upset = super::super::vehsim_words::lying_on_ground(f(i[31]), f(i[30]));
            let k = s.burn(&t, upset, dt);
            let row = Vec3::new(f(i[32]), f(i[33]), f(i[34]));
            let share = i[29] as i32 as f32 / i[28] as i32 as f32;
            let force = ((row * k) * f(i[27])) * share;
            for (l, g) in [force.x, force.y, force.z].into_iter().enumerate() {
                assert!(same(g.to_bits(), o[21 + l]), "{what}: force lane {l}: {:08x} against {:08x}", g.to_bits(), o[21 + l]);
            }
            assert_eq!(o[25], 1, "{what}: one push");
            assert_eq!(o[26], i[45], "{what}: the player's pad rumble");
        } else {
            charging += 1;
            s.locked = i[10] & 0x10 != 0;
            locked += s.locked as i32;
            s.found = if i[37] != 0 { (f(i[38]), i[39] as i32) } else { (0.0, -1) };
            // the drafter's chain: the manager's list, the entry, its car, its turbo - and the search for it
            let d = i[8] as i32;
            s.drafter_drafting = d >= 0 && d < i[41] as i32 && (i[42] >> (3 * d)) & 7 == 7 && i[40] != 0;
            drafters += (d != -1 && s.drafter_drafting) as i32;
            s.charge(&t, f(i[26]), dt);
            assert_eq!(o[25], 0, "{what}: no push");
        }
        check_fields(&s, &o, &what);
        assert!(same(s.trail_target.to_bits(), o[20]), "{what}: own trail {:08x} against {:08x}", s.trail_target.to_bits(), o[20]);
    }
    assert!(burning >= 900 && charging >= 900 && locked >= 100 && drafters >= 50, "{burning} {charging} {locked} {drafters}");
}
