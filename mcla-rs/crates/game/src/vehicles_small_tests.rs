//! Lamp, paint and reset routines of the vehicle code compared with the original (tools/vehicles_small_reference.py): each
//! runs the generated code unchanged on flat guest memory; every line of testdata/vehicles_small_ref.txt is
//! "<tag> <input words> -> <output words>" (hex; a word that reads 82xxxxxx is written with an h in front). A NaN is
//! compared only as a NaN (the harness's default NaN is not the console's).
use crate::vehicles::vehicles_words as vw;

fn words(s: &str) -> Vec<u32> {
    s.split_whitespace().map(|x| u32::from_str_radix(x.trim_start_matches('h'), 16).unwrap()).collect()
}

fn cases(tag: &'static str) -> impl Iterator<Item = (Vec<u32>, Vec<u32>)> {
    include_str!("testdata/vehicles_small_ref.txt").lines().filter_map(move |l| {
        let rest = l.strip_prefix(tag)?.strip_prefix(' ')?;
        let (i, o) = rest.split_once(" -> ")?;
        Some((words(i), words(o)))
    })
}

fn same(a: u32, b: u32) -> bool {
    a == b || (f32::from_bits(a).is_nan() && f32::from_bits(b).is_nan())
}

fn all_same(got: &[u32], want: &[u32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}");
    for (k, (&g, &w)) in got.iter().zip(want).enumerate() {
        assert!(same(g, w), "{what}: word {k}: {g:08x} against {w:08x}");
    }
}

fn f(w: u32) -> f32 {
    f32::from_bits(w)
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// F-44484: pow(x, 2.2) of the three channels, the real C library pow in the original.
#[test]
fn paint_pow_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("paint") {
        let got = vw::paint_pow([f(i[0]), f(i[1]), f(i[2]), f(i[3])]);
        all_same(&bits(&got), &o, &format!("{i:08x?}"));
        n += 1;
    }
    assert_eq!(n, 4000);
}

/// F-75961: the lights-on getter (the hour getter F-22302 real, the object's slot a stub answering from the case).
#[test]
fn traffic_lights_getter_matches_the_original() {
    let (mut n, mut on, mut played) = (0, 0, 0);
    for (i, o) in cases("lights") {
        // (the table row the selector byte picks: the first index for a set byte, else the second; the harness stores the
        // second index's row last, so it wins where they are equal. The hour slot: the first index's for a set byte, else
        // slot 2, which is stored last)
        let weather = if i[3] != 0 && i[4] != i[5] { f(i[6]) + f(i[7]) } else { f(i[8]) + f(i[9]) };
        let hour = f(if i[3] != 0 && i[4] != 2 { i[12] } else { i[13] });
        let got = vw::lights_on(f(i[0]), i[1] != 0, i[2] != 0, f(i[10]), f(i[11]), hour, f(i[14]), weather);
        assert_eq!(o[0], got as u32, "{i:08x?}");
        on += got as u32;
        n += 1;
        // (the play function: the light manager's 6.0 / 18.0, the object at +452 present and answering no)
        if i[10] == 0x40c0_0000 && i[11] == 0x4190_0000 && i[1] != 0 && i[2] == 0 {
            let play = crate::vehicles::traffic_lights_due(f(i[0]), hour, f(i[14]), weather);
            assert_eq!(o[0], play as u32, "play {i:08x?}");
            played += 1;
        }
    }
    assert!(n == 4000 && on > 500 && on < 3500, "{n} {on}");
    assert!(played > 300, "{played}");
}

/// F-40379: the nitro counter's reset.
#[test]
fn nitro_reset_matches_the_original() {
    let (mut n, mut played, mut negative) = (0, 0, 0);
    for (i, o) in cases("nitro") {
        let c = vw::nitro_count(i[1] as u16 as i16, i[0] as u16 as i16) as u16 as u32;
        assert_eq!(o, [c, c, 0, 0, 0], "{i:08x?}");
        n += 1;
        // (the play function, the cap 100 as `CarInstance::nitro_reset` has it; the u8 store clamps a negative count at 0)
        if i[1] == 100 {
            let play = crate::vehicles::nitro_start((i[0] as u16) as f32);
            assert_eq!(play as i16, (c as u16 as i16).max(0), "play {i:08x?}");
            played += 1;
            negative += ((c as u16 as i16) < 0) as u32;
        }
    }
    assert_eq!(n, 3000);
    assert!(played > 500 && negative > 100, "{played} {negative}");
}

/// F-42929: the window controller's reset (the sound stop F-31667 a stub that records r3 and r4).
#[test]
fn window_reset_matches_the_original() {
    let mut n = 0;
    for (i, o) in cases("winreset") {
        let r = vw::window_reset(i[0] != 0);
        assert_eq!((o[2], o[3], o[0], o[1]), (r.state, r.target, r.hold, r.time.to_bits()), "{i:08x?}");
        assert_eq!(o[4], r.stop_sound as u32, "{i:08x?}");
        if r.stop_sound {
            assert_eq!((o[5], o[6]), (0x1100, 0), "{i:08x?}");
        }
        // (+16 untouched)
        assert_eq!(o[7], 0xa5a50004);
        // (the play default: the state the reset's target leads to, and the reset's time)
        let w = crate::vehicles::WindowCtl::default();
        assert_eq!((w.state as u32, w.time.to_bits()), (o[3], o[1]), "play {i:08x?}");
        n += 1;
    }
    assert_eq!(n, 1000);
}

/// F-96984: the state table by attachment and the light set's levels (the render helpers stubs; pow real).
#[test]
fn lamp_state_levels_match_the_original() {
    let mut n = 0;
    for (i, o) in cases("lampstate") {
        let (h, h2, b, rev, flash, siren) = (f(i[0]), f(i[1]), f(i[2]), f(i[3]), f(i[4]), f(i[5]));
        let table = vw::setup_table(h, h2, b, siren, flash, rev);
        for e in 0..19 {
            let att = i[13 + 9 * e + 8] as usize;
            all_same(&[table[att].to_bits()], &[o[e]], &format!("{i:08x?} entry {e}"));
            // (the play table, with the second head state as the car keeps it)
            let got = crate::vehicles::setup_state([h, b, siren, flash, rev], h2, att as u8);
            all_same(&[got.to_bits()], &[o[e]], &format!("port {i:08x?} entry {e}"));
        }
        // (the 20th entry: the default row 0.5, 0.125, 0 and the shift state +392 as the cover's w)
        all_same(&o[19..23], &[i[6], 0x3f000000, 0x3e000000, 0], &format!("{i:08x?}"));
        let (pulse, intensity, one, flash_i, rec308) = (f(i[7]), f(i[11]), f(i[9]), f(i[8]), f(i[12]));
        all_same(&[o[23]], &[i[0]], "+2672");
        all_same(&[o[24]], &[vw::setup_glow(table[0]).to_bits()], &format!("{i:08x?} +2676"));
        all_same(&[o[25]], &[vw::neon_scale(pulse, intensity, one).to_bits()], &format!("{i:08x?} +2680"));
        all_same(&[o[26]], &[if i[10] != 0 { 1.0f32 } else { h }.to_bits()], "+2976");
        all_same(&[o[27]], &[i[0]], "+2980");
        all_same(&[o[28]], &[((flash_i * rec308) * one).to_bits()], &format!("{i:08x?} +2984"));
        // (the neon's siColor2 scale in the port: the colour (1, 1, 1, 1) leaves the scale itself)
        if i[9] == 0x3f800000 {
            let neon = crate::vehicles::ExtNeon { color: [1.0; 4], intensity, pulse };
            all_same(&[neon.si_color()[0].to_bits()], &[o[25]], &format!("port {i:08x?} +2680"));
        }
        // (+2624: the product of the third entry's two vectors, lane by lane)
        let lanes: Vec<u32> = (0..4).map(|k| (f(i[40 + k]) * f(i[44 + k])).to_bits()).collect();
        all_same(&lanes, &o[29..33], &format!("{i:08x?} +2624"));
        n += 1;
    }
    assert_eq!(n, 1000);
}

/// F-81135: a traffic car's flags, easing, the table, the glow fade and the level rows (the gate F-89245 answers from the
/// case, the upload F-49426 records its arrays).
#[test]
fn traffic_lamps_match_the_original() {
    let (mut n, mut gated, mut overridden) = (0, 0, 0);
    for (i, o) in cases("traffic") {
        let (d, dt, idx) = (f(i[0]), f(i[6]), i[2] as usize);
        if i[1] == 0 {
            // (the gate says no: nothing is written)
            gated += 1;
            all_same(&o[..5], &i[12..17], &format!("{i:08x?} untouched"));
            assert!(o[5..10].iter().all(|&b| b == 0xa5) && o[10..29].iter().all(|&w| w == 0xa5a5_a5a5), "{i:08x?}");
            assert!(o[29..48].iter().all(|&w| w == 0) && o[48..].iter().all(|&w| w == 0xa5a5_a5a5), "{i:08x?}");
            continue;
        }
        let flags = vw::traffic_flags(i[3 + idx] as u8);
        let mut s = [0.0f32; 5];
        for k in 0..5 {
            s[k] = crate::vehicles::ease_at(f(i[12 + k]), flags[k], f(i[7 + k]), dt);
            assert_eq!(o[5 + k], flags[k] as u32, "{i:08x?} target {k}");
        }
        all_same(&bits(&s), &o[..5], &format!("{i:08x?} states"));
        let over = i[21] != 0;
        let table: Vec<f32> = if over { overridden += 1; i[22..39].iter().map(|&w| f(w)).collect() } else { vw::traffic_table(s).to_vec() };
        let (fade, scale, factor) = (vw::traffic_fade(d), f(i[17 + idx]), f(i[20]));
        for e in 0..19 {
            let att = i[39 + 9 * e + 8] as usize;
            let st = table[att];
            all_same(&[vw::traffic_level(st, fade, scale, factor).to_bits()], &[o[10 + e]], &format!("{i:08x?} level {e}"));
            all_same(&[st.to_bits()], &[o[29 + e]], &format!("{i:08x?} state {e}"));
            if !over {
                // (the port's table and fade: the same numbers)
                let (ps, pl) = crate::vehicles::traffic_glow(s, att as u8, d, scale, factor);
                all_same(&[pl.to_bits()], &[o[10 + e]], &format!("port {i:08x?} level {e}"));
                all_same(&[ps.to_bits()], &[o[29 + e]], &format!("port {i:08x?} state {e}"));
            }
            let lanes: Vec<u32> = (0..4).map(|k| (f(i[39 + 9 * e + k]) * f(i[39 + 9 * e + 4 + k])).to_bits()).collect();
            all_same(&lanes, &o[48 + 4 * e..52 + 4 * e], &format!("{i:08x?} row {e}"));
        }
        n += 1;
    }
    assert!(n >= 700 && gated >= 100 && overridden >= 20, "{n} {gated} {overridden}");
}


/// F-10129: the lamp controller's targets and easing: head, brake, reversing, shift, the speed target, the lamp set's byte,
/// the neon flashes, then every record eased by F-24995 (real) at the frame time. The stealth check F-54047 and the engine's
/// speed F-12794 answer from the case (the stealth one only when the sim object has a +128); the hour getter, the move
/// helper and cos are real.
#[test]
fn lamp_controller_matches_the_original() {
    let (mut n, mut heads, mut shifts, mut played) = (0, 0, 0, 0);
    for (i, o) in cases("lamps") {
        let input = vw::LampIn {
            switch: i[0] != 0,
            pedal: (i[1] != 0).then(|| f(i[2])),
            stealth: i[3] != 0 && i[4] != 0,
            reverse_gear: i[5] == 0,
            speed: f(i[17]),
            garage: i[18] != 0,
            hour: f(i[19]),
            sunrise: f(i[20]),
            sunset: f(i[21]),
            storm: f(i[26]),
            fog: f(i[25]),
            cabin: i[27] != 0,
            always_rain: i[28] != 0,
            rpm: f(i[16]),
            shift_point: f(i[6 + i[5] as usize]),
            ext_rate: i[30] as u8 as i8,
            int_rate: i[31] as u8 as i8,
            clock: f(i[29]),
        };
        let out = vw::lamp_targets(&input);
        let dt = f(i[59]);
        // (rate, target, value) of the eight records at +160 + 20 k, then the shift record at +384
        let mut rec: Vec<(f32, bool, f32)> = (0..8).map(|k| (f(i[32 + k]), i[48 + k] != 0, f(i[40 + k]))).collect();
        rec[0].1 = out.head;
        rec[1].1 = out.head;
        rec[2].1 = out.brake;
        rec[3].1 = out.reverse;
        let mut shift = (f(i[56]), out.shift, f(i[57]));
        if out.rain_flash {
            rec[4].1 = true;
            rec[4].2 = 1.0;
        }
        for r in rec.iter_mut().chain(std::iter::once(&mut shift)) {
            r.2 = crate::vehicles::ease_at(r.2, r.1, r.0, dt);
        }
        let mut want: Vec<u32> = Vec::new();
        for k in 0..8 {
            want.push(rec[k].1 as u32);
            want.push(if k == 5 { 0x3f80_0000 } else { rec[k].2.to_bits() });
        }
        want.push(out.speed_target.map_or(0, f32::to_bits));
        want.push(shift.1 as u32);
        want.push(shift.2.to_bits());
        want.push(out.neon[0].to_bits());
        want.push(out.neon[1].to_bits());
        want.push(if out.set_byte == 2161 { 1 } else { 0xa5 });
        want.push(if out.set_byte == 2162 { 1 } else { 0xa5 });
        // (the record at +260's target is not written: its input stays; its value is forced to 1 after the easing)
        want[10] = i[53];
        all_same(&want, &o, &format!("{i:08x?}"));
        // (the play targets: the light manager's 6.0 / 18.0, a driver input present, the port's gear numbering: reverse -1,
        // else the gear index - 1 of the original's; the lamp set's byte 2161 is the head lamps' sun / weather / garage test)
        if i[1] != 0 && i[20] == 0x40c0_0000 && i[21] == 0x4190_0000 {
            let w = crate::vehicles::LampWorld { garage: i[18] != 0, cabin: i[27] != 0, hour: f(i[19]), storm: f(i[26]), fog: f(i[25]) };
            let gear = if i[5] == 0 { -1 } else { i[5] as i32 - 1 };
            let shift = Some((f(i[16]), f(i[6 + i[5] as usize])));
            let t = crate::vehicles::setup_lamp_targets(&w, i[0] == 0, i[3] != 0 && i[4] != 0, f(i[2]), gear, shift);
            assert_eq!((t.head as u32, t.brake as u32, t.reverse as u32, t.shift as u32), (o[0], o[4], o[6], o[17]), "play {i:08x?}");
            played += 1;
        }
        heads += out.head as u32;
        shifts += out.shift as u32;
        n += 1;
    }
    assert!(n == 2000 && heads > 300 && heads < 1700 && shifts > 300 && shifts < 1700, "{n} {heads} {shifts}");
    assert!(played > 500, "{played}");
}
