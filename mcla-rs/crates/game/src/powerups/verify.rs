//! The POWERUPS verification pass of 2026-11-09 (session puverify): functions of the power-up code run offline from the original's
//! own generated code (tools/pu_*_reference.py: recomp_dispatch, the unchanged code and its callee closure) and compared with
//! the port bit for bit. One test per tool; the recorded outputs are committed under testdata/, so the tests run without the recomp.
use super::*;

/// A data word; `h` in front marks one that reads 82xxxxxx (tools/cite_safe.py: kept apart from address citations).
fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.strip_prefix('h').unwrap_or(s), 16).unwrap()
}

/// F-14458 (a source's type roll, with F-52455 the n-th allowed type): "<ok> <bits> <r> <was> -> <+136 type> <+144 mystery>"
/// (tools/pu_roll_reference.py, testdata/pu_roll_ref.txt). A negative number (the word's top bit set) finds no type in the original
/// (F-52455 compares the index unsigned: -1) - the port's numbers never give one (`PowerUps::rand` masks 0x7fffffff, and `reset`
/// adds the source index to it, wrapping), so those lines are only counted.
#[test]
fn roll_matches_the_original() {
    let (mut n, mut neg) = (0, 0);
    for line in include_str!("../testdata/pu_roll_ref.txt").lines() {
        let (a, b) = line.split_once(" -> ").unwrap();
        let a: Vec<u32> = a.split(' ').map(hex).collect();
        let b: Vec<u32> = b.split(' ').map(hex).collect();
        let (ok, bits, r, was) = (a[0] != 0, a[1], a[2] as i32, a[3] != 0);
        // (a negative remainder: the original's index compare is unsigned, nothing is found, -1)
        if bits.count_ones() != 0 && (r >> 2) % bits.count_ones() as i32 <= -1 {
            assert_eq!(b[0], 0xFFFF_FFFF, "{line}");
            neg += 1;
            continue;
        }
        let (k, m) = roll(bits, ok, r, was);
        assert_eq!((k as u32, m as u32), (b[0], b[1]), "{line}");
        n += 1;
    }
    println!("roll: {n} compared, {neg} negative numbers not found by the original");
}

fn v3(w: &[u32]) -> Vec3 {
    Vec3::new(f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2]))
}

/// A frame from its twelve words: three rows, then the place.
fn frame(w: &[u32]) -> [Vec3; 4] {
    [v3(&w[0..3]), v3(&w[3..6]), v3(&w[6..9]), v3(&w[9..12])]
}

/// F-47137 (a target's rank, with the frame inverse F-43153 / F-30106 it calls) and F-33277 (the target search, the log call and the
/// team getter stubbed as the tool says), tools/pu_targets_reference.py -> testdata/pu_targets_ref.txt:
///   R <width> <effect> <frame 12> <target place 3> -> <rank>
///   T <mega> <back> <range> <width> <n> {<effect> <frame 12>} x n -> <found> <count> <car indexes>
/// The rank is compared bit for bit (a NaN would compare as its pattern); the search is compared by the cars it returns.
#[test]
fn rank_and_targets_match_the_original() {
    let (mut ranks, mut searches, mut found) = (0, 0, 0);
    for line in include_str!("../testdata/pu_targets_ref.txt").lines() {
        let (a, b) = line.split_once(" -> ").unwrap();
        let w: Vec<u32> = a.split(' ').skip(1).map(hex).collect();
        let want: Vec<u32> = b.split(' ').map(hex).collect();
        if a.starts_with("R ") {
            let got = rank_frame(&frame(&w[2..14]), v3(&w[14..17]), f32::from_bits(w[0]), w[1] == 3);
            assert_eq!(got.to_bits(), want[0], "{line}");
            ranks += 1;
        } else {
            let (mega, back, range, width, n) = (w[0] != 0, w[1] != 0, f32::from_bits(w[2]), f32::from_bits(w[3]), w[4] as usize);
            let cars: Vec<(usize, [Vec3; 4], bool)> = (0..n).map(|i| (i, frame(&w[6 + 13 * i..18 + 13 * i]), w[5 + 13 * i] == 3)).collect();
            // (a type of the tune: only the range and the width are read, and the mega kinds 12 .. 20 pick the "all of them" rule)
            let mut t = TypeTune::constructed(ICE);
            t.shoot_range = range;
            t.width = width;
            let kind = if mega { 12 } else { ICE };
            let got = targets_frames(&cars, 0, kind, back, &t);
            // (the original's found flag is the list not being empty; its count is the list's)
            assert_eq!(got.len() as u32, want[1], "{line}");
            assert_eq!(got.iter().map(|&i| i as u32).collect::<Vec<_>>(), want[2..].to_vec(), "{line}");
            assert_eq!(!got.is_empty() as u32, want[0], "{line}");
            searches += 1;
            found += (!got.is_empty()) as u32;
        }
    }
    println!("rank: {ranks} cases, targets: {searches} searches ({found} found something)");
}

/// F-32841 (the Shield's per-frame update; the generator is the game's multiply-with-carry pair): tools/pu_shield_tick_reference.py ->
/// testdata/pu_shield_tick_ref.txt, "<dt> <+376> <+384> <+388> <lo> <carry> -> <+376> <+384> <+388> <+392> <+396> <lo> <carry>"
/// (the object's +368 / +372 / +380 are the constructor's 0.18 / 0.24 / 1.0). +396 is not kept by the port: it is the second draw, so
/// the generator's state after it is compared instead.
#[test]
fn shield_tick_matches_the_original() {
    let mut n = 0;
    let mut drawn = 0;
    for line in include_str!("../testdata/pu_shield_tick_ref.txt").lines() {
        let (a, b) = line.split_once(" -> ").unwrap();
        let a: Vec<u32> = a.split(' ').map(hex).collect();
        let b: Vec<u32> = b.split(' ').map(hex).collect();
        let mut fx = CarFx { flash: f32::from_bits(a[1]), t2: f32::from_bits(a[2]), t3_clock: f32::from_bits(a[3]), t3: f32::from_bits(0xdead_0001), rng: crate::pu_bolt::Mwc { lo: a[4], carry: a[5] } };
        fx.shield_tick(f32::from_bits(a[0]));
        let got = [fx.flash.to_bits(), fx.t2.to_bits(), fx.t3_clock.to_bits(), fx.t3.to_bits(), fx.rng.lo, fx.rng.carry];
        assert_eq!(got, [b[0], b[1], b[2], b[3], b[5], b[6]], "{line}");
        // (+396 is the second number drawn: its bits follow from the state, so only its presence is checked)
        assert_eq!(b[4] != 0xdead_0002, b[3] != 0xdead_0001, "{line}");
        drawn += (b[3] != 0xdead_0001) as u32;
        n += 1;
    }
    println!("shield_tick: {n} cases, {drawn} drew numbers");
}

/// F-92307 (a laser layer's scale parameters, with the gradient noise it calls): tools/pu_laser_params_reference.py ->
/// testdata/pu_laser_params_ref.txt, "<7 record floats> <t> -> <4 floats>" (the fade is the caller's: F-23309 multiplies by it).
#[test]
fn laser_params_match_the_original() {
    let mut n = 0;
    for line in include_str!("../testdata/pu_laser_params_ref.txt").lines() {
        let (a, b) = line.split_once(" -> ").unwrap();
        let a: Vec<u32> = a.split(' ').map(hex).collect();
        let b: Vec<u32> = b.split(' ').map(hex).collect();
        let mut r = [0.0f32; 7];
        for k in 0..7 {
            r[k] = f32::from_bits(a[k]);
        }
        let got = crate::pu_fx::laser_params(&r, f32::from_bits(a[7]), 1.0);
        assert_eq!(got.map(f32::to_bits).to_vec(), b, "{line}");
        n += 1;
    }
    println!("laser_params: {n} cases");
}

/// F-28622 (PickUpManager_AddPowerUpSource: a zero second position makes a ring, else a row), with the ring F-32756, the row F-10244
/// and the pool's next element F-59872: tools/pu_hand_spots_reference.py -> testdata/pu_hand_spots_ref.txt (the level probe and the
/// element's type roll stubbed, as the tool says). "<pos 4> <width> <second 4> -> <n> {<probe start 3> <probe end 3> <+112> <+20> <+21>
/// <+23>} x n": each spot's probe segment is (x, y + up, z) .. (x, y - down, z) of the port's `Spot`, +112 its height, +21 its `last`.
#[test]
fn hand_spots_match_the_original() {
    let (mut n, mut spots, mut rows) = (0, 0, 0);
    for line in include_str!("../testdata/pu_hand_spots_ref.txt").lines() {
        let (a, b) = line.split_once(" -> ").unwrap();
        let a: Vec<u32> = a.split(' ').map(hex).collect();
        let b: Vec<u32> = b.split(' ').map(hex).collect();
        let second = v3(&a[5..8]);
        let got = hand_spots(v3(&a[0..3]), f32::from_bits(a[4]), second);
        assert_eq!(got.len() as u32, b[0], "{line}");
        for (i, s) in got.iter().enumerate() {
            let w = &b[1 + 10 * i..11 + 10 * i];
            let want = [w[0], w[1], w[2], w[3], w[4], w[5]];
            let have = [s.at.x, s.at.y + s.up, s.at.z, s.at.x, s.at.y - s.down, s.at.z].map(f32::to_bits);
            assert_eq!(have, want, "spot {i} of {line}");
            assert_eq!((s.height.to_bits(), w[7], s.last as u32, w[8], w[9]), (w[6], 1, w[8], 0, 0), "spot {i} of {line}");
        }
        spots += got.len();
        rows += (second != Vec3::ZERO) as u32;
        n += 1;
    }
    println!("hand_spots: {n} cases, {spots} spots, {rows} rows");
}

/// F-28343 (PickUpManager_CreatePowerUpSourcesFromCheckPoints; F-80951 is its tail-call wrapper), the level probe and the element's
/// type roll stubbed: tools/pu_checkpoint_spots_reference.py -> testdata/pu_checkpoint_spots_ref.txt, "<ordered> <n> {<line> <pos 3>
/// <end 3> <radius> <height>} x n -> <m> {<probe start 3> <probe end 3> <+112> <+20> <+21> <+23>} x m" (the checkpoint's +112 equal to
/// its +64, the port's `CpIn.pos`).
#[test]
fn checkpoint_spots_match_the_original() {
    let (mut n, mut spots) = (0, 0);
    for line in include_str!("../testdata/pu_checkpoint_spots_ref.txt").lines() {
        let (a, b) = line.split_once(" -> ").unwrap();
        let w: Vec<&str> = a.split(' ').collect();
        let ordered = hex(w[0]) != 0;
        let count: usize = w[1].parse().unwrap();
        let a: Vec<u32> = w[2..].iter().map(|x| hex(x)).collect();
        let mut b = b.split(' ');
        let m: usize = b.next().unwrap().parse().unwrap();
        let b: Vec<u32> = b.map(hex).collect();
        let cps: Vec<CpIn> = (0..count)
            .map(|i| {
                let c = &a[9 * i..9 * i + 9];
                CpIn { pos: v3(&c[1..4]), line_end: (c[0] != 0).then(|| v3(&c[4..7])), radius: f32::from_bits(c[7]), height: f32::from_bits(c[8]) }
            })
            .collect();
        let got = checkpoint_spots(&cps, ordered);
        assert_eq!(got.len(), m, "{line}");
        for (i, s) in got.iter().enumerate() {
            let w = &b[10 * i..10 * i + 10];
            let have = [s.at.x, s.at.y + s.up, s.at.z, s.at.x, s.at.y - s.down, s.at.z].map(f32::to_bits);
            assert_eq!(have.to_vec(), w[..6].to_vec(), "spot {i} of {line}");
            assert_eq!((s.height.to_bits(), 1, s.last as u32, 0), (w[6], w[7], w[8], w[9]), "spot {i} of {line}");
        }
        spots += got.len();
        n += 1;
    }
    println!("checkpoint_spots: {n} cases, {spots} spots");
}

/// F-19990 (Pulse's push; the two simulator hand-overs F-33609 / F-77639 and the body's mass and inertia calls are stubs, see the tool):
/// tools/pu_pulse_push_reference.py -> testdata/pu_pulse_push_ref.txt, "<3 rows> <force> <torque> <force_dir> <mass> <inertia 3> -> <the
/// world force x mass> <the world torque x inertia>". The port returns the world vectors; the original's mass and inertia products are
/// lane-wise multiplications done here.
#[test]
fn pulse_push_matches_the_original() {
    let mut n = 0;
    for line in include_str!("../testdata/pu_pulse_push_ref.txt").lines() {
        let (a, b) = line.split_once(" -> ").unwrap();
        let a: Vec<u32> = a.split(' ').map(hex).collect();
        let b: Vec<u32> = b.split(' ').map(hex).collect();
        let rows = [v3(&a[0..3]), v3(&a[3..6]), v3(&a[6..9])];
        let mut t = TypeTune::constructed(PULSE);
        t.force = v3(&a[9..12]);
        t.torque = v3(&a[12..15]);
        t.force_dir = v3(&a[15..18]);
        let (mass, inertia) = (f32::from_bits(a[18]), v3(&a[19..22]));
        let (f, q) = pulse_push_rows(&rows, &t);
        let got = [f.x * mass, f.y * mass, f.z * mass, q.x * inertia.x, q.y * inertia.y, q.z * inertia.z].map(f32::to_bits);
        assert_eq!(got.to_vec(), b, "{line}");
        n += 1;
    }
    println!("pulse_push: {n} cases");
}

/// F-66683 (a shot's frame: search until found, the clock, the landing after 0.25 s) and F-37047 (its first search when fired):
/// tools/pu_shot_step_reference.py -> testdata/pu_shot_step_ref.txt, the search F-33277 and the landing stubbed (recording; the search
/// is verified by `rank_and_targets_match_the_original`). S: "<kind> <found> <wait> <dt> <range> <width> <n listed> <ids> <n the search
/// finds> <ids> -> <found'> <wait'> <searches> <mega> <range> <width> <count'> <lands: id kind ...>". F: "<kind> <old wait> <range>
/// <width> <n found> <ids> -> <found'> <wait'> <searches> <mega> <range> <width> <count'>".
#[test]
fn shot_step_matches_the_original() {
    let (mut steps, mut firsts, mut landed) = (0, 0, 0);
    for line in include_str!("../testdata/pu_shot_step_ref.txt").lines() {
        let (a, b) = line.split_once(" -> ").unwrap();
        let a: Vec<&str> = a.split_whitespace().collect();
        let b: Vec<u32> = b.split_whitespace().map(hex).collect();
        let w: Vec<u32> = a[1..].iter().map(|x| hex(x)).collect();
        if a[0] == "S" {
            let (kind, found, wait, dt) = (w[0] as u8, w[1] != 0, f32::from_bits(w[2]), f32::from_bits(w[3]));
            let nl = w[6] as usize;
            let listed: Vec<usize> = w[7..7 + nl].iter().map(|&x| x as usize).collect();
            let finds: Vec<usize> = w[8 + nl..].iter().map(|&x| x as usize).collect();
            let mut s = Shot { kind, left: 1.0, found, targets: listed, wait };
            let mut searches = 0;
            let land = s.step(dt, || {
                searches += 1;
                finds.clone()
            });
            assert_eq!((s.found as u32, s.wait.to_bits(), searches), (b[0], b[1], b[2] as i32), "{line}");
            if searches == 1 {
                assert_eq!(is_mega_shot(kind) as u32, b[3], "{line}");
            }
            assert_eq!(s.targets.len() as u32, b[6], "{line}");
            let want: Vec<usize> = b[7..].chunks(2).map(|c| c[0] as usize).collect();
            // (the reference prints the landing's kind in decimal, which `hex` read as hex digits)
            let kind_digits = u32::from_str_radix(&kind.to_string(), 16).unwrap();
            assert!(b[7..].chunks(2).all(|c| c[1] == kind_digits), "{line}");
            assert_eq!(land.clone().unwrap_or_default(), want, "{line}");
            landed += land.map_or(0, |l| l.len());
            steps += 1;
        } else {
            // (the first search: the shot starts with the found flag and an empty clock - the original leaves its old clock where nothing
            // was found, and the first frame's step resets it before it is read)
            let kind = w[0] as u8;
            let finds: Vec<usize> = w[5..].iter().map(|&x| x as usize).collect();
            assert_eq!((is_mega_shot(kind) as u32, !finds.is_empty() as u32, finds.len() as u32), (b[3], b[0], b[6]), "{line}");
            assert_eq!(b[2], 1, "{line}");
            if !finds.is_empty() {
                assert_eq!(b[1], 0, "{line}");
            }
            firsts += 1;
        }
    }
    println!("shot_step: {steps} steps ({landed} landings), {firsts} first searches");
}
