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
