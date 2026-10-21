//! Step 23b: the open list of steps 22b and 23 - the statistics the port did not count (the
//! distance driven, on two wheels, in a wheelie, the jumps), the race goals of the race-end
//! statistics (aAOR / aAAR / aARC), the showroom's test drive (tTST, aTDV), the photo album's
//! slots (tPTS, aFAB) and the audio's slow-motion switch during the tutorials.
//!
//! The original (scratch `work/save/port/s23b/`):
//! - The odometer (sub_82562480, once a frame for the player's vehicle): speed = |(vx, vz)| x
//!   2.237 (mph); +20 += speed x the frame time x 1/3600 - miles; flushed into gkDS (add) by
//!   sub_822A61B0 (with vkDT for the vehicle). aDST: gkDS >= 5000.
//! - Two wheels (mcCarGyro sub_823663D0 0x823673D4..0x823674C0): while the gyro's lean state
//!   (+18) is on and the tuning has Lean (> 0), for the player: gk2W += speed (+224, m/s) x
//!   2.237 x frame time / 3600. a2WD: gk2W >= 50.
//! - Wheelie (mcBikeGyro sub_823688B8 0x8236A33C..0x8236A3E0): while the stunt state (+16) is
//!   above 0 (1 up, 99 coming down): gkWL += the same. aWHL: gkWL >= 50.
//! - Jumps (sub_82330378 0x82330554..0x82330720): no wheel down - the air time (+584) grows by
//!   the frame time; a wheel down with air time > 0 - for the player's vehicle gsJT += the air
//!   time, gmJD = max(gmJD, the flat distance from where it left the ground (+640) to here),
//!   then air time = -air time; a wheel down with air time <= 0 - air time 0, the take-off
//!   point = here. aJTM: gsJT >= 600; aJDS: gmJD >= 310.896.
//! - The race-end statistics (sub_822D0438 0x822D0BA0..0x822D0C0C -> sub_822CD720): won (place
//!   1), the time under the race's GoalTargetTime x 1000 ms (the city table's +136), the damage
//!   under a quarter of MaxDamage ((max - damage) > 0.75 max): rWIN / rBTT / rMDG of the race (max
//!   1); all three: rCGL of the race, aAOR; then, by the race's content (+144: ContentID, else 4
//!   for South Central, 1 otherwise), bit 0: aAAR once every Ordered / Circuit / Unordered / Blitz
//!   race (sub types 1, 2, 3, 5) not DevOnly of content bit 0 has rCGL; bit 2: aARC the same for
//!   content 4. (The races' GoalTargetTime / TargetDamage are those of the GOAL ATTACK mode
//!   - ArcadeMenuGoalAttack, pause MODES' GOAL ATTACK; the check runs only for a race started
//!   from it: sub_822D0438 0x822D0974 tests the mode object's +796 - `goal_attack`.)
//! - The showroom's TEST DRIVE (sub_826A6330): tTST + 1; the vehicle table's +48
//!   (ContentDownloadFlags, else ContentFlags - `tune/vehicle` lists) >= 4: aTDV (set once).
//! - The photo album (sub_8263DCF8): a picture saved into slot k of the album (16 slots,
//!   `PhotoAlbum.s0..s15`; PHOTO_ALBUM): a slot not used before raises the album's count (+856),
//!   aFAB as it reaches 16; tPTS + 1 for every picture; the slot marked used (+860).

use crate::game::{Game, Mode};
use glam::Vec3;

/// The player's vehicle as the statistics read it this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct S23Drive {
    pub present: bool,
    /// |(vx, vz)| and |v| in m/s
    pub vel_xz: f32,
    pub speed: f32,
    /// mcCarGyro's lean on (two wheels), mcBikeGyro's stunt state above 0 (a wheelie)
    pub two: bool,
    pub wheelie: bool,
    /// no wheel down
    pub airborne: bool,
    pub pos: Vec3,
}

/// The player's vehicle (the port's state: the wheels' contact, vehsim's `two` and `wheelie`).
pub fn drive_of(c: Option<&crate::vehicles::CarInstance>) -> S23Drive {
    let Some(c) = c else { return S23Drive::default() };
    let v = c.state.vel;
    let vs = c.state.veh.as_ref();
    S23Drive {
        present: true,
        vel_xz: Vec3::new(v.x, 0.0, v.z).length(),
        speed: v.length(),
        two: vs.map_or(false, |s| s.two != 0),
        wheelie: c.model.bike && vs.map_or(false, |s| s.wheelie == 1 || s.wheelie == 99),
        airborne: c.state.wheels.iter().take(if c.model.bike { 2 } else { 4 }).all(|w| !w.grounded),
        pos: c.state.pos,
    }
}

/// What the running game keeps for step 23b.
#[derive(Clone, Debug, Default)]
pub struct S23bLive {
    /// the jump's air time (+584; negative for the frame after a landing) and take-off point (+640)
    pub air: f32,
    pub takeoff: Option<Vec3>,
    /// the test drive seen last frame
    pub td_was: Option<usize>,
    /// South Central's set-up script: its clock (WAITUNWARPED 6000 once the player's car is
    /// there) and whether it has run since the career started
    pub sc_setup_t: f32,
    pub sc_setup_done: bool,
    /// the SERVICE STATION menu's REFILL NITRO row is enabled (nitrous fitted)
    pub gas_nitro: bool,
    /// a frontend sound object for the loop to play
    pub ui_obj: Option<&'static str>,
}

/// The gas stations of `game/MiscTasks/GasStations` (`0x53b4b66d`, started by the career's
/// start-up script): each a trigger at this centre with this radius (0x40..0x3c0; South
/// Central's `sc_GasStations`, `0x4398c445`, holds ten of them and one more but is started by
/// nothing).
pub const GAS_STATIONS: [([f32; 3], f32); 27] = [
    ([241.0, 25.76, -918.4], 12.0),
    ([516.8, 31.76, -1368.3], 12.0),
    ([519.7, 30.76, -1290.6], 12.0),
    ([816.06, 31.76, -1283.56], 15.0),
    ([-259.4, -3.06, -32.4], 12.0),
    ([-732.2, 2.76, -157.7], 15.0),
    ([-559.9, 39.76, -1784.9], 12.0),
    ([-2768.9, -29.76, -979.4], 12.0),
    ([1030.06, 6.76, -307.56], 12.0),
    ([38.0, 6.76, -415.0], 12.0),
    ([-598.9, -10.76, 339.9], 12.0),
    ([-1834.4, 3.7, -216.6], 15.0),
    ([516.6, 20.4, -751.6], 15.0),
    ([2348.3, -3.5, -263.3], 15.0),
    ([-607.0, -22.1, 1700.7], 12.0),
    ([-1112.4, -21.8, 1650.3], 15.0),
    ([-1715.6, -12.0, 1580.3], 12.0),
    ([-1796.6, -10.8, 1506.3], 12.0),
    ([-1830.6, -10.48, 895.3], 12.0),
    ([-163.6, -14.8, 514.3], 12.0),
    ([-197.6, -14.8, 518.3], 12.0),
    ([520.6, -13.8, 421.3], 12.0),
    ([1206.6, -16.0, 1401.3], 18.0),
    ([974.9, -15.0, 1188.3], 12.0),
    ([-1248.9, -7.9, 1280.3], 15.0),
    ([-156.9, -21.6, 1555.9], 15.0),
    ([946.3, -14.6, 456.0], 25.0),
];

/// The GasStations script's loop (0x438..0x716, every 10 ms) for one frame: what the player's
/// visit does now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GasStep {
    None,
    /// mcCareer_RefillOneNitro: one canister
    RefillOne,
    /// Form_ShowForm(61): the SERVICE STATION menu
    Menu,
}

/// The visit's locals: the station the player is in (mcCareer_IsInGasStation /
/// GetGasStationIDX), armed (l69), refilled (l70), the last refill's time (l63) and the
/// clock (AudioManager_GetActualTime, seconds).
#[derive(Clone, Debug)]
pub struct GasVisit {
    pub at: Option<usize>,
    pub armed: bool,
    pub refilled: bool,
    pub last: f32,
    pub clock: f32,
}

impl Default for GasVisit {
    fn default() -> Self {
        GasVisit { at: None, armed: true, refilled: false, last: -10.0, clock: 0.0 }
    }
}

impl GasVisit {
    /// One frame: `pos` / `speed` (m/s) of the player's car, `menu_open` the form up
    /// (mcCareer_ActivateGasMenu), `paused`, `police` (CheckPoliceActivity(1) >= 1), `menu_state`
    /// the UI state is Cruising_Offline or Racing_Payback. (The script's first WAIT 10000 and
    /// the trigger's rDelay 0.5 are left out: the port's start and the generic trigger's delay
    /// were not decoded.)
    pub fn step(&mut self, dt: f32, pos: Vec3, speed: f32, menu_open: bool, paused: bool, police: bool, menu_state: bool) -> GasStep {
        self.clock += dt;
        let flat = |c: [f32; 3]| Vec3::new(pos.x - c[0], 0.0, pos.z - c[2]).length();
        // (the trigger: bTriggerMultipleTimes, not only on entry, also while moving)
        if let Some(i) = GAS_STATIONS.iter().position(|(c, r)| flat(*c) < *r) {
            self.at = Some(i);
        }
        let mut out = GasStep::None;
        if self.at.is_some() && self.armed {
            if speed < 7.0 && !paused {
                if !police && self.clock >= self.last + 2.0 && menu_state {
                    out = GasStep::Menu;
                    self.armed = false;
                }
            } else if speed > 7.1 {
                if !self.refilled {
                    if self.clock >= self.last + 2.0 {
                        out = GasStep::RefillOne;
                    }
                    self.refilled = true;
                    self.last = self.clock;
                }
            } else {
                self.armed = false;
            }
        }
        // (0x716: the visit ends 10 m outside the radius, not while the menu is up)
        if let Some(i) = self.at {
            let (c, r) = GAS_STATIONS[i];
            if !menu_open && flat(c) - 10.0 > r {
                (self.at, self.armed, self.refilled) = (None, true, false);
            }
        }
        out
    }
}

impl Game {
    /// The SERVICE STATION menu comes up (mcGasStationMenuForm open, sub_82646618): the cursor
    /// on FULL FIX, REFILL NITRO enabled with nitrous fitted, UI_MENU_SELECT.
    pub fn gas_menu_open(&mut self, nitro: bool) {
        self.hangout_key = Some(crate::game::GAS_MENU.to_string());
        self.hangout_page = (0, 0);
        self.opt_x.s23b.gas_nitro = nitro;
        self.opt_x.s23b.ui_obj = Some("UI_MENU_SELECT");
    }
}

/// mph per m/s (0x8201CD1C) and hours per second (0x8202F80C).
pub const MPH: f32 = 2.237;
pub const PER_HOUR: f32 = 0.000_277_778;

/// The races of the city table that count for aAAR / aARC: sub types Ordered, Circuit,
/// Unordered, Blitz (1, 2, 3, 5 of sub_8238E748's table 0x827E930C).
pub const GOAL_SUB_TYPES: [&str; 4] = ["Ordered", "Circuit", "Unordered", "Blitz"];

/// A race's content (+144): ContentID, else 4 for South Central, 1 otherwise (sub_82388E48
/// 0x823895D0). No race file on the disc has a ContentID.
pub fn race_content(district: &str) -> u32 {
    if district.eq_ignore_ascii_case("SouthCentral") { 4 } else { 1 }
}

/// The vehicle table's +48 (ContentDownloadFlags, else ContentFlags) of the three lists.
pub fn content_flags(data_dir: &str, car: &str) -> u32 {
    static TABLE: std::sync::OnceLock<std::collections::HashMap<String, u32>> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut out = std::collections::HashMap::new();
        for f in ["0xe94bde63", "0xccb5daee", "0x3cb91343"] {
            let Ok(s) = std::fs::read(std::path::Path::new(data_dir).join("tune/vehicle").join(f)) else { continue };
            let s = String::from_utf8_lossy(&s);
            for block in s.split("Vehicle").skip(1) {
                let field = |k: &str| block.lines().map(|l| l.trim()).find_map(|l| l.strip_prefix(k).filter(|r| r.starts_with(|c: char| c.is_whitespace())).map(|r| r.trim().to_string()));
                let Some(name) = field("Name") else { continue };
                let v = field("ContentDownloadFlags").or_else(|| field("ContentFlags")).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                out.insert(name.to_ascii_lowercase(), v);
            }
        }
        out
    });
    t.get(&car.to_ascii_lowercase()).copied().unwrap_or(0)
}

/// The photo album's slot files (the port's PHOTO_ALBUM): `<data>/../photos/album/slot_NN.png`.
pub fn album_dir(data_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(data_dir).join("../photos/album")
}

pub const ALBUM_SLOTS: usize = 16;

impl Game {
    /// Once a frame from the step 23 tick: the driving statistics, the test drive.
    pub fn mc4b_tick(&mut self, dt: f32, d: S23Drive) {
        // ---- the showroom's test drive (sub_826A6330) ----
        let td = self.test_drive;
        if td.is_some() && self.opt_x.s23b.td_was.is_none() {
            self.mc4_stat_add("tTST", 0, 1.0);
            let car = td.and_then(|i| self.car_dirs.get(i)).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let flags = content_flags(&self.data_dir, &car);
            log::info!("test drive: {} (vehicle table +48 = {})", car, flags);
            if flags >= 4 {
                self.mc4_award("aTDV");
            }
        }
        self.opt_x.s23b.td_was = td;
        let sim = dt * self.mc4_time_scale();
        // ---- South Central's set-up script (career.sc_setup) ----
        if !self.opt_x.s23b.sc_setup_done && d.present && sim > 0.0 && matches!(self.mode, Mode::FreeRoam | Mode::Race) && self.career.loaded() {
            self.opt_x.s23b.sc_setup_t += sim;
            if self.opt_x.s23b.sc_setup_t >= 6.0 {
                self.opt_x.s23b.sc_setup_done = true;
                for id in self.career.sc_setup() {
                    log::info!("CareerSetup_SC: {} to READY_TO_START", id);
                }
            }
        }
        // ---- the driving statistics: only while the world runs ----
        if !d.present || sim <= 0.0 || !matches!(self.mode, Mode::FreeRoam | Mode::Race) || self.fresh_game {
            return;
        }
        let miles = |v: f32| (v * MPH * sim * PER_HOUR) as f64;
        self.mc4_stat_add("gkDS", 0, miles(d.vel_xz));
        if d.two {
            self.mc4_stat_add("gk2W", 0, miles(d.speed));
        }
        if d.wheelie {
            self.mc4_stat_add("gkWL", 0, miles(d.speed));
        }
        let x = &mut self.opt_x.s23b;
        if d.airborne {
            x.air += sim;
        } else if x.air > 0.0 {
            let air = x.air;
            let dist = x.takeoff.map_or(0.0, |t| Vec3::new(d.pos.x - t.x, 0.0, d.pos.z - t.z).length());
            x.air = -air;
            self.mc4_stat_add("gsJT", 0, air as f64);
            if dist as f64 > self.mc4_stat("gmJD", 0) {
                self.mc4_stat_set("gmJD", 0, dist as f64);
            }
            if air > 0.5 {
                log::info!("jump: {:.2} s in the air, {:.1} m (gsJT {:.1}, gmJD {:.1})", air, dist, self.mc4_stat("gsJT", 0), self.mc4_stat("gmJD", 0));
            }
        } else {
            x.air = 0.0;
            x.takeoff = Some(d.pos);
        }
    }

    /// sub_822CD720 from the race-end statistics: the race's goals (won, under its goal time,
    /// under a quarter of the damage), rCGL and aAOR when all three are met; aAAR / aARC once
    /// every race of its content has them.
    pub fn mc4_race_goals(&mut self, place: usize, time: Option<f32>, damage: f32) {
        if !self.goal_attack {
            return;
        }
        let Some(def) = self.current_race.and_then(|i| self.race_defs.get(i)).cloned() else {
            log::info!("race goals: no race in hand");
            return;
        };
        let Some(k) = self.mc4_race_index(&def.id) else {
            log::info!("race goals: {} is not in the city's race table", def.id);
            return;
        };
        let k = k as u32;
        let win = place == 1;
        let beat = time.map_or(false, |t| ((t * 1000.0).round() as i64) < ((def.goal_time * 1000.0).round() as i64));
        let dmg = (1.0 - damage) > 0.75;
        for (on, id) in [(beat, "rBTT"), (win, "rWIN"), (dmg, "rMDG")] {
            if on {
                self.mc4_stat_set(id, k, 1.0);
            }
        }
        log::info!("race goals: {} - won {}, time {:?} under {} s {}, damage {:.2} {}", def.id, win, time, def.goal_time, beat, damage, dmg);
        if !(win && beat && dmg) {
            return;
        }
        self.mc4_stat_set("rCGL", k, 1.0);
        self.mc4_award("aAOR");
        let content = race_content(&def.district);
        let (bit, id) = if content & 1 != 0 { (1, "aAAR") } else if content & 4 != 0 { (4, "aARC") } else { return };
        if self.mc4_awarded(id) {
            return;
        }
        let all = (0..self.race_table.len()).all(|j| {
            let Some(d) = self.race_defs.iter().find(|d| d.id.eq_ignore_ascii_case(&self.race_table[j])) else { return true };
            let counts = GOAL_SUB_TYPES.iter().any(|s| d.sub_type.eq_ignore_ascii_case(s)) && !d.dev_only && race_content(&d.district) & bit != 0;
            !counts || self.mc4_stat("rCGL", j as u32) == 1.0
        });
        if all {
            self.mc4_award(id);
        }
    }

    /// A race's goals as GOAL ATTACK's list shows them (sub_82655E58): the mask "0".."15" from
    /// the save's rWIN 8, rMDG 4, rBTT 2, rCGL 1 of the race's table index.
    pub fn mc4_goal_mask(&self, race: usize) -> u8 {
        let Some(k) = self.race_defs.get(race).and_then(|d| self.mc4_race_index(&d.id)) else { return 0 };
        [("rWIN", 8), ("rMDG", 4), ("rBTT", 2), ("rCGL", 1)].iter().filter(|(id, _)| self.mc4_stat(id, k as u32) != 0.0).map(|(_, b)| b).sum()
    }

    /// The goal panel's four lines for a race (columns 2..5 of the list: GOALATTACK_WIN,
    /// GOALATTACK_DAMAGE, GOAL_TIME with GOAL_TIME_ENTRY = the race's GoalTargetTime by
    /// sub_8223DE50 - set on a selection change, sub_82656048 -, GOALATTACK_ALLGOALS).
    pub fn mc4_goal_lines(&self, race: usize) -> [String; 4] {
        let ms = self.race_defs.get(race).map_or(0, |d| (d.goal_time * 1000.0).round() as i32);
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_string()).unwrap_or_else(|| d.to_string());
        [
            st("GOALATTACK_WIN", "WIN THIS RACE"),
            st("GOALATTACK_DAMAGE", "FINISH RACE WITH LESS THAN 25% DAMAGE"),
            fill_d(&st("GOAL_TIME", "FINISH THIS RACE IN UNDER <D>GOAL_TIME_ENTRY</D>"), &crate::flash_ui::race_time(ms)),
            st("GOALATTACK_ALLGOALS", "ACHIEVE ALL GOALS IN THE SAME RACE"),
        ]
    }

    /// The race start badge's second line in GOAL ATTACK (GetRaceInfo sub_82640AE0
    /// 0x82640D78..0x82640DB4): RACE_INFO_GOALATTACK_FORMAT "TARGET TIME: <D>RACE_INFO_GOAL_TIME</D>".
    pub fn mc4_goal_badge(&self) -> Option<String> {
        let d = self.current_race.and_then(|i| self.race_defs.get(i)).filter(|_| self.goal_attack)?;
        let f = self.career.strings.get("RACE_INFO_GOALATTACK_FORMAT").map(|v| v.to_string()).unwrap_or_else(|| "TARGET TIME: <D>RACE_INFO_GOAL_TIME</D>".to_string());
        Some(fill_d(&f, &crate::flash_ui::race_time((d.goal_time * 1000.0).round() as i32)))
    }

    /// A picture taken in photo mode (sub_8263DCF8): the album's slot it goes into - the first
    /// free one (the port has no PA_ChooseSlot page; with all sixteen used, slot 0 is written
    /// again) -, the count of used slots, aFAB as it reaches 16, tPTS + 1. The file to write.
    pub fn mc4b_photo_path(&mut self) -> String {
        let dir = album_dir(&self.data_dir);
        let _ = std::fs::create_dir_all(&dir);
        let used: Vec<bool> = (0..ALBUM_SLOTS).map(|k| dir.join(format!("slot_{k:02}.png")).exists()).collect();
        let slot = used.iter().position(|u| !u).unwrap_or(0);
        if !used[slot] {
            let count = used.iter().filter(|u| **u).count() + 1;
            if count == ALBUM_SLOTS {
                self.mc4_award("aFAB");
            }
        }
        self.mc4_stat_add("tPTS", 0, 1.0);
        self.dirty = true;
        log::info!("photo: album slot {} (tPTS {})", slot, self.mc4_stat("tPTS", 0));
        dir.join(format!("slot_{slot:02}.png")).to_string_lossy().to_string()
    }

    /// AudioManager_SetActiveTimeSlow's state: on from a tutorial's SetActiveTimeSlow(1) (before
    /// its ease to pause) until its form is closed (SetActiveTimeSlow(0), before the ease back);
    /// RACE_GH_02's lesson while box 76 is up (SetActiveTimeSlow(1) at 0x3e4, (0) after the box).
    pub fn mc4b_time_slow(&self) -> bool {
        let x = &self.opt_x.s23;
        x.run.as_ref().map_or(false, |r| r.slow && r.phase >= 1) || x.lesson == 1
    }
}

/// A string table text with its one `<D>name</D>` variable filled in.
pub fn fill_d(text: &str, value: &str) -> String {
    match (text.find("<D>"), text.find("</D>")) {
        (Some(a), Some(b)) if b > a => format!("{}{}{}", &text[..a], value, &text[b + 4..]),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GasStations (`game/MiscTasks/0x53b4b66d` 0x438..0x716): rolling through fills one canister once a
    /// visit; stopping opens the menu once a visit (not with the police about, not between 7.0 and 7.1); the
    /// visit ends 10 m outside the radius, not while the menu is up.
    #[test]
    fn gas_station_visit() {
        let c = Vec3::new(38.0, 6.76, -415.0);
        let far = c + Vec3::new(30.0, 0.0, 0.0);
        let dt = 0.01;
        let mut g = GasVisit::default();
        assert_eq!(g.step(dt, far, 20.0, false, false, false, true), GasStep::None);
        assert_eq!(g.at, None);
        // (rolling in: one canister, once)
        assert_eq!(g.step(dt, c, 20.0, false, false, false, true), GasStep::RefillOne);
        assert_eq!(g.at, Some(9));
        assert_eq!(g.step(dt, c, 20.0, false, false, false, true), GasStep::None);
        // (stopping within 2 s of the refill: nothing yet; then the menu, once)
        assert_eq!(g.step(dt, c, 0.0, false, false, false, true), GasStep::None);
        let mut n = 0;
        for _ in 0..300 {
            if g.step(dt, c, 0.0, n > 0, false, false, true) == GasStep::Menu {
                n += 1;
            }
        }
        assert_eq!(n, 1);
        // (the menu up: the visit is kept however far; closed, it ends at radius + 10)
        g.step(dt, far, 0.0, true, false, false, true);
        assert_eq!(g.at, Some(9));
        g.step(dt, c + Vec3::new(21.0, 0.0, 0.0), 0.0, false, false, false, true);
        assert_eq!(g.at, Some(9));
        g.step(dt, c + Vec3::new(22.5, 0.0, 0.0), 0.0, false, false, false, true);
        assert_eq!((g.at, g.armed, g.refilled), (None, true, false));
        // (a new visit with the police about: no menu; between 7.0 and 7.1 the visit is spent)
        g.clock += 5.0;
        assert_eq!(g.step(dt, c, 0.0, false, false, true, true), GasStep::None);
        assert!(g.armed);
        assert_eq!(g.step(dt, c, 7.05, false, false, false, true), GasStep::None);
        assert!(!g.armed);
        assert_eq!(g.step(dt, c, 20.0, false, false, false, true), GasStep::None);
        // (in a race that is not a payback: the refill only)
        let mut g = GasVisit::default();
        assert_eq!(g.step(dt, c, 0.0, false, false, false, false), GasStep::None);
        assert_eq!(g.step(dt, c, 20.0, false, false, false, false), GasStep::RefillOne);
    }
}
