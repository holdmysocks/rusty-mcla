//! The "alternative" hookmen: the cruising racers who hold no slot of the career's slot
//! system (`mcCareer_GetAltHookmanVisibility`, ids 300.. and 350..) - the red light racers
//! and the freeway racers. Each is a script of his own that the career keeps running once a
//! mission has started it:
//!
//! - **red light racers** `<d>_redlight_hk1.sco` (Hollywood `game/CruisingHookmen/0x18ac539b`,
//!   Beaches `0x44dd0df5`, Hills `0xb84e26f3`, Downtown `0x22a3b093`, South Central
//!   `sc/CruisingHookmen/0x47ffe3b8`): mcHookman_SetId 350 + district, SetType 2, a patrol loop
//!   of his own (Hollywood: 15 points), the district's racers to choose from (`+1808`), the 39
//!   red light start places of the game (`gen_prerace_red_*`) and the district's share of them,
//!   and the eighteen landmarks to race to. Map element subtype 1, icon 3 (the traffic light).
//!   Started by REDLIGHT_GH_01's script (all four of the city) and by the South Central
//!   missions' (`DELIVERY_SC`, `FIND_GH_27`, `BESTOF_GH_31`, `GARAGE_SC`).
//! - **freeway racers** `freeway_hookman1..8.sco` (`0xf0d3aa35`, `0xb4abf432`, `0xc4c0818b`,
//!   `0xfa9ed267`, `0x867251d6`, `0xda6b0840`, `sc/..0xf23f4230`, `sc/..0x0dca831c`): id 300 + n,
//!   type 5, one character (`GH_04`, ...), a loop of up to 24 points along the freeways, each
//!   with the greeting of its road (`GREETING_FREEWAY_405`, ...). Map subtype 5, icon 8.
//!   Started by REP_ON_HW_STRIP / BEAT_FREEWAY (1..6), REDLIGHT_GH_01 (4) and the South
//!   Central missions (7, 8).
//!
//! Both drive their loop with `game/CruisingHookmen/CruisingHookmanCore` like the slot men
//! (`roamers.rs`); a flash ends the core script and the man's own script runs the race.

use crate::game::Game;
use glam::Vec3;
use hashbrown::HashMap;
use mcla_formats::sco::Tok;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AltKind {
    RedLight,
    Freeway,
}

/// One alternative hookman's script.
#[derive(Clone, Debug)]
pub struct AltDef {
    pub kind: AltKind,
    /// mcHookman_SetId: 350 + district (red light), 300 + n (freeway)
    pub id: u32,
    /// the district (red light: the script's; freeway: the district of his loop's first point)
    pub district: usize,
    /// the script's name as the career lists it ("hw_redlight_hk1", "freeway_hookman4")
    pub script: String,
    /// the characters he can be (red light: the table at +1808; freeway: his one name)
    pub names: Vec<String>,
    /// the patrol loop (+0, count at +376)
    pub pts: Vec<Vec3>,
    /// freeway: the greeting scenario of each point (the road it is on)
    pub greet: Vec<String>,
    /// red light: the start places of the district (indices into `AltData::scenes`)
    pub spots: Vec<usize>,
}

/// One red light start place: the scene (`gen_prerace_red_hw00_01`) and the grid's middle and
/// heading (the table's second pose, +648 / +1132 - what the race's grid is made from).
#[derive(Clone, Debug)]
pub struct Scene {
    pub name: String,
    pub pos: Vec3,
    /// degrees, as the script hands it to Racer_InitializeResetPosition
    pub heading: f32,
    /// the table's first pose (+0 / +484): the grid's second row; a race back races to the
    /// landmark nearest its right-hand slot
    pub pos_a: Vec3,
    pub heading_a: f32,
}

#[derive(Default)]
pub struct AltData {
    pub defs: Vec<AltDef>,
    pub scenes: Vec<Scene>,
}

/// The red light hookmen's script names by district (Hollywood, Beaches, Hills, Downtown,
/// South Central - `mc4::DISTRICTS`).
pub const REDLIGHT_SCRIPTS: [(&str, &str); 5] = [("game", "hw_redlight_hk1"), ("game", "be_redlight_hk1"), ("game", "hl_redlight_hk1"), ("game", "dt_redlight_hk1"), ("sc", "sc_redlight_hk1")];

/// The missions whose scripts mark an alternative hookman visible at once
/// (mcCareer_SetAltHookmanVisibility(id, 1) in `game/career/missions/<ID>.sco`).
pub const VISIBLE_BY_MISSION: [(&str, &[u32]); 7] = [
    ("REDLIGHT_GH_01", &[350, 303]),
    ("BEAT_FREEWAY", &[300, 301, 302, 303, 304, 305]),
    ("RACE_FREEWAY", &[300]),
    ("DELIVERY_SC", &[354, 306, 307]),
    ("FIND_GH_27", &[354, 306, 307]),
    ("BESTOF_GH_31", &[354, 306, 307]),
    ("GARAGE_SC", &[354, 306, 307]),
];

fn num(t: &[Tok], i: usize) -> Option<f64> {
    match t.get(i) {
        Some(Tok::Num(v)) => Some(*v),
        _ => None,
    }
}

fn int(t: &[Tok], i: usize) -> Option<i64> {
    num(t, i).filter(|v| v.fract() == 0.0 && v.abs() < 1e7).map(|v| v as i64)
}

/// The vectors a script stores into arrays: `x, y, z, 3, k, 3, base` in the constant stream
/// (push x y z; push 3 k 3; <array base>; implode). (token index, base, k, value)
fn vectors(t: &[Tok]) -> Vec<(usize, i64, usize, Vec3)> {
    let mut out = Vec::new();
    for i in 0..t.len().saturating_sub(6) {
        let (Some(x), Some(y), Some(z)) = (num(t, i), num(t, i + 1), num(t, i + 2)) else { continue };
        if int(t, i + 3) != Some(3) || int(t, i + 5) != Some(3) {
            continue;
        }
        let (Some(k), Some(base)) = (int(t, i + 4), int(t, i + 6)) else { continue };
        if !(0..512).contains(&k) {
            continue;
        }
        out.push((i, base, k as usize, Vec3::new(x as f32, y as f32, z as f32)));
    }
    out
}

/// The patrol loop: its count is stored at +376 (`push N; push <struct local>; push 376; add;
/// set`), the points before it into the struct (red light: `<local>, 0`) or into a local array
/// that is copied in (freeway: local 27, each point followed by its greeting string).
fn patrol(t: &[Tok]) -> (Vec<Vec3>, Vec<String>) {
    let mut wants: Vec<(usize, i64)> = Vec::new();
    for i in 0..t.len().saturating_sub(2) {
        if let (Some(n), Some(b), Some(376)) = (int(t, i), int(t, i + 1), int(t, i + 2)) {
            if (3..=64).contains(&n) && b > 0 {
                wants.push((n as usize, b));
            }
        }
    }
    let vs = vectors(t);
    let mut by_base: HashMap<i64, Vec<Option<(Vec3, String)>>> = HashMap::new();
    for (i, base, k, v) in vs {
        let g = match t.get(i + 7) {
            Some(Tok::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let e = by_base.entry(base).or_default();
        if e.len() <= k {
            e.resize(k + 1, None);
        }
        if e[k].is_none() {
            e[k] = Some((v, g));
        }
    }
    let full = |l: &Vec<Option<(Vec3, String)>>, n: usize| l.len() >= n && l[..n].iter().all(|x| x.is_some());
    // (the first count whose array is there: the struct's own, else a local array of exactly
    // that many points)
    let pick = wants.iter().find_map(|&(n, b)| by_base.get(&b).filter(|l| full(l, n)).or_else(|| by_base.values().find(|l| l.len() == n && full(l, n))).or_else(|| by_base.values().filter(|l| full(l, n)).min_by_key(|l| l.len())).map(|l| (n, l)));
    match pick.map(|(n, l)| (n, l.clone())) {
        Some((n, l)) => (l[..n].iter().map(|x| x.as_ref().unwrap().0).collect(), l[..n].iter().map(|x| x.as_ref().unwrap().1.clone()).collect()),
        None => (Vec::new(), Vec::new()),
    }
}
/// The red light script's tables: the 39 start places (function 0x3c: scene name at +1296,
/// second pose at +648 / +1132), the district's share of them (`[a, 1, 1296, b, 1, B, 976]`:
/// place b of the district's list is place a of the game's) and the racers (`GH_01`, `GH_15`:
/// `[name, k, 1, B, 1808]`).
fn redlight_tables(t: &[Tok]) -> (Vec<Scene>, Vec<usize>, Vec<String>) {
    let mut names: Vec<Option<String>> = Vec::new();
    let mut pos: HashMap<usize, Vec3> = HashMap::new();
    let mut heading: HashMap<usize, f32> = HashMap::new();
    let mut pos_a: HashMap<usize, Vec3> = HashMap::new();
    let mut heading_a: HashMap<usize, f32> = HashMap::new();
    for (_, base, k, v) in vectors(t) {
        if base == 648 {
            pos.entry(k).or_insert(v);
        }
        if base == 0 {
            pos_a.entry(k).or_insert(v);
        }
    }
    let mut share: Vec<(usize, usize)> = Vec::new();
    let mut racers: Vec<String> = Vec::new();
    for i in 0..t.len() {
        if let Tok::Str(s) = &t[i] {
            if s.contains("RaceStart/") && int(t, i + 2) == Some(1) && int(t, i + 3) == Some(1296) {
                if let Some(k) = int(t, i + 1).filter(|k| (0..256).contains(k)) {
                    let k = k as usize;
                    if names.len() <= k {
                        names.resize(k + 1, None);
                    }
                    names[k].get_or_insert_with(|| s.clone());
                }
            }
            if s.starts_with("GH_") && s.len() <= 6 && int(t, i + 2) == Some(1) && int(t, i + 4) == Some(1808) && !racers.contains(s) {
                racers.push(s.clone());
            }
            continue;
        }
        // first heading: [h, k, 1, 484]
        if let (Some(h), Some(k), Some(1), Some(484)) = (num(t, i), int(t, i + 1), int(t, i + 2), int(t, i + 3)) {
            if (0..256).contains(&k) {
                heading_a.entry(k as usize).or_insert(h as f32);
            }
        }
        // heading: [h, k, 1, 1132]
        if let (Some(h), Some(k), Some(1), Some(1132)) = (num(t, i), int(t, i + 1), int(t, i + 2), int(t, i + 3)) {
            if (0..256).contains(&k) && !matches!(t.get(i.wrapping_sub(1)), Some(Tok::Str(_))) {
                heading.entry(k as usize).or_insert(h as f32);
            }
        }
        if let (Some(a), Some(1), Some(1296), Some(b), Some(1), Some(_), Some(976)) = (int(t, i), int(t, i + 1), int(t, i + 2), int(t, i + 3), int(t, i + 4), int(t, i + 5), int(t, i + 6)) {
            if (0..256).contains(&a) && (0..256).contains(&b) {
                share.push((b as usize, a as usize));
            }
        }
    }
    let scenes: Vec<Scene> = names
        .iter()
        .enumerate()
        .map(|(k, n)| Scene { name: n.clone().unwrap_or_default(), pos: pos.get(&k).copied().unwrap_or(Vec3::ZERO), heading: heading.get(&k).copied().unwrap_or(0.0), pos_a: pos_a.get(&k).copied().unwrap_or(Vec3::ZERO), heading_a: heading_a.get(&k).copied().unwrap_or(0.0) })
        .collect();
    // (a later store to the same entry wins; Hills' script stores entry 9 twice - place 16, then
    // 18 - and never entry 10, which keeps the array's zero: that hole is left out here)
    let mut by_b: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    for (b, a) in share {
        by_b.insert(b, a);
    }
    (scenes, by_b.into_values().collect(), racers)
}

fn read_script(data_dir: &str, sub: &str, name: &str) -> Option<Vec<Tok>> {
    let dir = std::path::Path::new(data_dir).join("script_obj").join(sub).join("CruisingHookmen");
    let file = [dir.join(format!("{name}.sco")), dir.join(format!("0x{:08x}", crate::mc4::joaat(&format!("{name}.sco"))))].into_iter().find(|p| p.is_file())?;
    let sco = std::fs::read(file).ok().and_then(|d| mcla_formats::sco::decode(&d).ok())?;
    Some(sco.consts())
}

impl AltData {
    pub fn load(data_dir: &str) -> AltData {
        let mut out = AltData::default();
        for (d, (sub, name)) in REDLIGHT_SCRIPTS.iter().enumerate() {
            let Some(t) = read_script(data_dir, sub, name) else { continue };
            let (pts, _) = patrol(&t);
            let (scenes, spots, names) = redlight_tables(&t);
            if out.scenes.is_empty() {
                out.scenes = scenes;
            }
            if pts.len() >= 3 {
                out.defs.push(AltDef { kind: AltKind::RedLight, id: 350 + d as u32, district: d, script: name.to_string(), names, pts, greet: Vec::new(), spots });
            }
        }
        for n in 1..=8u32 {
            let sub = if n >= 7 { "sc" } else { "game" };
            let name = format!("freeway_hookman{n}");
            let Some(t) = read_script(data_dir, sub, &name) else { continue };
            let (pts, greet) = patrol(&t);
            let who = t.iter().find_map(|x| if let Tok::Str(s) = x { (s.starts_with("GH_") && s.len() <= 6).then(|| s.clone()) } else { None });
            if pts.len() >= 3 {
                let district = crate::mc4::district_of_place(data_dir, pts[0]).unwrap_or(0);
                out.defs.push(AltDef { kind: AltKind::Freeway, id: 299 + n, district, script: name, names: who.into_iter().collect(), pts, greet, spots: Vec::new() });
            }
        }
        log::info!(
            "alt hookmen: {} red light racers ({} start places), {} freeway racers",
            out.defs.iter().filter(|d| d.kind == AltKind::RedLight).count(),
            out.scenes.len(),
            out.defs.iter().filter(|d| d.kind == AltKind::Freeway).count()
        );
        for d in &out.defs {
            log::info!("alt hookman {} {:?} {} ({}): {} loop points, names {:?}, {} start places", d.id, d.kind, d.script, crate::mc4::DISTRICTS[d.district.min(4)], d.pts.len(), d.names, d.spots.len());
        }
        out
    }
}

// ---- the freeway race (freeway_hookman<n>.sco 0x104a..0x1564) ----

/// The patrol point the man drives to next (0x119d): the nearest point farther than 20 m from
/// him (inside 20 m the point after it), then the one after that when the two lie on
/// different sides of him (their directions' dot product below 0).
pub fn freeway_next(pts: &[Vec3], pos: Vec3) -> usize {
    let n = pts.len();
    if n == 0 {
        return 0;
    }
    let mut best = (10000.0f32, 0usize);
    let mut k = 0;
    while k < n {
        let d = pos.distance(pts[k]);
        if d < best.0 && d > 20.0 {
            best = (d, k);
        } else if d < 20.0 {
            best.1 = if k + 1 >= n { 0 } else { k + 1 };
            k = n;
        }
        k += 1;
    }
    let i = best.1;
    let j = if i + 1 >= n { 0 } else { i + 1 };
    if (pts[i] - pos).dot(pts[j] - pos) < 0.0 {
        j
    } else {
        i
    }
}

/// The race's checkpoints: `count` patrol points from the next one on, round the loop
/// (CheckpointList_ResizeList(count), Checkpoint_SetPosition, activation radius 30). The
/// count is RAND_INT_RANGE(4, 6), at most the loop's points less one.
pub fn freeway_course(pts: &[Vec3], pos: Vec3, count: usize) -> (usize, Vec<Vec3>) {
    let n = pts.len();
    if n == 0 {
        return (0, Vec::new());
    }
    let count = count.min(n - 1).max(1);
    let first = freeway_next(pts, pos);
    let cps = (0..count).map(|k| pts[(first + k) % n]).collect();
    (first, cps)
}

/// The activation radius of the freeway race's checkpoints (0x1399 / 0x140a).
pub const FREEWAY_RADIUS: f32 = 30.0;

// ---- the red light race (`<d>_redlight_hk1.sco` 0x45d1..) ----

/// The start place: of the district's places the one nearest the player (0x47e7, VDIST).
pub fn redlight_spot(def: &AltDef, scenes: &[Scene], player: Vec3) -> Option<usize> {
    def.spots.iter().copied().filter(|&i| i < scenes.len()).min_by(|&a, &b| scenes[a].pos.distance(player).total_cmp(&scenes[b].pos.distance(player)))
}

/// The two grid slots: the place's middle moved 2.25 m to either side across its heading
/// (Math_VecRotateY of (-2.25, 0, 0) / (2.25, 0, 0)): (the hookman's - slot 0 -, the player's).
/// Yaw in the port's convention (the car faces (-sin, 0, -cos)).
pub fn redlight_grid(s: &Scene) -> [(Vec3, f32); 2] {
    let h = s.heading.to_radians();
    let right = Vec3::new(h.cos(), 0.0, -h.sin());
    [(s.pos - right * 2.25, h), (s.pos + right * 2.25, h)]
}

/// The landmark to race to (0x535d, "Goal selection at 45 / 65 / 90 degree view"): the
/// landmarks farther than 1000 m (+1796) from the player's grid slot whose direction is within
/// 0.8 rad of the way he faces - at most fifteen; with fewer than five the cone is widened to
/// 1.17 rad, then 1.4 rad -, and of these one at random, never the one taken last time
/// (RAND_INT_RANGE_DIFFERENT with the script's `LastGrid`).
pub fn redlight_goal(landmarks: &[Vec3], start: Vec3, fwd: Vec3, last: &mut usize, rnd: f32) -> Option<usize> {
    let pass = |limit: f32| -> Vec<usize> {
        let mut c = Vec::new();
        for (i, l) in landmarks.iter().enumerate() {
            let d = *l - start;
            if d.length() <= 1000.0 {
                continue;
            }
            let cosv = (fwd.dot(d) / (fwd.length() * d.length()).max(1e-6)).clamp(-1.0, 1.0);
            if cosv.acos() < limit && c.len() < 15 {
                c.push(i);
            }
        }
        c
    };
    let mut c = pass(0.8);
    if c.len() < 5 {
        c = pass(1.17);
    }
    if c.len() < 5 {
        c = pass(1.4);
    }
    if c.is_empty() {
        return None;
    }
    let n = c.len();
    if *last > n - 1 {
        *last = 0;
    }
    let mut k = ((rnd.clamp(0.0, 0.9999)) * n as f32) as usize;
    if n > 1 && k == *last {
        k = (k + 1 + ((rnd * 7919.0) as usize % (n - 1))) % n;
        if k == *last {
            k = (k + 1) % n;
        }
    }
    *last = k;
    Some(c[k])
}

// ---- the race in hand ----

/// What the port keeps of a race against an alternative hookman between its start and its
/// end.
#[derive(Default, Clone, Debug)]
pub struct RaceState {
    /// the race's difficulty: the man's (RaceData +3100 = mcHookman_GetDifficulty)
    pub diff: Option<usize>,
    /// a freeway race starts where the two are, as they are going: (player, hookman)
    pub rolling: Option<(crate::physics::CarState, crate::physics::CarState)>,
    /// the checkpoints' activation radius
    pub radius: f32,
    /// the red light men's last landmark (`LastGrid`, by id)
    pub last_goal: HashMap<u32, usize>,
    rng: u32,
    /// the registry's `raceBack`: 1 from a red light race's start, 0 once raced back
    pub back_offer: bool,
    /// RACE BACK chosen on the race-over menu: the caller makes the race
    pub back_req: bool,
    /// the race in hand: its man (id) and the start place it was run from
    pub last_uid: u32,
    pub last_scene: Option<usize>,
    /// the player won it (the script's local 125)
    pub won: Option<bool>,
    /// in-race taunts: on until the man has finished, the next one's race time (s)
    pub taunt_on: bool,
    pub taunt_at: f32,
    /// the man's profile (GH_01 ..): the voice of his lines (`Speech::say_as`)
    pub voice: String,
    /// a red light race is over: the police come up and the three units are made again
    /// (0x7b9b..0x7bab) - for the caller, who has the police
    pub police_after: bool,
}

impl RaceState {
    fn rnd(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x9e37_79b9;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / 16_777_216.0
    }
}

/// A race against an alternative hookman, ready to start.
pub struct AltStart {
    pub def: crate::races::RaceDef,
    /// the red light start's scene (the cine's key)
    pub scene: Option<String>,
    /// the announcement's text id ("PRERACE_RED_TOWER_RECORDS")
    pub announce: Option<String>,
    /// the speech scenario the man opens with (a freeway man's: the road of the finish)
    pub speech: Option<String>,
    /// the race is a mission's (REDLIGHT_GH_01's man)
    pub mission: bool,
}

pub const REDLIGHT_RACE_ID: &str = "game/Race/RedLight/red_light_race";
pub const FREEWAY_RACE_ID: &str = "game/Race/Freeway/freeway_race";

pub fn is_alt_race(id: &str) -> bool {
    id == REDLIGHT_RACE_ID || id == FREEWAY_RACE_ID
}

/// MCLA_OLD_REDLIGHT=1: the port's first red light racer, waiting at the nearest light.
pub fn old_redlight() -> bool {
    std::env::var("MCLA_OLD_REDLIGHT").map_or(false, |v| v != "0")
}

impl Game {
    /// The alternative hookman is visible on the maps: marked so in the career's block
    /// (mcCareer_GetAltHookmanVisibility), or by the script of a mission that has started.
    pub fn alt_visible(&self, id: u32) -> bool {
        // (test hook MCLA_ALT_VISIBLE=1: every one of them, as found)
        self.mc4_alt_visible(id) || VISIBLE_BY_MISSION.iter().any(|(m, ids)| ids.contains(&id) && self.career.mission_started(m)) || std::env::var("MCLA_ALT_VISIBLE").map_or(false, |v| v != "0")
    }

    /// The man's script runs: it is in the career's list of scripts (started by a mission's
    /// script, `mc4::MISSION_SCRIPTS`); a game without a career has all of them.
    pub fn alt_running(&self, script: &str) -> bool {
        if !self.career.loaded() {
            return true;
        }
        // (the career's list when it is known; else from the missions that have started - a
        // script a mission starts runs from then on, `mc4::MISSION_SCRIPTS`)
        match self.mc4_running_scripts() {
            Some(l) => l.iter().any(|n| n.eq_ignore_ascii_case(script)),
            None => crate::mc4::MISSION_SCRIPTS.iter().any(|(m, s, _, _)| s.rsplit('/').next().map_or(false, |n| n.eq_ignore_ascii_case(script)) && self.career.mission_started(m)),
        }
    }

    /// mcCareer_SetAltHookmanVisibility(id, 1): into the career's block (a block is made when
    /// the save has none, as `mc4_redlight_seen` does).
    pub fn alt_set_visible(&mut self, id: u32) {
        if self.mc4_alt_visible(id) {
            return;
        }
        let mut m = self.mc4.take().unwrap_or_else(|| crate::mc4::Mc4::seeded(self.fresh_game));
        match m.career.alt_hookmen.iter_mut().find(|a| a.uid == id) {
            Some(a) => a.visible = true,
            None => m.career.alt_hookmen.push(crate::mc4::AltHookman { uid: id, visible: true, extra: Default::default() }),
        }
        self.mc4 = Some(m);
        self.dirty = true;
    }

    /// The man has been flashed: his race (his script's flash branch - red light 0x3f44..,
    /// freeway 0xee1..).
    ///
    /// Red light: the player is taken to the district's start place nearest him, the man in
    /// slot 0 and the player in slot 1 beside him (`redlight_grid`), the start scene of the
    /// place plays, and the race is to one landmark (`redlight_goal`, its radius from +1712).
    /// The race data: one opponent, the difficulty the man's (0x3fa7..0x3fce, +3092 = 5).
    ///
    /// Freeway: no move, no scene, no countdown - the race is on where the two are
    /// (Racer_InitializeResetPosition at his own place, 0x1158..0x116a), through the next
    /// RAND_INT_RANGE(4, 6) points of his loop (`freeway_course`, radius 30), the opening line
    /// the greeting of the last one's road (0x1423..0x149a).
    pub fn alt_race(&mut self, data: &AltData, redlights: &mut crate::redlight::RedLights, vehicles: &crate::vehicles::Vehicles, man: &crate::roamers::Man) -> Option<AltStart> {
        let def = data.defs.iter().find(|d| d.id == man.uid)?;
        let player = vehicles.player()?;
        let pp = player.state.pos;
        self.opt_x.arcade.d.redlight_man = Some((man.name.clone(), man.car.clone(), man.config.clone()));
        self.man_mod = Some((man.name.clone(), man.mod_value));
        self.althk.diff = Some(man.difficulty.clamp(0, 3) as usize);
        self.althk.voice = man.name.clone();
        self.alt_race_begin(def.id);
        match def.kind {
            AltKind::RedLight => {
                // (the registry's raceBack = 1: RACE BACK is offered after this race, 0x461e)
                self.althk.back_offer = true;
                self.alt_redlight_start(def, data, redlights, pp, None, man)
            }
            AltKind::Freeway => {
                let hook = vehicles.cars.iter().find(|c| c.racer && c.vinyl_name == man.config)?;
                let hp = hook.state.pos;
                // RAND_INT_RANGE(4, 6), inclusive
                let count = 4 + ((self.althk.rnd() * 3.0) as usize).min(2);
                let (first, cps) = freeway_course(&def.pts, hp, count);
                let n = def.pts.len();
                let last = (first + cps.len().max(1) - 1) % n.max(1);
                let speech = def.greet.get(last).filter(|g| !g.is_empty()).cloned();
                let yaw = |f: Vec3| (-f.x).atan2(-f.z);
                self.althk.radius = FREEWAY_RADIUS;
                self.althk.rolling = Some((player.state.clone(), hook.state.clone()));
                redlights.announce = None;
                redlights.title_key = None;
                log::info!("freeway race: {} ({}) from point {} through {} points ({:?}), {:.0} m to the first, greeting {:?}, difficulty {}", man.name, def.script, first, cps.len(), cps.iter().map(|c| (c.x as i32, c.z as i32)).collect::<Vec<_>>(), cps[0].distance(hp), speech, man.difficulty);
                let d = crate::races::RaceDef {
                    id: FREEWAY_RACE_ID.to_string(),
                    name: "Freeway".to_string(),
                    race_type: "Ordered".to_string(),
                    sub_type: "Freeway".to_string(),
                    // (the career counts a won freeway race - stat kFWN, `career.rs`)
                    district: "Freeway".to_string(),
                    laps: 1,
                    beacon: pp,
                    grid: vec![(hp, yaw(hook.state.forward())), (pp, yaw(player.state.forward()))],
                    checkpoints: cps,
                    opponents: 1,
                    ..Default::default()
                };
                Some(AltStart { def: d, scene: None, announce: None, speech, mission: false })
            }
        }
    }

    /// What every race against the man starts with: who he is, no result yet, and the
    /// in-race taunts - the first RAND_INT_RANGE(30000, 40000) ms into the race (red light
    /// 0x5cce, freeway 0x10e4).
    fn alt_race_begin(&mut self, uid: u32) {
        self.althk.last_uid = uid;
        self.althk.won = None;
        self.althk.taunt_on = true;
        self.althk.taunt_at = 30.0 + self.althk.rnd() * 10.0;
    }

    /// The red light race from the district's place nearest `pp` (0x47e7). `back`: a race
    /// back - the finish is the landmark nearest the right-hand slot of the first pose of the
    /// place the last race was run from (0x78d3..0x7931: the grid's slot [1][1] into local 12,
    /// then 0x474c: the nearest landmark, no distance or cone rule); otherwise the landmark rule
    /// of `redlight_goal`. The opening line is the landmark's PRERACE_RED_<X>, a race back's
    /// PRERACE_RED_RACEBACK_<X> (+1300 / +1384, played with the start scene:
    /// CineScript_PrepareSpeechScenarioForNextCutscene, 0x567c..0x56ae).
    fn alt_redlight_start(&mut self, def: &AltDef, data: &AltData, redlights: &mut crate::redlight::RedLights, pp: Vec3, back: Option<usize>, man: &crate::roamers::Man) -> Option<AltStart> {
        let dist_name = crate::mc4::DISTRICTS[def.district.min(4)].to_string();
        let si = redlight_spot(def, &data.scenes, pp)?;
        let s = &data.scenes[si];
        let grid = redlight_grid(s);
        let start = grid[1].0;
        let fwd = Vec3::new(-grid[1].1.sin(), 0.0, -grid[1].1.cos());
        let marks: Vec<Vec3> = redlights.landmarks.iter().map(|l| l.pos).collect();
        let goal = match back.and_then(|b| data.scenes.get(b)) {
            Some(prev) => {
                let h = prev.heading_a.to_radians();
                let slot = prev.pos_a + Vec3::new(h.cos(), 0.0, -h.sin()) * 2.25;
                (0..marks.len()).min_by(|&a, &b| marks[a].distance(slot).total_cmp(&marks[b].distance(slot)))
            }
            None => {
                let r = self.althk.rnd();
                let mut last = self.althk.last_goal.get(&def.id).copied().unwrap_or(0);
                let g = redlight_goal(&marks, start, fwd, &mut last, r);
                self.althk.last_goal.insert(def.id, last);
                g
            }
        }?;
        let lm = &redlights.landmarks[goal];
        self.althk.radius = lm.radius;
        self.althk.rolling = None;
        self.althk.last_scene = Some(si);
        // the cine's own key of the place's scene
        let stem = s.name.rsplit('/').next().unwrap_or("").trim_end_matches("_generated").to_ascii_lowercase();
        let cine = redlights.spots.iter().find(|p| p.scene.to_ascii_lowercase().ends_with(&stem));
        if let Some(c) = cine {
            log::info!(
                "red light start {}: script grid vs scene movers - man {:.1} m, player {:.1} m, heading {:.1} / {:.1} deg",
                stem,
                grid[0].0.distance(c.rival.0),
                grid[1].0.distance(c.player.0),
                grid[1].1.to_degrees(),
                c.player.1.to_degrees()
            );
        }
        let cine_scene = cine.map(|c| c.scene.clone());
        let text = if back.is_some() { lm.text.replacen("PRERACE_RED_", "PRERACE_RED_RACEBACK_", 1) } else { lm.text.clone() };
        let (key, lpos) = (lm.key.clone(), lm.pos);
        redlights.announce = Some(text.clone());
        redlights.title_key = Some(key.clone());
        self.mc4_redlight_seen(&dist_name);
        log::info!("red light race{}: {} ({}) from {} ({:.0} m from the player) to {} ({:.0} m), difficulty {}", if back.is_some() { " back" } else { "" }, man.name, def.script, stem, s.pos.distance(pp), key, lpos.distance(start), man.difficulty);
        let d = crate::races::RaceDef {
            id: REDLIGHT_RACE_ID.to_string(),
            name: "red light race".to_string(),
            race_type: "Ordered".to_string(),
            sub_type: "RedLight".to_string(),
            district: dist_name,
            laps: 1,
            beacon: start,
            grid: grid.to_vec(),
            checkpoints: vec![lpos],
            opponents: 1,
            ..Default::default()
        };
        Some(AltStart { def: d, scene: cine_scene, announce: Some(text.clone()), speech: Some(text), mission: !man.mission.is_empty() })
    }

    /// RACE BACK (raceOverCommand `RaceBack`, 0x78b8..0x7931): the same man, from the place
    /// nearest the player (at the finish) back to the landmark by the last start; the
    /// registry's raceBack goes to 0 - no RACE BACK after a race back. The race's police are
    /// as they were (the script goes back to 0x4657, after EvaluatePolice).
    pub fn alt_race_back(&mut self, data: &AltData, redlights: &mut crate::redlight::RedLights, vehicles: &crate::vehicles::Vehicles, men: &[crate::roamers::Man]) -> Option<AltStart> {
        let uid = self.althk.last_uid;
        let def = data.defs.iter().find(|d| d.id == uid && d.kind == AltKind::RedLight)?;
        let man = men.iter().find(|m| m.uid == uid)?.clone();
        let pp = vehicles.player()?.state.pos;
        let prev = self.althk.last_scene;
        self.althk.back_offer = false;
        self.opt_x.arcade.d.redlight_man = Some((man.name.clone(), man.car.clone(), man.config.clone()));
        self.man_mod = Some((man.name.clone(), man.mod_value));
        self.althk.diff = Some(man.difficulty.clamp(0, 3) as usize);
        self.althk.voice = man.name.clone();
        self.alt_race_begin(uid);
        self.alt_redlight_start(def, data, redlights, pp, prev, &man)
    }

    /// RACE BACK stands on the race-over menu (sub_82663640 0x82663784..0x826637bc: the
    /// registry's raceBack set and not on line) - after a red light race that was not itself a
    /// race back.
    pub fn alt_race_back_offered(&self) -> bool {
        self.althk.back_offer && self.active_def.as_ref().map_or(false, |d| d.id == REDLIGHT_RACE_ID)
    }

    /// No police during a red light race: its EvaluatePolice (0x3fd3, called with the
    /// probability 0.0 at 0x3fc3) always ends in PoliceManager_DestroyAllPolice; after the race
    /// they are back (SetAllPoliceUnderground(0, 1), SetAllPoliceStreamUnlocked, the three
    /// cruisers 400..402 made again - 0x7b9b..0x7bab). A freeway race leaves the police be.
    pub fn alt_police_off(&self) -> bool {
        self.race.is_some() && self.active_def.as_ref().map_or(false, |d| d.id == REDLIGHT_RACE_ID)
    }

    /// The man's taunt now and then in the race (red light 0x6624..0x66b4, freeway
    /// 0x1b8f..0x1c20): while he has not finished, the speech system is free and the race clock
    /// has passed the time set, mcCareerSpeech_PlayTauntInRace (sub_826C2C90): with a police
    /// unit chasing - TAUNT_RACE_COP_CHASING_PLAYER (the player its target) or _HOOKMAN;
    /// otherwise by the player's place - first: TAUNT_RACE_IN_FIRST, last of more than two:
    /// TAUNT_RACE_IN_LAST, else TAUNT_RACE_NOT_IN_FIRST. Said by the man to the player; once it
    /// was said the next comes RAND_INT_RANGE(45000, 70000) ms on (`alt_taunt_said`).
    pub fn alt_taunt_tick(&mut self, vehicles: &crate::vehicles::Vehicles, chased: bool, busy: bool) -> Option<&'static str> {
        if !self.althk.taunt_on || !self.active_def.as_ref().map_or(false, |d| is_alt_race(&d.id)) {
            return None;
        }
        let race = self.race.as_ref()?;
        if race.countdown > 0.0 {
            return None;
        }
        if race.participants.get(1).and_then(|&i| race.finish_time.get(i)).map_or(false, |t| t.is_some()) {
            self.althk.taunt_on = false;
            return None;
        }
        if busy || race.time < self.althk.taunt_at {
            return None;
        }
        if chased {
            return Some("TAUNT_RACE_COP_CHASING_PLAYER");
        }
        let (place, field) = self.player_position(vehicles);
        Some(if place <= 1 {
            "TAUNT_RACE_IN_FIRST"
        } else if field > 2 && place >= field {
            "TAUNT_RACE_IN_LAST"
        } else {
            "TAUNT_RACE_NOT_IN_FIRST"
        })
    }

    pub fn alt_taunt_said(&mut self, scenario: &str) {
        let t = self.race.as_ref().map_or(0.0, |r| r.time);
        self.althk.taunt_at = t + 45.0 + self.althk.rnd() * 25.0;
        log::info!("alt hookman: {} at {:.1} s, the next at {:.1} s", scenario, t, self.althk.taunt_at);
    }

    /// The race is over for the player (`award_race` done): the result is kept. A freeway race
    /// has no race-over menu (`raceEndNoMenu`, freeway 0x2ae0): back to the cruise after its
    /// panels, the man's post-race line at once - mcCareerSpeech_PlayTauntpostRace(man, player,
    /// player first) (0x2af7..0x2b27: TAUNT_POST_IN_FIRST / TAUNT_POST_NOT_IN_FIRST,
    /// sub_826C2878). The red light man says his when the player leaves the menu (`alt_leave`).
    pub fn alt_finished(&mut self, place: usize) -> Option<&'static str> {
        let id = self.active_def.as_ref().map(|d| d.id.clone())?;
        if !is_alt_race(&id) {
            return None;
        }
        self.althk.won = Some(place == 1);
        self.althk.taunt_on = false;
        if id == FREEWAY_RACE_ID {
            self.career_over = true;
            return Some(if place == 1 { "TAUNT_POST_IN_FIRST" } else { "TAUNT_POST_NOT_IN_FIRST" });
        }
        None
    }

    /// Leaving a race against the man for the cruise: quit in the middle - TAUNT_POST_NOT_IN_FIRST
    /// (red light 0x6418, freeway 0x17f4); a red light race's RETURN TO CRUISE - by the result
    /// (0x79c4: local 125, the player first and the man not).
    pub fn alt_leave(&mut self, quit: bool) -> Option<&'static str> {
        let id = self.active_def.as_ref().map(|d| d.id.clone())?;
        if !is_alt_race(&id) {
            return None;
        }
        self.althk.taunt_on = false;
        if id == REDLIGHT_RACE_ID {
            self.althk.police_after = true;
        }
        let won = self.althk.won.take();
        if quit && won.is_none() {
            return Some("TAUNT_POST_NOT_IN_FIRST");
        }
        if id == REDLIGHT_RACE_ID {
            return Some(if won == Some(true) { "TAUNT_POST_IN_FIRST" } else { "TAUNT_POST_NOT_IN_FIRST" });
        }
        None
    }

    /// Right after `start_race_def`: a race against an alternative hookman gets its
    /// checkpoints' radius, and a freeway race goes on from where the two were, as they were
    /// going (no countdown, no start camera). Any other race clears the state.
    pub fn alt_after_start(&mut self, vehicles: &mut crate::vehicles::Vehicles) {
        let mine = self.active_def.as_ref().map_or(false, |d| is_alt_race(&d.id));
        if !mine {
            self.althk.diff = None;
            self.althk.rolling = None;
            return;
        }
        let radius = self.althk.radius.max(1.0);
        if let Some(r) = self.race.as_mut() {
            r.cp_specs = r.checkpoints.iter().map(|c| crate::checkpoints::CpSpec::with_radius(*c, radius)).collect();
            r.cp_radius = vec![radius; r.cp_specs.len()];
        }
        if let Some((p, h)) = self.althk.rolling.take() {
            if let Some(r) = self.race.as_mut() {
                r.countdown = 0.0;
                let parts = r.participants.clone();
                if let Some(c) = parts.first().and_then(|&i| vehicles.cars.get_mut(i)) {
                    c.state = p;
                    c.placed = true;
                }
                if let Some(c) = parts.get(1).and_then(|&i| vehicles.cars.get_mut(i)) {
                    c.state = h;
                    c.placed = true;
                }
            }
            vehicles.frozen = false;
            self.prerace = None;
            self.prerace_scene = None;
            log::info!("freeway race: rolling start");
        }
    }

    /// mcCharacterConfigData_GetCarConfigDataByVehicleName: the character's set-up of that
    /// vehicle at the mod value for the difficulty (REDLIGHT_GH_01's man: GH_01 in his
    /// `vp_vw_scirocco_88`).
    pub fn alt_character_car_by_vehicle(&self, character: &str, vehicle: &str, diff: i32) -> Option<(String, String, i32)> {
        let ch = self.rivals.characters.iter().find(|c| c.id.eq_ignore_ascii_case(character))?;
        let c = ch.cars.iter().find(|c| c.car.eq_ignore_ascii_case(vehicle))?;
        let m = self.mc4_samples_of(&c.car).map_or(0, |s| crate::mc4::setup_mod_value(s, self.player_perf(), diff));
        Some((c.car.clone(), c.config.clone(), m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Vec3> {
        vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 100.0), Vec3::new(0.0, 0.0, 100.0)]
    }

    #[test]
    fn freeway_next_point_is_ahead() {
        let p = square();
        // half way along the first side: the nearest point farther than 20 m is 0 or 1 (50 m
        // each); the first found is kept (strictly nearer replaces), then 0 and 1 lie on
        // either side of him: the next is 1
        assert_eq!(freeway_next(&p, Vec3::new(50.0, 0.0, 0.0)), 1);
        // just past point 1 (inside 20 m): the one after it
        assert_eq!(freeway_next(&p, Vec3::new(100.0, 0.0, 10.0)), 2);
        // near the last point: round to the first
        assert_eq!(freeway_next(&p, Vec3::new(5.0, 0.0, 100.0)), 0);
        let (first, cps) = freeway_course(&p, Vec3::new(50.0, 0.0, 0.0), 6);
        assert_eq!(first, 1);
        // at most the loop's points less one, round the loop
        assert_eq!(cps, vec![p[1], p[2], p[3]]);
    }

    #[test]
    fn redlight_grid_is_across_the_heading() {
        let s = Scene { name: String::new(), pos: Vec3::new(10.0, 0.0, 10.0), heading: 0.0, pos_a: Vec3::ZERO, heading_a: 0.0 };
        let g = redlight_grid(&s);
        // heading 0 faces -z: the hookman 2.25 m to the left (-x), the player to the right
        assert!((g[0].0 - Vec3::new(7.75, 0.0, 10.0)).length() < 1e-4);
        assert!((g[1].0 - Vec3::new(12.25, 0.0, 10.0)).length() < 1e-4);
        let s = Scene { heading: 90.0, ..s };
        let g = redlight_grid(&s);
        // heading 90 faces -x: right is +z... (cos 90, 0, -sin 90) = (0, 0, -1)
        assert!((g[1].0 - Vec3::new(10.0, 0.0, 7.75)).length() < 1e-4);
    }

    #[test]
    fn redlight_goal_cone_and_widening() {
        let start = Vec3::ZERO;
        let fwd = Vec3::new(0.0, 0.0, -1.0);
        // ahead (within 45 deg), beside (80 deg), behind, and one too near
        let l = vec![Vec3::new(0.0, 0.0, -2000.0), Vec3::new(2000.0, 0.0, -350.0), Vec3::new(0.0, 0.0, 2000.0), Vec3::new(0.0, 0.0, -500.0)];
        let mut last = 0;
        // fewer than five in any cone: the widest one's (0 and 1) are drawn from
        let mut seen = std::collections::HashSet::new();
        for r in 0..20 {
            let g = redlight_goal(&l, start, fwd, &mut last, r as f32 / 20.0).unwrap();
            assert!(g == 0 || g == 1);
            seen.insert(g);
        }
        assert_eq!(seen.len(), 2);
        // never the same twice running with two to choose from
        let mut last = 0;
        let a = redlight_goal(&l, start, fwd, &mut last, 0.1).unwrap();
        let b = redlight_goal(&l, start, fwd, &mut last, 0.1).unwrap();
        assert_ne!(a, b);
        // none: nothing farther than 1000 m ahead
        assert_eq!(redlight_goal(&l[3..], start, fwd, &mut 0, 0.5), None);
    }

    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn the_scripts_tables_are_read() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = AltData::load(&dir);
        assert_eq!(a.scenes.len(), 39);
        assert!(a.scenes.iter().all(|s| s.name.contains("gen_prerace_red_") && s.pos.length() > 10.0 && s.pos_a.length() > 10.0));
        // hw00's two poses (function 0x3c: +0 / +484, +648 / +1132)
        assert!((a.scenes[0].pos_a - Vec3::new(-1081.7, 9.86901, -533.636)).length() < 0.01 && (a.scenes[0].heading_a + 88.8085).abs() < 1e-3);
        assert!((a.scenes[0].pos - Vec3::new(-1095.98, 9.86459, -533.984)).length() < 0.01 && (a.scenes[0].heading + 90.2409).abs() < 1e-3);
        let rl: Vec<&AltDef> = a.defs.iter().filter(|d| d.kind == AltKind::RedLight).collect();
        let fw: Vec<&AltDef> = a.defs.iter().filter(|d| d.kind == AltKind::Freeway).collect();
        for d in &a.defs {
            eprintln!("{} {:?} {} pts {} names {:?} spots {}", d.id, d.kind, d.script, d.pts.len(), d.names, d.spots.len());
        }
        assert_eq!((rl.len(), fw.len()), (5, 8));
        // Hollywood: 15 points, the first (-17.1, 20.5, -804.5); racers GH_01 and GH_15; 25 places
        assert_eq!(rl[0].pts.len(), 15);
        assert!((rl[0].pts[0] - Vec3::new(-17.1113, 20.4915, -804.502)).length() < 0.01);
        assert_eq!(rl[0].names, vec!["GH_01".to_string(), "GH_15".to_string()]);
        assert_eq!(rl[0].spots, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 24, 25, 10, 12, 14, 15, 16, 17, 18, 19, 20, 27, 31, 32, 35, 38]);
        // the first freeway man: GH_04, 24 points, the first greeting CRENSHAW
        assert_eq!((fw[0].id, fw[0].names.clone(), fw[0].pts.len()), (300, vec!["GH_04".to_string()], 24));
        assert_eq!(fw[0].greet[0], "GREETING_FREEWAY_CRENSHAW");
        assert!((fw[0].pts[0] - Vec3::new(-656.277, -5.70096, 617.62)).length() < 0.01);
    }
}
