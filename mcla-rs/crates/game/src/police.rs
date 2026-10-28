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
    pullover_time: f32,
    speed_level: u8,
    /// the highest speed band of the offences so far (1..3; 0 = none): the fine counts that one
    speed_top: u8,
    /// the seconds counters of the per-second points (F-52240: they pay whole seconds): 0 speed,
    /// 1 wrong way, 2 burn-out, 3 two-wheel driving, 4 off-road
    sec: [f32; 5],
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
            pullover_time: 0.0,
            speed_level: 0,
            speed_top: 0,
            sec: [0.0; 5],
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

    /// Test helper: start a pursuit right now with a cruiser close by.
    pub fn debug_force(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles) {
        self.score = 700.0;
        self.dispatch = 0.0;
        if let Some((pos, fwd)) = vehicles.player().map(|c| (c.state.pos, c.state.forward())) {
            let ok = self.spawn_cop(gpu, world, vehicles, pos, fwd, 18.0, 80.0, None);
            log::info!("police: test cruiser spawned: {}", ok);
        }
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
    /// as the original's own showroom picture of it (GitHub #17: the port painted it all white). Which set-up the
    /// original's police manager gives its cars was not traced in its code.
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
        if (!self.enabled && !self.goons) || dt <= 0.0 {
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
            if d < detect {
                detected = true;
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
        let watching = detected || (matches!(self.heat, Heat::Pursuit | Heat::PullOver) && seen);

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
            if lvl > self.speed_level {
                add += match lvl {
                    3 => t("iHighSpeedStartPoints", 600.0),
                    2 => t("iMidSpeedStartPoints", 250.0),
                    _ => t("iLowSpeedStartPoints", 130.0),
                };
            }
            add += self.whole_seconds(0, dt) * match lvl {
                3 => t("iHighSpeedPerSecPoints", 10.0),
                2 => t("iMidSpeedPerSecPoints", 10.0),
                _ => t("iLowSpeedPerSecPoints", 38.0),
            };
            self.speed_level = lvl;
        } else if lvl == 0 {
            self.speed_level = 0;
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
            self.wrong_active = false;
        }
        if watching && self.wrong_way > t("fWrongWayThresholdTime", 1.2) {
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
            self.burnout_active = false;
        }
        if watching && self.burnout > t("fBurnOutThresholdTime", 1.5) {
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
                self.off_active = true;
                self.sec[4] = 0.0;
                add += t("iOffRoadStartPoints", 50.0);
            } else {
                add += self.whole_seconds(4, dt) * t("iOffRoadPerSecPoints", 50.0);
            }
        } else if !off_road {
            self.off_active = false;
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
        } else if !p_two {
            self.two_active = false;
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
        if damage > self.last_damage + 0.004 {
            let hit = vehicles.cars.iter().filter(|c| !c.is_player && c.state.pos.distance(ppos) < 3.4).map(|c| c.cop).fold(None, |a: Option<bool>, cop| Some(a.unwrap_or(false) || cop));
            if let Some(cop) = hit {
                if cop {
                    // ramming a cruiser is always noticed
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

        let max_score = t("iMaxLawBreakingScore", 1000.0);
        if breaking {
            self.score = (self.score + add).min(max_score);
        } else if !matches!(self.heat, Heat::Pursuit | Heat::Search) {
            let rate = if detected { t("iNoLawBrokenPerSecPoints", 10.0) } else { t("iUndetectedPerSecPoints", 30.0) };
            self.score = (self.score - rate * dt).max(0.0);
            if self.score <= 0.0 {
                self.offences = 0;
            }
        }

        // ---- state machine ----------------------------------------------------
        let mut event = None;
        let arrest_speed = t("fArrestTgtThresholdSpd", 10.0) / MPH;
        match self.heat {
            Heat::None => {
                if self.score >= t("iPursuitLawBreakScore", 600.0) {
                    self.heat = Heat::Pursuit;
                    self.last_known = ppos;
                    self.fine = self.law_fines();
                    self.lost = 0.0;
                    self.dispatch = t("fDispatchRdoDelayTime", 3.0);
                    event = Some(Event::Pursuit);
                } else if self.score >= t("iPullOverLawBreakScore", 250.0) {
                    self.heat = Heat::PullOver;
                    self.pullover_time = 0.0;
                    self.arrest = 0.0;
                    event = Some(Event::PullOver);
                }
            }
            Heat::PullOver => {
                self.pullover_time += dt;
                if speed < arrest_speed && nearest < t("fPullOverDist", 15.0) {
                    self.arrest += dt;
                    if self.arrest >= t("fArrestTime", 1.0) {
                        let fine = self.law_fines() as i64;
                        // (what the ticket is for - the panel lists it: 1 reckless driving,
                        // 2 traffic violation, 4 destruction of property)
                        self.ticket_for = (if self.offences & (LAW_BURNOUT | LAW_VEHICLE | LAW_POLICE_VEHICLE) != 0 { 1 } else { 0 }) | (if self.offences & (LAW_SPEED | LAW_WRONG_WAY | LAW_RED_LIGHT) != 0 { 2 } else { 0 }) | (if self.offences & LAW_PROP != 0 { 4 } else { 0 });
                        self.clear();
                        event = Some(Event::Ticket(fine));
                    }
                } else {
                    self.arrest = 0.0;
                    let flee_delay = t("fBullhornTime", 10.0) * t("fPullOverFleeDelayScale", 2.0);
                    if self.score >= t("iPursuitLawBreakScore", 600.0) || (self.pullover_time > flee_delay && speed > arrest_speed * 2.0) || nearest > 120.0 {
                        self.heat = Heat::Pursuit;
                        self.last_known = ppos;
                        self.fled = true;
                        self.fine = self.law_fines();
                        self.lost = 0.0;
                        self.dispatch = t("fDispatchRdoDelayTime", 3.0);
                        event = Some(Event::Pursuit);
                    }
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
                if speed < arrest_speed && nearest < t("fBlockDist", 6.0) + 2.5 {
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
                    event = Some(Event::Escaped);
                }
            }
            Heat::Busted => {
                self.busted_timer -= dt;
                if self.busted_timer <= 0.0 {
                    // (step 19d: the payback's chasers stay until their script takes them away)
                    let goons = self.goons;
                    self.clear();
                    self.goons = goons;
                    // the cruisers drive off
                    vehicles.cars.retain(|c| !c.cop || c.state.pos.distance(ppos) > 60.0);
                }
            }
        }

        // ---- cruisers ---------------------------------------------------------
        let chasing = matches!(self.heat, Heat::Pursuit | Heat::Search | Heat::PullOver | Heat::Busted);
        let target = match self.heat {
            Heat::Search => (self.last_known, Vec3::ZERO),
            _ => (ppos, pvel),
        };
        let cop_speed = match self.heat {
            Heat::Pursuit => (speed + 14.0).clamp(28.0, 85.0),
            Heat::PullOver => (speed + 6.0).clamp(18.0, 45.0),
            Heat::Search => 22.0,
            _ => 14.0,
        };
        for c in vehicles.cars.iter_mut().filter(|c| pursuer(c)) {
            if let Some(ai) = c.ai.as_mut() {
                if chasing && self.heat != Heat::Busted {
                    ai.chase = Some(target);
                    ai.target_speed = cop_speed;
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
