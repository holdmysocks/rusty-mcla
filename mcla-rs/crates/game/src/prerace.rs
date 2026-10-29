//! The pre-race scenes' own timing (CAREER_RACES.md [N-CR-2], "Pre-race length").
//!
//! Every race-start cine script (`Game/CineScripts/generated/racestart/<scene>_generated.sco`) carries
//! generic events as (name, "", seconds, 0, 0) records. Three of them time the start: `racename` (67: the
//! race's badge, form 70), `countdown` (58: byte +318 = 1 and START `game/racetypes/countdown_321Go_3sec`,
//! so GO is 3 s after it) and `bookmark_cut_to_game` (72) - or `bookmark` + `cam_to_game` in the starter
//! 1x1, grid 4x2_01 and taunt scenes - where the camera goes back to the game. The script runs on to its
//! end after the cut; the countdown runs by itself from its event. Read from the scripts with
//! `work/career2/ev.py` (the delivery intros' events are in `hudmap::DELIVERY_INTROS`).

/// Event times in scene seconds; `racename` is -1 where the scene has none (taunts 01..09). An event
/// record is (name, "", -1, start, duration, 0): `racename_len` is the badge's duration - form 70 is
/// shown by the event's start (`sub_823D76A8`, case 67) and closed by its end (`sub_823D6E38`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreraceEvents {
    pub scene: &'static str,
    pub racename: f32,
    pub racename_len: f32,
    pub cut: f32,
    pub countdown: f32,
}

impl PreraceEvents {
    /// Scene seconds of GO (`countdown_321Go_3sec`).
    pub fn go(&self) -> f32 {
        self.countdown + 3.0
    }
}

/// The scene's events by its name (`racestart/gen_prerace_grid_2x4_01` or the bare name, any case).
pub fn events(scene: &str) -> Option<PreraceEvents> {
    let name = scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase();
    PRERACE_EVENTS.iter().copied().find(|e| e.scene == name)
}

/// The start-scene table the original plays from: `tune/cutscene/racestartgridmapping_sc.tbl` (sub_823C8218
/// tries the South Central table first and takes `racestartgridmapping.tbl` only when that fails, so the
/// Complete Edition always plays the `_sc` one): `Grid <name> <key> { Cutscene <script> ... }` - grids 0..6,
/// Taunt 7, Safety 8. Scenes as cine names (`racestart/<scene>`, lower case), in the table's order.
#[derive(Default)]
pub struct GridTable {
    pub lists: Vec<(String, Vec<String>)>,
}

impl GridTable {
    pub fn load(data_dir: &str) -> GridTable {
        let dir = std::path::Path::new(data_dir).join("tune/cutscene");
        let text = std::fs::read_to_string(dir.join("racestartgridmapping_sc.tbl")).or_else(|_| std::fs::read_to_string(dir.join("racestartgridmapping.tbl"))).unwrap_or_default();
        GridTable::parse(&text)
    }

    pub fn parse(text: &str) -> GridTable {
        let mut lists: Vec<(String, Vec<String>)> = Vec::new();
        for line in text.lines() {
            let mut w = line.split_whitespace();
            match (w.next(), w.next()) {
                (Some("Grid"), Some(name)) => lists.push((name.to_string(), Vec::new())),
                (Some("Cutscene"), Some(script)) => {
                    if let Some(l) = lists.last_mut() {
                        let name = script.rsplit('/').next().unwrap_or(script).to_ascii_lowercase();
                        l.1.push(format!("racestart/{}", name.trim_end_matches("_generated")));
                    }
                }
                _ => {}
            }
        }
        GridTable { lists }
    }

    /// The table key of a grid name (its place in the table), case blind.
    pub fn key(&self, grid: &str) -> Option<usize> {
        self.lists.iter().position(|l| l.0.eq_ignore_ascii_case(grid))
    }
}

/// What the race script hands the picker (`PickRacestartCutsceneOrTauntFromGridEnum`, sub_823C55B8).
#[derive(Clone, Debug, Default)]
pub struct PickArgs {
    /// the race's GridType ("2x4_Race_Grid"); "1x1_Race_Grid" without opponents, "Safety" for SafetyCam 1
    pub grid: String,
    /// a single race (also restarted), or the first round of a series / tournament / mission
    pub first_race: bool,
    /// `!AllowRaceStarter`
    pub cam_only: bool,
    /// a racer on grid slot 0 or 1 drives a `_lr_` car (the Range Rover)
    pub front_lr: bool,
}

/// The original's start-scene picker. Each list is a shuffle bag (sub_823BE020 / sub_823C01C8): every scene
/// once per round, and a fresh round does not open with the scene that closed the last one. A taunt scene
/// (table key 7) comes for a first race, otherwise on a draw <= 0.333 - never on grid key 0 (1x1); in the
/// `_sc` table a `_lr_` car in front (and not camOnly) limits grids 1..5 to their first five scenes and the
/// taunts to their first four. Nothing else filters (no racer count, time, weather, district).
pub struct Picker {
    table: GridTable,
    bags: std::collections::HashMap<usize, (Vec<usize>, usize)>,
    rng: u32,
}

impl Picker {
    pub fn new(table: GridTable, seed: u32) -> Picker {
        Picker { table, bags: Default::default(), rng: seed | 1 }
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.rand() * n as f32) as usize).min(n.saturating_sub(1))
    }

    /// The next scene of list `key` from its bag; `limit` draws among the first `limit` scenes instead
    /// (the `_lr_` rule; how the original's bag serves it is not read - uniform here).
    fn draw(&mut self, key: usize, limit: Option<usize>) -> Option<String> {
        let n = self.table.lists.get(key)?.1.len();
        if n == 0 {
            return None;
        }
        if let Some(l) = limit {
            let i = self.below(l.min(n));
            return Some(self.table.lists[key].1[i].clone());
        }
        let (mut order, mut at) = self.bags.remove(&key).unwrap_or_default();
        if order.len() != n || at >= n {
            let last = if order.len() == n { order.last().copied() } else { None };
            order = (0..n).collect();
            for i in (1..n).rev() {
                let j = self.below(i + 1);
                order.swap(i, j);
            }
            if n > 1 && last == Some(order[0]) {
                let j = 1 + self.below(n - 1);
                order.swap(0, j);
            }
            at = 0;
        }
        let i = order[at];
        self.bags.insert(key, (order, at + 1));
        Some(self.table.lists[key].1[i].clone())
    }

    /// The scene to play, or None (the original then runs `Game/RaceTypes/generic_countdown`: an unknown
    /// grid such as `16_Race_Grid`).
    pub fn pick(&mut self, a: &PickArgs) -> Option<String> {
        let grid = self.table.key(&a.grid)?;
        let safety = a.grid.eq_ignore_ascii_case("Safety");
        let taunt = !safety && grid != 0 && (a.first_race || self.rand() <= 0.333);
        let key = if taunt { self.table.key("Taunt").unwrap_or(grid) } else { grid };
        let limit = if a.front_lr && !a.cam_only {
            if taunt { Some(4) } else if (1..=5).contains(&grid) { Some(5) } else { None }
        } else {
            None
        };
        self.draw(key, limit)
    }

    /// The list of a grid, as the table has it (test hooks).
    pub fn list(&self, grid: &str) -> Vec<String> {
        self.table.key(grid).map(|k| self.table.lists[k].1.clone()).unwrap_or_default()
    }
}

/// A scene event aimed at a racer slot (kind 2: `CineScript_Events_LaunchEventOnRacerSlot`, sub_823C5220) or
/// at the scene's speech (`playspeech`, `stopaudiostreams`), from the scene script's 7-word event records
/// (name, kind, racer name, slot, time, duration, f3 - f3 is 0 in every race-start scene). Slot i is the race's
/// racer list index i (`Cars_AssignIdToSlot`); `*_left` events go to slot 0, `*_right` to slot 1. `dur` 0 = no
/// end call. Read with `work/career2/re2/evrec.py` -> `work/career2/gen_ev.py`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotEvent {
    pub scene: &'static str,
    pub event: &'static str,
    pub slot: i32,
    pub t: f32,
    pub dur: f32,
}

/// The slot / speech events of a scene, in the script's order. (gesture_left / gesture_right before 1 s are
/// moved to 1 s in a race-start scene, 0x823C5264.)
pub fn slot_events(scene: &str) -> Vec<SlotEvent> {
    let name = scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase();
    SLOT_EVENTS.iter().filter(|e| e.scene == name).map(|e| {
        let mut e = *e;
        if (e.event == "gesture_left" || e.event == "gesture_right") && e.t < 1.0 {
            e.t = 1.0;
        }
        e
    }).collect()
}

/// The taunt speech scenario a scene prepares (`CineScript_RequestPreraceTauntScenarioAudio`, sub_823BFC78):
/// a name with TAUNT -> `TAUNT_PRERACE_HP` (sender slot 1, the hookman; receiver slot 0), else with PH ->
/// `TAUNT_PRERACE_PH` (sender slot 0), else `TAUNT_PRERACE_H` (slot 1 alone). None for other scenes.
pub fn taunt_kind(scene: &str) -> Option<&'static str> {
    let n = scene.rsplit('/').next().unwrap_or(scene).to_ascii_uppercase();
    if !n.contains("TAUNT") {
        return None;
    }
    Some(if n.contains("HP") { "HP" } else if n.contains("PH") { "PH" } else { "H" })
}

pub const SLOT_EVENTS: [SlotEvent; 311] = [
    SlotEvent { scene: "taunt_01", event: "gesture_left", slot: 0, t: 0.0, dur: 7.33333 },
    SlotEvent { scene: "taunt_01", event: "gesture_right", slot: 1, t: 0.0, dur: 7.33333 },
    SlotEvent { scene: "taunt_02", event: "gesture_left", slot: 0, t: 0.0, dur: 6.66667 },
    SlotEvent { scene: "taunt_02", event: "gesture_right", slot: 1, t: 0.0, dur: 6.66667 },
    SlotEvent { scene: "taunt_03", event: "gesture_left", slot: 0, t: 0.0, dur: 6.66667 },
    SlotEvent { scene: "taunt_03", event: "gesture_right", slot: 1, t: 0.0, dur: 6.66667 },
    SlotEvent { scene: "taunt_04", event: "gesture_left", slot: 0, t: 0.0, dur: 6.66667 },
    SlotEvent { scene: "taunt_04", event: "gesture_right", slot: 1, t: 0.0, dur: 6.66667 },
    SlotEvent { scene: "taunt_05", event: "gesture_left", slot: 0, t: 0.0, dur: 6.66667 },
    SlotEvent { scene: "taunt_05", event: "gesture_right", slot: 1, t: 0.0, dur: 6.66667 },
    SlotEvent { scene: "cut_racestart_1x8_01", event: "gesture_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_1x8_01", event: "gesture_right", slot: 1, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_1x8_02", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_1x8_02", event: "gesture_right", slot: 1, t: 4.36667, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_2x4_split_01", event: "gesture_left", slot: 0, t: 2.33333, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_2x4_split_01", event: "gesture_right", slot: 1, t: 3.7, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_2x4_split_02", event: "gesture_left", slot: 0, t: 1.33333, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_2x4_split_02", event: "gesture_right", slot: 1, t: 5.33333, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_01", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_01", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_02", event: "gesture_left", slot: 0, t: 4.7, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_02", event: "gesture_right", slot: 1, t: 6.33333, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_03", event: "gesture_left", slot: 0, t: 3.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_03", event: "gesture_right", slot: 1, t: 4.03333, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_04", event: "gesture_right", slot: 1, t: 4.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_04", event: "gesture_left", slot: 0, t: 5.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_split_01", event: "gesture_right", slot: 1, t: 4.03333, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_split_01", event: "gesture_left", slot: 0, t: 5.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_split_02", event: "gesture_left", slot: 0, t: 1.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_split_02", event: "gesture_right", slot: 1, t: 2.33333, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_safety_1", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_safety_1", event: "playspeech", slot: -1, t: 0.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_safety_1", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_safety_1", event: "gesture_listen_right", slot: 1, t: 1.33333, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_safety_1", event: "window_up_left", slot: 0, t: 6.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_hk_01", event: "gesture_right", slot: 1, t: 1.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_hk_plyr_01", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_hk_plyr_01", event: "gesture_left", slot: 0, t: 5.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_hk_plyr_02", event: "gesture_right", slot: 1, t: 2.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_hk_plyr_02", event: "gesture_left", slot: 0, t: 7.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_plyr_hk_02", event: "gesture_left", slot: 0, t: 6.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_plyr_hk_02", event: "gesture_right", slot: 1, t: 6.33333, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_plyr_hk_01", event: "gesture_right", slot: 1, t: 3.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_plyr_hk_01", event: "gesture_left", slot: 0, t: 7.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_1x8_01", event: "gesture_left", slot: 0, t: 3.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_1x8_01", event: "gesture_right", slot: 1, t: 4.03333, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_1x8_02", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_1x8_02", event: "gesture_right", slot: 1, t: 8.33333, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_01", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_01", event: "gesture_right", slot: 1, t: 1.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_02", event: "gesture_right", slot: 1, t: 1.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_02", event: "gesture_left", slot: 0, t: 5.5, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_03", event: "gesture_left", slot: 0, t: 5.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_03", event: "gesture_right", slot: 1, t: 5.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_04", event: "gesture_left", slot: 0, t: 5.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_04", event: "gesture_right", slot: 1, t: 6.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_split_01", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_split_01", event: "gesture_right", slot: 1, t: 2.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_split_02", event: "gesture_left", slot: 0, t: 2.83333, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_split_02", event: "gesture_right", slot: 1, t: 5.76667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_01", event: "gesture_right", slot: 1, t: 3.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_01", event: "gesture_left", slot: 0, t: 7.16667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_02", event: "gesture_right", slot: 1, t: 4.33333, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_02", event: "gesture_left", slot: 0, t: 6.33333, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_03", event: "gesture_left", slot: 0, t: 2.03333, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_03", event: "gesture_right", slot: 1, t: 2.03333, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_04", event: "gesture_left", slot: 0, t: 5.7, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_04", event: "gesture_right", slot: 1, t: 5.7, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_05", event: "gesture_left", slot: 0, t: 4.36667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_05", event: "gesture_right", slot: 1, t: 4.36667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_06", event: "gesture_left", slot: 0, t: 4.03333, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_06", event: "gesture_right", slot: 1, t: 4.03333, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_split_01", event: "gesture_left", slot: 0, t: 1.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_split_01", event: "gesture_right", slot: 1, t: 1.66667, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_split_02", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_split_02", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_4x2_01", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_4x2_01", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_4x2_02", event: "gesture_left", slot: 0, t: 4.8, dur: 0.0 },
    SlotEvent { scene: "cut_racestarter_4x2_02", event: "gesture_right", slot: 1, t: 4.8, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_delivery_1x1_01", event: "gesture_neutral", slot: 0, t: 2.33333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_delivery_1x1_02", event: "gesture_neutral", slot: 0, t: 2.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_delivery_1x1_03", event: "gesture_neutral", slot: 0, t: 2.16667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_1x1_01", event: "gesture_neutral", slot: 0, t: 3.5, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_1x1_02", event: "gesture_neutral", slot: 0, t: 1.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_1x1_03", event: "gesture_neutral", slot: 0, t: 1.33333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_1x8_07", event: "gesture_right", slot: 1, t: 0.0, dur: 6.26667 },
    SlotEvent { scene: "gen_prerace_grid_1x8_07", event: "gesture_left", slot: 0, t: 4.0, dur: 2.26667 },
    SlotEvent { scene: "gen_prerace_grid_2x4_01", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 5.16667 },
    SlotEvent { scene: "gen_prerace_grid_2x4_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 5.16667 },
    SlotEvent { scene: "gen_prerace_grid_2x4_02", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_02", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_02", event: "gesture_thm_dwn_lft", slot: 0, t: 1.5, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_02", event: "window_up_left", slot: 0, t: 5.33333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_03", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_03", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 3.66667 },
    SlotEvent { scene: "gen_prerace_grid_2x4_03", event: "gesture_thm_dwn_rgt", slot: 1, t: 3.5, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_04", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 5.66667 },
    SlotEvent { scene: "gen_prerace_grid_2x4_04", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_05", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_05", event: "gesture_listen_right", slot: 1, t: 0.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_06", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 5.33333 },
    SlotEvent { scene: "gen_prerace_grid_2x4_06", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_07", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_07", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 3.86667 },
    SlotEvent { scene: "gen_prerace_grid_2x4_07", event: "gesture_thm_dwn_rgt", slot: 1, t: 3.9, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_02", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 4.6 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_02", event: "gesture_thm_dwn_rgt", slot: 1, t: 0.23333, dur: 1.66667 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_02", event: "gesture_listen_right", slot: 1, t: 1.93333, dur: 2.66667 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_03", event: "gesture_left", slot: 0, t: 0.0, dur: 8.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_03", event: "gesture_right", slot: 1, t: 0.0, dur: 8.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_04", event: "gesture_left", slot: 0, t: 0.0, dur: 9.1 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_04", event: "gesture_right", slot: 1, t: 0.0, dur: 9.1 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_05", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_05", event: "gesture_thm_dwn_lft", slot: 0, t: 0.5, dur: 2.6 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_05", event: "gesture_listen_right", slot: 1, t: 0.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_05", event: "gesture_listen_left", slot: 0, t: 3.13333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_05", event: "window_up_left", slot: 0, t: 5.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_06", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_06", event: "gesture_thm_dwn_lft", slot: 0, t: 0.5, dur: 2.6 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_06", event: "gesture_listen_left", slot: 0, t: 3.13333, dur: 2.53333 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_07", event: "gesture_right", slot: 1, t: 1.0, dur: 5.13333 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_07", event: "gesture_left", slot: 0, t: 2.5, dur: 3.63333 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_08", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 2.46667 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_08", event: "gesture_listen_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_08", event: "gesture_thm_dwn_rgt", slot: 1, t: 3.16667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_02", event: "gesture_listen_right", slot: 1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_02", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_03", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_03", event: "gesture_neutral", slot: 0, t: 1.16667, dur: 1.4 },
    SlotEvent { scene: "gen_prerace_grid_3x3_03", event: "gesture_listen_left", slot: 0, t: 2.6, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_04", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 2.8 },
    SlotEvent { scene: "gen_prerace_grid_3x3_04", event: "gesture_listen_right", slot: 1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_04", event: "gesture_thm_dwn_lft", slot: 0, t: 2.83333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_05", event: "gesture_listen_left", slot: 0, t: 0.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_05", event: "gesture_thm_dwn_rgt", slot: 1, t: 1.0, dur: 1.33333 },
    SlotEvent { scene: "gen_prerace_grid_3x3_05", event: "gesture_listen_right", slot: 1, t: 2.36667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_06", event: "gesture_listen_left", slot: 0, t: 0.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_06", event: "gesture_thm_dwn_rgt", slot: 1, t: 2.0, dur: 1.93333 },
    SlotEvent { scene: "gen_prerace_grid_3x3_06", event: "gesture_listen_right", slot: 1, t: 3.96667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_07", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 3.66667 },
    SlotEvent { scene: "gen_prerace_grid_3x3_07", event: "gesture_listen_left", slot: 0, t: 2.83333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_07", event: "gesture_neutral", slot: 1, t: 3.83333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_08", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 6.83333 },
    SlotEvent { scene: "gen_prerace_grid_3x3_08", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 3.66667 },
    SlotEvent { scene: "gen_prerace_grid_3x3_08", event: "gesture_thm_dwn_rgt", slot: 1, t: 4.03333, dur: 2.8 },
    SlotEvent { scene: "gen_prerace_grid_4x2_01", event: "gesture_right", slot: 1, t: 1.0, dur: 10.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_01", event: "gesture_left", slot: 0, t: 2.83333, dur: 8.16667 },
    SlotEvent { scene: "gen_prerace_grid_4x2_02", event: "gesture_neutral", slot: 0, t: 0.0, dur: 7.6 },
    SlotEvent { scene: "gen_prerace_grid_4x2_02", event: "gesture_neutral", slot: 1, t: 0.0, dur: 7.6 },
    SlotEvent { scene: "gen_prerace_grid_4x2_03", event: "gesture_neutral", slot: 0, t: 0.0, dur: 8.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_03", event: "gesture_neutral", slot: 1, t: 0.0, dur: 8.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_04", event: "gesture_neutral", slot: 0, t: 0.0, dur: 9.1 },
    SlotEvent { scene: "gen_prerace_grid_4x2_04", event: "gesture_neutral", slot: 1, t: 0.0, dur: 9.1 },
    SlotEvent { scene: "gen_prerace_grid_4x2_05", event: "gesture_neutral", slot: 0, t: 0.0, dur: 7.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_05", event: "gesture_neutral", slot: 1, t: 0.0, dur: 7.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_06", event: "gesture_neutral", slot: 1, t: 2.0, dur: 4.8 },
    SlotEvent { scene: "gen_prerace_grid_4x2_06", event: "gesture_neutral", slot: 0, t: 4.0, dur: 2.8 },
    SlotEvent { scene: "gen_prerace_grid_4x2_07", event: "gesture_neutral", slot: 1, t: 1.0, dur: 5.13333 },
    SlotEvent { scene: "gen_prerace_grid_4x2_07", event: "gesture_neutral", slot: 0, t: 2.5, dur: 3.63333 },
    SlotEvent { scene: "gen_prerace_grid_4x2_08", event: "gesture_neutral", slot: 0, t: 2.66667, dur: 4.16667 },
    SlotEvent { scene: "gen_prerace_grid_4x2_08", event: "gesture_neutral", slot: 1, t: 3.66667, dur: 3.16667 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 6.63333 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "gesture_neutral", slot: 1, t: 0.83333, dur: 2.63333 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "gesture_listen_right", slot: 1, t: 3.5, dur: 8.73333 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "gesture_thm_dwn_lft", slot: 0, t: 6.66667, dur: 5.56667 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "window_up_left", slot: 0, t: 9.73333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "stopaudiostreams", slot: -1, t: 13.2, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 2.46667 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "window_down_right", slot: 1, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "gesture_listen_left", slot: 0, t: 1.33333, dur: 7.63333 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "gesture_thm_dwn_rgt", slot: 1, t: 3.56667, dur: 5.4 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "window_up_right", slot: 1, t: 6.83333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "stopaudiostreams", slot: -1, t: 9.96667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_03", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_03", event: "gesture_listen_right", slot: 1, t: 0.33333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_03", event: "gesture_thm_dwn_lft", slot: 0, t: 1.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_03", event: "window_up_left", slot: 1, t: 3.23333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_03", event: "stopaudiostreams", slot: -1, t: 6.8, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "window_down_right", slot: 1, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "gesture_listen_left", slot: 0, t: 0.33333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "gesture_thm_dwn_rgt", slot: 1, t: 1.2, dur: 2.93333 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "gesture_listen_right", slot: 1, t: 4.16667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "window_up_right", slot: 1, t: 4.2, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "stopaudiostreams", slot: -1, t: 8.9, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "gesture_listen_right", slot: 1, t: 0.33333, dur: 4.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "gesture_thm_dwn_lft", slot: 0, t: 2.33333, dur: 2.3 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "gesture_neutral", slot: 1, t: 4.36667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "gesture_listen_left", slot: 0, t: 4.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "window_up_left", slot: 0, t: 6.5, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "stopaudiostreams", slot: -1, t: 11.4, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_06", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_06", event: "gesture_thm_dwn_rgt", slot: 1, t: 2.5, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_06", event: "stopaudiostreams", slot: -1, t: 7.7, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_07", event: "gesture_listen_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_07", event: "gesture_thm_dwn_rgt", slot: 1, t: 6.33333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_07", event: "stopaudiostreams", slot: -1, t: 10.2333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_08", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 5.8 },
    SlotEvent { scene: "gen_prerace_starter_2x4_08", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 5.8 },
    SlotEvent { scene: "gen_prerace_starter_2x4_08", event: "stopaudiostreams", slot: -1, t: 6.8, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_09", event: "gesture_neutral", slot: 1, t: 0.0, dur: 3.63333 },
    SlotEvent { scene: "gen_prerace_starter_2x4_09", event: "gesture_listen_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_09", event: "gesture_listen_right", slot: 1, t: 3.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_09", event: "stopaudiostreams", slot: -1, t: 10.2333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_10", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_10", event: "gesture_listen_right", slot: 1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_10", event: "stopaudiostreams", slot: -1, t: 7.3, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_11", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 9.13333 },
    SlotEvent { scene: "gen_prerace_starter_2x4_11", event: "gesture_listen_left", slot: 0, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_11", event: "gesture_thm_dwn_rgt", slot: 1, t: 9.16667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_11", event: "stopaudiostreams", slot: -1, t: 15.4333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_01", event: "gesture_left", slot: 0, t: 0.83333, dur: 9.36667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_01", event: "gesture_right", slot: 1, t: 0.83333, dur: 9.36667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_02", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.16667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_02", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.16667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_03", event: "gesture_left", slot: 0, t: 0.83333, dur: 5.06667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_03", event: "gesture_right", slot: 1, t: 0.83333, dur: 5.06667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_04", event: "gesture_left", slot: 0, t: 0.83333, dur: 5.33333 },
    SlotEvent { scene: "gen_prerace_starter_3x3_04", event: "gesture_right", slot: 1, t: 0.83333, dur: 5.33333 },
    SlotEvent { scene: "gen_prerace_starter_3x3_05", event: "gesture_left", slot: 0, t: 0.83333, dur: 7.66667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_05", event: "gesture_right", slot: 1, t: 0.83333, dur: 7.66667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_06", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.86667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_06", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.86667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_07", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.5 },
    SlotEvent { scene: "gen_prerace_starter_3x3_07", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.5 },
    SlotEvent { scene: "gen_prerace_starter_3x3_08", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.83333 },
    SlotEvent { scene: "gen_prerace_starter_3x3_08", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.83333 },
    SlotEvent { scene: "gen_prerace_starter_3x3_09", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.33333 },
    SlotEvent { scene: "gen_prerace_starter_3x3_09", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.33333 },
    SlotEvent { scene: "gen_prerace_starter_3x3_10", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.46667 },
    SlotEvent { scene: "gen_prerace_starter_3x3_10", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.46667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_01", event: "gesture_left", slot: 0, t: 0.83333, dur: 9.36667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_01", event: "gesture_right", slot: 1, t: 0.83333, dur: 9.36667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_02", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.16667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_02", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.16667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_03", event: "gesture_left", slot: 0, t: 0.83333, dur: 5.06667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_03", event: "gesture_right", slot: 1, t: 0.83333, dur: 5.06667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_04", event: "gesture_left", slot: 0, t: 0.83333, dur: 5.33333 },
    SlotEvent { scene: "gen_prerace_starter_4x2_04", event: "gesture_right", slot: 1, t: 0.83333, dur: 5.33333 },
    SlotEvent { scene: "gen_prerace_starter_4x2_05", event: "gesture_left", slot: 0, t: 0.83333, dur: 7.66667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_05", event: "gesture_right", slot: 1, t: 0.83333, dur: 7.66667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_06", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.86667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_06", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.86667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_07", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.5 },
    SlotEvent { scene: "gen_prerace_starter_4x2_07", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.5 },
    SlotEvent { scene: "gen_prerace_starter_4x2_08", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.83333 },
    SlotEvent { scene: "gen_prerace_starter_4x2_08", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.83333 },
    SlotEvent { scene: "gen_prerace_starter_4x2_09", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.33333 },
    SlotEvent { scene: "gen_prerace_starter_4x2_09", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.33333 },
    SlotEvent { scene: "gen_prerace_starter_4x2_10", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.46667 },
    SlotEvent { scene: "gen_prerace_starter_4x2_10", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.46667 },
    SlotEvent { scene: "gen_prerace_taunt_01", event: "gesture_right", slot: 1, t: 2.1, dur: 4.9 },
    SlotEvent { scene: "gen_prerace_taunt_01", event: "gesture_left", slot: 0, t: 3.3, dur: 3.7 },
    SlotEvent { scene: "gen_prerace_taunt_02", event: "gesture_left", slot: 0, t: 1.2, dur: 4.8 },
    SlotEvent { scene: "gen_prerace_taunt_02", event: "gesture_right", slot: 1, t: 2.73333, dur: 3.26667 },
    SlotEvent { scene: "gen_prerace_taunt_03", event: "gesture_right", slot: 1, t: 1.83333, dur: 3.16667 },
    SlotEvent { scene: "gen_prerace_taunt_03", event: "gesture_left", slot: 0, t: 3.3, dur: 1.7 },
    SlotEvent { scene: "gen_prerace_taunt_07", event: "gesture_left", slot: 0, t: 0.0, dur: 8.0 },
    SlotEvent { scene: "gen_prerace_taunt_07", event: "gesture_right", slot: 1, t: 2.6, dur: 5.4 },
    SlotEvent { scene: "gen_prerace_taunt_09", event: "gesture_right", slot: 1, t: 0.0, dur: 9.0 },
    SlotEvent { scene: "gen_prerace_taunt_09", event: "gesture_left", slot: 0, t: 3.66667, dur: 5.33333 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 3.33333 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "playspeech", slot: -1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "gesture_talk_right", slot: 1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "gesture_talk_left", slot: 0, t: 4.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "window_up_left", slot: 0, t: 7.33333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 5.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 4.33333 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "playspeech", slot: -1, t: 0.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "gesture_talk_left", slot: 0, t: 4.36667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "gesture_listen_right", slot: 1, t: 5.16667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "window_up_left", slot: 0, t: 7.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "gesture_talk_right", slot: 1, t: 0.66667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "gesture_listen_left", slot: 0, t: 0.66667, dur: 3.73333 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "playspeech", slot: -1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "gesture_talk_left", slot: 0, t: 4.43333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "window_up_left", slot: 0, t: 8.63333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_h_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 6.0 },
    SlotEvent { scene: "gen_prerace_taunt_h_01", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 6.0 },
    SlotEvent { scene: "gen_prerace_taunt_h_01", event: "playspeech", slot: -1, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_01", event: "gesture_talk_left", slot: 0, t: 0.0, dur: 4.13333 },
    SlotEvent { scene: "gen_prerace_taunt_ph_01", event: "gesture_listen_right", slot: 1, t: 1.0, dur: 3.1 },
    SlotEvent { scene: "gen_prerace_taunt_ph_01", event: "playspeech", slot: -1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_01", event: "gesture_talk_right", slot: 1, t: 4.13333, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_01", event: "gesture_listen_left", slot: 0, t: 4.16667, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_02", event: "gesture_talk_left", slot: 0, t: 0.0, dur: 5.13333 },
    SlotEvent { scene: "gen_prerace_taunt_ph_02", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 3.36667 },
    SlotEvent { scene: "gen_prerace_taunt_ph_02", event: "playspeech", slot: -1, t: 1.0, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_02", event: "gesture_talk_right", slot: 1, t: 3.5, dur: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_02", event: "gesture_listen_left", slot: 0, t: 5.16667, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_1x1_01", event: "gesture_neutral", slot: 0, t: 1.33333, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_01", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 1.8 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_01", event: "gesture_listen_right", slot: 1, t: 3.0, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_02", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_02", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 1.8 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_02", event: "gesture_listen_right", slot: 1, t: 3.0, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_03", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_03", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 1.8 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_03", event: "gesture_listen_right", slot: 1, t: 3.0, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_01", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 1.8 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_01", event: "gesture_listen_right", slot: 1, t: 3.0, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_02", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_02", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 1.8 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_02", event: "gesture_listen_right", slot: 1, t: 3.0, dur: 0.0 },
];

/// A start scene's racer events in flight (main.rs `prerace_acts_*`).
pub struct Acts {
    pub scene: String,
    pub events: Vec<SlotEvent>,
    pub fired: Vec<bool>,
    /// the scene's clock and the clip's length
    pub t: f32,
    pub duration: f32,
    /// slot i -> the car of the race's racer i
    pub racers: Vec<usize>,
    pub ends: Vec<(usize, f32)>,
    /// the taunt speech was prepared
    pub taunt: bool,
}

/// The speech profile (GH_01 .., SH_01 ..) of a driver's character set (`tune/audio/CareerVoices.xml` and
/// `SC_CareerVoices.xml`: `<Voice name="DRV_MB_007_SET" ProfileName="GH_01" ...>`); the in-car set
/// `drv_mb_007_lod02_set` is the same character.
pub fn voice_of(data_dir: &str, set: &str) -> Option<String> {
    static MAP: std::sync::OnceLock<std::collections::HashMap<String, String>> = std::sync::OnceLock::new();
    let map = MAP.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        for f in ["CareerVoices.xml", "SC_CareerVoices.xml"] {
            let t = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/audio").join(f)).unwrap_or_default();
            for line in t.lines() {
                let get = |k: &str| line.split(&format!("{k}=\"")).nth(1).and_then(|r| r.split('"').next()).map(|v| v.to_string());
                if let (Some(n), Some(p)) = (get("name"), get("ProfileName")) {
                    if n.starts_with("DRV_") || n.starts_with("MEC_") {
                        m.entry(n.to_ascii_uppercase()).or_insert(p);
                    }
                }
            }
        }
        m
    });
    let key = set.to_ascii_uppercase().replace("_LOD02_", "_").replace("_LOD2_", "_");
    // (the hi-res in-car set `drv_mb_007_set_h`, cinedriver::DRIVER_MAPS, is the same character)
    let key = key.strip_suffix("_H").map_or(key.clone(), |k| k.to_string());
    map.get(&key).filter(|p| !p.starts_with("PLAYER") && !p.starts_with("MISSING")).cloned()
}

pub const PRERACE_EVENTS: [PreraceEvents; 82] = [
    PreraceEvents { scene: "gen_prerace_grid_1x1_01", racename: 1.8, racename_len: 2.0, cut: 6.3, countdown: 4.33333 },
    PreraceEvents { scene: "gen_prerace_grid_1x1_02", racename: 1.3, racename_len: 1.96667, cut: 5.66667, countdown: 3.7 },
    PreraceEvents { scene: "gen_prerace_grid_1x1_03", racename: 1.26667, racename_len: 1.93333, cut: 5.46667, countdown: 3.5 },
    PreraceEvents { scene: "gen_prerace_grid_1x8_07", racename: 1.53333, racename_len: 2.0, cut: 6.26667, countdown: 4.3 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_01", racename: 1.03333, racename_len: 2.0, cut: 5.16667, countdown: 3.2 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_02", racename: 1.26667, racename_len: 1.66667, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_03", racename: 1.13333, racename_len: 1.66667, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_04", racename: 1.33333, racename_len: 1.7, cut: 5.66667, countdown: 3.7 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_05", racename: 1.16667, racename_len: 2.0, cut: 5.5, countdown: 3.53333 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_06", racename: 1.16667, racename_len: 1.93333, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_07", racename: 1.13333, racename_len: 1.93333, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_02", racename: 1.13333, racename_len: 1.96667, cut: 4.6, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_03", racename: 2.0, racename_len: 2.0, cut: 5.0, countdown: 5.0 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_04", racename: 3.66667, racename_len: 2.0, cut: 6.1, countdown: 6.1 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_05", racename: 1.3, racename_len: 2.0, cut: 5.66667, countdown: 3.7 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_06", racename: 1.16667, racename_len: 2.0, cut: 5.66667, countdown: 3.7 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_07", racename: 2.13333, racename_len: 2.0, cut: 6.13333, countdown: 6.13333 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_08", racename: 1.33333, racename_len: 2.0, cut: 5.83333, countdown: 3.86667 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_02", racename: 1.26667, racename_len: 1.5, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_03", racename: 1.16667, racename_len: 1.9, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_04", racename: 1.4, racename_len: 2.0, cut: 6.1, countdown: 4.13333 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_05", racename: 1.46667, racename_len: 2.0, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_06", racename: 1.1, racename_len: 1.66667, cut: 5.5, countdown: 3.53333 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_07", racename: 1.5, racename_len: 2.0, cut: 6.13333, countdown: 4.16667 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_08", racename: 2.0, racename_len: 2.0, cut: 6.83333, countdown: 4.86667 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_01", racename: 5.5, racename_len: 2.0, cut: 8.0, countdown: 8.0 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_02", racename: 1.33333, racename_len: 2.0, cut: 4.6, countdown: 4.6 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_03", racename: 2.0, racename_len: 2.0, cut: 5.0, countdown: 5.0 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_04", racename: 3.66667, racename_len: 2.0, cut: 6.1, countdown: 6.1 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_05", racename: 2.0, racename_len: 5.0, cut: 7.0, countdown: 7.0 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_06", racename: 3.8, racename_len: 2.0, cut: 6.8, countdown: 6.8 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_07", racename: 2.13333, racename_len: 2.0, cut: 6.13333, countdown: 6.13333 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_08", racename: 2.83333, racename_len: 2.0, cut: 6.83333, countdown: 6.83333 },
    PreraceEvents { scene: "gen_prerace_starter_1x1_01", racename: 6.73333, racename_len: 2.0, cut: 9.23333, countdown: 9.23333 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_01", racename: 7.7, racename_len: 2.0, cut: 12.2, countdown: 10.2 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_02", racename: 3.03333, racename_len: 2.0, cut: 8.96667, countdown: 6.96667 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_03", racename: 1.76667, racename_len: 2.0, cut: 5.8, countdown: 3.8 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_04", racename: 3.16667, racename_len: 2.0, cut: 7.93333, countdown: 5.9 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_05", racename: 4.36667, racename_len: 2.0, cut: 10.4, countdown: 8.4 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_06", racename: 2.16667, racename_len: 2.0, cut: 6.7, countdown: 4.7 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_07", racename: 2.93333, racename_len: 2.0, cut: 9.23333, countdown: 7.23333 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_08", racename: 1.33333, racename_len: 2.0, cut: 5.8, countdown: 3.8 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_09", racename: 4.66667, racename_len: 2.0, cut: 9.2, countdown: 7.23333 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_10", racename: 1.33333, racename_len: 2.0, cut: 6.3, countdown: 4.3 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_11", racename: 5.03333, racename_len: 4.4, cut: 14.4333, countdown: 12.4333 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_01", racename: 7.7, racename_len: 2.0, cut: 10.2, countdown: 10.2 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_02", racename: 3.03333, racename_len: 2.0, cut: 7.0, countdown: 7.0 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_03", racename: 1.33333, racename_len: 2.0, cut: 5.9, countdown: 3.83333 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_04", racename: 3.66667, racename_len: 2.0, cut: 6.16667, countdown: 6.16667 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_05", racename: 6.0, racename_len: 2.0, cut: 8.5, countdown: 8.5 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_06", racename: 2.16667, racename_len: 2.0, cut: 4.7, countdown: 4.7 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_07", racename: 2.5, racename_len: 2.0, cut: 7.33333, countdown: 7.33333 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_08", racename: 2.16667, racename_len: 2.0, cut: 4.66667, countdown: 4.66667 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_09", racename: 4.66667, racename_len: 2.0, cut: 7.16667, countdown: 7.16667 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_10", racename: 1.33333, racename_len: 2.0, cut: 4.3, countdown: 4.3 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_01", racename: 7.7, racename_len: 2.0, cut: 10.2, countdown: 10.2 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_02", racename: 3.03333, racename_len: 2.0, cut: 7.0, countdown: 7.0 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_03", racename: 1.33333, racename_len: 2.0, cut: 5.9, countdown: 3.83333 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_04", racename: 3.66667, racename_len: 2.0, cut: 6.16667, countdown: 6.16667 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_05", racename: 6.0, racename_len: 2.0, cut: 8.5, countdown: 8.5 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_06", racename: 2.16667, racename_len: 2.0, cut: 4.7, countdown: 4.7 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_07", racename: 2.5, racename_len: 2.0, cut: 7.33333, countdown: 7.33333 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_08", racename: 2.16667, racename_len: 2.0, cut: 4.66667, countdown: 4.66667 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_09", racename: 4.66667, racename_len: 2.0, cut: 7.16667, countdown: 7.16667 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_10", racename: 1.33333, racename_len: 2.0, cut: 4.3, countdown: 4.3 },
    PreraceEvents { scene: "gen_prerace_taunt_01", racename: -1.0, racename_len: 0.0, cut: 7.2, countdown: 7.2 },
    PreraceEvents { scene: "gen_prerace_taunt_02", racename: -1.0, racename_len: 0.0, cut: 6.7, countdown: 6.7 },
    PreraceEvents { scene: "gen_prerace_taunt_03", racename: -1.0, racename_len: 0.0, cut: 5.03333, countdown: 5.03333 },
    PreraceEvents { scene: "gen_prerace_taunt_07", racename: -1.0, racename_len: 0.0, cut: 8.0, countdown: 8.0 },
    PreraceEvents { scene: "gen_prerace_taunt_09", racename: -1.0, racename_len: 0.0, cut: 9.0, countdown: 9.0 },
    PreraceEvents { scene: "gen_prerace_taunt_hp_01", racename: 1.83333, racename_len: 1.83333, cut: 7.33333, countdown: 5.36667 },
    PreraceEvents { scene: "gen_prerace_taunt_hp_02", racename: 1.33333, racename_len: 2.03333, cut: 7.66667, countdown: 5.7 },
    PreraceEvents { scene: "gen_prerace_taunt_hp_03", racename: 2.66667, racename_len: 2.0, cut: 8.63333, countdown: 6.66667 },
    PreraceEvents { scene: "gen_prerace_taunt_h_01", racename: 1.0, racename_len: 2.0, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "gen_prerace_taunt_ph_01", racename: 2.6, racename_len: 1.3, cut: 7.33333, countdown: 5.36667 },
    PreraceEvents { scene: "gen_prerace_taunt_ph_02", racename: 1.6, racename_len: 2.0, cut: 7.66667, countdown: 5.7 },
    PreraceEvents { scene: "sc_gen_prerace_grid_1x1_01", racename: 1.66667, racename_len: 2.0, cut: 6.0, countdown: 4.0 },
    PreraceEvents { scene: "sc_gen_prerace_grid_2x4_01", racename: 1.66667, racename_len: 2.0, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "sc_gen_prerace_grid_2x4_02", racename: 1.66667, racename_len: 2.0, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "sc_gen_prerace_grid_2x4_03", racename: 1.66667, racename_len: 2.0, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "sc_gen_prerace_grid_2x4_split_01", racename: 1.66667, racename_len: 2.0, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "sc_gen_prerace_grid_2x4_split_02", racename: 1.66667, racename_len: 2.0, cut: 6.0, countdown: 4.03333 },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerace_scene_events() {
        // (values as in the scripts; the AROUND HOLLYWOOD recording's placement -> GO 11.7 s fits a
        // starter scene's countdown + 3 plus the 0.8 s from placement to the scene)
        let e = events("racestart/gen_prerace_starter_2x4_01").unwrap();
        assert_eq!((e.racename, e.countdown, e.cut), (7.7, 10.2, 12.2));
        assert!((e.go() - 13.2).abs() < 1e-4);
        assert_eq!(events("gen_prerace_grid_1x1_01").unwrap().countdown, 4.33333);
        // (the badge's own length: the racename record's duration - the AROUND HOLLYWOOD recording's
        // taunt HP_02 shows it 8.19 -> ~10.4 s, its event says 1.333 + 2.033)
        let hp = events("racestart/gen_prerace_taunt_HP_02").unwrap();
        assert_eq!((hp.racename, hp.racename_len, hp.countdown, hp.cut), (1.33333, 2.03333, 5.7, 7.66667));
        // (South Central's scene is not taken for the other city's of the same name)
        assert_eq!(events("racestart/SC_gen_prerace_grid_1x1_01").unwrap().countdown, 4.0);
        assert_eq!(events("racestart/gen_prerace_grid_4x2_01").unwrap().cut, 8.0);
        assert!(events("racestart/gen_prerace_delivery_1x1_01").is_none());
        // (the taunt HP_01's racer events: slot 0 listens and rolls its left window, slot 1 talks first)
        let ev = slot_events("racestart/gen_prerace_taunt_HP_01");
        assert_eq!(ev.iter().map(|e| (e.event, e.slot)).collect::<Vec<_>>(), vec![("gesture_listen_left", 0), ("window_down_left", 0), ("playspeech", -1), ("gesture_talk_right", 1), ("gesture_talk_left", 0), ("window_up_left", 0)]);
        assert_eq!((ev[0].t, ev[0].dur), (0.0, 3.33333));
        assert_eq!(taunt_kind("racestart/gen_prerace_taunt_PH_02"), Some("PH"));
        assert_eq!(taunt_kind("racestart/gen_prerace_taunt_H_01"), Some("H"));
        assert_eq!(taunt_kind("racestart/gen_prerace_starter_2x4_01"), None);
        // (the grid table: a 4x2 grid plays the 3x3 grid scenes and the 2x4 starter scenes, never the 4x2 ones)
        let tbl = "Grid\t1x1_Race_Grid\t0\n{\n\tCutscene\tgen_prerace_grid_1x1_01_generated\n}\nGrid\t4x2_Race_Grid\t4\n{\n\tCutscene game/CineScripts/generated/RaceStart/sc_gen_prerace_grid_2x4_01_generated\n\tCutscene\tgen_prerace_starter_2x4_01_generated\n\tCutscene\tgen_prerace_starter_2x4_02_generated\n}\nGrid\tTaunt\t7\n{\n\tCutscene\tgen_prerace_taunt_HP_01_generated\n}\n";
        let mut p = Picker::new(GridTable::parse(tbl), 7);
        assert_eq!(p.list("4x2_Race_Grid"), vec!["racestart/sc_gen_prerace_grid_2x4_01", "racestart/gen_prerace_starter_2x4_01", "racestart/gen_prerace_starter_2x4_02"]);
        // a first race on a grid gets a taunt; the 1x1 grid never does
        let first = PickArgs { grid: "4x2_Race_Grid".into(), first_race: true, ..Default::default() };
        assert_eq!(p.pick(&first).as_deref(), Some("racestart/gen_prerace_taunt_hp_01"));
        let one = PickArgs { grid: "1x1_Race_Grid".into(), first_race: true, ..Default::default() };
        assert_eq!(p.pick(&one).as_deref(), Some("racestart/gen_prerace_grid_1x1_01"));
        assert!(p.pick(&PickArgs { grid: "16_Race_Grid".into(), ..Default::default() }).is_none());
        // the bag: every scene once per round, no repeat across the round's seam
        let mut q = Picker::new(GridTable::parse(tbl), 99);
        let k = q.table.key("4x2_Race_Grid").unwrap();
        let seen: Vec<String> = (0..30).map(|_| q.draw(k, None).unwrap()).collect();
        for r in seen.chunks(3) {
            let mut v = r.to_vec();
            v.sort();
            v.dedup();
            assert_eq!(v.len(), 3);
        }
        assert!(seen.windows(2).all(|w| w[0] != w[1]));
        for e in PRERACE_EVENTS.iter() {
            // the camera never goes back to the game before the countdown starts
            assert!(e.cut >= e.countdown && e.countdown > 3.0 && e.cut - e.countdown < 2.1, "{}", e.scene);
        }
    }
}
