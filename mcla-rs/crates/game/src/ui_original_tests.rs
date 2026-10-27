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
