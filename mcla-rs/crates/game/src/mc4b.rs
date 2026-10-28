//! Step 23b: the open list of steps 22b and 23 - the statistics the port did not count (the
//! distance driven, on two wheels, in a wheelie, the jumps), the race goals of the race-end
//! statistics (aAOR / aAAR / aARC), the showroom's test drive (tTST, aTDV), the photo album's
//! slots (tPTS, aFAB) and the audio's slow-motion switch during the tutorials.
//!
//! The original (scratch `work/save/port/s23b/`):
//! - The odometer (F-53047, once a frame for the player's vehicle): speed = |(vx, vz)| x
//!   2.237 (mph); +20 += speed x the frame time x 1/3600 - miles; flushed into gkDS (add) by
//!   F-15445 (with vkDT for the vehicle). aDST: gkDS >= 5000.
//! - Two wheels (mcCarGyro F-10637 A-68829..A-14840): while the gyro's lean state
//!   (+18) is on and the tuning has Lean (> 0), for the player: gk2W += speed (+224, m/s) x
//!   2.237 x frame time / 3600. a2WD: gk2W >= 50.
//! - Wheelie (mcBikeGyro F-63842 A-50974..A-48212): while the stunt state (+16) is
//!   above 0 (1 up, 99 coming down): gkWL += the same. aWHL: gkWL >= 50.
//! - Jumps (F-97237 A-55250..A-91720): no wheel down - the air time (+584) grows by
//!   the frame time; a wheel down with air time > 0 - for the player's vehicle gsJT += the air
//!   time, gmJD = max(gmJD, the flat distance from where it left the ground (+640) to here),
//!   then air time = -air time; a wheel down with air time <= 0 - air time 0, the take-off
//!   point = here. aJTM: gsJT >= 600; aJDS: gmJD >= 310.896.
//! - The race-end statistics (F-98556 A-15849..A-93208 -> F-96807): won (place
//!   1), the time under the race's GoalTargetTime x 1000 ms (the city table's +136), the damage
//!   under a quarter of MaxDamage ((max - damage) > 0.75 max): rWIN / rBTT / rMDG of the race (max
//!   1); all three: rCGL of the race, aAOR; then, by the race's content (+144: ContentID, else 4
//!   for South Central, 1 otherwise), bit 0: aAAR once every Ordered / Circuit / Unordered / Blitz
//!   race (sub types 1, 2, 3, 5) not DevOnly of content bit 0 has rCGL; bit 2: aARC the same for
//!   content 4. (The races' GoalTargetTime / TargetDamage are those of the GOAL ATTACK mode
//!   - ArcadeMenuGoalAttack, pause MODES' GOAL ATTACK; the check runs only for a race started
//!   from it: F-98556 A-77758 tests the mode object's +796 - `goal_attack`.)
//! - The showroom's TEST DRIVE (F-22067): tTST + 1; the vehicle table's +48
//!   (ContentDownloadFlags, else ContentFlags - `tune/vehicle` lists) >= 4: aTDV (set once).
//! - The photo album (F-67304): a picture saved into slot k of the album (16 slots,
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
    /// set down (`CarInstance::placed`)
    pub placed: bool,
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
        // (mcBikeGyro's stunt state is `bg.state`; `wheelie` is the older bike path's)
        wheelie: c.model.bike && vs.map_or(false, |s| s.bg.state > 0 || s.wheelie == 1 || s.wheelie == 99),
        airborne: c.state.wheels.iter().take(if c.model.bike { 2 } else { 4 }).all(|w| !w.grounded),
        pos: c.state.pos,
        placed: c.placed,
    }
}

/// What the running game keeps for step 23b.
#[derive(Clone, Debug, Default)]
pub struct S23bLive {
    /// the jump's air time (+584; negative for the frame after a landing) and take-off point (+640)
    pub air: f32,
    pub takeoff: Option<Vec3>,
    /// the player's place last frame (a move the driving cannot make is the game's)
    pub last_pos: Option<Vec3>,
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

/// The content mask (0x827E9174) of the running game: 317, what startup adds (F-68286)
/// with no package downloaded (tDL2 = 317 in every sample).
pub const CONTENT_MASK: u32 = 317;

/// The vehicle table's ContentFlags (+52; none read as 0, in the mask) and PortalRewardIdx
/// (+56; -1 none) of a vehicle (parser F-87336).
pub fn vehicle_content(data_dir: &str, car: &str) -> (u32, i32) {
    static TABLE: std::sync::OnceLock<std::collections::HashMap<String, (u32, i32)>> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut out = std::collections::HashMap::new();
        for f in ["0xe94bde63", "0xccb5daee", "0x3cb91343"] {
            let Ok(s) = std::fs::read(std::path::Path::new(data_dir).join("tune/vehicle").join(f)) else { continue };
            let s = String::from_utf8_lossy(&s);
            for block in s.split("Vehicle").skip(1) {
                let field = |k: &str| block.lines().map(|l| l.trim()).find_map(|l| l.strip_prefix(k).filter(|r| r.starts_with(|c: char| c.is_whitespace())).map(|r| r.trim().to_string()));
                let Some(name) = field("Name") else { continue };
                let flags = field("ContentFlags").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                let portal = field("PortalRewardIdx").and_then(|v| v.parse::<i32>().ok()).unwrap_or(-1);
                out.insert(name.to_ascii_lowercase(), (flags, portal));
            }
        }
        out
    });
    t.get(&car.to_ascii_lowercase()).copied().unwrap_or((0, -1))
}

impl Game {
    /// A vehicle the game offers (F-87177): its ContentFlags in the content mask, else a
    /// portal reward the career holds - the statistic `tREW` of its index above 0 (earned on
    /// line, F-53037). Only `vp_aud_r8_08` (ContentFlags 2, index 0) is outside the mask.
    pub fn mc4_vehicle_available(&self, car: &str) -> bool {
        let (flags, portal) = vehicle_content(&self.data_dir, car);
        // (the port has no online play: a portal reward is offered like any other car -
        // docs/DECISIONS.md, 2026-10-22; the original also asks `self.mc4_stat("tREW", portal) > 0`)
        flags == 0 || flags & CONTENT_MASK != 0 || portal >= 0
    }
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
    /// The SERVICE STATION menu comes up (mcGasStationMenuForm open, F-78208): the cursor
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
/// Unordered, Blitz (1, 2, 3, 5 of F-93280's table 0x827E930C).
pub const GOAL_SUB_TYPES: [&str; 4] = ["Ordered", "Circuit", "Unordered", "Blitz"];

/// A race's content (+144): ContentID, else 4 for South Central, 1 otherwise (F-95490
/// A-45474). No race file on the disc has a ContentID.
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
/// (test runs: MCLA_ALBUM_DIR puts the album elsewhere)
pub fn album_dir(data_dir: &str) -> std::path::PathBuf {
    if let Ok(d) = std::env::var("MCLA_ALBUM_DIR") {
        return std::path::PathBuf::from(d);
    }
    std::path::Path::new(data_dir).join("../photos/album")
}

pub const ALBUM_SLOTS: usize = 16;

impl Game {
    /// Once a frame from the step 23 tick: the driving statistics, the test drive.
    pub fn mc4b_tick(&mut self, dt: f32, d: S23Drive) {
        // ---- the showroom's test drive (F-22067) ----
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
        // (the flush F-15445 adds the same distance to the vehicle's own `vkDT` - the sub id is the vehicle table's index, kept
        // here under the vehicle's name like the importer's keys - sets the top speed `gvTS` (operation 2, the larger) once
        // the speed is over 10 mph, and adds the hours: `ghTP` always, `khTC` and `ghTC` while the game is in "Cruising"
        // (free roam). The hours' clock is taken as the game's own, stopped in the pause and slowed by slow motion
        // (the clock object's +20: not checked); the first flush's hours are not decoded.)
        let car = self.car_dirs.get(self.current_car).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_ascii_lowercase());
        if let Some(car) = car {
            *self.career.ostats.entry(format!("vkDT/{car}")).or_insert(0.0) += miles(d.vel_xz);
        }
        let mph = (d.speed * MPH) as f64;
        if mph > 10.0 {
            let was = self.mc4_stat("gvTS", 0);
            self.mc4_stat_set("gvTS", 0, was.max(mph));
        }
        let hours = (sim * PER_HOUR) as f64;
        self.mc4_stat_add("ghTP", 0, hours);
        if self.mode == Mode::FreeRoam {
            self.mc4_stat_add("khTC", 0, hours);
            self.mc4_stat_add("ghTC", 0, hours);
        }
        if d.two {
            self.mc4_stat_add("gk2W", 0, miles(d.speed));
        }
        if d.wheelie {
            self.mc4_stat_add("gkWL", 0, miles(d.speed));
        }
        let x = &mut self.opt_x.s23b;
        // (the port's own: a car the game moves - set down 3 cm up, put on a grid, warped - is
        // no jump; the original sets them down on the ground and only a wheel down writes the
        // take-off point. So no air and no take-off point across such a move.)
        let warped = x.last_pos.map_or(true, |p| p.distance(d.pos) > d.speed * sim * 2.0 + 2.0);
        x.last_pos = Some(d.pos);
        if !d.placed || warped {
            x.air = 0.0;
            x.takeoff = None;
            return;
        }
        if d.airborne && x.takeoff.is_none() {
            return;
        }
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

    /// F-96807 from the race-end statistics: the race's goals (won, under its goal time,
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

    /// A race's goals as GOAL ATTACK's list shows them (F-33135): the mask "0".."15" from
    /// the save's rWIN 8, rMDG 4, rBTT 2, rCGL 1 of the race's table index.
    pub fn mc4_goal_mask(&self, race: usize) -> u8 {
        let Some(k) = self.race_defs.get(race).and_then(|d| self.mc4_race_index(&d.id)) else { return 0 };
        [("rWIN", 8), ("rMDG", 4), ("rBTT", 2), ("rCGL", 1)].iter().filter(|(id, _)| self.mc4_stat(id, k as u32) != 0.0).map(|(_, b)| b).sum()
    }

    /// The goal panel's four lines for a race (columns 2..5 of the list: GOALATTACK_WIN,
    /// GOALATTACK_DAMAGE, GOAL_TIME with GOAL_TIME_ENTRY = the race's GoalTargetTime by
    /// F-26034 - set on a selection change, F-28168 -, GOALATTACK_ALLGOALS).
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

    /// The race start badge's second line in GOAL ATTACK (GetRaceInfo F-24355
    /// A-69161..A-73729) and in a time trial (event kind 2, the jump table A-51741 ->
    /// A-63596: the race's target time, F-89467): RACE_INFO_GOALATTACK_FORMAT
    /// "TARGET TIME: <D>RACE_INFO_GOAL_TIME</D>" (work/hookmen2 `cap/tt_07`).
    pub fn mc4_goal_badge(&self) -> Option<String> {
        let d = self.current_race.and_then(|i| self.race_defs.get(i))?;
        let ms = if self.goal_attack {
            (d.goal_time * 1000.0).round() as i32
        } else {
            (self.mc4_tt_run(&d.id)?.2 as f64 * 1000.0).round() as i32
        };
        let f = self.career.strings.get("RACE_INFO_GOALATTACK_FORMAT").map(|v| v.to_string()).unwrap_or_else(|| "TARGET TIME: <D>RACE_INFO_GOAL_TIME</D>".to_string());
        Some(fill_d(&f, &crate::flash_ui::race_time(ms)))
    }

    /// A picture taken in photo mode, written at once into the slot the album's chooser opens on
    /// (F-93904: the first free one, slot 0 when all sixteen are used) - the test hook
    /// MCLA_PHOTO_TEST=album; the game's own flow chooses the slot (`photoalbum`).
    pub fn mc4b_photo_path(&mut self) -> String {
        let dir = album_dir(&self.data_dir);
        let used: Vec<bool> = (0..ALBUM_SLOTS).map(|k| dir.join(format!("slot_{k:02}.png")).exists()).collect();
        let slot = used.iter().position(|u| !u).unwrap_or(0);
        self.mc4b_photo_save(slot)
    }

    /// A picture saved into slot k of the album (F-67304): a slot not used before raises
    /// the count of used slots (+856), aFAB as it reaches 16; tPTS + 1 for every picture, an
    /// overwrite too. The file to write.
    pub fn mc4b_photo_save(&mut self, slot: usize) -> String {
        let dir = album_dir(&self.data_dir);
        let _ = std::fs::create_dir_all(&dir);
        let used: Vec<bool> = (0..ALBUM_SLOTS).map(|k| dir.join(format!("slot_{k:02}.png")).exists()).collect();
        let slot = slot.min(ALBUM_SLOTS - 1);
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
        // (SST_Tutorial: SetActiveTimeSlow(1) at 0xe9 before the ease, 0 at 0x22f once form 78 is closed)
        x.run.as_ref().map_or(false, |r| r.slow && r.phase >= 1) || x.lesson == 1 || matches!(x.sst_tut, crate::mc4::SstTut::Ease(_) | crate::mc4::SstTut::Box78)
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

    /// F-87177 on the disc's vehicle table: the R8 alone is outside the mask (ContentFlags 2,
    /// PortalRewardIdx 0) and comes with tREW/0 - which the port, without online play, offers
    /// anyway (docs/DECISIONS.md); the base cars, the DUB cars and the cut GT-R are offered.
    #[test]
    fn vehicles_outside_the_content_mask() {
        let dir = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        if !std::path::Path::new(&dir).join("tune/vehicle/0xe94bde63").exists() {
            return;
        }
        assert_eq!(vehicle_content(&dir, "vp_aud_r8_08"), (2, 0));
        let mut g = Game::new(Vec::new(), 0);
        g.data_dir = dir;
        assert!(g.mc4_vehicle_available("vp_aud_r8_08"));
        assert!(g.mc4_vehicle_available(crate::catalog::CUT_GTR));
        assert!(g.mc4_vehicle_available("vp_am_vantage_07"));
        assert!(g.mc4_vehicle_available("vp_lam_murcielago_06_dub"));
    }
}
