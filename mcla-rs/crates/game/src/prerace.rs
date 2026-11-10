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
/// shown by the event's start (`F-91263`, case 67) and closed by its end (`F-26895`).
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

/// The start-scene table the original plays from: `tune/cutscene/racestartgridmapping_sc.tbl` (F-36338
/// tries the South Central table first and takes `racestartgridmapping.tbl` only when that fails, so the
/// Complete Edition always plays the `_sc` one): `Grid <name> <key> { Cutscene <script> ... }` - grids 0..6,
/// Taunt 7, Safety 8. Scenes as cine names (`racestart/<scene>`, lower case), in the table's order.
#[derive(Default)]
pub struct GridTable {
    pub lists: Vec<(String, Vec<String>)>,
    /// each grid's integer key (`Grid <name> <key>`), by place
    pub keys: Vec<i32>,
    /// the South Central table was loaded (the mapper's +28: the `_lr_` sub-lists apply)
    pub sc: bool,
}

impl GridTable {
    pub fn load(data_dir: &str) -> GridTable {
        let dir = std::path::Path::new(data_dir).join("tune/cutscene");
        match std::fs::read_to_string(dir.join("racestartgridmapping_sc.tbl")) {
            Ok(t) => GridTable { sc: true, ..GridTable::parse(&t) },
            Err(_) => GridTable::parse(&std::fs::read_to_string(dir.join("racestartgridmapping.tbl")).unwrap_or_default()),
        }
    }

    pub fn parse(text: &str) -> GridTable {
        let mut lists: Vec<(String, Vec<String>)> = Vec::new();
        let mut keys: Vec<i32> = Vec::new();
        for line in text.lines() {
            let mut w = line.split_whitespace();
            match (w.next(), w.next()) {
                (Some("Grid"), Some(name)) => {
                    keys.push(w.next().and_then(|k| k.parse().ok()).unwrap_or(lists.len() as i32));
                    lists.push((name.to_string(), Vec::new()));
                }
                (Some("Cutscene"), Some(script)) => {
                    if let Some(l) = lists.last_mut() {
                        let name = script.rsplit('/').next().unwrap_or(script).to_ascii_lowercase();
                        l.1.push(format!("racestart/{}", name.trim_end_matches("_generated")));
                    }
                }
                _ => {}
            }
        }
        GridTable { lists, keys, sc: false }
    }

    /// The place of the grid with integer key `key` (F-24279 searches the entries for it).
    pub fn place_of_key(&self, key: i32) -> Option<usize> {
        self.keys.iter().position(|&k| k == key)
    }

    /// The table key of a grid name (its place in the table), case blind.
    pub fn key(&self, grid: &str) -> Option<usize> {
        self.lists.iter().position(|l| l.0.eq_ignore_ascii_case(grid))
    }
}

/// What the race script hands the picker (`PickRacestartCutsceneOrTauntFromGridEnum`, F-10331).
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

/// The original's start-scene picker (docs: cutscenes/prerace_pick.md). Each list is a shuffle bag (order,
/// cursor), shuffled once at load in table order (F-75403 -> F-11038) and drawn by F-27986: every scene once
/// per round, and a fresh round does not open with the scene that closed the last one. The taunt draw is
/// always taken (one step of the generator, F-10331); a taunt (key 7) comes for a first race or a draw
/// <= 0.333, never on key 0 (1x1); in the `_sc` table a `_lr_` car in front (and not camOnly) limits places
/// 1..5 to their first five scenes and the taunts to their first four (F-24279 -> F-96888). camOnly skips
/// names with `_racestarter_` (none in the tables). The generator is the original's MWC arithmetic
/// (0x82860880, a = 0x5CDCFAA7); approx.: its seed and the other functions drawing from it in the original
/// are not followed - the picker owns it.
pub struct Picker {
    table: GridTable,
    bags: Vec<(Vec<u32>, usize)>,
    rng: crate::vehicles::parked::Mwc,
}

impl Picker {
    pub fn new(table: GridTable, seed: u32) -> Picker {
        let bags = table.lists.iter().map(|l| ((0..l.1.len() as u32).collect(), 0)).collect();
        let mut p = Picker { table, bags, rng: crate::vehicles::parked::Mwc { lo: seed, carry: 1 } };
        for k in 0..p.bags.len() {
            p.shuffle(k);
        }
        p
    }

    /// F-11038: order[i] = i, then each order[i] swapped with order[(lo & 0x7FFFFFFF) % n]; cursor 0.
    fn shuffle(&mut self, k: usize) {
        let n = self.bags[k].0.len();
        for i in 0..n {
            self.bags[k].0[i] = i as u32;
        }
        for i in 0..n {
            let j = self.rng.index(n as u32) as usize;
            self.bags[k].0.swap(i, j);
        }
        self.bags[k].1 = 0;
    }

    /// F-14689: the first m = min(lim + 1, n) places set to 0 .. m - 1 and each swapped with one of them
    /// (the places after keep what they held); cursor 0.
    fn shuffle_first(&mut self, k: usize, lim: usize) {
        let n = self.bags[k].0.len();
        let m = (lim + 1).min(n);
        for i in 0..m {
            self.bags[k].0[i] = i as u32;
        }
        let d = lim.min(n - 1) + 1;
        for i in 0..m {
            let j = self.rng.index(d as u32) as usize;
            self.bags[k].0.swap(i, j);
        }
        self.bags[k].1 = 0;
    }

    fn name(&self, k: usize, i: u32) -> Option<String> {
        self.table.lists[k].1.get(i as usize).cloned()
    }

    /// F-27986: the scene at the cursor; then (n > 1) the cursor steps on, and at the end of the round the bag
    /// is shuffled again and, if it opens with the round's last scene, its first two swap.
    fn draw(&mut self, k: usize) -> Option<String> {
        let n = self.bags[k].0.len();
        if n == 0 {
            return None;
        }
        let cur = self.bags[k].0[self.bags[k].1];
        if n > 1 {
            self.bags[k].1 += 1;
            if self.bags[k].1 >= n {
                let last = self.bags[k].0[n - 1];
                self.shuffle(k);
                if self.bags[k].0[0] == last {
                    self.bags[k].0.swap(0, 1);
                }
            }
        }
        self.name(k, cur)
    }

    /// F-96888: a draw among places 0 ..= lim. A scene at the cursor within them steps the cursor on (a round
    /// of min(lim + 1, n)); one beyond them, or the round's end, shuffles the first places again (F-14689,
    /// the swap against the old order[min(lim, n - 1)]), and a scene beyond them gives order[lim].
    fn draw_limited(&mut self, k: usize, lim: usize) -> Option<String> {
        let n = self.bags[k].0.len();
        if n == 0 {
            return None;
        }
        let cur = self.bags[k].0[self.bags[k].1];
        if n <= 1 {
            return self.name(k, cur);
        }
        let mut stay = false;
        if cur as usize <= lim {
            self.bags[k].1 += 1;
            stay = self.bags[k].1 < (lim + 1).min(n);
        }
        if !stay {
            let last = self.bags[k].0[lim.min(n - 1)];
            self.shuffle_first(k, lim);
            if self.bags[k].0[0] == last {
                self.bags[k].0.swap(0, 1);
            }
        }
        if cur as usize > lim {
            let i = self.bags[k].0[lim];
            return self.name(k, i);
        }
        self.name(k, cur)
    }

    /// The scene to play, or None (the original then runs `Game/RaceTypes/generic_countdown`: an unknown
    /// grid such as `16_Race_Grid`). Safety (SafetyCam 1) is its own native (F-35574): a plain draw.
    pub fn pick(&mut self, a: &PickArgs) -> Option<String> {
        if a.grid.eq_ignore_ascii_case("Safety") {
            let k = self.table.key("Safety")?;
            return self.draw(k);
        }
        // (the scripts' grid: the race file's GridType as the parser reads it, 0 without opponents)
        let grid = if a.grid.eq_ignore_ascii_case("1x1_Race_Grid") { 0 } else { grid_type_key(&a.grid) };
        self.pick_key(grid, a.first_race, a.cam_only, a.front_lr)
    }

    /// PickRacestartCutsceneOrTauntFromGridEnum (F-10331) on the grid key `grid`, then the mapper's pick F-24279.
    /// The draw first, whatever the grid; a taunt (key 7) for a first race or a draw <= 0.333, never on key 0 (an
    /// unknown grid's -1 can have one); the debug params forcepreracetaunt / disablepreracetaunt are off and no
    /// network session runs. A key the table does not have: None.
    pub fn pick_key(&mut self, grid: i32, first_race: bool, cam_only: bool, front_lr: bool) -> Option<String> {
        let r = self.rng.float();
        let taunt = (first_race || !(r > 0.333f32)) && grid != 0;
        let key = if taunt { 7 } else { grid };
        let k = self.table.place_of_key(key)?;
        let a = PickArgs { cam_only, front_lr, ..Default::default() };
        if a.cam_only {
            for _ in 0..self.bags[k].0.len() {
                let s = self.draw(k)?;
                if !s.contains("_racestarter_") {
                    return Some(s);
                }
            }
            return None;
        }
        if self.table.sc && a.front_lr {
            if (1..6).contains(&k) {
                return self.draw_limited(k, 4);
            }
            if k == 7 {
                return self.draw_limited(k, 3);
            }
        }
        self.draw(k)
    }

    /// The list of a grid, as the table has it (test hooks).
    pub fn list(&self, grid: &str) -> Vec<String> {
        self.table.key(grid).map(|k| self.table.lists[k].1.clone()).unwrap_or_default()
    }
}

/// A scene event aimed at a racer slot (kind 2: `CineScript_Events_LaunchEventOnRacerSlot`, F-51323) or
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
    /// the record's f3 (the event's +44): 1.0 on a gesture turns its driver's jaw flap on for the event (`jaw_start`)
    pub f3: f32,
}

/// The slot / speech events of a scene, in the script's order, each with the start the slot launch gives it
/// (`slot_event_start`).
pub fn slot_events(scene: &str) -> Vec<SlotEvent> {
    let name = scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase();
    SLOT_EVENTS.iter().filter(|e| e.scene == name).map(|e| {
        let mut e = *e;
        e.t = slot_event_start(scene, e.event, e.t);
        e
    }).collect()
}

/// The generic event types (mcGenericEventType's 97-name table, F-49543), in type order.
pub const EVENT_TYPES: [&str; 97] = [
    "gesture_neutral", "gesture_left", "gesture_right", "gesture_talk_left", "gesture_talk_right", "gesture_mild", "gesture_win",
    "gesture_listen_left", "gesture_listen_right", "gesture_thm_dwn_lft", "gesture_thm_dwn_rgt", "showdriver", "hidedriver", "horn",
    "open_door_right", "close_door_right", "open_door_left", "close_door_left", "open_left_door", "close_left_door", "slam_door_left",
    "slam_door_right", "car_on", "car_off", "car_restore", "car_left_shudder", "car_right_shudder", "car_rear_shudder",
    "car_left_rear_shudder", "window_down_left", "window_down_right", "window_up_left", "window_up_right", "headlights_on",
    "headlights_off", "engine_on", "engine_off", "carignition", "racer_enablemusic", "racer_disablemusic", "playspeech",
    "speechsystem_play", "musicplay", "musicstop", "musicpause", "musicnext", "musicprev", "stopaudiostreams", "gesture_casual_right",
    "gesture_look_right", "gesture_mild_right", "gesture_disappointed", "gesture_copfidget", "gesture_cophandsup", "gesture_findsh04",
    "jawflap", "prerace_burnout", "particles", "countdown", "go5sec", "go1sec", "go", "arrest_splash_off", "arrest_splash",
    "arrest_panel", "ticket_splash", "ticket_panel", "racename", "fade_to_black", "fade_to_scene", "cam_grain_on", "cam_grain_off",
    "bookmark_cut_to_game", "bookmark", "cut_to_game", "cam_noupright", "cam_dof", "cam_fov", "cam_to_game", "cam_gps_warp_out",
    "cam_to_garage", "cam_to_rmr", "cam_to_menu", "menu_to_cam", "garage_to_cam", "garage_lighting", "game_lighting", "rmr_to_cam",
    "big_map_up", "big_map_down", "dof_near_distance", "dof_far_distance", "dof_transition_time", "roll_credits",
    "cam_to_gps_tutorial", "bw_toggle_on", "bw_toggle_off",
];

/// An event name's type (F-49543): the first table name the event name contains (the C runtime's strstr, case
/// kept), else -1 (logged; the event is still made).
pub fn event_type(name: &str) -> i32 {
    EVENT_TYPES.iter().position(|t| name.contains(t)).map_or(-1, |i| i as i32)
}

/// The start a slot event is launched at (`CineScript_Events_LaunchEventOnRacerSlot`, F-51323): a
/// `gesture_left` / `gesture_right` type (1 / 2) before 1 s starts at 1 s when the current scene's name (the
/// short form, `racestart/...`) contains `racestart`; any other start is kept. The event is made only when the
/// slot holds a racer id above -1 whose racer exists; else its node goes back to the pool.
pub fn slot_event_start(scene: &str, event: &str, t: f32) -> f32 {
    if t < 1.0 && scene.contains("racestart") && matches!(event_type(event), 1 | 2) { 1.0 } else { t }
}

/// Whether an event's start turns its driver's jaw flap on (racer +688 = 1, F-80291), F-91263: the gestures 0 .. 10
/// and 48 .. 50 when their f3 is 1.0 (|f3 - 1| < 0.001 in single precision), and jawflap (55) always (no scene has one).
/// The jaw flap is set nowhere else (the racer's constructor and resets clear it): it follows these events, not the
/// speech.
pub fn jaw_start(event: &str, f3: f32) -> bool {
    jaw_start_type(event_type(event), f3)
}

/// `jaw_start` on an event type.
pub fn jaw_start_type(ty: i32, f3: f32) -> bool {
    match ty {
        0..=10 | 48..=50 => (f3 - 1.0f32).abs() < 0.001f32,
        55 => true,
        _ => false,
    }
}

/// Whether an event's stop turns its driver's jaw flap off (racer +688 = 0, F-63941), F-26895: the gestures 0 .. 10,
/// 48 .. 51 and 54 when their f3 is 1.0, and jawflap (55) always.
pub fn jaw_stop(event: &str, f3: f32) -> bool {
    jaw_stop_type(event_type(event), f3)
}

/// `jaw_stop` on an event type.
pub fn jaw_stop_type(ty: i32, f3: f32) -> bool {
    match ty {
        0..=10 | 48..=51 | 54 => (f3 - 1.0f32).abs() < 0.001f32,
        55 => true,
        _ => false,
    }
}

/// A race file's GridType as the race parser reads it (F-53452): its place among None, 2x4_Race_Grid,
/// 2x4_Split_Race_Grid, 3x3_Race_Grid, 4x2_Race_Grid, 3x3_Split_Race_Grid, 1x8_Race_Grid (an ASCII case-blind
/// compare), else -1 (the parser logs it; `16_Race_Grid` of be_ordered_mb_04_multi).
pub fn grid_type_key(name: &str) -> i32 {
    const TYPES: [&str; 7] = ["None", "2x4_Race_Grid", "2x4_Split_Race_Grid", "3x3_Race_Grid", "4x2_Race_Grid", "3x3_Split_Race_Grid", "1x8_Race_Grid"];
    TYPES.iter().position(|t| t.eq_ignore_ascii_case(name)).map_or(-1, |p| p as i32)
}

/// The taunt speech scenario a scene prepares (`CineScript_RequestPreraceTauntScenarioAudio`, F-56020):
/// a name with TAUNT -> `TAUNT_PRERACE_HP` (sender slot 1, the hookman; receiver slot 0), else with PH ->
/// `TAUNT_PRERACE_PH` (sender slot 0), else `TAUNT_PRERACE_H` (slot 1 alone). None for other scenes.
/// The whole name upper-cased (the C runtime's ASCII a..z) and searched, as F-56020 does; it then requests the
/// scenario with speakers A, B = the racers of slots 0, 1 (PH: 1, 0), and only when both slots hold a racer.
pub fn taunt_kind(scene: &str) -> Option<&'static str> {
    let n = scene.to_ascii_uppercase();
    if !n.contains("TAUNT") {
        return None;
    }
    Some(if n.contains("HP") { "HP" } else if n.contains("PH") { "PH" } else { "H" })
}

pub const SLOT_EVENTS: [SlotEvent; 465] = [
    SlotEvent { scene: "taunt_01", event: "gesture_left", slot: 0, t: 0.0, dur: 7.33333, f3: 0.0 },
    SlotEvent { scene: "taunt_01", event: "gesture_right", slot: 1, t: 0.0, dur: 7.33333, f3: 0.0 },
    SlotEvent { scene: "taunt_02", event: "gesture_left", slot: 0, t: 0.0, dur: 6.66667, f3: 0.0 },
    SlotEvent { scene: "taunt_02", event: "gesture_right", slot: 1, t: 0.0, dur: 6.66667, f3: 0.0 },
    SlotEvent { scene: "taunt_03", event: "gesture_left", slot: 0, t: 0.0, dur: 6.66667, f3: 0.0 },
    SlotEvent { scene: "taunt_03", event: "gesture_right", slot: 1, t: 0.0, dur: 6.66667, f3: 0.0 },
    SlotEvent { scene: "taunt_04", event: "gesture_left", slot: 0, t: 0.0, dur: 6.66667, f3: 0.0 },
    SlotEvent { scene: "taunt_04", event: "gesture_right", slot: 1, t: 0.0, dur: 6.66667, f3: 0.0 },
    SlotEvent { scene: "taunt_05", event: "gesture_left", slot: 0, t: 0.0, dur: 6.66667, f3: 0.0 },
    SlotEvent { scene: "taunt_05", event: "gesture_right", slot: 1, t: 0.0, dur: 6.66667, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x8_01", event: "gesture_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x8_01", event: "gesture_right", slot: 1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x8_02", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x8_02", event: "gesture_right", slot: 1, t: 4.36667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_2x4_split_01", event: "gesture_left", slot: 0, t: 2.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_2x4_split_01", event: "gesture_right", slot: 1, t: 3.7, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_2x4_split_02", event: "gesture_left", slot: 0, t: 1.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_2x4_split_02", event: "gesture_right", slot: 1, t: 5.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_01", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_01", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_02", event: "gesture_left", slot: 0, t: 4.7, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_02", event: "gesture_right", slot: 1, t: 6.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_03", event: "gesture_left", slot: 0, t: 3.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_03", event: "gesture_right", slot: 1, t: 4.03333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_04", event: "gesture_right", slot: 1, t: 4.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_04", event: "gesture_left", slot: 0, t: 5.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_split_01", event: "gesture_right", slot: 1, t: 4.03333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_split_01", event: "gesture_left", slot: 0, t: 5.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_split_02", event: "gesture_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_3x3_split_02", event: "gesture_right", slot: 1, t: 2.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_safety_1", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_safety_1", event: "playspeech", slot: -1, t: 0.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_safety_1", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_safety_1", event: "gesture_listen_right", slot: 1, t: 1.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_safety_1", event: "window_up_left", slot: 0, t: 6.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_hk_01", event: "gesture_right", slot: 1, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_hk_plyr_01", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_hk_plyr_01", event: "gesture_left", slot: 0, t: 5.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_hk_plyr_02", event: "gesture_right", slot: 1, t: 2.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_hk_plyr_02", event: "gesture_left", slot: 0, t: 7.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_plyr_hk_02", event: "gesture_left", slot: 0, t: 6.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_plyr_hk_02", event: "gesture_right", slot: 1, t: 6.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_plyr_hk_01", event: "gesture_right", slot: 1, t: 3.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_taunt_plyr_hk_01", event: "gesture_left", slot: 0, t: 7.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_1x8_01", event: "gesture_left", slot: 0, t: 3.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_1x8_01", event: "gesture_right", slot: 1, t: 4.03333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_1x8_02", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_1x8_02", event: "gesture_right", slot: 1, t: 8.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_01", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_01", event: "gesture_right", slot: 1, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_02", event: "gesture_right", slot: 1, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_02", event: "gesture_left", slot: 0, t: 5.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_03", event: "gesture_left", slot: 0, t: 5.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_03", event: "gesture_right", slot: 1, t: 5.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_04", event: "gesture_left", slot: 0, t: 5.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_04", event: "gesture_right", slot: 1, t: 6.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_split_01", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_split_01", event: "gesture_right", slot: 1, t: 2.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_split_02", event: "gesture_left", slot: 0, t: 2.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_2x4_split_02", event: "gesture_right", slot: 1, t: 5.76667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_01", event: "gesture_right", slot: 1, t: 3.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_01", event: "gesture_left", slot: 0, t: 7.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_02", event: "gesture_right", slot: 1, t: 4.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_02", event: "gesture_left", slot: 0, t: 6.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_03", event: "gesture_left", slot: 0, t: 2.03333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_03", event: "gesture_right", slot: 1, t: 2.03333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_04", event: "gesture_left", slot: 0, t: 5.7, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_04", event: "gesture_right", slot: 1, t: 5.7, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_05", event: "gesture_left", slot: 0, t: 4.36667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_05", event: "gesture_right", slot: 1, t: 4.36667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_06", event: "gesture_left", slot: 0, t: 4.03333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_06", event: "gesture_right", slot: 1, t: 4.03333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_split_01", event: "gesture_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_split_01", event: "gesture_right", slot: 1, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_split_02", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_3x3_split_02", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_4x2_01", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_4x2_01", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_4x2_02", event: "gesture_left", slot: 0, t: 4.8, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestarter_4x2_02", event: "gesture_right", slot: 1, t: 4.8, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_delivery_1x1_01", event: "gesture_neutral", slot: 0, t: 2.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_delivery_1x1_02", event: "gesture_neutral", slot: 0, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_delivery_1x1_03", event: "gesture_neutral", slot: 0, t: 2.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_1x1_01", event: "gesture_neutral", slot: 0, t: 3.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_1x1_02", event: "gesture_neutral", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_1x1_03", event: "gesture_neutral", slot: 0, t: 1.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_1x8_07", event: "gesture_right", slot: 1, t: 0.0, dur: 6.26667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_1x8_07", event: "gesture_left", slot: 0, t: 4.0, dur: 2.26667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_01", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 5.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 5.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_02", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_02", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_02", event: "gesture_thm_dwn_lft", slot: 0, t: 1.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_02", event: "window_up_left", slot: 0, t: 5.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_03", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_03", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 3.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_03", event: "gesture_thm_dwn_rgt", slot: 1, t: 3.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_04", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 5.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_04", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_05", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_05", event: "gesture_listen_right", slot: 1, t: 0.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_06", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 5.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_06", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_07", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_07", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 3.86667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_07", event: "gesture_thm_dwn_rgt", slot: 1, t: 3.9, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_02", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 4.6, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_02", event: "gesture_thm_dwn_rgt", slot: 1, t: 0.23333, dur: 1.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_02", event: "gesture_listen_right", slot: 1, t: 1.93333, dur: 2.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_03", event: "gesture_left", slot: 0, t: 0.0, dur: 8.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_03", event: "gesture_right", slot: 1, t: 0.0, dur: 8.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_04", event: "gesture_left", slot: 0, t: 0.0, dur: 9.1, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_04", event: "gesture_right", slot: 1, t: 0.0, dur: 9.1, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_05", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_05", event: "gesture_thm_dwn_lft", slot: 0, t: 0.5, dur: 2.6, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_05", event: "gesture_listen_right", slot: 1, t: 0.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_05", event: "gesture_listen_left", slot: 0, t: 3.13333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_05", event: "window_up_left", slot: 0, t: 5.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_06", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_06", event: "gesture_thm_dwn_lft", slot: 0, t: 0.5, dur: 2.6, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_06", event: "gesture_listen_left", slot: 0, t: 3.13333, dur: 2.53333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_07", event: "gesture_right", slot: 1, t: 1.0, dur: 5.13333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_07", event: "gesture_left", slot: 0, t: 2.5, dur: 3.63333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_08", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 2.46667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_08", event: "gesture_listen_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_2x4_split_08", event: "gesture_thm_dwn_rgt", slot: 1, t: 3.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_02", event: "gesture_listen_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_02", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_03", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_03", event: "gesture_neutral", slot: 0, t: 1.16667, dur: 1.4, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_03", event: "gesture_listen_left", slot: 0, t: 2.6, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_04", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 2.8, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_04", event: "gesture_listen_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_04", event: "gesture_thm_dwn_lft", slot: 0, t: 2.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_05", event: "gesture_listen_left", slot: 0, t: 0.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_05", event: "gesture_thm_dwn_rgt", slot: 1, t: 1.0, dur: 1.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_05", event: "gesture_listen_right", slot: 1, t: 2.36667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_06", event: "gesture_listen_left", slot: 0, t: 0.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_06", event: "gesture_thm_dwn_rgt", slot: 1, t: 2.0, dur: 1.93333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_06", event: "gesture_listen_right", slot: 1, t: 3.96667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_07", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 3.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_07", event: "gesture_listen_left", slot: 0, t: 2.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_07", event: "gesture_neutral", slot: 1, t: 3.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_08", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 6.83333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_08", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 3.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_3x3_08", event: "gesture_thm_dwn_rgt", slot: 1, t: 4.03333, dur: 2.8, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_01", event: "gesture_right", slot: 1, t: 1.0, dur: 10.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_01", event: "gesture_left", slot: 0, t: 2.83333, dur: 8.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_02", event: "gesture_neutral", slot: 0, t: 0.0, dur: 7.6, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_02", event: "gesture_neutral", slot: 1, t: 0.0, dur: 7.6, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_03", event: "gesture_neutral", slot: 0, t: 0.0, dur: 8.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_03", event: "gesture_neutral", slot: 1, t: 0.0, dur: 8.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_04", event: "gesture_neutral", slot: 0, t: 0.0, dur: 9.1, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_04", event: "gesture_neutral", slot: 1, t: 0.0, dur: 9.1, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_05", event: "gesture_neutral", slot: 0, t: 0.0, dur: 7.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_05", event: "gesture_neutral", slot: 1, t: 0.0, dur: 7.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_06", event: "gesture_neutral", slot: 1, t: 2.0, dur: 4.8, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_06", event: "gesture_neutral", slot: 0, t: 4.0, dur: 2.8, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_07", event: "gesture_neutral", slot: 1, t: 1.0, dur: 5.13333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_07", event: "gesture_neutral", slot: 0, t: 2.5, dur: 3.63333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_08", event: "gesture_neutral", slot: 0, t: 2.66667, dur: 4.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_grid_4x2_08", event: "gesture_neutral", slot: 1, t: 3.66667, dur: 3.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 6.63333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "gesture_neutral", slot: 1, t: 0.83333, dur: 2.63333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "gesture_listen_right", slot: 1, t: 3.5, dur: 8.73333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "gesture_thm_dwn_lft", slot: 0, t: 6.66667, dur: 5.56667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "window_up_left", slot: 0, t: 9.73333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_01", event: "stopaudiostreams", slot: -1, t: 13.2, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 2.46667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "window_down_right", slot: 1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "gesture_listen_left", slot: 0, t: 1.33333, dur: 7.63333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "gesture_thm_dwn_rgt", slot: 1, t: 3.56667, dur: 5.4, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "window_up_right", slot: 1, t: 6.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_02", event: "stopaudiostreams", slot: -1, t: 9.96667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_03", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_03", event: "gesture_listen_right", slot: 1, t: 0.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_03", event: "gesture_thm_dwn_lft", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_03", event: "window_up_left", slot: 1, t: 3.23333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_03", event: "stopaudiostreams", slot: -1, t: 6.8, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "window_down_right", slot: 1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "gesture_listen_left", slot: 0, t: 0.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "gesture_thm_dwn_rgt", slot: 1, t: 1.2, dur: 2.93333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "gesture_listen_right", slot: 1, t: 4.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "window_up_right", slot: 1, t: 4.2, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_04", event: "stopaudiostreams", slot: -1, t: 8.9, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "gesture_listen_right", slot: 1, t: 0.33333, dur: 4.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "gesture_thm_dwn_lft", slot: 0, t: 2.33333, dur: 2.3, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "gesture_neutral", slot: 1, t: 4.36667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "gesture_listen_left", slot: 0, t: 4.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "window_up_left", slot: 0, t: 6.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_05", event: "stopaudiostreams", slot: -1, t: 11.4, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_06", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_06", event: "gesture_thm_dwn_rgt", slot: 1, t: 2.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_06", event: "stopaudiostreams", slot: -1, t: 7.7, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_07", event: "gesture_listen_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_07", event: "gesture_thm_dwn_rgt", slot: 1, t: 6.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_07", event: "stopaudiostreams", slot: -1, t: 10.2333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_08", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 5.8, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_08", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 5.8, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_08", event: "stopaudiostreams", slot: -1, t: 6.8, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_09", event: "gesture_neutral", slot: 1, t: 0.0, dur: 3.63333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_09", event: "gesture_listen_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_09", event: "gesture_listen_right", slot: 1, t: 3.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_09", event: "stopaudiostreams", slot: -1, t: 10.2333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_10", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_10", event: "gesture_listen_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_10", event: "stopaudiostreams", slot: -1, t: 7.3, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_11", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 9.13333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_11", event: "gesture_listen_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_11", event: "gesture_thm_dwn_rgt", slot: 1, t: 9.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_2x4_11", event: "stopaudiostreams", slot: -1, t: 15.4333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_01", event: "gesture_left", slot: 0, t: 0.83333, dur: 9.36667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_01", event: "gesture_right", slot: 1, t: 0.83333, dur: 9.36667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_02", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_02", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_03", event: "gesture_left", slot: 0, t: 0.83333, dur: 5.06667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_03", event: "gesture_right", slot: 1, t: 0.83333, dur: 5.06667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_04", event: "gesture_left", slot: 0, t: 0.83333, dur: 5.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_04", event: "gesture_right", slot: 1, t: 0.83333, dur: 5.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_05", event: "gesture_left", slot: 0, t: 0.83333, dur: 7.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_05", event: "gesture_right", slot: 1, t: 0.83333, dur: 7.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_06", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.86667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_06", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.86667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_07", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.5, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_07", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.5, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_08", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.83333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_08", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.83333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_09", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_09", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_10", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.46667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_3x3_10", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.46667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_01", event: "gesture_left", slot: 0, t: 0.83333, dur: 9.36667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_01", event: "gesture_right", slot: 1, t: 0.83333, dur: 9.36667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_02", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_02", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_03", event: "gesture_left", slot: 0, t: 0.83333, dur: 5.06667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_03", event: "gesture_right", slot: 1, t: 0.83333, dur: 5.06667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_04", event: "gesture_left", slot: 0, t: 0.83333, dur: 5.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_04", event: "gesture_right", slot: 1, t: 0.83333, dur: 5.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_05", event: "gesture_left", slot: 0, t: 0.83333, dur: 7.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_05", event: "gesture_right", slot: 1, t: 0.83333, dur: 7.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_06", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.86667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_06", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.86667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_07", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.5, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_07", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.5, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_08", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.83333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_08", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.83333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_09", event: "gesture_left", slot: 0, t: 0.83333, dur: 6.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_09", event: "gesture_right", slot: 1, t: 0.83333, dur: 6.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_10", event: "gesture_left", slot: 0, t: 0.83333, dur: 3.46667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_4x2_10", event: "gesture_right", slot: 1, t: 0.83333, dur: 3.46667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_01", event: "gesture_right", slot: 1, t: 2.1, dur: 4.9, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_01", event: "gesture_left", slot: 0, t: 3.3, dur: 3.7, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_02", event: "gesture_left", slot: 0, t: 1.2, dur: 4.8, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_02", event: "gesture_right", slot: 1, t: 2.73333, dur: 3.26667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_03", event: "gesture_right", slot: 1, t: 1.83333, dur: 3.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_03", event: "gesture_left", slot: 0, t: 3.3, dur: 1.7, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_07", event: "gesture_left", slot: 0, t: 0.0, dur: 8.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_07", event: "gesture_right", slot: 1, t: 2.6, dur: 5.4, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_09", event: "gesture_right", slot: 1, t: 0.0, dur: 9.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_09", event: "gesture_left", slot: 0, t: 3.66667, dur: 5.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 3.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "playspeech", slot: -1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "gesture_talk_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "gesture_talk_left", slot: 0, t: 4.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_01", event: "window_up_left", slot: 0, t: 7.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 5.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 4.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "playspeech", slot: -1, t: 0.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "gesture_talk_left", slot: 0, t: 4.36667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "gesture_listen_right", slot: 1, t: 5.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_02", event: "window_up_left", slot: 0, t: 7.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "window_down_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "gesture_talk_right", slot: 1, t: 0.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "gesture_listen_left", slot: 0, t: 0.66667, dur: 3.73333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "playspeech", slot: -1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "gesture_talk_left", slot: 0, t: 4.43333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_hp_03", event: "window_up_left", slot: 0, t: 8.63333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_h_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 6.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_h_01", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 6.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_h_01", event: "playspeech", slot: -1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_01", event: "gesture_talk_left", slot: 0, t: 0.0, dur: 4.13333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_01", event: "gesture_listen_right", slot: 1, t: 1.0, dur: 3.1, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_01", event: "playspeech", slot: -1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_01", event: "gesture_talk_right", slot: 1, t: 4.13333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_01", event: "gesture_listen_left", slot: 0, t: 4.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_02", event: "gesture_talk_left", slot: 0, t: 0.0, dur: 5.13333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_02", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 3.36667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_02", event: "playspeech", slot: -1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_02", event: "gesture_talk_right", slot: 1, t: 3.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_taunt_ph_02", event: "gesture_listen_left", slot: 0, t: 5.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_1x1_01", event: "gesture_neutral", slot: 0, t: 1.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_01", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 1.8, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_01", event: "gesture_listen_right", slot: 1, t: 3.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_02", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_02", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 1.8, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_02", event: "gesture_listen_right", slot: 1, t: 3.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_03", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_03", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 1.8, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_03", event: "gesture_listen_right", slot: 1, t: 3.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_01", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_01", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 1.8, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_01", event: "gesture_listen_right", slot: 1, t: 3.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_02", event: "gesture_listen_left", slot: 0, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_02", event: "gesture_talk_right", slot: 1, t: 0.0, dur: 1.8, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_grid_2x4_split_02", event: "gesture_listen_right", slot: 1, t: 3.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_payback_get_keys", event: "gesture_neutral", slot: 0, t: 20.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x2_02", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x2_02", event: "gesture_left", slot: 0, t: 3.03333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x2_03", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x2_03", event: "gesture_left", slot: 0, t: 3.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x2_04", event: "gesture_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x2_04", event: "gesture_right", slot: 1, t: 3.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x2_05", event: "gesture_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_racestart_1x2_05", event: "gesture_right", slot: 1, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "cut_redlight_01", event: "gesture_right", slot: 1, t: 0.0, dur: 5.0, f3: 0.0 },
    SlotEvent { scene: "cut_redlight_01", event: "gesture_left", slot: 0, t: 0.0, dur: 5.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_be00_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_be00_01", event: "gesture_talk_right", slot: 1, t: 2.0, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_be00_01", event: "playspeech", slot: 1, t: 2.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_be01_01", event: "gesture_talk_right", slot: 1, t: 2.0, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_be01_01", event: "gesture_listen_left", slot: 0, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_be01_01", event: "playspeech", slot: 1, t: 2.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_be02_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 4.33333, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_be02_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 4.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_be02_01", event: "playspeech", slot: 1, t: 2.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt00_01", event: "gesture_listen_left", slot: 0, t: 1.33333, dur: 5.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt00_01", event: "gesture_talk_right", slot: 1, t: 1.33333, dur: 5.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_dt00_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt01_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt01_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_dt01_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt02_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt02_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_dt02_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt03_01", event: "gesture_listen_left", slot: 0, t: 1.33333, dur: 5.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt03_01", event: "gesture_talk_right", slot: 1, t: 1.33333, dur: 5.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_dt03_01", event: "playspeech", slot: 1, t: 2.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt04_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt04_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_dt04_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt05_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt05_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_dt05_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt06_01", event: "gesture_listen_left", slot: 0, t: 1.33333, dur: 5.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_dt06_01", event: "gesture_talk_right", slot: 1, t: 1.33333, dur: 5.33333, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_dt06_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl00_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hl00_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl00_01", event: "playspeech", slot: 1, t: 1.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl01_01", event: "gesture_talk_right", slot: 1, t: 1.33333, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hl01_01", event: "gesture_listen_left", slot: 0, t: 1.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl01_01", event: "playspeech", slot: 1, t: 1.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl02_01", event: "gesture_talk_right", slot: 1, t: 1.33333, dur: 5.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hl02_01", event: "gesture_listen_left", slot: 0, t: 1.33333, dur: 5.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl02_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl03_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hl03_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl03_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl04_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hl04_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl04_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl05_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 4.66667, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hl05_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 4.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl05_01", event: "playspeech", slot: 1, t: 2.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl06_01", event: "gesture_listen_left", slot: 0, t: 2.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hl06_01", event: "gesture_talk_right", slot: 1, t: 2.16667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hl06_01", event: "playspeech", slot: 1, t: 2.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw00_01", event: "gesture_talk_right", slot: 1, t: 2.33333, dur: 4.33333, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hw00_01", event: "playspeech", slot: 1, t: 2.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw00_01", event: "gesture_listen_left", slot: 0, t: 3.66667, dur: 3.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw01_01", event: "gesture_listen_left", slot: 0, t: 2.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw01_01", event: "gesture_talk_right", slot: 1, t: 2.5, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hw01_01", event: "playspeech", slot: 1, t: 2.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw02_01", event: "gesture_listen_left", slot: 0, t: 2.33333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw02_01", event: "gesture_talk_right", slot: 1, t: 2.33333, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hw02_01", event: "playspeech", slot: 1, t: 2.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw03_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 4.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw03_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 4.66667, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hw03_01", event: "playspeech", slot: 1, t: 2.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw04_01", event: "gesture_listen_left", slot: 0, t: 1.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw04_01", event: "gesture_talk_right", slot: 1, t: 1.83333, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hw04_01", event: "playspeech", slot: 1, t: 2.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw05_01", event: "gesture_talk_left", slot: 0, t: 1.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw05_01", event: "gesture_listen_right", slot: 1, t: 1.83333, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hw05_01", event: "playspeech", slot: 1, t: 2.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw06_01", event: "gesture_talk_right", slot: 1, t: 1.33333, dur: 5.33333, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hw06_01", event: "gesture_listen_left", slot: 0, t: 1.33333, dur: 5.33333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw06_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw07_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hw07_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw07_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw08_01", event: "gesture_listen_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "gen_prerace_red_hw08_01", event: "gesture_talk_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_red_hw08_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_01", event: "gesture_right", slot: 1, t: 0.166667, dur: 8.66667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_01", event: "gesture_left", slot: 0, t: 6.0, dur: 3.23333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_04", event: "gesture_right", slot: 1, t: 4.33333, dur: 4.0, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_04", event: "gesture_left", slot: 0, t: 5.53333, dur: 2.8, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_05", event: "gesture_right", slot: 1, t: 2.1, dur: 5.53333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_05", event: "gesture_left", slot: 0, t: 3.7, dur: 3.93333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_07", event: "gesture_left", slot: 0, t: 2.83333, dur: 2.16667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_07", event: "gesture_right", slot: 1, t: 5.56667, dur: 2.76667, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_09", event: "gesture_left", slot: 0, t: 2.8, dur: 4.53333, f3: 0.0 },
    SlotEvent { scene: "gen_prerace_starter_09", event: "gesture_right", slot: 1, t: 4.53333, dur: 2.8, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc00_01", event: "gesture_talk_right", slot: 1, t: 1.33333, dur: 5.33333, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc00_01", event: "gesture_listen_left", slot: 0, t: 1.33333, dur: 5.33333, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc00_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc01_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 4.66667, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc01_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 4.66667, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc01_01", event: "playspeech", slot: 1, t: 2.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc02_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc02_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc02_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc03_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc03_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc03_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc04_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc04_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc04_01", event: "playspeech", slot: 1, t: 1.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc05_01", event: "gesture_listen_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc05_01", event: "gesture_talk_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc05_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc06_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 4.66667, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc06_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 4.66667, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc06_01", event: "playspeech", slot: 1, t: 2.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc07_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 4.66667, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc07_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 4.66667, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc07_01", event: "playspeech", slot: 1, t: 2.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc08_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 0.0, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc08_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc08_01", event: "playspeech", slot: 1, t: 1.83333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc09_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 4.66667, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc09_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 4.66667, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc09_01", event: "playspeech", slot: 1, t: 2.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc10_01", event: "gesture_talk_right", slot: 1, t: 1.33333, dur: 5.33333, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc10_01", event: "gesture_listen_left", slot: 0, t: 1.33333, dur: 5.33333, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc10_01", event: "playspeech", slot: 1, t: 2.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc11_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 4.66667, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc11_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 4.66667, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc11_01", event: "playspeech", slot: 1, t: 2.5, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc12_01", event: "gesture_listen_left", slot: 0, t: 1.66667, dur: 4.66667, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc12_01", event: "gesture_talk_right", slot: 1, t: 1.66667, dur: 4.66667, f3: 1.0 },
    SlotEvent { scene: "sc_gen_prerace_red_sc12_01", event: "playspeech", slot: 1, t: 2.16667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_starter_2x4_01", event: "gesture_listen_right", slot: 1, t: 0.0, dur: 9.13333, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_starter_2x4_01", event: "gesture_listen_left", slot: 0, t: 1.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_starter_2x4_01", event: "gesture_thm_dwn_rgt", slot: 1, t: 13.8333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_starter_2x4_01", event: "stopaudiostreams", slot: -1, t: 19.1333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_h_01", event: "playspeech", slot: -1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_h_01", event: "gesture_talk_right", slot: 1, t: 0.666667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_h_01", event: "gesture_listen_left", slot: 0, t: 3.43333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_hp_01", event: "playspeech", slot: -1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_hp_01", event: "gesture_talk_right", slot: 1, t: 0.666667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_hp_01", event: "gesture_listen_left", slot: 0, t: 3.43333, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_hp_02", event: "playspeech", slot: -1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_hp_02", event: "gesture_talk_right", slot: 1, t: 0.666667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_hp_02", event: "gesture_listen_left", slot: 0, t: 0.666667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_ph_01", event: "playspeech", slot: -1, t: 0.0, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_ph_01", event: "gesture_talk_right", slot: 1, t: 0.666667, dur: 0.0, f3: 0.0 },
    SlotEvent { scene: "sc_gen_prerace_taunt_ph_01", event: "gesture_listen_left", slot: 0, t: 0.666667, dur: 0.0, f3: 0.0 },
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
    /// the started events that end at a time (dur > 0): (event index, end); the others run to FinishScene
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

pub const PRERACE_EVENTS: [PreraceEvents; 136] = [
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
    PreraceEvents { scene: "cut_racestart_1x1_01", racename: 1.86667, racename_len: 2.0, cut: 4.36667, countdown: 4.36667 },
    PreraceEvents { scene: "cut_racestart_1x1_02", racename: 5.83333, racename_len: 2.0, cut: 8.33333, countdown: 8.33333 },
    PreraceEvents { scene: "cut_racestart_1x1_03", racename: 10.8667, racename_len: 2.0, cut: 13.3667, countdown: 13.3667 },
    PreraceEvents { scene: "cut_racestart_1x2_02", racename: 9.16667, racename_len: 2.0, cut: 11.6667, countdown: 11.6667 },
    PreraceEvents { scene: "cut_racestart_1x2_03", racename: 8.2, racename_len: 2.0, cut: 10.7, countdown: 10.7 },
    PreraceEvents { scene: "cut_racestart_1x2_04", racename: 2.16667, racename_len: 2.0, cut: 4.66667, countdown: 4.66667 },
    PreraceEvents { scene: "cut_racestart_1x2_05", racename: 6.16667, racename_len: 2.0, cut: 8.66667, countdown: 8.66667 },
    PreraceEvents { scene: "cut_racestart_1x8_01", racename: 5.86667, racename_len: 2.0, cut: 12.0, countdown: 9.0 },
    PreraceEvents { scene: "cut_racestart_1x8_02", racename: 8.5, racename_len: 2.0, cut: 14.0, countdown: 11.0 },
    PreraceEvents { scene: "cut_racestart_2x4_split_01", racename: 7.2, racename_len: 2.0, cut: 12.6667, countdown: 9.66667 },
    PreraceEvents { scene: "cut_racestart_2x4_split_02", racename: 8.53333, racename_len: 2.0, cut: 14.3333, countdown: 11.3333 },
    PreraceEvents { scene: "cut_racestart_3x3_01", racename: 8.16667, racename_len: 2.0, cut: 13.6667, countdown: 10.6667 },
    PreraceEvents { scene: "cut_racestart_3x3_02", racename: 8.83333, racename_len: 2.0, cut: 11.3333, countdown: 11.3333 },
    PreraceEvents { scene: "cut_racestart_3x3_03", racename: 7.33333, racename_len: 2.5, cut: 10.3333, countdown: 10.3333 },
    PreraceEvents { scene: "cut_racestart_3x3_04", racename: 9.16667, racename_len: 2.0, cut: 11.6667, countdown: 11.6667 },
    PreraceEvents { scene: "cut_racestart_3x3_split_01", racename: 6.83333, racename_len: 2.0, cut: 9.33333, countdown: 9.33333 },
    PreraceEvents { scene: "cut_racestart_3x3_split_02", racename: 7.83333, racename_len: 2.0, cut: 13.3333, countdown: 10.3333 },
    PreraceEvents { scene: "cut_racestart_safety_1", racename: 1.66667, racename_len: 2.0, cut: 6.0, countdown: 4.0 },
    PreraceEvents { scene: "cut_racestart_taunt_hk_01", racename: 9.16667, racename_len: 2.0, cut: 11.6667, countdown: 11.6667 },
    PreraceEvents { scene: "cut_racestart_taunt_hk_plyr_01", racename: 7.5, racename_len: 2.0, cut: 10.0, countdown: 10.0 },
    PreraceEvents { scene: "cut_racestart_taunt_hk_plyr_02", racename: 5.0, racename_len: 2.0, cut: 11.0, countdown: 11.0 },
    PreraceEvents { scene: "cut_racestart_taunt_plyr_hk_01", racename: 7.83333, racename_len: 2.0, cut: 10.3333, countdown: 10.3333 },
    PreraceEvents { scene: "cut_racestart_taunt_plyr_hk_02", racename: 6.53333, racename_len: 2.0, cut: 9.03333, countdown: 9.03333 },
    PreraceEvents { scene: "cut_racestarter_1x8_01", racename: 7.16667, racename_len: 2.0, cut: 12.6667, countdown: 9.66667 },
    PreraceEvents { scene: "cut_racestarter_1x8_02", racename: 9.83333, racename_len: 2.0, cut: 15.3333, countdown: 12.3333 },
    PreraceEvents { scene: "cut_racestarter_2x4_01", racename: 5.86667, racename_len: 2.0, cut: 8.36667, countdown: 8.36667 },
    PreraceEvents { scene: "cut_racestarter_2x4_02", racename: 4.0, racename_len: 2.0, cut: 11.0, countdown: 8.0 },
    PreraceEvents { scene: "cut_racestarter_2x4_03", racename: 5.0, racename_len: 2.0, cut: 7.5, countdown: 7.5 },
    PreraceEvents { scene: "cut_racestarter_2x4_04", racename: 5.86667, racename_len: 1.66667, cut: 14.3333, countdown: 11.3333 },
    PreraceEvents { scene: "cut_racestarter_2x4_split_01", racename: 2.5, racename_len: 2.5, cut: 12.0, countdown: 9.0 },
    PreraceEvents { scene: "cut_racestarter_2x4_split_02", racename: 4.36667, racename_len: 2.0, cut: 13.6667, countdown: 10.6667 },
    PreraceEvents { scene: "cut_racestarter_3x3_01", racename: 7.2, racename_len: 1.66667, cut: 9.36667, countdown: 9.36667 },
    PreraceEvents { scene: "cut_racestarter_3x3_02", racename: 7.16667, racename_len: 2.0, cut: 12.6667, countdown: 9.66667 },
    PreraceEvents { scene: "cut_racestarter_3x3_03", racename: 0.666667, racename_len: 2.0, cut: 3.16667, countdown: 3.16667 },
    PreraceEvents { scene: "cut_racestarter_3x3_04", racename: 3.16667, racename_len: 5.33333, cut: 12.0, countdown: 9.0 },
    PreraceEvents { scene: "cut_racestarter_3x3_05", racename: 5.33333, racename_len: 3.96667, cut: 12.8, countdown: 9.8 },
    PreraceEvents { scene: "cut_racestarter_3x3_06", racename: 7.83333, racename_len: 2.0, cut: 13.3333, countdown: 10.3333 },
    PreraceEvents { scene: "cut_racestarter_3x3_split_01", racename: 1.16667, racename_len: 2.0, cut: 9.33333, countdown: 3.66667 },
    PreraceEvents { scene: "cut_racestarter_3x3_split_02", racename: 9.5, racename_len: 2.0, cut: 18.6667, countdown: 15.6667 },
    PreraceEvents { scene: "cut_racestarter_4x2_01", racename: 3.43333, racename_len: 2.0, cut: 8.96667, countdown: 5.96667 },
    PreraceEvents { scene: "cut_racestarter_4x2_02", racename: 5.0, racename_len: 2.0, cut: 7.5, countdown: 7.5 },
    PreraceEvents { scene: "gen_prerace_grid_1x2_01", racename: 2.0, racename_len: 2.0, cut: 6.7, countdown: 6.7 },
    PreraceEvents { scene: "gen_prerace_grid_1x2_02", racename: 2.0, racename_len: 2.0, cut: 6.83333, countdown: 6.83333 },
    PreraceEvents { scene: "gen_prerace_grid_1x2_03", racename: 2.0, racename_len: 2.0, cut: 5.46667, countdown: 5.46667 },
    PreraceEvents { scene: "gen_prerace_starter_01", racename: 6.73333, racename_len: 2.0, cut: 9.23333, countdown: 9.23333 },
    PreraceEvents { scene: "gen_prerace_starter_04", racename: 7.2, racename_len: 2.0, cut: 9.7, countdown: 9.7 },
    PreraceEvents { scene: "gen_prerace_starter_05", racename: 5.13333, racename_len: 2.0, cut: 7.63333, countdown: 7.63333 },
    PreraceEvents { scene: "gen_prerace_starter_07", racename: 3.83333, racename_len: 2.0, cut: 6.33333, countdown: 6.33333 },
    PreraceEvents { scene: "gen_prerace_starter_09", racename: 4.66667, racename_len: 2.0, cut: 7.16667, countdown: 7.16667 },
    PreraceEvents { scene: "sc_gen_prerace_starter_2x4_01", racename: 9.66667, racename_len: 4.46667, cut: 14.4, countdown: 16.0667 },
    PreraceEvents { scene: "sc_gen_prerace_taunt_h_01", racename: 2.63333, racename_len: 2.0, cut: 7.66667, countdown: 5.66667 },
    PreraceEvents { scene: "sc_gen_prerace_taunt_hp_01", racename: 2.63333, racename_len: 2.0, cut: 7.66667, countdown: 5.66667 },
    PreraceEvents { scene: "sc_gen_prerace_taunt_hp_02", racename: 2.63333, racename_len: 2.0, cut: 7.66667, countdown: 5.66667 },
    PreraceEvents { scene: "sc_gen_prerace_taunt_ph_01", racename: 2.63333, racename_len: 2.0, cut: 7.66667, countdown: 5.66667 },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The taunt request against the original run offline (tools/taunt_request_reference.py: F-56020 on 26 names x
    /// 12 slot / racer sets): the scenario is TAUNT_PRERACE_<taunt_kind>, A / B the racers of slots 0 / 1 (PH: 1 / 0),
    /// and nothing without a taunt kind or with a slot that holds no racer.
    #[test]
    fn taunt_request_matches_the_original() {
        let text = include_str!("testdata/taunt_request_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (name, rest) = line.rsplit_once('|').unwrap();
            let (args, out) = rest.split_once(" -> ").unwrap();
            let v: Vec<i32> = args.split(' ').map(|x| x.parse().unwrap()).collect();
            let has = |s: i32| s >= 0 && s < 8 && (v[2] >> s) & 1 == 1;
            let want = match taunt_kind(name) {
                Some(k) if has(v[0]) && has(v[1]) => {
                    let (a, b) = if k == "PH" { (v[1], v[0]) } else { (v[0], v[1]) };
                    format!("TAUNT_PRERACE_{k} {a} {b}")
                }
                _ => "-".to_string(),
            };
            assert_eq!(out, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 312);
    }

    /// The slot launch against the original run offline (tools/slot_event_reference.py: F-51323 with the event
    /// type look-up and strstr, 8 scene names x 23 event names x 4 starts, 736 cases): the event made or not, its
    /// type, racer and start equal.
    #[test]
    fn slot_event_matches_the_original() {
        let text = include_str!("testdata/slot_event_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let mut it = line.splitn(3, '|');
            let (scene, event, rest) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
            let (args, out) = rest.split_once(" -> ").unwrap();
            let a: Vec<&str> = args.split(' ').collect();
            let (slot, t, dur): (usize, f32, f32) = (a[0].parse().unwrap(), a[1].parse().unwrap(), a[2].parse().unwrap());
            let ids: Vec<i32> = a[3].split(',').map(|x| x.parse().unwrap()).collect();
            let racers: u32 = a[4].parse().unwrap();
            let id = ids[slot];
            let want = if id > -1 && id < 8 && (racers >> id) & 1 == 1 {
                format!("{} {} {} {} 0.5", event_type(event), id, slot_event_start(scene, event, t), dur)
            } else {
                "free-".to_string()
            };
            let got: Vec<String> = out.split(' ').map(|x| x.parse::<f32>().map_or(x.to_string(), |v| v.to_string())).collect();
            assert_eq!(got.join(" "), want, "{line}");
            n += 1;
        }
        assert_eq!(n, 736);
    }

    /// The pick against the original run offline (tools/prerace_pick_reference.py: F-10331 and F-24279 with the
    /// bags' code, the load shuffle of every bag, then 30 picks per case on keys -2..9, 3,000 cases): every
    /// result (grid place and item, or none) and the generator's two words equal.
    #[test]
    fn pick_matches_the_original() {
        let text = include_str!("testdata/prerace_pick_ref.txt");
        let mut cases = 0;
        for line in text.lines() {
            let (head, tail) = line.split_once(" -> ").unwrap();
            let h: Vec<&str> = head.split(' ').collect();
            let nums = |s: &str| s.split(',').map(|x| x.parse::<i32>().unwrap()).collect::<Vec<i32>>();
            let (keys, counts) = (nums(h[0]), nums(h[1]));
            let starter: Vec<u32> = h[2].split(',').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let lists = (0..9).map(|g| (format!("g{g}"), (0..counts[g] as usize).map(|i| {
                if (starter[g] >> i) & 1 == 1 { format!("x_racestarter_{g}_{i}") } else { format!("s_{g}_{i}") }
            }).collect())).collect();
            let table = GridTable { lists, keys, sc: h[3] == "1" };
            let mut p = Picker::new(table, h[4].parse().unwrap());
            let (picks, state) = tail.split_once(" | ").unwrap();
            let want: Vec<&str> = picks.split(',').collect();
            for (op, w) in h[5].split(';').zip(&want) {
                let o = nums(op);
                let got = p.pick_key(o[0], o[1] == 1, o[2] == 1, o[3] == 1).map_or("-".to_string(), |s| {
                    let g = (0..9).find(|&g| p.table.lists[g].1.contains(&s)).unwrap();
                    format!("{g}.{}", p.table.lists[g].1.iter().position(|x| *x == s).unwrap())
                });
                assert_eq!(&got, w, "{line}");
            }
            let st: Vec<u32> = state.split(' ').map(|x| x.parse().unwrap()).collect();
            assert_eq!((p.rng.lo, p.rng.carry), (st[0], st[1]), "{line}");
            cases += 1;
        }
        assert_eq!(cases, 3000);
    }

    /// The race parser's GridType lookup F-53452 run offline on 25 names (tools/prerace_pick_reference.py).
    #[test]
    fn grid_type_key_matches_the_original() {
        let text = include_str!("testdata/grid_type_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (name, key) = line.rsplit_once('|').unwrap();
            assert_eq!(grid_type_key(name), key.parse::<i32>().unwrap(), "{line}");
            n += 1;
        }
        assert_eq!(n, 25);
    }

    /// The bags against the original run offline (tools/prerace_bag_reference.py: F-11038's shuffle, then 40
    /// draws of F-27986 or F-96888 per case, 3,000 cases): every pick, the order words, the cursor and the
    /// generator's two words equal.
    #[test]
    fn bags_match_the_original() {
        let text = include_str!("testdata/prerace_bag_ref.txt");
        let mut cases = 0;
        for line in text.lines() {
            let (head, tail) = line.split_once(" -> ").unwrap();
            let h: Vec<&str> = head.split(' ').collect();
            let n: usize = h[0].parse().unwrap();
            let lo: u32 = h[1].parse().unwrap();
            let carry: u32 = h[2].parse().unwrap();
            let ops: Vec<i32> = h[3].split(',').map(|x| x.parse().unwrap()).collect();
            let (picks, state) = tail.split_once(" | ").unwrap();
            let want: Vec<usize> = picks.split(',').map(|x| x.parse().unwrap()).collect();
            let st: Vec<&str> = state.split(' ').collect();
            let order: Vec<u32> = st[0].split(',').map(|x| x.parse().unwrap()).collect();
            let table = GridTable { lists: vec![("G".into(), (0..n).map(|i| format!("s{}", i)).collect())], keys: vec![0], sc: false };
            let mut p = Picker { table, bags: vec![(vec![0; n], 0)], rng: crate::vehicles::parked::Mwc { lo, carry } };
            p.shuffle(0);
            let got: Vec<usize> = ops.iter().map(|&o| {
                let s = if o < 0 { p.draw(0) } else { p.draw_limited(0, o as usize) };
                s.unwrap()[1..].parse().unwrap()
            }).collect();
            assert_eq!(got, want, "{}", line);
            assert_eq!(p.bags[0].0, order, "{}", line);
            assert_eq!(p.bags[0].1, st[1].parse::<usize>().unwrap(), "{}", line);
            assert_eq!((p.rng.lo, p.rng.carry), (st[2].parse::<u32>().unwrap(), st[3].parse::<u32>().unwrap()), "{}", line);
            cases += 1;
        }
        assert_eq!(cases, 3000);
    }

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
        // (the jaw flap: no event of the taunt has f3 1.0 - no jaw moves; in the red light start hw00 the hookman's
        // talk_right has it, his jaw moves from 2.33 s for 4.33 s)
        let jaws = |sc: &str| slot_events(sc).iter().filter(|e| jaw_start(e.event, e.f3) || jaw_stop(e.event, e.f3)).map(|e| (e.event, e.slot, e.t, e.dur)).collect::<Vec<_>>();
        assert!(jaws("racestart/gen_prerace_taunt_HP_01").is_empty());
        assert_eq!(jaws("racestart/gen_prerace_red_hw00_01"), vec![("gesture_talk_right", 1, 2.33333, 4.33333)]);
        // (39 race-start rows have f3 1.0, every one a gesture_talk_right / gesture_listen_right on slot 1; all others 0)
        let jaw: Vec<_> = SLOT_EVENTS.iter().filter(|e| e.f3 != 0.0).collect();
        assert_eq!(jaw.len(), 39);
        assert!(jaw.iter().all(|e| e.f3 == 1.0 && e.slot == 1 && matches!(e.event, "gesture_talk_right" | "gesture_listen_right")));
        assert!(!jaw_start("gesture_disappointed", 1.0) && jaw_stop("gesture_disappointed", 1.0) && !jaw_start("gesture_copfidget", 1.0) && !jaw_stop("gesture_copfidget", 1.0));
        assert!(jaw_start("gesture_look_right", 1.0005) && !jaw_start("gesture_look_right", 1.002) && jaw_start("jawflap", 0.0) && jaw_stop("jawflap", 0.0));
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
        // (an unknown grid is key -1: no list of its own, but a first race still gets a taunt - F-53452, F-10331)
        assert_eq!(p.pick(&PickArgs { grid: "16_Race_Grid".into(), first_race: true, ..Default::default() }).as_deref(), Some("racestart/gen_prerace_taunt_hp_01"));
        // the bag: every scene once per round, no repeat across the round's seam
        let mut q = Picker::new(GridTable::parse(tbl), 99);
        let k = q.table.key("4x2_Race_Grid").unwrap();
        let seen: Vec<String> = (0..30).map(|_| q.draw(k).unwrap()).collect();
        for r in seen.chunks(3) {
            let mut v = r.to_vec();
            v.sort();
            v.dedup();
            assert_eq!(v.len(), 3);
        }
        assert!(seen.windows(2).all(|w| w[0] != w[1]));
        for e in PRERACE_EVENTS.iter() {
            // the countdown starts after 3 s, and the camera goes back to the game at or after it - except in the South
            // Central starter, whose script counts down at 16.0667 s, after its cut at 14.4 s (the scene runs on)
            assert!(e.countdown > 3.0 && (e.cut >= e.countdown || e.scene == "sc_gen_prerace_starter_2x4_01"), "{}", e.scene);
        }
    }

    /// The jaw flap rule against the original's Start / Stop handlers F-91263 / F-26895 run offline
    /// (work/driverlight/jaw/jaw_ref.py): 97 event types x 8 f3 values on a racer whose +688 holds 0x5a; Start leaves 1
    /// where `jaw_start` says, Stop 0 where `jaw_stop` says, and every other case keeps 0x5a. The types that reach an
    /// unmodelled manager global (an "error" line, none a gesture) are skipped.
    #[test]
    fn jaw_rules_match_the_original() {
        let (mut n, mut skipped) = (0, 0);
        for l in include_str!("testdata/jaw_ref.txt").lines() {
            let p: Vec<&str> = l.split_whitespace().collect();
            let ty: usize = p[0].parse().unwrap();
            let f3 = f32::from_bits(u32::from_str_radix(p[1], 16).unwrap());
            if p[2] == "error" {
                assert!(!matches!(ty, 0..=10 | 48..=55), "{l}");
                skipped += 1;
                continue;
            }
            let (s688, e688) = (u8::from_str_radix(p[3], 16).unwrap(), u8::from_str_radix(p[5], 16).unwrap());
            // (by type: a name does not always give its own type - `gesture_mild_right` contains `gesture_mild`, type 5)
            assert_eq!(s688, if jaw_start_type(ty as i32, f3) { 1 } else { 0x5a }, "start {l}");
            assert_eq!(e688, if jaw_stop_type(ty as i32, f3) { 0 } else { 0x5a }, "stop {l}");
            n += 1;
        }
        assert_eq!((n, skipped), (656, 120));
    }
}
