//! Police: law-breaking score, pull-over, pursuit, search and arrest.
//!
//! All numbers come from the game's own `tune/police/globals.xml`
//! (`mcPoliceTuningGlobals`); patrol cars start on the routes of
//! `tune/police/patrolpath.xml`. The cruiser is `vp_chv_police_96`.

use crate::props::Inst;
use crate::render::Gpu;
use crate::vehicles::Vehicles;
use crate::world::World;
use glam::Vec3;
use hashbrown::HashMap;
use std::path::Path;

pub const COP_CAR: &str = "vp_chv_police_96";
const MPH: f32 = 2.236_94;

#[derive(Default)]
pub struct Tuning {
    v: HashMap<String, f32>,
    /// the `content="int_array"` members (`iLawFines`), in file order
    arrays: HashMap<String, Vec<i32>>,
}

impl Tuning {
    pub fn load(path: &Path) -> Tuning {
        let mut v = HashMap::new();
        let mut arrays = HashMap::new();
        if let Ok(t) = std::fs::read_to_string(path) {
            let mut open: Option<(String, Vec<i32>)> = None;
            for line in t.lines() {
                let l = line.trim();
                if let Some((name, list)) = open.as_mut() {
                    if l.starts_with("</") {
                        arrays.insert(std::mem::take(name), std::mem::take(list));
                        open = None;
                    } else if let Ok(n) = l.parse::<i32>() {
                        list.push(n);
                    }
                    continue;
                }
                if l.starts_with('<') && l.ends_with("content=\"int_array\">") {
                    let name = l[1..].split_whitespace().next().unwrap_or("").to_string();
                    open = Some((name, Vec::new()));
                    continue;
                }
                let Some(rest) = l.strip_prefix('<') else { continue };
                let Some(sp) = rest.find(" value=\"") else { continue };
                let name = &rest[..sp];
                let val = &rest[sp + 8..];
                let Some(end) = val.find('"') else { continue };
                if let Ok(f) = val[..end].parse::<f32>() {
                    v.insert(name.to_string(), f);
                }
            }
        }
        Tuning { v, arrays }
    }

    /// One entry of an int array member; the members the file lists fewer values for read 0.
    pub fn array_at(&self, key: &str, i: usize) -> i32 {
        self.arrays.get(key).and_then(|a| a.get(i)).copied().unwrap_or(0)
    }

    pub fn get(&self, key: &str, default: f32) -> f32 {
        self.v.get(key).copied().unwrap_or(default)
    }
}

/// `<Points content="vector3_array">` blocks of patrolpath.xml.
fn load_patrol_paths(path: &Path) -> Vec<Vec<Vec3>> {
    let mut out = Vec::new();
    let Ok(t) = std::fs::read_to_string(path) else { return out };
    let mut cur: Option<Vec<Vec3>> = None;
    for line in t.lines() {
        let l = line.trim();
        if l.starts_with("<Points") {
            cur = Some(Vec::new());
        } else if l.starts_with("</Points") {
            if let Some(c) = cur.take() {
                if c.len() >= 2 {
                    out.push(c);
                }
            }
        } else if let Some(c) = cur.as_mut() {
            let f: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if f.len() == 3 {
                c.push(Vec3::new(f[0], f[1], f[2]));
            }
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Heat {
    None,
    /// a cruiser wants the player to stop
    PullOver,
    Pursuit,
    /// lost sight of the player; the pursuit ends when the search runs out
    Search,
    Busted,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Event {
    PullOver,
    Pursuit,
    Escaped,
    /// stopped for the cruiser: fine
    Ticket(i64),
    /// arrested during a pursuit: fine
    Busted(i64),
}

/// A cruiser's state in the pull-over or pursuit tactic (GitHub #17; docs/reference/hookmen/pursuit_tactics.md).
/// Pursuit (F-68032's FSM): CatchUp, AlongSide, Block, Halt (Delay is `flee_hold`, Search `Heat::Search`, Arrest the
/// port's busted test); pull-over (F-36876's): CatchUp, AvoidCollision (`Avoid`), Halt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Tac {
    #[default]
    CatchUp,
    AlongSide,
    Block,
    Halt,
    Avoid,
}

/// One cruiser's tactic state and its brain's goal plan (F-74140 / F-67303).
#[derive(Clone, Copy, Debug, Default)]
struct CopTac {
    st: Tac,
    /// seconds in `st`
    t: f32,
    /// the side of the target it drives beside: true = the target's right (F-14848)
    side: bool,
    side_t: f32,
    /// AlongSide: seconds out of reach (> 1.5 x fAlongSideDist)
    far_t: f32,
    /// pull-over Halt: seconds the gap has been growing while the target moves
    grow_t: f32,
    d_prev: f32,
    /// the plan's countdown (+588), its two goals and the speed to arrive with (-1: full pace)
    replan: f32,
    goals: [Vec3; 2],
    speed: f32,
    plan_fwd: Vec3,
    plan_speed: f32,
    head_on: bool,
}

/// The race's cop zone (PoliceManager +188 zone, +192 lap, +196 state, +212 timer).
#[derive(Clone, Debug)]
pub struct CopZone {
    pub idx: usize,
    pub lap: usize,
    /// 0 waiting for its lap, 1 armed, 2 set off
    pub state: u8,
    pub timer: f32,
    /// the player set it off
    pub player: bool,
    /// a cruiser waits there (the port's: where the original's unit comes from was not read)
    pub stationed: bool,
}

/// One of the three cruisers the hookmen's scripts make (`create_units`).
#[derive(Clone, Debug)]
pub struct Unit {
    /// 400 + i
    pub id: u32,
    /// where it was made and comes up (+912), and its heading (degrees, the script's)
    pub home: Vec3,
    pub heading: f32,
    /// its underground slot (+896)
    pub slot: Vec3,
    /// underground (+880)
    pub under: bool,
}

/// The three of `red_light_hk1` 0x436e / `freeway_hookman<n>` 0x2c73: (home, heading, slot).
pub const SCRIPT_UNITS: [([f32; 3], f32, [f32; 3]); 3] = [
    ([2030.1, 6.8, -813.7], 90.0, [-510.0, 9.5, -400.0]),
    ([-646.8, 18.5, -643.8], 90.0, [-500.0, 9.5, -400.0]),
    ([-2434.4, 1.9, -577.9], 84.9, [-490.0, 9.5, -400.0]),
];

pub struct Police {
    /// the scripted cruisers 400..402 (`create_units`)
    pub units: Vec<Unit>,
    pub tun: Tuning,
    paths: Vec<Vec<Vec3>>,
    pub heat: Heat,
    /// law-breaking score 0..iMaxLawBreakingScore
    pub score: f32,
    pub fine: f32,
    /// distinct laws broken since the score was last clear (bit mask)
    offences: u32,
    /// what the last ticket was for: 1 reckless driving, 2 traffic violation, 4 property
    pub ticket_for: u32,
    lost: f32,
    seen_time: f32,
    pub search: f32,
    arrest: f32,
    busted_timer: f32,
    /// the cops' arrest is over (its hold run out): a race running ends back in the cruise (F-22530)
    pub arrest_over: bool,
    /// the race's cop zone (F-88125 picks it, F-71212: the zone machine) - None outside a race
    pub zone: Option<CopZone>,
    /// the zone's chase is due: the pursuit starts on the next update
    zone_go: bool,
    /// the zone of the race running was picked
    pub zone_picked: bool,
    /// the pull-over's bullhorn (its tactic +92 = 2 at iBullhornLawBreakScore, F-22530): seconds left of
    /// fBullhornTime; Some(0) = run out (the 599 cap is lifted)
    bullhorn: Option<f32>,
    /// the ticket hold (FsmPullOver StateTicket, F-25878 / F-46174): seconds left before COP_PULLOVER
    ticket_hold: Option<f32>,
    /// COP_PULLOVER is running (CopPullover.sco): the police wait for `ticket_done` or `suspect_fled`
    ticketing: bool,
    /// after a tactic on the player ends: no scoring for 5 s (the record's +116, F-22530 / F-54211)
    grace: f32,
    /// the pursuit after a bolt from the ticket starts with its cruisers held this long (the Pursuit
    /// tactic's Delay state, F-10824 / F-50798: request +20 = the bolt's time x fPullOverFleeDelayScale)
    flee_hold: f32,
    /// each pursuing cruiser's tactic state (by car uid), and the target's speed at the last update (tactic +64)
    tacs: HashMap<u32, CopTac>,
    last_tspeed: f32,
    /// the highest speed band whose start points were paid (the record's "started" bits +100: kept to `clear`)
    speed_level: u8,
    /// the highest speed band of the offences so far (1..3; 0 = none): the fine counts that one
    speed_top: u8,
    /// the seconds counters of the per-second points (F-52240: they pay whole seconds): 0 speed,
    /// 1 wrong way, 2 burn-out, 3 two-wheel driving, 4 off-road, 5 the fall below iPullOverLawBreakScore,
    /// 6 the climb from it
    sec: [f32; 7],
    /// the player is off the roads, since the start points were paid
    off_active: bool,
    /// two-wheel driving has been going on since its start points were paid
    two_active: bool,
    /// the jump in the air has been paid (the original's byte +140)
    jump_paid: bool,
    /// the pull-over was run from (the fine's `iPullOverFleeFine`)
    fled: bool,
    /// the share of the career (the Progression system's fraction, 0..1) that cuts the fine;
    /// set by the game each frame
    pub progress: f32,
    wrong_way: f32,
    wrong_active: bool,
    burnout: f32,
    burnout_active: bool,
    nav_check: f32,
    against_traffic: bool,
    last_knocked: u32,
    last_damage: f32,
    spawn_timer: f32,
    /// how long the running pursuit has lasted (seconds)
    pub chase_time: f32,
    dispatch: f32,
    last_known: Vec3,
    /// time until the next radio report of the target's position
    radio: f32,
    time: f32,
    rng: u32,
    glow_buf: Option<wgpu::Buffer>,
    glow_count: u32,
    pub enabled: bool,
    /// junction the player was last seen approaching on red
    red_junction: Option<usize>,
    /// step 19d: the pursuit under way is the payback client's friends' (`start_goons`):
    /// their cars (`CarInstance::goon`) are the pursuers, no cruiser is sent, nothing is
    /// lit, and the two results are the payback leg's to deal with
    pub goons: bool,
}

const LAW_SPEED: u32 = 1;
const LAW_WRONG_WAY: u32 = 2;
const LAW_BURNOUT: u32 = 4;
const LAW_PROP: u32 = 8;
const LAW_VEHICLE: u32 = 16;
const LAW_POLICE_VEHICLE: u32 = 32;
const LAW_RED_LIGHT: u32 = 64;
const LAW_ROOF_JUMP: u32 = 128;
const LAW_TWO_WHEEL: u32 = 256;
const LAW_OFF_ROAD: u32 = 512;
const MAX_GLOWS: usize = 16;

impl Police {
    pub fn new() -> Police {
        Police {
            tun: Tuning::default(),
            paths: Vec::new(),
            heat: Heat::None,
            score: 0.0,
            fine: 0.0,
            offences: 0,
            ticket_for: 0,
            lost: 0.0,
            seen_time: 0.0,
            search: 0.0,
            arrest: 0.0,
            busted_timer: 0.0,
            arrest_over: false,
            zone: None,
            zone_go: false,
            zone_picked: false,
            tacs: HashMap::new(),
            last_tspeed: 0.0,
            bullhorn: None,
            ticket_hold: None,
            ticketing: false,
            grace: 0.0,
            flee_hold: 0.0,
            speed_level: 0,
            speed_top: 0,
            sec: [0.0; 7],
            off_active: false,
            two_active: false,
            jump_paid: false,
            fled: false,
            progress: 1.0,
            wrong_way: 0.0,
            wrong_active: false,
            burnout: 0.0,
            burnout_active: false,
            nav_check: 0.0,
            against_traffic: false,
            last_knocked: 0,
            last_damage: 0.0,
            spawn_timer: 8.0,
            chase_time: 0.0,
            dispatch: 0.0,
            last_known: Vec3::ZERO,
            radio: 0.0,
            time: 0.0,
            rng: 0x51ed_270b,
            glow_buf: None,
            glow_count: 0,
            enabled: true,
            red_junction: None,
            goons: false,
            units: Vec::new(),
        }
    }

    pub fn load(&mut self, data_dir: &str) {
        let dir = Path::new(data_dir).join("tune/police");
        self.tun = Tuning::load(&dir.join("globals.xml"));
        self.paths = load_patrol_paths(&dir.join("patrolpath.xml"));
        log::info!("police: {} tuning values, {} patrol paths", self.tun.v.len(), self.paths.len());
    }

    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x % 100000) as f32 / 100000.0
    }

    /// PoliceManager_CreatePolice for the three units (only while the police are unlocked -
    /// F-17632): made at their home, sent underground at once. A unit of the same id is
    /// made again.
    pub fn create_units(&mut self, unlocked: bool) {
        if !unlocked {
            return;
        }
        for (i, (home, heading, slot)) in SCRIPT_UNITS.iter().enumerate() {
            let id = 400 + i as u32;
            self.units.retain(|u| u.id != id);
            self.units.push(Unit { id, home: Vec3::from(*home), heading: *heading, slot: Vec3::from(*slot), under: true });
        }
        log::info!("police: units 400..402 made, underground");
    }

    /// PoliceManager_DestroyAllPolice: every cruiser and every unit.
    pub fn destroy_all(&mut self, vehicles: &mut Vehicles) {
        self.reset(vehicles);
        if !self.units.is_empty() {
            log::info!("police: all destroyed ({} units)", self.units.len());
        }
        self.units.clear();
    }

    /// PoliceManager_SetAllPoliceUnderground(1, 1): the units go underground (no new car is put
    /// up for them; a unit's car already on the road is left where it is - approx.).
    pub fn units_down(&mut self, vehicles: &mut Vehicles) {
        for u in self.units.iter_mut() {
            u.under = true;
        }
        // (an underground unit has no car on the road, and so none for IsAnyPoliceCloseToPlayer,
        // F-91297, which skips the police whose underground flag +880 is set)
        vehicles.cars.retain(|c| !(c.cop && c.vinyl_name.starts_with("police_unit:")));
    }

    /// PoliceManager_SetAllPoliceUnderground(0, 1): the units come up at their surface place;
    /// the port puts each there (`update_units`) while the player is within streaming reach.
    pub fn units_up(&mut self) {
        for u in self.units.iter_mut() {
            u.under = false;
        }
    }

    /// The units that are up stand at their home as the script made them (heading in degrees -
    /// the port's yaw is the heading in radians) while the player is within 380 m; a unit whose
    /// car has gone counts as underground again.
    pub fn update_units(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles) {
        let Some(pp) = vehicles.player().map(|c| c.state.pos) else { return };
        let Some(model) = vehicles.models.iter().find(|m| m.name == COP_CAR).cloned() else { return };
        for u in self.units.iter_mut() {
            let tag = format!("police_unit:{}", u.id);
            let has = vehicles.cars.iter().any(|c| c.cop && c.vinyl_name == tag);
            if u.under || has || u.home.distance(pp) > 380.0 {
                continue;
            }
            if vehicles.cars.iter().any(|c| c.state.pos.distance(u.home) < 6.0) {
                continue;
            }
            let idx = vehicles.spawn(gpu, model.clone(), u.home + Vec3::Y * 0.8, u.heading.to_radians(), [0.92, 0.92, 0.92, 0.0], false);
            let c = &mut vehicles.cars[idx];
            c.cop = true;
            c.vinyl_name = tag;
            Self::dress_cruiser(world, vehicles, idx);
            log::info!("police: unit {} up at its home ({:.0} m from the player)", u.id, u.home.distance(pp));
        }
    }

    /// Forget everything (new session / race start).
    pub fn reset(&mut self, vehicles: &mut Vehicles) {
        self.heat = Heat::None;
        self.score = 0.0;
        self.fine = 0.0;
        self.offences = 0;
        self.speed_level = 0;
        self.lost = 0.0;
        self.search = 0.0;
        self.arrest = 0.0;
        self.spawn_timer = 8.0;
        vehicles.cars.retain(|c| !c.cop);
    }

    /// Keep the arrest going for this long (its cutscene is playing).
    pub fn hold_busted(&mut self, secs: f32) {
        if self.heat == Heat::Busted {
            self.busted_timer = self.busted_timer.max(secs);
        }
    }

    /// PoliceManager_EvaluateCopZones (F-88125): of the race's zones ONE is picked, and ONE lap
    /// (`rand % laps` when the race has more than one, else lap 0); lap 0's zone is armed at once,
    /// another's when the race's lap changes to it (F-82127, "[COP ZONE] lap [%d], cop zone enabled").
    /// (approx.: the port's own random numbers, not the game's LCG)
    pub fn pick_zone(&mut self, zones: usize, laps: usize) {
        self.zone_go = false;
        self.zone = (zones > 0).then(|| {
            let idx = (self.rand() * zones as f32) as usize % zones;
            let lap = if laps > 1 { (self.rand() * laps as f32) as usize % laps } else { 0 };
            log::info!("police: cop zone {} enabled on lap {}", idx, lap);
            CopZone { idx, lap, state: if lap == 0 { 1 } else { 0 }, timer: 0.0, player: false, stationed: false }
        });
        if zones == 0 {
            log::info!("police: no cops in this race");
        }
    }

    /// The zone machine of a race (F-71212), per frame: `lap` is the race's current lap, `inside`
    /// whether a racer is in the zone (and whether it is the player). State 1 (armed): the first
    /// racer in the zone sets it off (state 2, 5.5 s). State 2: when the time is up the pursuit
    /// starts. (approx./not ported: the pursuit on an AI racer - the original's target is the racer
    /// that set it off, or with iRaceTgtAiProbPrcnt the nearest AI racer; the port's police chase
    /// only the player, so a zone set off for an AI racer starts nothing; the earlier start when the
    /// target comes in range (F-12032) and the state 3 wait after it)
    pub fn zone_step(&mut self, lap: usize, inside: Option<bool>, dt: f32) -> Option<&'static str> {
        let Some(z) = self.zone.as_mut() else { return None };
        let mut say = None;
        match z.state {
            0 if lap == z.lap => {
                z.state = 1;
                log::info!("police: lap {}, cop zone enabled", lap);
            }
            1 => {
                if let Some(player) = inside {
                    z.state = 2;
                    z.timer = 5.5;
                    z.player = player;
                    log::info!("police: cop zone {} set off by {}", z.idx, if player { "the player" } else { "an AI racer" });
                    say = Some("PLAYER_COPS_BEGIN_PURSUIT");
                }
            }
            2 => {
                z.timer -= dt;
                if z.timer < 0.0 {
                    z.state = 0;
                    z.lap = usize::MAX;
                    if z.player && matches!(self.heat, Heat::None | Heat::PullOver) {
                        self.zone_go = true;
                    }
                }
            }
            _ => {}
        }
        say
    }

    /// Test helper: start a pursuit right now with a cruiser close by.
    pub fn debug_force(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles) {
        self.score = 700.0;
        // (started as a cop zone's chase is: no cop has to see the car first)
        self.zone_go = true;
        self.dispatch = 0.0;
        if let Some((pos, fwd)) = vehicles.player().map(|c| (c.state.pos, c.state.forward())) {
            let ok = self.spawn_cop(gpu, world, vehicles, pos, fwd, 18.0, 80.0, None);
            log::info!("police: test cruiser spawned: {}", ok);
        }
    }

    /// CopPullover.sco is over with the ticket: the police record is cleared (ResetAllPolice / the tactic's end)
    /// and nothing is scored for 5 s.
    pub fn ticket_done(&mut self) {
        self.clear();
        self.grace = 5.0;
    }

    /// The suspect bolted from the ticket scene `secs` into its walk (PoliceManager_SuspectFleeing, F-72120):
    /// the run is on the fine (`iPullOverFleeFine`) and a pursuit starts whose cruisers wait
    /// secs x fPullOverFleeDelayScale first.
    pub fn suspect_fled(&mut self, secs: f32, at: Vec3) {
        self.ticketing = false;
        self.heat = Heat::Pursuit;
        self.fled = true;
        self.score = self.score.max(self.tun.get("iPursuitLawBreakScore", 600.0));
        self.flee_hold = secs * self.tun.get("fPullOverFleeDelayScale", 2.0);
        self.last_known = at;
        self.fine = self.law_fines();
        self.lost = 0.0;
        self.arrest = 0.0;
        self.dispatch = self.tun.get("fDispatchRdoDelayTime", 3.0);
    }

    pub fn lights_on(&self) -> bool {
        !matches!(self.heat, Heat::None) && !self.goons
    }

    /// Step 19d: one of the payback client's friends comes onto a road near the player (the
    /// original keeps the three "underground" and streams them in for the pursuit - where,
    /// was not read: the port puts them where it puts a cruiser sent to a pursuit).
    pub fn spawn_goon(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, model: std::sync::Arc<crate::vehicles::CarModel>, color: [f32; 4], set_up: &str, driver: &str) -> Option<usize> {
        let (center, heading) = vehicles.player().map(|c| (c.state.pos, c.state.forward()))?;
        let nav = world.nav.as_ref()?;
        let cands: Vec<usize> = nav
            .links_near(center, 260.0)
            .filter(|&i| {
                let l = &nav.links[i];
                let d = l.a.distance(center);
                !l.next.is_empty() && !l.prev.is_empty() && d > 110.0 && d < 260.0 && l.len > 12.0
            })
            .collect();
        if cands.is_empty() {
            return None;
        }
        let mut best = None;
        let mut bs = f32::NEG_INFINITY;
        for _ in 0..8 {
            let c = cands[(self.rand() * cands.len() as f32) as usize % cands.len()];
            let l = &nav.links[c];
            let pos = l.point_at(0.5) + Vec3::Y * 0.8;
            if !world.ready_at(pos) || vehicles.cars.iter().any(|v| v.state.pos.distance(pos) < 14.0) {
                continue;
            }
            // (from behind, as pursuers come)
            let s = -(l.a - center).normalize_or(Vec3::X).dot(heading) + self.rand() * 0.5;
            if s > bs {
                bs = s;
                best = Some(c);
            }
        }
        let best = best?;
        let l = &nav.links[best];
        let pos = l.point_at(0.5) + Vec3::Y * 0.8;
        let dir = l.dir();
        let idx = vehicles.spawn(gpu, model, pos, (-dir.x).atan2(-dir.z), color, false);
        let c = &mut vehicles.cars[idx];
        c.goon = true;
        c.pinned = true;
        c.vinyl_name = set_up.to_string();
        c.driver_set = driver.to_string();
        if let Some(ai) = c.ai.as_mut() {
            ai.link = Some(best);
            ai.t = 0.5;
            ai.target_speed = 14.0;
        }
        log::info!("police: a friend of the client ({}) comes at {:.0} m", set_up, pos.distance(center));
        Some(idx)
    }

    /// Step 19d: PoliceManager_StartPursuit(player, 3, 0, 0, 0) for the payback's chasers:
    /// the pursuit tactic on the player with the cars made for it. The law-breaking score
    /// goes to its maximum (F-33550 A-46466: at least the table's first entry).
    pub fn start_goons(&mut self, vehicles: &mut Vehicles) {
        // (approx.: the cruisers on the road are taken off for the chase - whether the
        // original's tactic can pick one of them beside the three was not read)
        vehicles.cars.retain(|c| !c.cop);
        self.goons = true;
        self.heat = Heat::Pursuit;
        self.score = self.tun.get("iMaxLawBreakingScore", 1000.0);
        self.fine = 0.0;
        self.lost = 0.0;
        self.search = 0.0;
        self.arrest = 0.0;
        self.chase_time = 0.0;
        self.last_known = vehicles.player().map_or(Vec3::ZERO, |c| c.state.pos);
    }

    /// The payback's chase is over (PoliceManager_DestroyPolice for the three once
    /// `Shutdown_PaybackOpponents` is set): the cars go, the police are as before it.
    pub fn end_goons(&mut self, vehicles: &mut Vehicles) {
        vehicles.cars.retain(|c| !c.goon);
        if self.goons {
            self.goons = false;
            self.clear();
        }
    }

    /// Law-breaking score as 0..1.
    pub fn level(&self) -> f32 {
        (self.score / self.tun.get("iMaxLawBreakingScore", 1000.0)).clamp(0.0, 1.0)
    }

    fn spawn_cop(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, center: Vec3, heading: Vec3, min_d: f32, max_d: f32, near: Option<Vec3>) -> bool {
        let Some(model) = vehicles.models.iter().find(|m| m.name == COP_CAR).cloned() else { return false };
        let Some(nav) = world.nav.as_ref() else { return false };
        let around = near.unwrap_or(center);
        let radius = if near.is_some() { 60.0 } else { max_d };
        let cands: Vec<usize> = nav
            .links_near(around, radius)
            .filter(|&i| {
                let l = &nav.links[i];
                let d = l.a.distance(center);
                !l.next.is_empty() && !l.prev.is_empty() && d > min_d && d < max_d && l.len > 12.0
            })
            .collect();
        if cands.is_empty() {
            return false;
        }
        // during a pursuit cruisers prefer to come from ahead
        let mut best = cands[(self.rand() * cands.len() as f32) as usize % cands.len()];
        if self.heat == Heat::Pursuit {
            let mut bs = f32::NEG_INFINITY;
            for _ in 0..6 {
                let c = cands[(self.rand() * cands.len() as f32) as usize % cands.len()];
                let s = (nav.links[c].a - center).normalize_or(Vec3::X).dot(heading) + self.rand() * 0.5;
                if s > bs {
                    bs = s;
                    best = c;
                }
            }
        }
        let l = &nav.links[best];
        let pos = l.point_at(0.5) + Vec3::Y * 0.8;
        if !world.ready_at(pos) || vehicles.cars.iter().any(|c| c.state.pos.distance(pos) < 14.0) {
            return false;
        }
        let dir = l.dir();
        let idx = vehicles.spawn(gpu, model, pos, (-dir.x).atan2(-dir.z), [0.92, 0.92, 0.92, 0.0], false);
        let c = &mut vehicles.cars[idx];
        c.cop = true;
        if let Some(ai) = c.ai.as_mut() {
        Self::dress_cruiser(world, vehicles, idx);
            ai.link = Some(best);
            ai.t = 0.5;
            ai.target_speed = 14.0;
        }
        log::debug!("police: cruiser spawned at {:?}", pos);
        true
    }

    /// Put a cruiser in wait beside the road at `pos` (the cop zones of a race).
    pub fn station(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, pos: Vec3, slot: usize) -> bool {
        let Some(model) = vehicles.models.iter().find(|m| m.name == COP_CAR).cloned() else { return false };
    /// A cruiser wears its stock set-up's paint and plate (`resources/vehicle/configs/vp_chv_police_96.xcc`: black body,
    /// light grey where the model marks the sixth paint area - doors and roof -, grey metallic bumpers): black and white,
    /// as the original's own showroom picture of it (GitHub #17: the port painted it all white). This is the set-up the
    /// original hands every cruiser: its police manager loads this file once at start-up (F-46996 through F-67050), and
    /// the spawn (F-49535) copies that set-up (F-62644) when the car asked for is this one - which every script's
    /// CreatePolice call asks for - instead of loading one by name (F-51560).
    fn dress_cruiser(world: &World, vehicles: &mut Vehicles, idx: usize) {
        let Ok(b) = std::fs::read(world.assets.root.join("resources/vehicle/configs").join(format!("{COP_CAR}.xcc"))) else { return };
        let Some(s) = crate::mc4::setup_look(&b) else { return };
        let c = &mut vehicles.cars[idx];
        c.color = s.color;
        c.look = Some(std::sync::Arc::new(s.look));
        let plate = crate::mc4::setup_plate(&b);
        if !plate.is_empty() {
            c.plate_text = plate;
            c.plate = None;
        }
    }

        let Some(nav) = world.nav.as_ref() else { return false };
        let Some((li, t, d)) = nav.nearest_link(pos) else { return false };
        if d > 60.0 {
            return false;
        }
        let l = &nav.links[li];
        let dir = l.dir();
        let right = Vec3::new(-dir.z, 0.0, dir.x);
        // at the kerb if there is room, else in the lane
        let mut p = l.point_at(t) + Vec3::Y * 0.8;
        let from = p + Vec3::Y * 0.3;
        if world.collider.raycast(from, right, 5.2).is_none() {
            p += right * 3.4;
        }
        if !world.ready_at(p) || vehicles.cars.iter().any(|c| c.state.pos.distance(p) < 7.0) {
            return false;
        }
        let idx = vehicles.spawn(gpu, model, p, (-dir.x).atan2(-dir.z), [0.92, 0.92, 0.92, 0.0], false);
        let c = &mut vehicles.cars[idx];
        c.cop = true;
        c.pinned = true;
        Self::dress_cruiser(world, vehicles, idx);
        c.parked = Some(usize::MAX - 9000 - slot);
        c.settle = 1.5;
        if let Some(ai) = c.ai.as_mut() {
            ai.link = Some(li);
            ai.t = t;
            ai.target_speed = 14.0;
        }
        log::info!("police: cruiser in wait at ({:.0}, {:.0}, {:.0})", p.x, p.y, p.z);
        true
    }

    /// Advance the police simulation. `knocked` is the running count of props the cars
    /// have knocked over. Returns what happened this frame.
    pub fn update(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, knocked: u32, dt: f32) -> Option<Event> {
        match self.heat {
            Heat::Pursuit | Heat::Search => self.chase_time += dt,
            Heat::PullOver | Heat::Busted => self.chase_time = 0.0,
            Heat::None => {}
        }
        if (!self.enabled && !self.goons) || dt <= 0.0 || self.ticketing {
            return None;
        }
        self.time += dt;
        // (step 19d: in the payback's chase the pursuers are the client's friends)
        let goons = self.goons;
        let pursuer = move |c: &crate::vehicles::CarInstance| if goons { c.goon } else { c.cop };
        // cruisers lying in wait join in once there is something to chase
        if !matches!(self.heat, Heat::None) {
            for c in vehicles.cars.iter_mut().filter(|c| c.cop && c.parked.is_some()) {
                c.parked = None;
                c.pinned = false;
            }
        }
        let Some(p) = vehicles.player() else { return None };
        let ppos = p.state.pos;
        let pvel = p.state.vel;
        let pfwd = p.state.forward();
        let pright = p.state.right();
        let speed = pvel.length();
        let mph = speed * MPH;
        let slip = p.state.wheels.iter().map(|w| w.slip).fold(0.0, f32::max);
        let damage = p.state.damage;
        let p_air = p.state.airborne_time;
        // (F-81388: the gyro's two-wheel state, +18, is not 0)
        let p_two = p.state.veh.as_ref().map_or(false, |v| v.two != 0);
        let tun = std::mem::take(&mut self.tun);
        let t = |k: &str, d: f32| tun.get(k, d);

        // ---- what the cruisers can see ---------------------------------------
        let detect = t("fDetectDist", 40.0);
        let vision = t("fVisionDist", 500.0);
        let mut detected = false;
        let mut seen = false;
        let mut nearest = f32::INFINITY;
        let mut n_cops = 0;
        for c in vehicles.cars.iter().filter(|c| pursuer(c)) {
            n_cops += 1;
            let d = c.state.pos.distance(ppos);
            nearest = nearest.min(d);
            // (F-94676 for a free cop, F-52240: within fDetectDist in 3D and a clear line from 2 m above the cop
            // to 2 m above the target, F-69576; the half angle fDetectHalfAngle 180 is no cone)
            if d <= detect && !matches!(self.heat, Heat::Pursuit | Heat::Search | Heat::PullOver | Heat::Busted) {
                let (from, to) = (c.state.pos + Vec3::Y * 2.0, ppos + Vec3::Y * 2.0);
                if world.collider.raycast(from, (to - from).normalize_or(Vec3::X), from.distance(to)).is_none() {
                    detected = true;
                }
            }
            if d < vision {
                let from = c.state.pos + Vec3::Y * 1.2;
                let to = ppos + Vec3::Y * 1.0;
                let dir = (to - from).normalize_or(Vec3::X);
                if d < 12.0 || world.collider.raycast(from, dir, (d - 2.0).max(0.1)).is_none() {
                    seen = true;
                }
            }
        }
        // (a free cop touching the player is a detecting cop as well: F-52240's fallback, the cop's contacts)
        let touched = damage > self.last_damage + 0.004 && vehicles.cars.iter().any(|c| pursuer(c) && c.state.pos.distance(ppos) < 3.4);
        detected |= touched;
        // (F-52240: with a tactic on the player that has a cop, offences count with no sight test; else only a
        // detecting cop counts them - and nothing is scored in the 5 s after a tactic on the player ended)
        let in_tactic = n_cops > 0 && !matches!(self.heat, Heat::None);
        let scoring = self.grace <= 0.0;
        self.grace = (self.grace - dt).max(0.0);
        let watching = scoring && (in_tactic || detected);

        // ---- offences ---------------------------------------------------------
        let mut breaking = false;
        let mut add = 0.0f32;
        // speeding: three bands above the limit
        // (F-36879: the road's limit in mph, `fSpeedLimitLow` where there is none; a band starts
        // at the larger of limit + offset and limit x (100 + percent) / 100)
        let limit = world.nav.as_ref().and_then(|n| n.speed_limit_at(ppos)).filter(|l| *l != 0.0).map_or(t("fSpeedLimitLow", 50.0), |l| l * MPH);
        let band = |off: &str, pct: &str, d_off: f32, d_pct: f32| (limit + t(off, d_off)).max(limit * (t(pct, d_pct) + 100.0) * 0.01);
        let lvl: u8 = if mph > band("fSpeedLimitHiOffset", "iSpeedLimitHiPrcnt", 45.0, 80.0) {
            3
        } else if mph > band("fSpeedLimitMidOffset", "iSpeedLimitMidPrcnt", 30.0, 50.0) {
            2
        } else if mph > band("fSpeedLimitLowOffset", "iSpeedLimitLowPrcnt", 15.0, 1.0) {
            1
        } else {
            0
        };
        if watching && lvl > 0 {
            breaking = true;
            self.offences |= LAW_SPEED;
            self.speed_top = self.speed_top.max(lvl);
            // (F-52240 / F-19359: a band higher than any paid pays its start points that tick and nothing else;
            // then the three bands share one seconds counter, the current band's rate. Dropping to no speeding
            // re-arms nothing: the started bits stay until the record is cleared)
            if lvl > self.speed_level {
                add += match lvl {
                    3 => t("iHighSpeedStartPoints", 600.0),
                    2 => t("iMidSpeedStartPoints", 250.0),
                    _ => t("iLowSpeedStartPoints", 130.0),
                };
                self.speed_level = lvl;
            } else {
                add += self.whole_seconds(0, dt) * match lvl {
                    3 => t("iHighSpeedPerSecPoints", 10.0),
                    2 => t("iMidSpeedPerSecPoints", 10.0),
                    _ => t("iLowSpeedPerSecPoints", 38.0),
                };
            }
        }
        // driving against the traffic
        self.nav_check -= dt;
        if self.nav_check <= 0.0 {
            self.nav_check = 0.2;
            self.against_traffic = false;
            if speed > 6.0 {
                if let Some(nav) = world.nav.as_ref() {
                    if let Some((li, _t, d)) = nav.nearest_link(ppos) {
                        if d < 5.0 && nav.links[li].dir().dot(pvel / speed) < -0.6 {
                            // a lane running our way right next to it means we are just near the centre line
                            let ok_lane = nav.links_near(ppos, 9.0).any(|j| {
                                let l = &nav.links[j];
                                l.dir().dot(pvel / speed) > 0.6 && l.point_at(l.project(ppos)).distance(ppos) < 4.0
                            });
                            self.against_traffic = !ok_lane;
                        }
                    }
                }
            }
        }
        if self.against_traffic {
            self.wrong_way += dt;
        } else {
            self.wrong_way = 0.0;
        }
        // (a started offence is on every frame its condition holds; the threshold only for its start)
        if watching && self.against_traffic && (self.wrong_active || self.wrong_way > t("fWrongWayThresholdTime", 1.2)) {
            breaking = true;
            self.offences |= LAW_WRONG_WAY;
            if !self.wrong_active {
                self.wrong_active = true;
                self.sec[1] = 0.0;
                add += t("iWrongWayStartPoints", 130.0);
            } else {
                add += self.whole_seconds(1, dt) * t("iWrongWayPerSecPoints", 25.0);
            }
        }
        // burn-outs
        if slip > 1.6 && speed < 9.0 {
            self.burnout += dt;
        } else {
            self.burnout = 0.0;
        }
        if watching && self.burnout > 0.0 && (self.burnout_active || self.burnout > t("fBurnOutThresholdTime", 1.5)) {
            breaking = true;
            self.offences |= LAW_BURNOUT;
            if !self.burnout_active {
                self.burnout_active = true;
                self.sec[2] = 0.0;
                add += t("iBurnOutStartPoints", 130.0);
            } else {
                add += self.whole_seconds(2, dt) * t("iBurnOutPerSecPoints", 25.0);
            }
        }
        // off the roads (F-90092: no polygon of the mesh with the ai_road material under the car)
        let off_road = crate::offroad::mesh().map_or(false, |m| !m.on_road(ppos));
        if watching && off_road {
            breaking = true;
            self.offences |= LAW_OFF_ROAD;
            if !self.off_active {
                // (its seconds counter is not reset at the start, as the speed bands')
                self.off_active = true;
                add += t("iOffRoadStartPoints", 50.0);
            } else {
                add += self.whole_seconds(4, dt) * t("iOffRoadPerSecPoints", 50.0);
            }
        }
        // driving on two wheels: start points, then per second
        if watching && p_two {
            breaking = true;
            self.offences |= LAW_TWO_WHEEL;
            if !self.two_active {
                self.two_active = true;
                self.sec[3] = 0.0;
                add += t("iTwoWheelStartPoints", 50.0);
            } else {
                add += self.whole_seconds(3, dt) * t("iTwoWheelPerSecPoints", 50.0);
            }
        }
        // a jump: in the air for longer than fAirJumpThresholdTime (F-29122, the car sim's air
        // time), paid once per jump
        if p_air > t("fAirJumpThresholdTime", 0.8) {
            if watching && !self.jump_paid {
                self.jump_paid = true;
                breaking = true;
                self.offences |= LAW_ROOF_JUMP;
                add += t("iJumpOffRoofTopPoints", 500.0);
            }
        } else {
            self.jump_paid = false;
        }
        // running a red light: approached on red, then drove into the box
        match vehicles.signals.ahead(ppos, pfwd, vehicles.clock) {
            Some((ji, stop, 0)) if stop < 25.0 => self.red_junction = Some(ji),
            Some((_, _, s)) if s != 0 => self.red_junction = None,
            _ => {}
        }
        if let (Some(ji), Some(inside)) = (self.red_junction, vehicles.signals.inside(ppos)) {
            if ji == inside {
                self.red_junction = None;
                if watching && speed > 4.0 {
                    breaking = true;
                    self.offences |= LAW_RED_LIGHT;
                    add += t("iRedLightPoints", 250.0);
                }
            }
        }
        // property damage
        if knocked > self.last_knocked {
            if watching {
                breaking = true;
                self.offences |= LAW_PROP;
                add += (knocked - self.last_knocked) as f32 * t("iDamagePropPoints", 250.0);
            }
            self.last_knocked = knocked;
        }
        // hitting other vehicles
        let mut hit_cop = false;
        if damage > self.last_damage + 0.004 {
            let hit = vehicles.cars.iter().filter(|c| !c.is_player && c.state.pos.distance(ppos) < 3.4).map(|c| c.cop).fold(None, |a: Option<bool>, cop| Some(a.unwrap_or(false) || cop));
            if let Some(cop) = hit {
                if cop {
                    // ramming a cruiser is always noticed
                    hit_cop = true;
                    breaking = true;
                    self.offences |= LAW_POLICE_VEHICLE;
                    add += t("iDamagePoliceVehPoints", 600.0);
                } else if watching {
                    breaking = true;
                    self.offences |= LAW_VEHICLE;
                    add += t("iDamageOtherVehPoints", 130.0);
                }
            }
        }
        self.last_damage = damage;

        // (AddPoints, F-19359: the score is held to iMaxLawBreakingScore, in a pull-over to iPursuitLawBreakScore - 1
        // until its bullhorn has run out, in a pursuit to iMax - 1)
        let max_score = t("iMaxLawBreakingScore", 1000.0);
        let pull = t("iPullOverLawBreakScore", 250.0);
        let cap = match self.heat {
            Heat::PullOver if self.bullhorn != Some(0.0) => t("iPursuitLawBreakScore", 600.0) - 1.0,
            Heat::Pursuit | Heat::Search | Heat::Busted => max_score - 1.0,
            _ => max_score,
        };
        if breaking {
            self.score = (self.score + add).clamp(0.0, cap);
        }
        if scoring {
            // (F-52240 after the offences: under iPullOverLawBreakScore the score falls iNoLawBrokenPerSecPoints a
            // whole second while no offence is counted - iUndetectedPerSecPoints is read by nothing -; from it on it
            // climbs iPullOverPerSecPoints a whole second, offences or not, and never falls)
            if self.score > 0.0 && self.score < pull {
                if !breaking {
                    self.score = (self.score - self.whole_seconds(5, dt) * t("iNoLawBrokenPerSecPoints", 10.0)).max(0.0);
                    if self.score <= 0.0 {
                        self.offences = 0;
                    }
                }
            } else if self.score >= pull && self.score < max_score {
                let n = self.whole_seconds(6, dt);
                if n > 0.0 {
                    self.score = (self.score + n * t("iPullOverPerSecPoints", 10.0)).clamp(0.0, cap);
                }
            }
        }

        // ---- state machine ----------------------------------------------------
        let mut event = None;
        let arrest_speed = t("fArrestTgtThresholdSpd", 10.0) / MPH;
        match self.heat {
            Heat::None => {
                // (F-52240's ladder needs a detecting cop; StartTactic, F-33550, raises the score to the tactic's
                // floor - the zone's scripted chase too)
                let zone = std::mem::take(&mut self.zone_go);
                if zone || (detected && self.score >= t("iPursuitLawBreakScore", 600.0)) {
                    self.score = self.score.max(t("iPursuitLawBreakScore", 600.0));
                    self.heat = Heat::Pursuit;
                    self.last_known = ppos;
                    self.fine = self.law_fines();
                    self.lost = 0.0;
                    self.dispatch = t("fDispatchRdoDelayTime", 3.0);
                    event = Some(Event::Pursuit);
                } else if detected && self.score >= pull {
                    self.heat = Heat::PullOver;
                    self.bullhorn = None;
                    self.ticket_hold = None;
                    self.arrest = 0.0;
                    event = Some(Event::PullOver);
                }
            }
            Heat::PullOver => {
                // (F-22530's pull-over block: at iBullhornLawBreakScore the bullhorn and fBullhornTime; hitting a
                // police car turns it into a pursuit at once; else the score - held at 599 until the bullhorn has run
                // out - reaching iPursuitLawBreakScore does)
                if self.bullhorn.is_none() && self.score >= t("iBullhornLawBreakScore", 500.0) {
                    self.bullhorn = Some(t("fBullhornTime", 10.0));
                    log::info!("police: bullhorn");
                }
                if let Some(b) = self.bullhorn.as_mut() {
                    *b = (*b - dt).max(0.0);
                }
                if let Some(h) = self.ticket_hold.as_mut() {
                    // (StateTicket: the cop halted, fArrestStateTime - 0.5 s more when the car is damaged out -,
                    // then the tactic's result and COP_PULLOVER)
                    *h -= dt;
                    if *h <= 0.0 {
                        self.ticket_hold = None;
                        self.ticketing = true;
                        event = Some(Event::Ticket(self.law_fines() as i64));
                    }
                } else if hit_cop || self.score >= t("iPursuitLawBreakScore", 600.0) {
                    self.heat = Heat::Pursuit;
                    self.last_known = ppos;
                    self.fine = self.law_fines();
                    self.lost = 0.0;
                    self.dispatch = t("fDispatchRdoDelayTime", 3.0);
                    event = Some(Event::Pursuit);
                } else if speed < arrest_speed && nearest < t("fPullOverDist", 15.0) + 1.0 {
                    // (F-29906's arrest timer: a cop of the tactic within its radius + 1 m - the pull-over's
                    // fPullOverDist, F-66453 -, the target under fArrestTgtThresholdSpd)
                    self.arrest += dt;
                    if self.arrest >= t("fArrestTime", 1.0) {
                        self.ticket_hold = Some(t("fArrestStateTime", 1.75) + if damage >= 0.999 { 0.5 } else { 0.0 });
                        // (what the ticket is for - the panel lists it: 1 reckless driving,
                        // 2 traffic violation, 4 destruction of property)
                        self.ticket_for = (if self.offences & (LAW_BURNOUT | LAW_VEHICLE | LAW_POLICE_VEHICLE) != 0 { 1 } else { 0 }) | (if self.offences & (LAW_SPEED | LAW_WRONG_WAY | LAW_RED_LIGHT) != 0 { 2 } else { 0 }) | (if self.offences & LAW_PROP != 0 { 4 } else { 0 });
                    }
                } else {
                    self.arrest = 0.0;
                }
            }
            Heat::Pursuit => {
                self.fine = self.law_fines();
                if seen {
                    self.lost = 0.0;
                    self.last_known = ppos;
                } else {
                    self.lost += dt;
                    if self.lost > t("fLostTgtThresholdTime", 1.5) {
                        self.heat = Heat::Search;
                        self.search = 0.0;
                        self.seen_time = 0.0;
                        self.radio = t("fMinPerRdoTime", 15.0);
                    }
                }
                // (F-29906: within the pursuit's radius, fSafeDist (F-54202), + 1 m)
                if speed < arrest_speed && nearest < t("fSafeDist", 30.0) + 1.0 {
                    self.arrest += dt;
                    if self.arrest >= t("fArrestTime", 1.0) {
                        self.heat = Heat::Busted;
                        // (F-56938: the arrest ends after fArrestStateTime, 0.5 s longer when the
                        // target is damaged out - the enter state F-69250 only stops a car whose damage
                        // points are under its MaxDamage and then sets the tactic byte +68)
                        self.busted_timer = t("fArrestStateTime", 1.75) + if damage >= 0.999 { 0.5 } else { 0.0 };
                        event = Some(Event::Busted(self.fine as i64));
                    }
                } else {
                    self.arrest = 0.0;
                }
            }
            Heat::Search => {
                self.search += dt;
                // periodic radio reports: someone may have called in where the target is
                self.radio -= dt;
                if self.radio <= 0.0 {
                    let (lo, hi) = (t("fMinPerRdoTime", 15.0), t("fMaxPerRdoTime", 30.0));
                    self.radio = lo + (hi - lo) * self.rand();
                    if self.rand() * 100.0 < t("iPerRdoTgtLocProbPrcnt", 70.0) {
                        self.last_known = ppos;
                    }
                }
                if seen {
                    self.seen_time += dt;
                    if self.seen_time > t("fReacquireTgtThresholdTime", 0.3) {
                        self.heat = Heat::Pursuit;
                        self.lost = 0.0;
                    }
                } else {
                    self.seen_time = 0.0;
                }
                if self.heat == Heat::Search && self.search > t("fSearchMaxTime", 35.0) {
                    self.clear();
                    self.grace = 5.0;
                    event = Some(Event::Escaped);
                }
            }
            Heat::Busted => {
                self.busted_timer -= dt;
                if self.busted_timer <= 0.0 {
                    // (step 19d: the payback's chasers stay until their script takes them away)
                    let goons = self.goons;
                    self.clear();
                    self.grace = 5.0;
                    self.goons = goons;
                    // (F-22530: the arrest tactic succeeded - for a cops' tactic, not the payback's
                    // goons, a race running gets raceOverCommand `returnToCruise`)
                    self.arrest_over = !goons;
                    // the cruisers drive off
                    vehicles.cars.retain(|c| !c.cop || c.state.pos.distance(ppos) > 60.0);
                }
            }
        }

        // ---- cruisers ---------------------------------------------------------
        let chasing = matches!(self.heat, Heat::Pursuit | Heat::Search | Heat::PullOver | Heat::Busted);
        self.flee_hold = (self.flee_hold - dt).max(0.0);
        if !matches!(self.heat, Heat::Pursuit | Heat::PullOver) {
            self.tacs.clear();
        }
        let tactic = matches!(self.heat, Heat::Pursuit | Heat::PullOver) && self.flee_hold <= 0.0;
        let pull = self.heat == Heat::PullOver;
        let safe = t("fSafeDist", 30.0);
        let along = t("fAlongSideDist", 5.0);
        let period = t("fMinSetGoalTime", 1.0);
        let moving_v = t("fMovingSpeed", 1.0) * 0.447;
        let t_moving = speed > moving_v;
        let dropped = self.last_tspeed - speed;
        self.last_tspeed = speed;
        // (every cop of the tactic within r of the target: F-90575, F-36876)
        let within = |r: f32| vehicles.cars.iter().filter(|c| pursuer(c)).all(|c| c.state.pos.distance(ppos) < r);
        let (all_block, all_halt) = (within(0.5 * safe), within(safe - 1.0));
        let on_road = |q: Vec3| crate::offroad::mesh().map_or(true, |m| m.on_road(q));
        for c in vehicles.cars.iter_mut().filter(|c| pursuer(c)) {
            let Some(ai) = c.ai.as_mut() else { continue };
            ai.chase_exact = false;
            if self.flee_hold > 0.0 && self.heat == Heat::Pursuit {
                // (the Delay state: brain command 1, halt, until its time is up)
                ai.chase = Some((c.state.pos, Vec3::ZERO));
            } else if tactic {
                // ---- the tactic's state and its brain (docs/reference/hookmen/pursuit_tactics.md) ----
                let tac = self.tacs.entry(c.uid).or_default();
                let (cpos, cvel, cfwd, cright) = (c.state.pos, c.state.vel, c.state.forward(), c.state.right());
                let cspeed = cvel.length();
                let to = ppos - cpos;
                let d = to.length();
                // (the brain's predicates, F-74140's helpers: in front of the cop, on its right, going the same way,
                // facing the same way, the angle off the cop's right axis, and "closing in": the cop moving, faster
                // than the target and heading at it within 25.8 deg)
                let ahead = cfwd.dot(to) > 0.0;
                let on_right = cright.dot(to) > 0.0;
                let same_dir = pvel.dot(cvel) > 0.0;
                let same_facing = pfwd.dot(cfwd) > 0.0;
                let bearing = cright.angle_between(to).to_degrees();
                let closing = |r: f32| (r == 0.0 || d < r) && cspeed > moving_v && cspeed > speed && to.normalize_or_zero().dot(cvel / cspeed.max(1e-6)) > 0.9;
                // (head-on, +774: in front, coming the other way, seen, travel directions more than fHeadOnAngle apart)
                let tdir = if t_moving { pvel.normalize_or_zero() } else { pfwd };
                let cdir = if cspeed > moving_v { cvel / cspeed } else { cfwd };
                let was_head_on = tac.head_on;
                tac.head_on = ahead && !same_dir && !same_facing && seen && tdir.angle_between(cdir).to_degrees() > t("fHeadOnAngle", 145.0);
                // (F-14848: beside the target on the side over the road, the right first; else the side the cop is on)
                let pick_side = || {
                    let (r, l) = (on_road(ppos + pright * along), on_road(ppos - pright * along));
                    if r { true } else if l { false } else { !on_right }
                };
                let enter = |tac: &mut CopTac, st: Tac| {
                    tac.st = st;
                    tac.t = 0.0;
                    tac.far_t = 0.0;
                    tac.side_t = 0.0;
                    tac.grow_t = 0.0;
                    tac.replan = 0.0;
                };
                tac.t += dt;
                let st = tac.st;
                if pull {
                    // pull-over (F-36876; CatchUp F-91234, AvoidCollision F-18622 / F-69591, Halt F-82776)
                    if st != Tac::Halt && all_halt && !t_moving && (cspeed < 4.47 || !closing(0.0)) {
                        enter(tac, Tac::Halt);
                    } else {
                        match st {
                            Tac::CatchUp if tac.t > 0.5 && closing(50.0) => {
                                enter(tac, Tac::Avoid);
                                tac.side = pick_side();
                                // (on entering: the cop's velocity x 0.7, F-19124)
                                c.state.vel *= 0.7;
                            }
                            Tac::Avoid => {
                                tac.side_t += dt;
                                if tac.side_t >= 1.0 {
                                    tac.side_t = 0.0;
                                    tac.side = pick_side();
                                }
                                if tac.t > 0.5 && d >= 50.0 {
                                    enter(tac, Tac::CatchUp);
                                }
                            }
                            Tac::Halt => {
                                tac.grow_t = if t_moving && d > tac.d_prev { tac.grow_t + dt } else { 0.0 };
                                if tac.grow_t > 0.5 || d >= safe {
                                    enter(tac, Tac::CatchUp);
                                }
                            }
                            _ => {}
                        }
                    }
                    // (every update: a stopped target and the cop closing in within 7 m - its velocity halved)
                    if !t_moving && closing(7.0) {
                        c.state.vel *= 0.5;
                    }
                } else {
                    // pursuit (F-68032; CatchUp F-88702, AlongSide F-85137, Block F-85039, Halt F-84018)
                    if !matches!(st, Tac::Halt | Tac::Block) && dropped > 4.0 && d < 5.0 {
                        // (the target just stopped hard beside it)
                        enter(tac, Tac::Halt);
                    } else {
                        match st {
                            Tac::CatchUp if d < safe => {
                                if !t_moving {
                                    enter(tac, Tac::Halt);
                                } else if tac.t > 3.0 && same_dir && ahead && bearing > 75.0 && bearing < 105.0 {
                                    enter(tac, Tac::AlongSide);
                                    tac.side = pick_side();
                                }
                            }
                            Tac::AlongSide => {
                                if d >= 1.5 * along {
                                    tac.far_t += dt;
                                    if tac.far_t > 10.0 {
                                        enter(tac, Tac::CatchUp);
                                    }
                                } else {
                                    tac.far_t = 0.0;
                                    tac.side_t += dt;
                                    if tac.side_t >= 1.0 {
                                        tac.side_t = 0.0;
                                        tac.side = pick_side();
                                    }
                                }
                                // (on its side - the target on the cop's other hand -, level or ahead, facing the same way)
                                if tac.st == Tac::AlongSide && same_facing && on_right != tac.side && !ahead {
                                    enter(tac, Tac::Block);
                                }
                            }
                            Tac::Block if !all_block => enter(tac, Tac::CatchUp),
                            Tac::Halt if !(d < safe && speed * MPH < t("fArrestTgtThresholdSpd", 10.0)) => enter(tac, Tac::CatchUp),
                            _ => {}
                        }
                    }
                }
                if tac.st != st {
                    log::info!("police: cruiser {} {:?} -> {:?} ({:.0} m)", c.uid, st, tac.st, d);
                }
                tac.d_prev = d;
                // ---- the goal plan (F-67303): every fMinSetGoalTime, half that head-on; at once when the target has
                // turned more than 30 deg or changed speed by more than 15 mph (then its own place and speed) ----
                let per = if tac.head_on { 0.5 * period } else { period };
                tac.replan -= dt;
                let turned = tac.plan_fwd != Vec3::ZERO && tac.plan_fwd.angle_between(pfwd) > 0.5236;
                let changed = (speed - tac.plan_speed).abs() > 6.705;
                if tac.replan <= 0.0 || (was_head_on && !tac.head_on) || (tac.replan < 0.3 * per && (turned || changed)) {
                    let quick = tac.replan > 0.0 && !(was_head_on && !tac.head_on);
                    tac.replan = per;
                    tac.plan_fwd = pfwd;
                    tac.plan_speed = speed;
                    let ho = if tac.head_on { 2.0 } else { 1.0 };
                    let (g, v) = if quick {
                        // (the junction prediction on a turn is not ported: the target's place)
                        ([ppos, ppos], speed)
                    } else {
                        match (tac.st, pull) {
                            // Pursuit brain (F-15067): 1 and 2 periods ahead; a stopped target: fSafeDist off it on the
                            // cop's side. Arrival speed: full pace beyond 60 m, 1.2 x the target's when behind it going
                            // the same way, nothing (full pace) otherwise
                            (Tac::CatchUp, false) => {
                                if t_moving {
                                    let g0 = ppos + pvel * per * ho;
                                    ([g0, g0 + pvel * per * ho], if d < 2.0 * safe && same_dir && ahead { 1.2 * speed } else { -1.0 })
                                } else {
                                    let g = ppos + (cpos - ppos).normalize_or(-pfwd) * safe;
                                    ([g, g], 0.0)
                                }
                            }
                            // PullOver brain (F-50661's command; goal F-51787): the target itself, or fPullOverDist behind it stopped
                            (Tac::CatchUp, true) => {
                                let g = if t_moving { ppos } else { ppos - pfwd * t("fPullOverDist", 15.0) };
                                ([g, g], if t_moving { speed } else { 0.0 })
                            }
                            // AlongSide brain (F-13443): fAlongSideDist beside the target, 1.15 periods ahead
                            (Tac::AlongSide | Tac::Avoid, _) => {
                                let b = ppos + pright * if tac.side { along } else { -along };
                                let k = 1.15 * per * ho;
                                let g0 = b + pvel * k;
                                ([g0, g0 + pvel * k], if t_moving { speed } else { 0.0 })
                            }
                            // Stopping (Block, Halt): stay
                            _ => ([ppos, ppos], 0.0),
                        }
                    };
                    tac.goals = g;
                    tac.speed = v;
                }
                // the goals in turn, 5 m each (the racing brain's goal pair, F-67303)
                let goal = if cpos.distance(tac.goals[0]) < 5.0 { tac.goals[1] } else { tac.goals[0] };
                ai.chase = Some((goal, Vec3::ZERO));
                ai.chase_exact = true;
                // (the speed to arrive with: -1 is the racer's full pace; the port's driver has a cruise speed, not an
                // arrival speed - it gets the speed it can still brake from to that one over the distance left)
                ai.target_speed = if tac.speed < 0.0 { 85.0 } else { (tac.speed * tac.speed + 2.0 * 7.0 * cpos.distance(goal)).sqrt().min(85.0) };
                // ---- the brain's overrides (F-88118 puts them over the driver's input) ----
                let away = Some(if on_right { -1.0 } else { 1.0 });
                ai.ovr = match (tac.st, pull) {
                    (Tac::CatchUp, false) if (!t_moving && d < safe) || (ahead && same_dir && d < 20.0 && cspeed > speed) => Some(crate::ai::Overrides { brake: Some(1.0), ..Default::default() }),
                    (Tac::AlongSide | Tac::Avoid, _) if (!t_moving && d < along + 1.0) || (t_moving && same_dir && !ahead && cspeed > speed && d > 1.5 * along) => Some(crate::ai::Overrides { brake: Some(1.0), ..Default::default() }),
                    // Block (F-56982): handbrake, no throttle, full lock towards the target
                    (Tac::Block, _) => Some(crate::ai::Overrides { handbrake: Some(true), throttle: Some(0.0), steer: Some(if on_right { 1.0 } else { -1.0 }), brake: None }),
                    // Halt (F-29519): stopped beside the target; steering away while closing in
                    (Tac::Halt, false) => Some(crate::ai::Overrides { brake: (d < safe).then_some(1.0), steer: if closing(safe) { away } else { None }, ..Default::default() }),
                    (Tac::Halt, true) => Some(crate::ai::Overrides { brake: Some(1.0), ..Default::default() }),
                    _ => None,
                };
            } else if let Some(ai) = Some(ai) {
                let target = match self.heat {
                    Heat::Search => (self.last_known, Vec3::ZERO),
                    _ => (ppos, pvel),
                };
                ai.ovr = None;
                if chasing && self.heat != Heat::Busted {
                    ai.chase = Some(target);
                    ai.target_speed = 22.0;
                } else if self.heat == Heat::Busted {
                    ai.chase = Some((c.state.pos, Vec3::ZERO));
                } else if ai.chase.is_some() {
                    ai.chase = None;
                    ai.path = None;
                    ai.link = None;
                    ai.target_speed = 14.0;
                }
            }
        }
        // patrol / dispatch
        self.spawn_timer -= dt;
        self.dispatch -= dt;
        if self.spawn_timer <= 0.0 && !self.goons {
            match self.heat {
                Heat::None | Heat::PullOver => {
                    if n_cops == 0 {
                        // a patrol route passing nearby, else any street
                        let near = self.paths.iter().flatten().copied().filter(|q| {
                            let d = q.distance(ppos);
                            d > 150.0 && d < 380.0
                        }).min_by(|a, b| a.distance(ppos).partial_cmp(&b.distance(ppos)).unwrap_or(std::cmp::Ordering::Equal));
                        let ok = match near {
                            Some(q) => self.spawn_cop(gpu, world, vehicles, ppos, pfwd, 120.0, 400.0, Some(q)),
                            None => self.rand() < 0.5 && self.spawn_cop(gpu, world, vehicles, ppos, pfwd, 150.0, 300.0, None),
                        };
                        self.spawn_timer = if ok { 20.0 } else { 6.0 };
                    } else {
                        self.spawn_timer = 10.0;
                    }
                }
                Heat::Pursuit | Heat::Search => {
                    let want = if self.fine > 1500.0 { 3 } else { 2 };
                    if n_cops < want && self.dispatch <= 0.0 {
                        if self.spawn_cop(gpu, world, vehicles, ppos, pfwd, 130.0, 260.0, None) {
                            self.dispatch = 7.0;
                            // (an underground unit is streamed in for the pursuit: it is that car)
                            if let Some(u) = self.units.iter_mut().find(|u| u.under) {
                                u.under = false;
                                if let Some(c) = vehicles.cars.last_mut().filter(|c| c.cop) {
                                    c.vinyl_name = format!("police_unit:{}", u.id);
                                }
                                log::info!("police: unit {} streamed in for the pursuit", u.id);
                            }
                        }
                    }
                    self.spawn_timer = 1.0;
                }
                Heat::Busted => {}
            }
        }
        self.tun = tun;
        self.update_glows(gpu, vehicles);
        event
    }

    /// The fine (F-83946, the routine that fills the arrest form): the highest speed band's
    /// `iLawFines` entry, plus the entry of every other offence (bit n of the original's word
    /// = entry n; a hit on a cruiser sets 0x20 and 0x40, the second one is taken back off),
    /// plus for a pursuit `iPerSecPursuitFine` x its length (rounded half away from zero) and
    /// `iPullOverFleeFine` after running from a pull-over; cut to 1/4, 1/3, 1/2 below 25 %,
    /// 50 %, 75 % of the career, at least 100, at most `iMaxFine`.
    /// The whole seconds that counter `i` has run through (the original subtracts 1 s while the
    /// counter stands above 1 s).
    fn whole_seconds(&mut self, i: usize, dt: f32) -> f32 {
        self.sec[i] += dt;
        let mut n = 0.0;
        while self.sec[i] > 1.0 {
            self.sec[i] -= 1.0;
            n += 1.0;
        }
        n
    }

    fn law_fines(&self) -> f32 {
        let t = |i: usize| self.tun.array_at("iLawFines", i);
        let mut f = 0i32;
        if self.speed_top > 0 {
            f += t(self.speed_top as usize - 1);
        }
        for (flag, entry) in [(LAW_PROP, 4), (LAW_VEHICLE, 7), (LAW_POLICE_VEHICLE, 5) /* (0x20 + 0x40 - entry 6) */, (LAW_RED_LIGHT, 9), (LAW_BURNOUT, 11), (LAW_WRONG_WAY, 13), (LAW_ROOF_JUMP, 12), (LAW_TWO_WHEEL, 10), (LAW_OFF_ROAD, 3)] {
            if self.offences & flag != 0 {
                f += t(entry);
            }
        }
        if matches!(self.heat, Heat::Pursuit | Heat::Search | Heat::Busted) {
            let x = self.tun.get("iPerSecPursuitFine", 10.0) * self.chase_time;
            f += (if x < 0.0 { x - 0.5 } else { x + 0.5 }) as i32;
            if self.fled {
                f += self.tun.get("iPullOverFleeFine", 300.0) as i32;
            }
        }
        f = if self.progress < 0.25 {
            f / 4
        } else if self.progress < 0.5 {
            f / 3
        } else if self.progress < 0.75 {
            f / 2
        } else {
            f
        };
        if f < 100 {
            return 100.0;
        }
        f.min(self.tun.get("iMaxFine", 5000.0) as i32) as f32
    }

    fn clear(&mut self) {
        self.heat = Heat::None;
        self.score = 0.0;
        self.fine = 0.0;
        self.offences = 0;
        self.speed_level = 0;
        self.speed_top = 0;
        self.fled = false;
        // (F-88677 clears the whole record: the started bits and the counters too)
        self.off_active = false;
        self.two_active = false;
        self.wrong_active = false;
        self.burnout_active = false;
        self.sec = [0.0; 7];
        self.bullhorn = None;
        self.ticket_hold = None;
        self.ticketing = false;
        self.flee_hold = 0.0;
        self.arrest = 0.0;
        self.lost = 0.0;
        self.search = 0.0;
        self.spawn_timer = 12.0;
    }

    /// Roof light positions and colours (red / blue, alternating) of the cruisers with
    /// their lights on.
    pub fn light_bar(&self, vehicles: &Vehicles) -> Vec<(Vec3, [f32; 3])> {
        let mut out = Vec::new();
        if !self.lights_on() {
            return out;
        }
        let phase = (self.time * 5.0) as i32 % 2 == 0;
        for c in vehicles.cars.iter().filter(|c| c.cop) {
            let up = c.state.up();
            let right = c.state.right();
            let top = c.state.pos + up * 1.30 - c.state.forward() * 0.45;
            let (l, r) = if phase { ([1.0, 0.05, 0.05], [0.0, 0.0, 0.0]) } else { ([0.0, 0.0, 0.0], [0.1, 0.25, 1.0]) };
            if phase {
                out.push((top - right * 0.38, l));
            } else {
                out.push((top + right * 0.38, [0.1, 0.25, 1.0]));
            }
            let _ = r;
        }
        out
    }

    fn update_glows(&mut self, gpu: &Gpu, vehicles: &Vehicles) {
        let bar = self.light_bar(vehicles);
        let mut insts = [Inst { pos_scale: [0.0; 4], rot_tint: [0.0; 4], fill: [0; 4], ..Inst::NONE }; MAX_GLOWS];
        let n = bar.len().min(MAX_GLOWS);
        for (i, (p, c)) in bar.iter().take(n).enumerate() {
            insts[i] = Inst { pos_scale: [p.x, p.y, p.z, -0.55], rot_tint: [c[0], c[1], c[2], 2.2], fill: [0; 4], ..Inst::NONE };
        }
        if self.glow_buf.is_none() {
            self.glow_buf = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("police glows"),
                size: (std::mem::size_of::<Inst>() * MAX_GLOWS) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(b) = self.glow_buf.as_ref() {
            gpu.queue.write_buffer(b, 0, bytemuck::cast_slice(&insts));
        }
        self.glow_count = n as u32;
    }

    /// Additive light-bar flares. Sets the glow pipeline.
    pub fn draw_glows<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu) {
        let Some(buf) = self.glow_buf.as_ref() else { return };
        if self.glow_count == 0 {
            return;
        }
        pass.set_pipeline(&gpu.pipeline_glow);
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.draw(0..4, 0..self.glow_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cop(arrays: &[(&str, Vec<i32>)]) -> Police {
        let mut p = Police::new();
        for (k, v) in arrays {
            p.tun.arrays.insert(k.to_string(), v.clone());
        }
        p.tun.v.insert("iMaxFine".into(), 5000.0);
        p.tun.v.insert("iPerSecPursuitFine".into(), 10.0);
        p.tun.v.insert("iPullOverFleeFine".into(), 300.0);
        p
    }

    /// F-83946: the sum over the offences, the pursuit's seconds, the flee fine, the career cut
    /// and the clamp, as the routine reads them.
    #[test]
    fn fine_of_the_original() {
        let mut p = cop(&[("iLawFines", vec![100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, 210, 220, 230])]);
        // one speed band (the highest, entry 2), nothing else: 120
        p.offences = LAW_SPEED;
        p.speed_top = 3;
        assert_eq!(p.law_fines(), 120.0);
        // plus a cruiser hit (entry 5, the 0x40 entry cancels) and a prop (entry 4)
        p.offences |= LAW_POLICE_VEHICLE | LAW_PROP;
        assert_eq!(p.law_fines(), 120.0 + 150.0 + 140.0);
        // a 20.04 s pursuit adds round(200.4) = 200, running from the pull-over 300
        p.heat = Heat::Pursuit;
        p.chase_time = 20.04;
        p.fled = true;
        assert_eq!(p.law_fines(), 410.0 + 200.0 + 300.0);
        // the career cuts it
        p.progress = 0.6;
        assert_eq!(p.law_fines(), 455.0);
        p.progress = 0.3;
        assert_eq!(p.law_fines(), 303.0);
        p.progress = 0.1;
        assert_eq!(p.law_fines(), 227.0);
        // never under 100, never over iMaxFine
        let mut q = cop(&[("iLawFines", vec![100; 14])]);
        q.progress = 0.0;
        q.offences = LAW_SPEED;
        q.speed_top = 1;
        assert_eq!(q.law_fines(), 100.0);
        q.progress = 1.0;
        q.heat = Heat::Pursuit;
        q.chase_time = 1000.0;
        assert_eq!(q.law_fines(), 5000.0);
    }
}

/// The arrest's scene (CopArrest.sco 0xc33 - 0xeef): one cop in the latest tactic -> `cut_cop_arrest` or `_07`
/// (RAND_INT_RANGE(0, 1), upper bound included: F-32828), more -> `_02`, `_03` or `_08` (RAND_INT_RANGE(0, 2)).
/// `r` stands in for the game's random number. (The payback's goons scenes and the wiped-out biker's
/// `cut_cop_arrest_bikestand` are not ported.)
pub fn arrest_scene(cops: usize, r: u32) -> &'static str {
    if cops <= 1 {
        ["cut_cop_arrest_07", "cut_cop_arrest"][(r % 2) as usize]
    } else {
        ["cut_cop_arrest_02", "cut_cop_arrest_03", "cut_cop_arrest_08"][(r % 3) as usize]
    }
}

/// When the arrest scene's `gesture_cophandsup` puts the player's hands up: (start, duration) in the scene's
/// seconds, from the generated scene scripts' event records (slot 0 = the player). `cut_cop_arrest` aims its
/// one at slot 1, the first cop car, whose driver is off - so no hands up there.
pub fn arrest_hands_up(scene: &str) -> Option<(f32, f32)> {
    Some(match scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase().as_str() {
        "cut_cop_arrest_02" => (1.8333, 4.1667),
        "cut_cop_arrest_03" => (1.3333, 3.6667),
        "cut_cop_arrest_07" => (3.5, 1.5),
        "cut_cop_arrest_08" => (0.0, 6.6667),
        _ => return None,
    })
}

/// The police scenes' place (CopPullover.sco 0x54c, CopArrest.sco 0xa1f): the player's car put upright where it
/// stands with its heading (Racer_InitializeResetPosition, F-40200, and ResetWithDamageIntact, F-67152), and the
/// scene's frame - that heading alone, snapped to the ground from 5 m above to 5 m below and leaned with it when it
/// is off by more than 0.002 rad (SetGameCoordSysBindingCopyRacer, F-94483: F-88556, F-33119); never the car's
/// own pitch or roll. (The nearest scene safezone or road point, FindClosestSceneSafezone, is not ported: the
/// car's own place, the original's "none found".)
pub fn scene_frame(vehicles: &mut Vehicles, world: &World) -> Option<glam::Mat4> {
    let c = vehicles.player_mut()?;
    let f = c.state.forward();
    let q = glam::Quat::from_rotation_y((-f.x).atan2(-f.z));
    let pos = c.state.pos;
    if let Some(vt) = c.params().veh.clone() {
        crate::physics::vehsim::set_pose(&mut c.state, &vt, &world.collider, [q * Vec3::X, q * Vec3::Y, q * Vec3::Z, pos], Vec3::ZERO, Vec3::ZERO);
    } else {
        c.state.rot = q;
        c.state.vel = Vec3::ZERO;
        c.state.ang_vel = Vec3::ZERO;
    }
    c.pose_history.reset();
    let (place, lean) = match world.collider.ground(pos, 5.0, 5.0) {
        Some(h) if h.normal.angle_between(Vec3::Y) > 0.002 => (Vec3::new(pos.x, h.point.y, pos.z), glam::Quat::from_rotation_arc(Vec3::Y, h.normal.normalize())),
        Some(h) => (Vec3::new(pos.x, h.point.y, pos.z), glam::Quat::IDENTITY),
        None => (pos, glam::Quat::IDENTITY),
    };
    Some(glam::Mat4::from_rotation_translation(lean * q, place))
}

/// The ticket scene after the officer's walk (CopPullover.sco 0x57f - 0x75b): a motorcycle's `cut_cop_fidget_bike`,
/// a car whose type name holds `_lr_` `sc_cut_cop_ticket_suv`, else `cut_cop_fidget_02`.
pub fn ticket_scene(car: &str, bike: bool) -> &'static str {
    if bike {
        "cut_cop_fidget_bike"
    } else if car.to_ascii_lowercase().contains("_lr_") {
        "sc_cut_cop_ticket_suv"
    } else {
        "cut_cop_fidget_02"
    }
}

#[cfg(test)]
mod scene_tests {
    use super::*;

    #[test]
    fn the_arrest_scene_goes_by_the_cops_of_the_chase() {
        // (CopArrest.sco: RAND_INT_RANGE's upper bound is included - two scenes for one cop, three for more)
        assert_eq!([0, 1].map(|r| arrest_scene(1, r)), ["cut_cop_arrest_07", "cut_cop_arrest"]);
        assert_eq!([0, 1, 2].map(|r| arrest_scene(3, r)), ["cut_cop_arrest_02", "cut_cop_arrest_03", "cut_cop_arrest_08"]);
        assert_eq!(arrest_hands_up("cut_cop_arrest"), None);
        assert_eq!(arrest_hands_up("cutscene/CUT_COP_ARREST_07"), Some((3.5, 1.5)));
    }

    #[test]
    fn the_ticket_scene_goes_by_the_car() {
        assert_eq!(ticket_scene("vp_dge_charger_69", false), "cut_cop_fidget_02");
        assert_eq!(ticket_scene("vp_cad_escalade_lr_06", false), "sc_cut_cop_ticket_suv");
        assert_eq!(ticket_scene("vp_duc_monster_09", true), "cut_cop_fidget_bike");
    }
}
