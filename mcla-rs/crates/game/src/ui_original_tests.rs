//! Small UI functions against the original's own code run offline (tools/ui_game_reference.py ->
//! testdata/ui_game_ref.txt).

use super::*;

fn lines(tag: &'static str) -> impl Iterator<Item = (&'static str, &'static str)> {
    include_str!("testdata/ui_game_ref.txt").lines().filter_map(move |l| l.strip_prefix(tag)?.strip_prefix(' ')?.split_once(" -> "))
}

/// F-56191: the PowerUptype cell (the chosen cell gets the word, the other is untouched)
#[test]
fn powerup_cell_matches_the_original() {
    let mut n = 0;
    for (a, out) in lines("PT") {
        let a: Vec<i32> = a.split(' ').map(|w| w.parse().unwrap()).collect();
        let w: Vec<u32> = out.split(' ').map(|w| u32::from_str_radix(w, 16).unwrap()).collect();
        let (cell, other) = if a[1] == 0 { (w[0], w[1]) } else { (w[1], w[0]) };
        assert_eq!((cell as i32, other), (crate::powerups::hud_cell(a[0]), 0xaaaa_aaaa), "PT {a:?}");
        n += 1;
    }
    assert_eq!(n, 64);
}

/// F-73259, F-29965, F-22161, F-44052, F-90804: the photo album's prompt-bar lists (the album's prompt sets 7..11)
#[test]
fn photo_prompt_rows_match_the_original() {
    let mut n = 0;
    for ((_, out), set) in lines("PR").zip(7u8..) {
        // (icon:key words; a key may hold a space - "Zoom In/Out")
        let mut rows: Vec<String> = Vec::new();
        for w in out.split(' ') {
            match rows.last_mut() {
                Some(r) if !w.contains(':') => *r += &format!(" {w}"),
                _ => rows.push(w.to_string()),
            }
        }
        let want: Vec<(String, i32)> = rows.iter().map(|r| r.split_once(':').unwrap()).map(|(i, k)| (k.to_string(), i.parse().unwrap())).collect();
        let got: Vec<(String, i32)> = crate::photoalbum::prompt_rows(set).iter().map(|&(k, i)| (k.to_string(), i)).collect();
        assert_eq!(got, want, "prompt set {set}");
        n += 1;
    }
    assert_eq!(n, 5);
}

/// F-84077 (mcBaseMenu vt+84), F-46420 (mcBaseMenu vt+88), F-79373 (mcSeriesMenu), F-69707 (mcTournamentMenu)
#[test]
fn dialog_descriptions_match_the_original() {
    let want: std::collections::HashMap<&str, &str> = lines("VD").collect();
    assert_eq!(GARAGE_CONFIRM_DESC, want["F-84077"]);
    let mut g = Game::new(Vec::new(), 0);
    assert_eq!(g.view_replay_desc(false), want["F-46420"]);
    g.career.tour_table = Some(Default::default());
    assert_eq!(g.view_replay_desc(false), want["F-69707"]);
    g.series = Some(Default::default());
    assert_eq!(g.view_replay_desc(false), want["F-79373"]);
}

/// F-83952: the GPS map keeps one target - a place picked replaces a mission set as the target and back; the arrow
/// takes the target's own position
#[test]
fn one_gps_target() {
    let mut g = Game::new(Vec::new(), 0);
    g.set_gps_target(None, Some("RACE_GH_06".into()));
    let p = glam::Vec3::new(10.0, 0.0, -20.0);
    g.set_gps_target(Some(p), Some("RACE_GH_06".into()));
    assert_eq!((g.gps_pick, g.mission_target.as_deref()), (Some(p), None));
    assert_eq!(g.gps_target_pos(), Some(p));
    assert!(!g.gps_on_man());
    g.set_gps_target(None, Some("RACE_GH_06".into()));
    assert_eq!((g.gps_pick, g.mission_target.as_deref()), (None, Some("RACE_GH_06")));
    g.set_gps_target(None, None);
    assert_eq!(g.gps_target_pos(), None);
}

/// F-64065 -> F-94071: a race script's teardown takes the HUD arrow's target away; the map keeps its selection
/// and only the next SET arms the arrow again
#[test]
fn race_teardown_clears_the_gps_arrow() {
    let mut g = Game::new(Vec::new(), 0);
    let p = glam::Vec3::new(10.0, 0.0, -20.0);
    g.set_gps_target(Some(p), None);
    g.gps_race_teardown();
    assert_eq!(g.gps_target_pos(), Some(p));
    g.bmt_goal = Some(glam::Vec3::ZERO);
    g.gps_race_teardown();
    assert!(g.gps_armed);
    g.bmt_goal = None;
    g.gps_race_teardown();
    assert_eq!((g.gps_armed, g.gps_target_pos(), g.gps_pick), (false, None, Some(p)));
    g.gps_race_teardown();
    assert!(!g.gps_armed);
    g.set_gps_target(Some(p), None);
    assert_eq!(g.gps_target_pos(), Some(p));
}

/// F-71884 (with F-43310), F-51730, F-57096: the GPS map's bank and experience panel - every variable
/// in order, both meters' bits, and `panelisvisible` against the original's code run offline
/// (tools/ui_gps_panel_reference.py -> testdata/ui_gps_panel_ref.txt; the string table a stub giving "{key}")
#[test]
fn gps_panel_matches_the_original() {
    const KINDS: [&str; 10] = ["Tuner", "Muscle", "Exotic", "SportBike", "Luxury", "Cop", "SUV", "LowRider", "Misc1", "D0"];
    let st = |k: &str| if k == "Navmap_Bank_Desc" { "$<D>Navmap_Bank_Amount</D>".to_string() } else { format!("{{{k}}}") };
    let (mut np, mut nv) = (0, 0);
    for l in include_str!("testdata/ui_gps_panel_ref.txt").lines() {
        let (case, out) = l.split_once(" ->").unwrap();
        let h: Vec<u32> = case.split(' ').skip(1).map(|w| u32::from_str_radix(w, 16).unwrap()).collect();
        let sets: Vec<(&str, &str)> = out.split('\t').skip(1).filter_map(|s| s.split_once('=')).collect();
        if case.starts_with('V') {
            let want = sets.iter().find(|s| s.0 == "I:panelisvisible").unwrap().1 == "1";
            assert_eq!(gps_panel_visible(f32::from_bits(h[2]), f32::from_bits(h[3]), h[0] as i32, f32::from_bits(h[1])), want, "{l}");
            nv += 1;
            continue;
        }
        let (vars, meters) = gps_panel_vars(h[0] as usize, h[1] as usize, h[2] as i64, h[3] as i64, h[4] as usize, KINDS[h[5] as usize], (h[6] as i64, h[7] as i64, h[8] as i64, h[9] as i64, h[10] as usize), f32::from_bits(h[11]), f32::from_bits(h[12]), &st("VEHNAME"), &st);
        let strs: Vec<(String, String)> = sets.iter().filter_map(|(k, v)| k.strip_prefix("S:").map(|k| (k.to_string(), v.to_string()))).collect();
        assert_eq!(vars, strs, "{l}");
        let fl = |n: &str| u32::from_str_radix(sets.iter().find(|s| s.0 == n).unwrap().1, 16).unwrap();
        assert_eq!([meters[0].to_bits(), meters[1].to_bits()], [fl("F:rank_meter"), fl("F:veh_meter")], "{l}");
        np += 1;
    }
    assert_eq!((np, nv), (450, 150));
}

/// F-90999, F-66209 (with F-37810, F-37290, F-36590, F-56767, F-66886, F-95591): the GPS map's outline test and pull-back
/// against the original's code run offline on the "sc" outline (tools/ui_nav_outline_reference.py ->
/// testdata/ui_nav_outline_ref.txt)
#[test]
fn nav_outline_matches_the_original() {
    let (mut ni, mut np) = (0, 0);
    for l in include_str!("testdata/ui_nav_outline_ref.txt").lines() {
        let (case, out) = l.split_once(" ->").unwrap();
        let h: Vec<f32> = case.split(' ').skip(1).map(|w| f32::from_bits(u32::from_str_radix(w, 16).unwrap())).collect();
        let o: Vec<u32> = out.split(' ').skip(1).map(|w| u32::from_str_radix(w, 16).unwrap()).collect();
        if case.starts_with('I') {
            assert_eq!(crate::hudmap::in_outline(h[0], h[1]), o[0] == 1, "{l}");
            ni += 1;
            continue;
        }
        let r = crate::hudmap::outline_pull_back(glam::Vec3::new(h[0], h[1], h[2]), glam::Vec3::new(h[3], h[4], h[5]));
        assert_eq!([r.x.to_bits(), r.y.to_bits(), r.z.to_bits()], [o[0], o[1], o[2]], "{l}");
        np += 1;
    }
    assert_eq!((ni, np), (1000, 3000));
}

/// F-97746 with its force flag (the GPS map's stick released, F-63282): the element taken against the original's code run
/// offline on random element lists (tools/ui_nav_pick_reference.py -> testdata/ui_nav_pick_ref.txt); elements hidden
/// (+8 set) or not shown (+9 clear) are not candidates, as the port's place list holds none
#[test]
fn nav_pick_matches_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/ui_nav_pick_ref.txt").lines() {
        let (case, out) = l.split_once(" ->").unwrap();
        let (head, order) = case.split_once(" |").unwrap();
        let w: Vec<u32> = head.split(' ').skip(1).map(|w| u32::from_str_radix(w, 16).unwrap()).collect();
        let f = |i: usize| f32::from_bits(w[i]);
        let els: Vec<(glam::Vec3, bool)> = (0..w[3] as usize).map(|k| (glam::Vec3::new(f(4 + 5 * k), f(5 + 5 * k), f(6 + 5 * k)), w[7 + 5 * k] == 0 && w[8 + 5 * k] != 0)).collect();
        let order: Vec<usize> = order.split_whitespace().map(|s| s.parse().unwrap()).collect();
        let cand: Vec<usize> = order.into_iter().filter(|&i| els[i].1).collect();
        let pts: Vec<glam::Vec3> = cand.iter().map(|&i| els[i].0).collect();
        let got = crate::hudmap::pick_nearest(glam::Vec3::new(f(1), 0.0, f(2)), f(0), &pts).map_or(-1, |j| cand[j] as i32);
        let o: Vec<i32> = out.split_whitespace().map(|s| s.parse().unwrap()).collect();
        assert_eq!((got >= 0, got), (o[0] == 1, o[1]), "{l}");
        n += 1;
    }
    assert_eq!(n, 3000);
}

/// F-31695's 1 m dead band: whether the GPS map's cursor move is taken and the cursor stored, against the original's code
/// run offline (its outline test stubbed to answer inside; tools/ui_nav_deadband_reference.py -> testdata/ui_nav_deadband_ref.txt)
#[test]
fn cursor_dead_band_matches_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/ui_nav_deadband_ref.txt").lines() {
        let (case, out) = l.split_once(" -> ").unwrap();
        let w: Vec<f32> = case.split(' ').map(|v| f32::from_bits(v.parse().unwrap())).collect();
        let o: Vec<u32> = out.split(' ').map(|v| v.parse().unwrap()).collect();
        let (old, p) = (glam::Vec3::new(w[0], w[1], w[2]), glam::Vec3::new(w[3], w[4], w[5]));
        let moved = !crate::hudmap::cursor_dead_band(old, p);
        let kept = if moved { p } else { old };
        assert_eq!((moved, [kept.x, kept.y, kept.z].map(f32::to_bits).to_vec()), (o[0] == 1, o[1..].to_vec()), "{l}");
        n += 1;
    }
    assert_eq!(n, 4000);
}

/// F-70194: the ring colour of the GPS map's types 7 / 8 / 9 from the element's colour word, alpha kept, against the
/// original's code run offline on every byte value and random words (tools/ui_element_colour_reference.py ->
/// testdata/ui_element_colour_ref.txt)
#[test]
fn element_colour_matches_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/ui_element_colour_ref.txt").lines() {
        let (w, out) = l.split_once(" -> ").unwrap();
        let w: u32 = w.parse().unwrap();
        let want: Vec<u32> = out.split(' ').map(|v| u32::from_str_radix(v, 16).unwrap()).collect();
        let b = [(w >> 16) & 255, (w >> 8) & 255, w & 255, w >> 24].map(|v| v as i32);
        assert_eq!(crate::hudmap::word_bytes(b).map(f32::to_bits).to_vec(), want, "{l}");
        // (a colour given as floats reaches the word by truncation: the bytes' own values come back)
        assert_eq!(crate::hudmap::element_colour(crate::hudmap::word_bytes(b)).map(f32::to_bits).to_vec(), want, "{l}");
        n += 1;
    }
    assert_eq!(n, 1024);
}

/// F-68183, the seconds format of the arrest panel's PURSUIT DURATION, against the original run offline (tools/save_hms_reference.py).
#[test]
fn hms_matches_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/save_hms_ref.txt").lines() {
        let (b, want) = l.split_once(" -> ").unwrap();
        assert_eq!(crate::game::hms_text(f32::from_bits(b.parse().unwrap())), want, "{l}");
        n += 1;
    }
    assert_eq!(n, 1016);
}

/// F-98009, the HUD map's tick: clock, checkpoint strobe and pulse (fused steps, the pulse wrapped by F-43389, pi kept) against
/// the original's code run offline on 5,000 cases, a third of them landing on pi or the strobe's bound
/// (tools/ui_map_tick_reference.py -> testdata/ui_map_tick_ref.txt)
#[test]
fn map_tick_matches_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/ui_map_tick_ref.txt").lines() {
        let (case, out) = l.split_once(" -> ").unwrap();
        let c: Vec<u32> = case.split(' ').map(|v| v.parse().unwrap()).collect();
        let o: Vec<u32> = out.split(' ').map(|v| v.parse().unwrap()).collect();
        let f = |i: usize| f32::from_bits(c[i]);
        let (t, s, p) = crate::hudmap::map_tick(f(0), f(1), f(2), f(7), f(4), f(5), f(6), c[8] as usize);
        assert_eq!([t, s, p].map(f32::to_bits).to_vec(), o[..3].to_vec(), "{l}");
        assert_eq!(o[3], c[3].wrapping_add(1), "{l}");
        n += 1;
    }
    assert_eq!(n, 5000);
}

/// F-42900, one minimap rim dot: its place (the translation of both matrices it hands the mesh draw), the selected dot's
/// alpha, the mesh slot by kind (2 the flag +4, 10 the flag base +8, else the sphere +0) and the fixed 6-unit matrix, against
/// the original's code run offline on 3,000 cases (tools/ui_rim_dot_reference.py -> testdata/ui_rim_dot_ref.txt)
#[test]
fn rim_dot_matches_the_original() {
    let mut n = 0;
    let e = 3037515234u32; // (-5.245e-7)
    let six = [6.0f32.to_bits(), 0, e, 0, (-0.0f32).to_bits(), (-6.0f32).to_bits(), 0, 0, e, 0, (-6.0f32).to_bits(), 0];
    for l in include_str!("testdata/ui_rim_dot_ref.txt").lines() {
        let (case, out) = l.split_once(" -> ").unwrap();
        let c: Vec<u32> = case.split(' ').map(|v| v.parse().unwrap()).collect();
        let o: Vec<u32> = out.split(' ').map(|v| v.parse().unwrap()).collect();
        let (x, z, a) = crate::hudmap::rim_dot(f32::from_bits(c[0]), c[2] == 1, f32::from_bits(c[3]));
        let m = &o[7..];
        assert_eq!([x.to_bits(), 0, z.to_bits()], [m[12], m[13], m[14]], "{l}");
        assert_eq!([m[12], m[13], m[14]], [m[28], m[29], m[30]], "{l}");
        assert_eq!(a.to_bits(), o[1], "{l}");
        assert_eq!(o[0], match c[1] { 2 => 4, 10 => 8, _ => 0 }, "{l}");
        assert_eq!(m[..12].to_vec(), six.to_vec(), "{l}");
        n += 1;
    }
    assert_eq!(n, 3000);
}

/// F-75199 (projection), F-72569 (inside) and F-40572 (the rim-dot queue: angle and colour word) run offline on the
/// original's code (tools/ui_rim_queue_reference.py): every word equal.
#[test]
fn rim_queue_matches_the_original() {
    let mut n = [0; 3];
    for l in include_str!("testdata/ui_rim_queue_ref.txt").lines() {
        let (case, out) = l.split_once(" -> ").unwrap();
        let c: Vec<u32> = case.split(' ').map(|v| v.parse().unwrap()).collect();
        let o: Vec<u32> = out.split(' ').map(|v| v.parse().unwrap()).collect();
        let f = |i: usize| f32::from_bits(c[i]);
        let m = glam::Mat4::from_cols_array(&std::array::from_fn(|i| f(1 + i)));
        let q = crate::hudmap::project_point(&m, glam::Vec3::new(f(17), f(18), f(19)));
        match c[0] {
            0 => assert_eq!([q.x, q.y, q.z].map(f32::to_bits).to_vec(), o, "{l}"),
            1 => assert_eq!((crate::hudmap::projected_len(q) < f(21)) as u32, o[0], "{l}"),
            _ => {
                let a = crate::hudmap::rim_angle(q);
                let w = crate::hudmap::rim_word([f(22), f(23), f(24), f(25)]);
                let same = a.to_bits() == o[0] || (a.is_nan() && f32::from_bits(o[0]).is_nan());
                assert!(same, "{l}: angle {a}");
                assert_eq!(w.map(u32::from).to_vec(), o[1..5].to_vec(), "{l}");
                assert_eq!([o[5], o[6], o[7]], [100, c[27], c[26] & 255], "{l}");
            }
        }
        n[c[0] as usize] += 1;
    }
    assert_eq!(n, [1000; 3]);
}
/// F-45776 (an icon matrix turned by the map angle, with F-92665's rotation) run offline on the original's code
/// (tools/ui_turn_y_reference.py): x y z of every row equal.
#[test]
fn turn_y_matches_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/ui_turn_y_ref.txt").lines() {
        let (case, out) = l.split_once(" -> ").unwrap();
        let c: Vec<u32> = case.split(' ').map(|v| v.parse().unwrap()).collect();
        let o: Vec<u32> = out.split(' ').map(|v| v.parse().unwrap()).collect();
        let m = glam::Mat4::from_cols_array(&std::array::from_fn(|i| f32::from_bits(c[i])));
        let r = crate::hudmap::turn_y(m, f32::from_bits(c[16])).to_cols_array();
        let got: Vec<u32> = (0..4).flat_map(|i| (0..3).map(move |k| (i, k))).map(|(i, k)| r[4 * i + k].to_bits()).collect();
        assert_eq!(got, o, "{l}");
        n += 1;
    }
    assert_eq!(n, 3000);
}

/// GET_CONTROLLER_LEFTX's getter (F-80539) run offline on the original's code for every stick byte
/// (tools/ui_leftx_reference.py): with the input open, (byte - 127.5) x 1/127 held to -1..1 equals `mc4::stick_float`
/// bit for bit; while the UI captures input, 0. So VEHICLE SELECT's stick steps at byte 192 and up / 63 and down
/// (191 and 64 give exactly 0.5 / -0.5, which CarSelection's > 0.5 / < -0.5 do not take).
#[test]
fn leftx_getter_matches_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/ui_leftx_ref.txt").lines() {
        let (case, out) = l.split_once(" -> ").unwrap();
        let c: Vec<u32> = case.split(' ').map(|v| v.parse().unwrap()).collect();
        let want: u32 = out.parse().unwrap();
        let got = if c[0] == 1 { 0.0f32 } else { crate::mc4::stick_float(c[1] as u8) };
        assert_eq!(got.to_bits(), want, "{l}");
        n += 1;
    }
    assert_eq!(n, 512);
    let side = |b: u8| (crate::mc4::stick_float(b) < -0.5, crate::mc4::stick_float(b) > 0.5);
    assert_eq!((side(63), side(64), side(191), side(192)), ((true, false), (false, false), (false, false), (false, true)));
}

/// The wager form's amount set-up (F-19872) run offline on the original's code (tools/ui_wager_amount_reference.py):
/// for each base and difficulty the range, the amount and the step equal `wager_range`.
#[test]
fn wager_range_matches_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/ui_wager_amount_ref.txt").lines() {
        let (case, out) = l.split_once(" -> ").unwrap();
        let c: Vec<i64> = case.split(' ').map(|v| v.parse().unwrap()).collect();
        let o: Vec<i32> = out.split(' ').map(|v| v.parse().unwrap()).collect();
        let (lo, hi, mid, step) = crate::game::wager_range(c[0] as i32, c[1] as u32);
        assert_eq!((lo, hi, mid, step), (o[0], o[1], o[2], o[3]), "{l}");
        n += 1;
    }
    assert_eq!(n, 4000);
}

/// The menu slider's left / right (F-59022) run offline on 6,000 cases (tools/ui_slider_reference.py): handled or
/// not, the value after, the sound played and the change delegate, all equal to `game::slider_event`.
#[test]
fn slider_event_matches_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/ui_slider_ref.txt").lines() {
        let (case, out) = l.split_once(" -> ").unwrap();
        let c: Vec<i32> = case.split(' ').map(|v| v.parse().unwrap()).collect();
        let o: Vec<&str> = out.split(' ').collect();
        let (ret, v, snd, dl) = crate::game::slider_event(c[1], c[2], c[3], c[4], c[5] != 0, c[0] as u32, c[6] != 0);
        assert_eq!((ret as u32, v, snd.unwrap_or("-"), dl as u32), (o[0].parse().unwrap(), o[1].parse().unwrap(), o[2], o[3].parse().unwrap()), "{l}");
        n += 1;
    }
    assert_eq!(n, 6000);
}
