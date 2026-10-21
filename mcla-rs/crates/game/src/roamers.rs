//! Roaming racers: the hookmen cruising the districts in free roam. Pull up behind one and
//! flash the headlights: a local hookman races to the start line of an event, where the
//! event itself begins; the time trial's and the tournament's man open their panels.
//!
//! Who cruises is the career's slot system (`mc4::SlotSystem`, step 19b): three slots a
//! district (`tune/career/0xc72d4489`), each with the script of its loop (`script_obj/*/
//! CruisingHookmen/<district>_slotN`: a ring of five to ten junctions); a mission asks for a
//! slot for its hookman, the free slots of an unlocked district are filled with generic
//! local hookmen - one Easy, one Medium, one Hard for the player's vehicle. Beside them a
//! district has its time trial man and its tournament man on loops of their own
//! (`<d>_time_trial_hk1`, `<d>_tournament_hk1`).

use crate::game::Game;
use crate::mc4::{SlotSystem, DISTRICTS, DISTRICT_CODES, MISSION_HOOKMEN, PRIO_MISSION};
use crate::render::Gpu;
use crate::vehicles::{Vehicles, PALETTE};
use crate::world::World;
use glam::Vec3;
use hashbrown::HashMap;

pub struct Challenge {
    pub race: usize,
    pub goal: Vec3,
    pub time: f32,
    /// once an opponent is at the line: the thirty seconds the others have (`mc4`)
    pub close: crate::mc4::BmtClose,
    /// distance the player had to cover at the start (m)
    pub dist: f32,
    /// the hookman's difficulty (his slot's), when he is no mission's man
    pub diff: Option<usize>,
    /// the three races the hookman keeps (race table places): the one he goes to first
    pub races: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ManKind {
    /// a hookman of a cruising slot: a mission's or a generic local one
    Slot,
    /// hookman type 3, ids 250..254
    TimeTrial,
    /// hookman type 4, ids 200..
    Tournament,
    /// hookman type 2, ids 350..354: a district's red light racer (`althk.rs`)
    RedLight,
    /// hookman type 5, ids 300..307: a freeway racer (`althk.rs`)
    Freeway,
}

/// An alternative hookman's draw: who he is, his vehicle and how hard (his script's
/// mcHookman_SetName / SetDifficulty, mcCharacterConfigData_GetCarConfigData).
#[derive(Clone, Debug)]
struct AltRoll {
    name: String,
    car: String,
    config: String,
    mod_value: i32,
    difficulty: i32,
    mission: bool,
}

/// A hookman on his loop. While his car is not in the world he moves along the loop as a
/// place only.
#[derive(Clone, Debug)]
pub struct Man {
    pub key: String,
    pub kind: ManKind,
    pub district: usize,
    /// the character ("GH_23")
    pub name: String,
    pub car: String,
    pub config: String,
    /// 0 Easy .. 3 Impossible (the icon's ring)
    pub difficulty: i32,
    /// the mission whose man he is
    pub mission: String,
    /// the tournament's event key
    pub event: String,
    pub hidden: bool,
    pub pts: Vec<Vec3>,
    /// metres along the loop from its first point
    pub at: f32,
    /// step 19d: his vehicle's mod value (the slot system's: 0..100), -1 = his set-up's own
    pub mod_value: i32,
    /// an alternative hookman's id (mcHookman_SetId: 300.., 350..), 0 for the others
    pub uid: u32,
}

/// A hookman script of a district's special man: (kind, district, id, character, set-up, loop).
pub struct Special {
    pub kind: ManKind,
    pub district: usize,
    pub id: u32,
    pub name: String,
    pub config: String,
    pub pts: Vec<Vec3>,
}

pub struct Roamers {
    pub wanted: usize,
    timer: f32,
    load_timer: f32,
    sync_timer: f32,
    rng: u32,
    pub challenge: Option<Challenge>,
    /// a roamer close enough to challenge: its name, for the prompt
    pub near: Option<String>,
    /// set for one frame when a challenge has just been accepted
    pub accepted: bool,
    /// the roamer in reach is the hookman the career asks for
    pub near_is_rival: bool,
    /// (step 23b) the type of the hookman in reach (mcHookman_GetType: 1 a slot's man, 2 red
    /// light, 3 time trial, 4 tournament, 5 freeway; 0 none or not one of the men)
    pub near_type: u8,
    /// the challenge that just ended was against that hookman
    pub won_against_rival: bool,
    rival_challenged: bool,
    /// cruising loops of the hookmen (junction to junction)
    pub slots: Vec<Vec<Vec3>>,
    /// the loops by the name of their script ("hw_slot2")
    pub loops: HashMap<String, Vec<Vec3>>,
    /// the time trial and tournament men's scripts
    pub specials: Vec<Special>,
    /// the career's slot system
    pub sys: SlotSystem,
    /// everybody who is cruising now
    pub men: Vec<Man>,
    evaluated: bool,
    last_car: String,
    /// (difficulty, loop) - kept for the callers of the first version
    pub cruisers: Vec<(usize, Vec<Vec3>)>,
    /// the red light and freeway racers' scripts (`althk.rs`)
    pub alt: crate::althk::AltData,
    alt_roll: HashMap<u32, AltRoll>,
    /// the men drawn at least once (a red light man's first draw is Easy)
    alt_drawn: Vec<u32>,
    /// an alternative hookman has just been flashed: his race is for the caller to start
    pub alt_flash: Option<Man>,
    alt_car: String,
    /// the clock at the last update: a gap is a race or a scene - Script_SuspendAllOpponents
    alt_seen: f64,
    alt_suspended: bool,
}

fn is_racer_model(n: &str) -> bool {
    !n.starts_with("va_") && !n.contains("police") && !n.contains("citybus")
}

fn loop_len(pts: &[Vec3]) -> f32 {
    (0..pts.len()).map(|i| pts[i].distance(pts[(i + 1) % pts.len()])).sum()
}

/// The place `at` metres along the loop, the direction there and the junction ahead.
pub fn loop_at(pts: &[Vec3], at: f32) -> (Vec3, Vec3, usize) {
    if pts.is_empty() {
        return (Vec3::ZERO, Vec3::NEG_Z, 0);
    }
    let len = loop_len(pts).max(1.0);
    let mut at = at.rem_euclid(len);
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        let l = a.distance(b);
        if at <= l {
            return (a.lerp(b, at / l.max(1e-3)), (b - a).normalize_or(Vec3::NEG_Z), (i + 1) % pts.len());
        }
        at -= l;
    }
    (pts[0], Vec3::NEG_Z, 1 % pts.len())
}

/// How far along the loop the point of it nearest to `p` is.
pub fn loop_project(pts: &[Vec3], p: Vec3) -> f32 {
    let mut best = (f32::MAX, 0.0);
    let mut run = 0.0;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        let ab = b - a;
        let l = ab.length();
        let t = if l > 1e-3 { ((p - a).dot(ab) / (l * l)).clamp(0.0, 1.0) } else { 0.0 };
        let d = (a + ab * t).distance(p);
        if d < best.0 {
            best = (d, run + l * t);
        }
        run += l;
    }
    best.1
}

/// The junctions of a hookman script: stored one after the other as (x, y, z, 3, index, 3, ...).
fn script_points(t: &[mcla_formats::sco::Tok]) -> Vec<Vec3> {
    use mcla_formats::sco::Tok;
    let num = |i: usize| if let Some(Tok::Num(v)) = t.get(i) { Some(*v as f32) } else { None };
    let mut pts: Vec<Vec3> = Vec::new();
    for i in 0..t.len() {
        if let (Some(x), Some(y), Some(z), Some(a), Some(k), Some(b)) = (num(i), num(i + 1), num(i + 2), num(i + 3), num(i + 4), num(i + 5)) {
            if a == 3.0 && b == 3.0 && k == pts.len() as f32 && (x.abs() > 2.0 || z.abs() > 2.0) {
                pts.push(Vec3::new(x, y, z));
            }
        }
    }
    pts
}

impl Roamers {
    pub fn new() -> Roamers {
        Roamers {
            wanted: 2,
            timer: 3.0,
            load_timer: 1.0,
            sync_timer: 0.0,
            rng: 0x51ed_270b,
            challenge: None,
            near: None,
            accepted: false,
            near_is_rival: false,
            near_type: 0,
            won_against_rival: false,
            rival_challenged: false,
            slots: Vec::new(),
            loops: HashMap::new(),
            specials: Vec::new(),
            sys: SlotSystem::default(),
            men: Vec::new(),
            evaluated: false,
            last_car: String::new(),
            cruisers: Vec::new(),
            alt: Default::default(),
            alt_roll: HashMap::new(),
            alt_drawn: Vec::new(),
            alt_flash: None,
            alt_car: String::new(),
            alt_seen: 0.0,
            alt_suspended: false,
        }
    }

    /// Read the cruising loops, the special men's scripts and the slot system's tables.
    pub fn load_slots(&mut self, data_dir: &str) {
        use mcla_formats::sco::Tok;
        for sub in ["script_obj/game/CruisingHookmen", "script_obj/sc/CruisingHookmen"] {
            let Ok(rd) = std::fs::read_dir(std::path::Path::new(data_dir).join(sub)) else { continue };
            let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            files.sort();
            for f in files {
                // (the slot scripts are small; the big ones are the meeting logic)
                if std::fs::metadata(&f).map_or(true, |m| m.len() > 1500) {
                    continue;
                }
                let Some(sco) = std::fs::read(&f).ok().and_then(|d| mcla_formats::sco::decode(&d).ok()) else { continue };
                let t = sco.consts();
                let pts = script_points(&t);
                let stem = f.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                if pts.len() >= 3 && stem.contains("_slot") && !stem.starts_with("0x") {
                    let d = self.cruisers.len() % 3;
                    self.cruisers.push((d, pts.clone()));
                }
                if pts.len() >= 3 {
                    self.loops.insert(stem.clone(), pts.clone());
                }
                // (`<d>_time_trial_hk1`, `<d>_tournament_hk1`: mcHookman_SetId / SetType /
                // SetName / SetCarConfigName and the patrol points)
                for (d, code) in DISTRICT_CODES.iter().enumerate() {
                    for (kind, tail, id) in [(ManKind::TimeTrial, "_time_trial_hk1", 250 + d as u32), (ManKind::Tournament, "_tournament_hk1", 200 + d as u32)] {
                        if stem != format!("0x{:08x}", crate::mc4::joaat(&format!("{code}{tail}.sco"))) || pts.len() < 3 {
                            continue;
                        }
                        let strs: Vec<&String> = t.iter().filter_map(|x| if let Tok::Str(s) = x { Some(s) } else { None }).collect();
                        let config = strs.iter().find(|s| s.to_ascii_lowercase().contains("_vp")).map(|s| s.to_string()).unwrap_or_default();
                        let name = strs.iter().find(|s| (s.starts_with("GH_") || s.starts_with("SH_")) && !s.to_ascii_lowercase().contains("_vp")).map(|s| s.to_string()).unwrap_or_default();
                        self.specials.push(Special { kind, district: d, id, name, config, pts: pts.clone() });
                    }
                }
                if pts.len() >= 3 && !self.slots.iter().any(|s| s[0].distance(pts[0]) < 1.0) {
                    self.slots.push(pts);
                }
            }
        }
        self.alt = crate::althk::AltData::load(data_dir);
        self.sys = SlotSystem::load(data_dir);
        log::info!("roamers: {} cruising loops, {} special hookmen, {} slots, {} characters", self.slots.len(), self.specials.len(), self.sys.slots.iter().map(|s| s.len()).sum::<usize>(), self.sys.characters.iter().map(|c| c.len()).sum::<usize>());
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng % 10000) as f32 / 10000.0
    }

    pub fn cancel(&mut self, vehicles: &mut Vehicles) {
        if self.challenge.take().is_some() {
            for c in vehicles.cars.iter_mut().filter(|c| c.racer) {
                if let Some(ai) = c.ai.as_mut() {
                    ai.path = None;
                    ai.target_speed = 22.0;
                }
            }
        }
    }

    /// The seconds left of the thirty the others have once an opponent is at the line.
    pub fn countdown(&self) -> Option<f32> {
        self.challenge.as_ref().and_then(|c| c.close.since).map(|s| (30.0 - s).max(0.0))
    }

    /// The slot system follows the career (the mission scripts' mcSlotSystem_RequestSlot -
    /// retried every second until a slot is had -, the core scripts' RequestSlotRemoval with
    /// evaluation when the mission is over, mcSlotSystem_UnlockDistrict's evaluation, and the
    /// garage script's ClearAllDistricts / EvaluateAllDistricts - taken here as "the player
    /// has another vehicle"), and the list of cruising men is made from it.
    pub fn sync(&mut self, game: &mut Game, player_car: &str) {
        if self.sys.slots.is_empty() {
            return;
        }
        let perf = game.player_perf();
        let mut eval: Vec<usize> = Vec::new();
        for d in 0..5 {
            let locked = !game.mc4_district_open(DISTRICTS[d]);
            if self.sys.locked[d] && !locked {
                eval.push(d);
            }
            self.sys.locked[d] = locked;
            for i in 0..self.sys.slots[d].len() {
                self.sys.slots[d][i].hidden = game.mc4_slot_hidden(d, i);
            }
        }
        // the missions' men
        let rows: Vec<_> = MISSION_HOOKMEN.iter().collect();
        for row in rows {
            let active = game.career.mission_active(row.0);
            let d = row.1;
            let held = self.sys.slots.get(d).map_or(false, |l| l.iter().any(|s| s.hookman.as_ref().map_or(false, |h| h.mission == row.0 && (row.2.is_empty() || h.name == row.2))));
            if active && !held {
                let mut h = game.mc4_mission_hookman(row);
                if row.2.is_empty() {
                    // (REPEAT_BESTOF_<D>: mcSlotSystem_FindRandomAvailableCharacter(d) and a
                    // set-up of his)
                    let Some(name) = self.sys.random_character(d) else { continue };
                    let Some((car, config, m)) = game.mc4_character_car(&name, row.5) else { continue };
                    h.difficulty = match crate::mc4::gap_difficulty(game.mc4_samples_of(&car).map_or(0.0, |s| crate::mc4::setup_perf(s, m)) - perf) { x if x <= 0 => 1, 5 => 3, x => x };
                    (h.name, h.car, h.config, h.mod_value) = (name, car, config, m);
                }
                if self.sys.request(h, PRIO_MISSION) {
                    log::info!("slots: {} gets a slot of {} for {}", row.2, DISTRICTS[d], row.0);
                }
            } else if !active && held {
                while let Some(d) = self.sys.remove_mission(row.0) {
                    log::info!("slots: the man of {} leaves {}", row.0, DISTRICTS[d]);
                    eval.push(d);
                }
            }
        }
        let car = player_car.to_string();
        if !self.evaluated || car != self.last_car {
            if self.evaluated {
                self.sys.clear_all();
            }
            self.evaluated = true;
            self.last_car = car;
            eval = (0..5).collect();
        }
        eval.sort();
        eval.dedup();
        for d in eval {
            let g: &Game = game;
            self.sys.evaluate(d, perf, &mut |name, diff| g.mc4_character_car(name, diff));
        }
        // who is out
        let mut men: Vec<Man> = Vec::new();
        for d in 0..self.sys.slots.len() {
            for (i, s) in self.sys.slots[d].iter().enumerate() {
                let Some(h) = s.hookman.as_ref() else { continue };
                let stem = s.script.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
                let Some(pts) = self.loops.get(&stem) else { continue };
                men.push(Man { key: format!("slot:{d}:{i}"), kind: ManKind::Slot, district: d, name: h.name.clone(), car: h.car.clone(), config: h.config.clone(), difficulty: h.difficulty, mission: h.mission.clone(), event: String::new(), hidden: s.hidden && h.mission.is_empty(), pts: pts.clone(), at: 0.0, mod_value: h.mod_value, uid: 0 });
            }
        }
        for sp in &self.specials {
            let (on, event) = match sp.kind {
                ManKind::TimeTrial => (game.mc4_tt_open(sp.district), String::new()),
                _ => { let e = game.mc4_tournament_man(sp.district); (e.is_some(), e.unwrap_or_default()) }
            };
            if !on {
                continue;
            }
            let low = sp.config.to_ascii_lowercase();
            let car = low.find("_vp_").or_else(|| low.find("_vpd_")).map(|i| low[i + 1..].to_string()).unwrap_or_default();
            // (generic_timetrial_hookman.sco 0x137 / 0x167: difficulty 1; generic_tournament_hookman.sco
            // 0x14d / 0x17e: 2)
            let diff = if sp.kind == ManKind::TimeTrial { 1 } else { 2 };
            men.push(Man { key: format!("{}:{}", if sp.kind == ManKind::TimeTrial { "tt" } else { "tour" }, sp.district), kind: sp.kind, district: sp.district, name: sp.name.clone(), car, config: sp.config.clone(), difficulty: diff, mission: String::new(), event, hidden: false, pts: sp.pts.clone(), at: 0.0, mod_value: -1, uid: 0 });
        }
        // the red light and freeway racers whose scripts run
        self.alt_men(game, player_car, &mut men);
        for m in men.iter_mut() {
            // (a man who stays keeps his place; a new one starts at the first point of the
            // loop - Racer_InitializeResetPosition with patrol point 0)
            if let Some(o) = self.men.iter().find(|o| o.key == m.key && o.config == m.config) {
                m.at = o.at;
            }
        }
        if men.len() != self.men.len() || men.iter().zip(self.men.iter()).any(|(a, b)| a.key != b.key || a.config != b.config) {
            for m in &men {
                log::info!("slots: {} {} ({}{}) cruises {} in his {}, difficulty {}{}", m.key, m.name, m.config, if m.mission.is_empty() { String::new() } else { format!(", {}", m.mission) }, DISTRICTS[m.district.min(4)], m.car, m.difficulty, if m.hidden { ", hidden" } else { "" });
            }
        }
        self.men = men;
    }

    /// The alternative hookmen whose scripts the career runs (`Game::alt_running`), each on the
    /// loop of his script. Who he is, his vehicle and difficulty are drawn when his script
    /// starts and again when the player's vehicle type changes or the opponents were
    /// suspended (a race, a mission): the red light man's script 0x3c68..0x3e4e - a new
    /// difficulty RAND_INT_RANGE(0, 3), one of the district's racers, his set-up for it -, the
    /// freeway man's 0xced..0xea7 (difficulty and set-up, his name stays). At the start the red
    /// light man is Easy (mcHookman_SetDifficulty(h, 0) in the script's main), the freeway man
    /// already drawn. Hollywood's red light man while REDLIGHT_GH_01 is not complete is GH_01
    /// in his `vp_vw_scirocco_88` (0x35e0..0x365a) and is not drawn again. He is not shown on
    /// the maps until he is visible (`Game::alt_visible`); hidden, he still drives.
    fn alt_men(&mut self, game: &Game, player_car: &str, men: &mut Vec<Man>) {
        if self.alt_car != player_car || self.alt_suspended {
            let keep_mission = !self.alt_car.is_empty() && self.alt_car == player_car;
            self.alt_roll.retain(|_, r| keep_mission && r.mission);
            self.alt_car = player_car.to_string();
            self.alt_suspended = false;
        }
        let defs = self.alt.defs.clone();
        for d in defs {
            if !game.alt_running(&d.script) {
                continue;
            }
            let mission = d.id == 350 && game.career.mission_active("REDLIGHT_GH_01");
            if self.alt_roll.get(&d.id).map_or(false, |r| r.mission != mission) {
                self.alt_roll.remove(&d.id);
            }
            if !self.alt_roll.contains_key(&d.id) {
                let first = !self.alt_drawn.contains(&d.id);
                let roll = if mission {
                    game.alt_character_car_by_vehicle("GH_01", "vp_vw_scirocco_88", 0).map(|(car, config, m)| AltRoll { name: "GH_01".into(), car, config, mod_value: m, difficulty: 0, mission: true })
                } else {
                    // RAND_INT_RANGE is inclusive (the freeway men's red rings - Impossible - in
                    // the 100 % save's map, `gps_whole`)
                    let diff = if d.kind == crate::althk::AltKind::RedLight && first { 0 } else { ((self.rand() * 4.0) as i32).min(3) };
                    let name = if d.names.is_empty() { None } else { Some(d.names[((self.rand() * d.names.len() as f32) as usize).min(d.names.len() - 1)].clone()) };
                    name.and_then(|n| game.mc4_character_car(&n, diff).map(|(car, config, m)| AltRoll { name: n, car, config, mod_value: m, difficulty: diff, mission: false }))
                };
                match roll {
                    Some(r) => {
                        log::info!("alt hookman {} ({}): {} in his {} ({}), difficulty {}", d.id, d.script, r.name, r.car, r.config, r.difficulty);
                        self.alt_roll.insert(d.id, r);
                        self.alt_drawn.push(d.id);
                    }
                    None => continue,
                }
            }
            let Some(r) = self.alt_roll.get(&d.id) else { continue };
            let kind = if d.kind == crate::althk::AltKind::RedLight { ManKind::RedLight } else { ManKind::Freeway };
            let key = format!("{}:{}", if kind == ManKind::RedLight { "rl" } else { "fw" }, d.id);
            men.push(Man { key, kind, district: d.district, name: r.name.clone(), car: r.car.clone(), config: r.config.clone(), difficulty: r.difficulty, mission: if r.mission { "REDLIGHT_GH_01".into() } else { String::new() }, event: String::new(), hidden: !game.alt_visible(d.id), pts: d.pts.clone(), at: 0.0, mod_value: r.mod_value, uid: d.id });
        }
    }

    /// The cruising men for the maps: place, direction, difficulty.
    pub fn marks(&self, vehicles: &Vehicles) -> Vec<(Vec3, Vec3, usize)> {
        self.men
            .iter()
            .filter(|m| !m.hidden && m.kind == ManKind::Slot)
            .map(|m| match vehicles.cars.iter().find(|c| c.racer && c.vinyl_name == m.config) {
                Some(c) => (c.state.pos, c.state.forward(), m.difficulty.clamp(0, 3) as usize),
                None => { let (p, d, _) = loop_at(&m.pts, m.at); (p, d, m.difficulty.clamp(0, 3) as usize) }
            })
            .collect()
    }

    /// Returns the race to start when a run to the start line has just ended.
    pub fn update(&mut self, game: &mut Game, vehicles: &mut Vehicles, gpu: &mut Gpu, world: &mut World, flash: bool, dt: f32) -> Option<usize> {
        self.near = None;
        self.accepted = false;
        self.near_type = 0;
        if flash && std::env::var("MCLA_ALT_LOG").is_ok() {
            log::info!("roamers: update with a flash ({} men, challenge {})", self.men.len(), self.challenge.is_some());
        }
        let Some(player) = vehicles.player() else { return None };
        let (pp, pf, pname, pspeed) = (player.state.pos, player.state.forward(), player.model.name.clone(), player.state.speed());
        // (a gap in the cruise - a race, a scene, a menu - is the opponents' suspension: the
        // alternative hookmen draw again, `alt_men`)
        let s_clock = vehicles.clock as f64;
        if s_clock - self.alt_seen > 2.0 {
            self.alt_suspended = true;
        }
        self.alt_seen = s_clock;
        self.sync_timer -= dt;
        if self.sync_timer <= 0.0 {
            self.sync_timer = 1.0;
            self.sync(game, &pname);
        }
        // (test hooks: MCLA_HOOKMAN_TEST=<key prefix> - `tt:0`, `tour`, `slot:0:1` - puts that man
        // near the player; the self-test MCLA_ROAM_TEST alone takes a slot hookman)
        let only = std::env::var("MCLA_HOOKMAN_TEST").ok().or_else(|| std::env::var("MCLA_ROAM_TEST").ok().map(|_| "slot".to_string()));
        // ---- the men go round their loops; those near the player are cars ----
        // (step 19d: a cruising brain that was given no speed drives at its rail's own -
        // sub_82745EE0: the road's, `mc4::RAIL_SPEED`)
        const CRUISE: f32 = crate::mc4::RAIL_SPEED;
        self.timer -= dt;
        self.load_timer -= dt;
        let mut spawn: Option<usize> = None;
        for (k, m) in self.men.iter_mut().enumerate() {
            match vehicles.cars.iter().position(|c| c.racer && c.vinyl_name == m.config) {
                Some(ci) => {
                    let cp = vehicles.cars[ci].state.pos;
                    m.at = loop_project(&m.pts, cp);
                    // (a hidden hookman is found within 80 m: mcHookman_Hide(h, 0), and the core
                    // script then shows his slot - mcSlotSystem_UnlockSlot; CruisingHookmanCore
                    // 0x389..0x3d3, 0x7ad..0x8aa)
                    if m.hidden && cp.distance(pp) < 80.0 && m.uid != 0 {
                        // (CruisingHookmanCore 0x7ad..0x8aa: type 5 "HOOKMAN_FOUND_FREEWAY", type
                        // 2 "HOOKMAN_FOUND_REDLIGHT", mcHookman_Hide(h, 0); his script then marks
                        // him visible - freeway_hookman1.sco 0xeb1, the red light man's 0x3e99)
                        m.hidden = false;
                        let key = if m.kind == ManKind::Freeway { "HOOKMAN_FOUND_FREEWAY" } else { "HOOKMAN_FOUND_REDLIGHT" };
                        if let Some(t) = game.career.strings.get(key).map(|x| x.to_uppercase()) {
                            game.show_message(&t);
                        }
                        game.alt_set_visible(m.uid);
                        log::info!("alt hookman {} found ({:.0} m)", m.uid, cp.distance(pp));
                    }
                    if m.hidden && cp.distance(pp) < 80.0 {
                        m.hidden = false;
                        if let Some(i) = m.key.rsplit(':').next().and_then(|v| v.parse::<usize>().ok()) {
                            self.sys.unlock_slot(m.district, &m.name);
                            game.mc4_slot_found(m.district, i);
                        }
                    }
                    if cp.distance(pp) > 420.0 && self.challenge.is_none() {
                        vehicles.cars.remove(ci);
                    }
                }
                None => {
                    m.at += CRUISE * dt;
                    let d = loop_at(&m.pts, m.at).0.distance(pp);
                    // (test hook MCLA_HOOKMAN_TEST=<key prefix>: that man alone, put near the player)
                    let wanted = only.as_deref().map_or(true, |o| m.key.starts_with(o) && !vehicles.cars.iter().any(|c| c.racer));
                    if wanted && spawn.is_none() && self.challenge.is_none() && ((d > 120.0 && d < 330.0) || only.is_some()) {
                        spawn = Some(k);
                    }
                }
            }
        }
        let mut ready = None;
        if let Some(k) = spawn.filter(|_| self.timer <= 0.0) {
            let m = self.men[k].clone();
            match vehicles.models.iter().find(|x| x.name.eq_ignore_ascii_case(&m.car)).cloned() {
                None => {
                    // his car has to be in memory first
                    if self.load_timer <= 0.0 {
                        self.load_timer = 2.0;
                        if let Some(d) = game.car_dirs.iter().find(|d| d.file_name().map_or(false, |f| f.to_string_lossy().eq_ignore_ascii_case(&m.car))).cloned() {
                            if let Err(e) = vehicles.load_model(gpu, world, &d) {
                                log::warn!("roamers: {}: {}", m.car, e);
                            }
                        } else {
                            log::warn!("roamers: no vehicle {} for {}", m.car, m.name);
                        }
                    }
                }
                Some(model) => ready = Some((m, model)),
            }
        }
        let Some(nav) = world.nav.as_ref() else { return None };
        if let Some((m, model)) = ready.filter(|(_, model)| (is_racer_model(&model.name) && model.name != pname) || only.is_some()) {
            {
                {
                    self.timer = 1.0;
                    let (mut pos, mut dir, next) = loop_at(&m.pts, m.at);
                    if only.is_some() && pos.distance(pp) > 330.0 {
                        // (test hook: the man is put on a road near the player)
                        if let Some(li) = nav.links_near(pp, 320.0).find(|&i| { let l = &nav.links[i]; let d = l.a.distance(pp); !l.next.is_empty() && d > 60.0 && d < 200.0 && l.len > 20.0 }) {
                            pos = nav.links[li].point_at(0.5);
                            dir = nav.links[li].dir();
                        }
                    }
                    let n = m.pts.len();
                    // three times round
                    let stops: Vec<Vec3> = (0..n * 3).map(|j| m.pts[(next + j) % n]).collect();
                    let (line, _) = nav.route_through(pos, dir, &stops, 25.0);
                    let at = if line.len() > 8 { line[0] } else { pos };
                    if let Some((li, t, d)) = nav.nearest_link(at) {
                        if d < 30.0 && !vehicles.cars.iter().any(|c| c.state.pos.distance(at) < 25.0) {
                            let l = &nav.links[li];
                            let dir = if line.len() > 8 { (line[1] - line[0]).normalize_or(l.dir()) } else { l.dir() };
                            let rgb = game.rivals.paint_of(&m.config);
                            let color = rgb.map(|c| [c[0], c[1], c[2], 1.0]).unwrap_or(PALETTE[(self.rand() * PALETTE.len() as f32) as usize % PALETTE.len()]);
                            let idx = vehicles.spawn(gpu, model, at + Vec3::Y * 0.8, (-dir.x).atan2(-dir.z), color, false);
                            let c = &mut vehicles.cars[idx];
                            c.racer = true;
                            // (the man the career is waiting for answers to his name)
                            c.driver = if !m.mission.is_empty() && game.career.hookman_mission() == Some(m.mission.as_str()) { m.name.clone() } else { String::new() };
                            c.vinyl_name = m.config.clone();
                            // (step 19d: the vehicle's mod value is the hookman's - Racer_SetModValue)
                            if m.mod_value >= 0 {
                                c.set_perf(m.mod_value.clamp(0, 100) as f32 / 100.0);
                            }
                            log::info!("roamers: {} ({:?}, {}, difficulty {}) in his {} {:.0} m away", m.name, m.kind, DISTRICTS[m.district.min(4)], m.difficulty, c.model.name, at.distance(pp));
                            if let Some(ai) = c.ai.as_mut() {
                                ai.link = Some(li);
                                ai.t = t;
                                ai.target_speed = CRUISE;
                                ai.aggression = 0.7;
                                if line.len() > 8 {
                                    ai.path = Some(std::sync::Arc::new(line));
                                    ai.path_i = 0;
                                }
                            }
                        }
                    }
                }
            }
        }
        // the run to the start line
        if let Some(ch) = self.challenge.as_mut() {
            ch.time += dt;
            let rival = vehicles.cars.iter().filter(|c| c.racer).map(|c| c.state.pos).min_by(|a, b| a.distance_squared(ch.goal).partial_cmp(&b.distance_squared(ch.goal)).unwrap());
            if std::env::var("MCLA_ROAM_TEST").is_ok() && (ch.time % 10.0) < dt {
                log::info!("roamers: t {:.0} rival {:.0} m, player {:.0} m from the line", ch.time, rival.map_or(-1.0, |r| r.distance(ch.goal)), pp.distance(ch.goal));
            }
            // (flashing the headlights again skips the run: straight to the race, no reputation)
            if flash && ch.time > 0.5 {
                game.opt_x.arcade.bmt_left.set(None);
                let race = ch.race;
                self.challenge = None;
                self.won_against_rival = self.rival_challenged;
                vehicles.cars.retain(|c| !c.racer);
                return Some(race);
            }
            // (the line is one checkpoint at the race's start beacon with an activation radius
            // of 20: generic_local_hookman.sco 0x1091..0x11c9)
            // (by the checkpoint's own test - checkpoints::CpSpec::hit, radius 20 - from where each
            // car was a step ago to 2 m ahead of it; "there" is a distance under the radius)
            let line = crate::checkpoints::CpSpec::with_radius(ch.goal, 20.0);
            let reach = |c: &crate::physics::CarState| -> f32 {
                let d = c.pos.distance(ch.goal);
                if line.hit(crate::checkpoints::ahead_point(c.pos, c.vel), c.pos - c.vel * dt) { 0.0 } else { d.max(20.001) }
            };
            let player_d = vehicles.cars.iter().find(|c| c.is_player).map_or(pp.distance(ch.goal), |c| reach(&c.state));
            let player_there = player_d < 20.0;
            // (an opponent at the line first does not end the run: "<NAME> FINISHED", and
            // the others have thirty seconds - the hookmen's scripts, `mc4::BmtClose`)
            let dists: Vec<f32> = vehicles.cars.iter().filter(|c| c.racer).map(|c| reach(&c.state)).collect();
            let step = ch.close.step(player_d, &dists, 20.0, dt);
            if step == crate::mc4::BmtStep::FirstIn {
                let who = vehicles.cars.iter().filter(|c| c.racer).min_by(|a, b| a.state.pos.distance_squared(ch.goal).partial_cmp(&b.state.pos.distance_squared(ch.goal)).unwrap()).map(|c| if c.driver.is_empty() { self.men.iter().find(|m| m.config == c.vinyl_name).map(|m| game.career.name_of(&m.name).to_uppercase()).unwrap_or_else(|| crate::game::pretty_car_name(&c.model.name).to_uppercase()) } else { game.career.name_of(&c.driver).to_uppercase() }).unwrap_or_default();
                let text = game.career.strings.get("PLAYER_FINISHED").filter(|s| s.contains("<D>")).map(|s| { let a = s.find("<D>").unwrap_or(0); let b = s.find("</D>").map_or(s.len(), |e| e + 4); format!("{}{}{}", &s[..a], who, &s[b..]).to_uppercase() }).unwrap_or_else(|| format!("{who} FINISHED"));
                game.show_message(&text);
            }
            // (the thirty seconds are shown: the script turns the race timer on - HUD control
            // 6, property 0 - with the target time 30000 ms, 0x1635..0x167f; the port prints
            // the time left)
            // (step 19c: on the race clock itself - `Game::mc4_bmt_hud`)
            game.opt_x.arcade.bmt_left.set(ch.close.since.map(|s| (crate::mc4::BMT_CLOSE_SECS - s).max(0.0)));
            if let crate::mc4::BmtStep::Over(s15b_place) = step {
                game.opt_x.arcade.bmt_left.set(None);
                let race = ch.race;
                log::info!("roamers: run to the line over after {:.1} s, player first: {}, place {} ({:?} s after the first man)", ch.time, player_there, s15b_place, ch.close.since);
                // (the run to the line gives the reputation - and only it - half an event's
                // experience by the place: sub_826B99A0; the event is as hard as the rival is)
                // (the racers are the player, the man and whoever rides with him - `+3080 = extra
                // + 2` in the hookmen's scripts; the place is the rank at the line; the
                // difficulty is the hookman's own - mcHookman_GetDifficulty: his mission's
                // set-up, or the difficulty his slot was filled for)
                let field = vehicles.cars.iter().filter(|c| c.racer).count() + 1;
                let place = s15b_place;
                let diff = game.mc4_hookman_diff().filter(|_| self.rival_challenged).or(ch.diff).unwrap_or_else(|| vehicles.cars.iter().find(|c| c.racer).map_or(1, |c| game.mc4_difficulty_of_setup(&c.model.name, &c.vinyl_name)).clamp(0, 3) as usize);
                let exp = crate::rewards::beat_me_there_exp(game.rep, diff, place, field);
                game.rep += exp;
                let what = if place == 1 { "BEAT ME THERE" } else { "BEATEN TO THE LINE" };
                if exp > 0 {
                    game.show_message(&format!("{what}   +{exp} REP"));
                } else {
                    game.show_message(what);
                }
                self.challenge = None;
                self.won_against_rival = self.rival_challenged;
                vehicles.cars.retain(|c| !c.racer);
                return Some(race);
            }
            // self-test: skip the drive
            // (MCLA_ROAM_TEST=late leaves the player where he is: the rival gets there first)
            if std::env::var("MCLA_ROAM_TEST").map_or(false, |v| v != "late") && ch.time > 6.0 {
                let goal = ch.goal;
                if let Some(c) = vehicles.player_mut() {
                    c.state = crate::physics::CarState::new(goal + Vec3::Y * 0.8, 0.0);
                    c.placed = false;
                }
                return None;
            }
            // a rival who got away is waiting at the line; the meeting itself stays on
            if ch.time > 150.0 + ch.dist / 6.0 {
                game.show_message("CHALLENGE OVER");
                self.cancel(vehicles);
            }
            return None;
        }
        // someone to flash? Racer_CanFlashHookman: within 30 m, ahead, at about our speed
        // (`mc4::can_flash`; its line-of-sight ray is not cast here)
        let test = std::env::var("MCLA_ROAM_TEST").is_ok();
        let mut best: Option<(usize, f32)> = None;
        for (i, c) in vehicles.cars.iter().enumerate() {
            if !c.racer {
                continue;
            }
            let rel = c.state.pos - pp;
            let d = rel.length();
            let ok = crate::mc4::can_flash(d, pspeed - c.state.speed(), pf.dot(rel.normalize_or_zero()));
            // (step 19d: and nothing stands between the two - a segment from 0.5 above the one
            // to 0.5 above the other, sub_822AB318 0x822AB540..0x822AB61C; the port asks the
            // city's collision)
            let ok = ok && {
                let from = pp + Vec3::Y * 0.5;
                let to = c.state.pos + Vec3::Y * 0.5;
                let len = from.distance(to);
                len < 0.5 || world.collider.raycast(from, (to - from) / len, len).is_none()
            };
            // (the self-test goes for the career's rival wherever he is)
            let d = if test && !c.driver.is_empty() { d * 0.01 } else { d };
            if (ok || (test && d < 400.0)) && best.map_or(true, |b| d < b.1) {
                best = Some((i, d));
            }
        }
        if flash && std::env::var("MCLA_ALT_LOG").is_ok() {
            log::info!("roamers: flash - {} racer cars, in reach {:?}", vehicles.cars.iter().filter(|c| c.racer).count(), best.map(|b| (vehicles.cars[b.0].vinyl_name.clone(), b.1)));
        }
        let Some((ri, _)) = best else { return None };
        let man = self.men.iter().find(|m| m.config == vehicles.cars[ri].vinyl_name).cloned();
        self.near = Some(match (&man, vehicles.cars[ri].driver.is_empty()) {
            (_, false) => game.career.name_of(&vehicles.cars[ri].driver),
            (Some(m), _) => game.career.name_of(&m.name),
            _ => format!("THE {}", crate::game::pretty_car_name(&vehicles.cars[ri].model.name).to_uppercase()),
        });
        self.near_is_rival = !vehicles.cars[ri].driver.is_empty();
        self.near_type = match man.as_ref().map(|m| m.kind) {
            Some(ManKind::Slot) => 1,
            Some(ManKind::RedLight) => 2,
            Some(ManKind::TimeTrial) => 3,
            Some(ManKind::Tournament) => 4,
            Some(ManKind::Freeway) => 5,
            None => 0,
        };
        if !flash {
            return None;
        }
        // the time trial's and the tournament's man: their panels (forms 65 and 66)
        if let Some(m) = man.as_ref() {
            match m.kind {
                ManKind::TimeTrial => {
                    log::info!("roamers: the time trial man of {} flashed", DISTRICTS[m.district.min(4)]);
                    game.mc4_tt_panel_open(m.district);
                    return None;
                }
                ManKind::Tournament => {
                    log::info!("roamers: the tournament man of {} flashed ({})", DISTRICTS[m.district.min(4)], m.event);
                    game.mc4_tournament_open(&m.event);
                    return None;
                }
                ManKind::RedLight | ManKind::Freeway => {
                    log::info!("roamers: the {:?} man {} ({}) flashed", m.kind, m.uid, m.name);
                    self.alt_flash = Some(m.clone());
                    return None;
                }
                ManKind::Slot => {}
            }
        }
        // the hookman's own event, if the career names one
        let scripted = if vehicles.cars[ri].driver.is_empty() { None } else { game.career.hookman_race() }.and_then(|r| (0..game.race_defs.len()).find(|&i| game.race_defs[i].id.to_ascii_lowercase().ends_with(r.as_str()))).and_then(|i| game.race_start(i).map(|s| (i, s, s.distance(pp))));
        // a local hookman's choice (mcRaceArray_*: `mc4_local_races`)
        let (rpos, rfwd, top) = (vehicles.cars[ri].state.pos, vehicles.cars[ri].state.forward(), vehicles.cars[ri].model.params.top_speed);
        let rnd = (self.rand() * 10000.0) as u32;
        let kept = if scripted.is_some() { Vec::new() } else { game.mc4_local_races(rpos, rfwd, man.as_ref().map_or(6, |m| m.district as i32), rnd) };
        let pick = scripted.or_else(|| kept.first().and_then(|&t| game.mc4_table_race(t)).map(|(i, s)| (i, s, s.distance(pp))));
        let Some((race, goal, dist)) = pick else {
            game.show_message("NO EVENT NEARBY");
            return None;
        };
        let rc = &mut vehicles.cars[ri];
        if let Some(ai) = rc.ai.as_mut() {
            let route = nav.route_points(rpos, Some(rfwd), goal);
            log::info!("roamers: rival {:.0} m from the line, route {:?} points, ends {:.0} m from it", rpos.distance(goal), route.as_ref().map(|r| r.len()), route.as_ref().and_then(|r| r.last()).map_or(-1.0, |e| e.distance(goal)));
            if let Some(path) = route {
                ai.path = Some(std::sync::Arc::new(path));
                ai.path_i = 0;
            }
            ai.target_speed = (top * 0.7).clamp(28.0, 55.0);
            ai.aggression = 1.0;
        }
        log::info!("roamers: challenge, race {} start {:.0} m away (the hookman keeps {:?})", game.race_defs[race].name, dist, kept.iter().filter_map(|&t| game.race_table.get(t)).map(|n| n.rsplit('/').next().unwrap_or("")).collect::<Vec<_>>());
        if test {
            // the self-driving test player heads for the line as well
            let route = nav.route_points(pp, Some(pf), goal);
            if let Some(ai) = vehicles.player_mut().and_then(|c| c.ai.as_mut()) {
                ai.path = route.map(std::sync::Arc::new);
                ai.path_i = 0;
                ai.target_speed = 30.0;
            }
        }
        // (step 19d: the meeting - no form: the splash and the man's greeting, or, with police
        // within 250 of the player, "COPS ARE TOO CLOSE" and no run to the line - `mc4_meet`)
        self.accepted = true;
        self.rival_challenged = !vehicles.cars[ri].driver.is_empty();
        let s19d_cops = vehicles.cars.iter().any(|c| c.cop && c.state.pos.distance(pp) < 250.0);
        let s19d_man = man.as_ref().map(|m| (m.name.clone(), m.car.clone(), m.config.clone()));
        let s19d_skip = game.mc4_meet(s19d_man, race, &kept, s19d_cops, !kept.is_empty() && !self.rival_challenged);
        if s19d_skip {
            log::info!("roamers: police within 250 m - the race starts at its grid");
            self.won_against_rival = self.rival_challenged;
            vehicles.cars.retain(|c| !c.racer && !c.cop);
            return Some(race);
        }
        // (the others go back to being places on their loops)
        let keep = vehicles.cars[ri].vinyl_name.clone();
        vehicles.cars.retain(|c| !c.racer || c.vinyl_name == keep);
        // (step 19d: every police car goes underground for the run and the race -
        // PoliceManager_SetAllPoliceUnderground(1, 1))
        vehicles.cars.retain(|c| !c.cop);
        self.challenge = Some(Challenge { race, goal, time: 0.0, dist, close: Default::default(), diff: man.as_ref().filter(|m| m.mission.is_empty()).map(|m| m.difficulty.clamp(0, 3) as usize), races: kept });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_man_goes_round_his_loop() {
        let pts = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 100.0), Vec3::new(0.0, 0.0, 100.0)];
        assert_eq!(loop_len(&pts), 400.0);
        let (p, d, next) = loop_at(&pts, 150.0);
        assert!(p.distance(Vec3::new(100.0, 0.0, 50.0)) < 1e-3 && d.distance(Vec3::Z) < 1e-3 && next == 2);
        // (round and round)
        assert!(loop_at(&pts, 550.0).0.distance(p) < 1e-3);
        assert!((loop_project(&pts, Vec3::new(104.0, 0.0, 50.0)) - 150.0).abs() < 1e-3);
        assert!((loop_project(&pts, Vec3::new(20.0, 0.0, 97.0)) - 280.0).abs() < 1e-3);
    }
}
