//! Flag games: the `CaptureTheFlag` events of `tune/race` played against AI drivers.
//!
//! In the original these modes (flag games, stockpile, keep away) exist on line only - only the
//! network race core starts them and every other car is a player. Playing them off line
//! against computer drivers is the port's own addition; the drivers' rules below are the
//! port's, not the original's (see SAVE_WORK.md, "Flag games offline").
//!
//! The definitions give one or two start grids and a list of points:
//! * `CTF_BaseWar` - two grids; each team's flag stands at its base (the points next to
//!   the grids). Take the other team's flag home; touching your own flag out in the
//!   field sends it back.
//! * `CTF_SplitBase` - two grids, one neutral flag at one of the remaining points; bring
//!   it to your base.
//! * `Stockpile_SplitBase` - as above with a flag on every point; most flags wins.
//! * `CTF_FFA` / `Stockpile_FFA` - one grid, many points all over the district: everyone
//!   for themselves, the flag has to be taken to a drop point.
//! * `KeepAway` - one flag; whoever holds it longest wins.
//! A flag changes hands when another side's car touches the carrier.

use crate::ai::AiDriver;
use crate::races::RaceDef;
use crate::render::Gpu;
use crate::rivals::Rival;
use crate::vehicles::Vehicles;
use glam::Vec3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    BaseWar,
    SplitBase,
    Stockpile,
    Ffa,
    StockpileFfa,
    KeepAway,
}

pub struct Flag {
    pub home: Vec3,
    pub pos: Vec3,
    /// participant carrying it
    pub carrier: Option<usize>,
    /// base war: the side it belongs to
    pub owner: Option<usize>,
    /// delivered (stockpile)
    pub done: bool,
    immune: f32,
}

/// What the HUD shows.
#[derive(Clone, Default)]
pub struct FlagHud {
    pub title: String,
    pub score: String,
    pub objective: String,
    pub target: Option<Vec3>,
    /// minimap dots: (position, colour)
    pub blips: Vec<(Vec3, [u8; 3])>,
    pub countdown: f32,
}

pub struct FlagGame {
    pub kind: Kind,
    pub name: String,
    /// car index per participant (the player first)
    pub cars: Vec<usize>,
    /// side per participant (team games: 0 / 1; everyone for themselves: the own index)
    pub side: Vec<usize>,
    pub names: Vec<String>,
    pub flags: Vec<Flag>,
    /// where each side delivers (one shared drop point in the free-for-all games)
    pub bases: Vec<Vec3>,
    points: Vec<Vec3>,
    pub score: Vec<f32>,
    pub goal: f32,
    pub time: f32,
    pub countdown: f32,
    teams: bool,
    rng: u32,
    think: f32,
    /// what just happened (shown for a few seconds)
    pub banner: Option<(String, f32)>,
}

const PICKUP: f32 = 7.5;
const DELIVER: f32 = 15.0;
const TOUCH: f32 = 3.7;
const TIME_LIMIT: f32 = 600.0;

fn dxz(a: Vec3, b: Vec3) -> f32 {
    ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

impl FlagGame {
    pub fn is_flag_event(def: &RaceDef) -> bool {
        def.race_type == "CaptureTheFlag"
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng % 100_000) as f32 / 100_000.0
    }

    /// Set the field up: the player and (in team games) two team mates on the first grid,
    /// the others on the second.
    pub fn start(def: &RaceDef, vehicles: &mut Vehicles, gpu: &Gpu, rivals: &[Rival], names: &dyn Fn(&str) -> String, seed: u32) -> Option<FlagGame> {
        let kind = match def.sub_type.as_str() {
            "CTF_BaseWar" => Kind::BaseWar,
            "CTF_SplitBase" => Kind::SplitBase,
            "Stockpile_SplitBase" => Kind::Stockpile,
            "CTF_FFA" => Kind::Ffa,
            "Stockpile_FFA" => Kind::StockpileFfa,
            "KeepAway" => Kind::KeepAway,
            _ => return None,
        };
        if def.grid.is_empty() || def.checkpoints.is_empty() {
            return None;
        }
        let teams = matches!(kind, Kind::BaseWar | Kind::SplitBase | Kind::Stockpile) && !def.grid2.is_empty();
        let kind = if teams || !matches!(kind, Kind::BaseWar | Kind::SplitBase | Kind::Stockpile) { kind } else { Kind::Ffa };
        vehicles.cars.retain(|c| c.is_player);
        let pi = vehicles.cars.iter().position(|c| c.is_player)?;
        let mut g = FlagGame { kind, name: def.name.clone(), cars: vec![pi], side: vec![0], names: vec!["YOU".into()], flags: Vec::new(), bases: Vec::new(), points: Vec::new(), score: Vec::new(), goal: 3.0, time: 0.0, countdown: 4.5, teams, rng: seed | 1, think: 0.0, banner: None };
        let (p0, yaw0) = def.grid[0];
        vehicles.cars[pi].state = crate::physics::CarState::new(p0 + Vec3::Y * 0.9, yaw0);
        vehicles.cars[pi].placed = false;
        vehicles.cars[pi].ai = None;
        // the others: named rivals in their own cars where there are any
        let racers: Vec<_> = vehicles.models.iter().filter(|m| !m.name.starts_with("va_") && !m.name.contains("police")).cloned().collect();
        if racers.is_empty() {
            return None;
        }
        let n_other = 5usize;
        for k in 0..n_other {
            let side = if teams { if k < 2 { 0 } else { 1 } } else { k + 1 };
            let slot = if teams && side == 1 { def.grid2.get(k - 2) } else { def.grid.get(k + 1) };
            let Some(&(p, yaw)) = slot.or(def.grid.last()) else { continue };
            let mut model = racers[(k + 1) % racers.len()].clone();
            // team colours; everyone for themselves keeps the rival's own paint
            let mut color = if teams { if side == 0 { [0.10, 0.28, 0.85, 1.0] } else { [0.85, 0.10, 0.08, 1.0] } } else { crate::vehicles::PALETTE[(k * 3 + 1) % crate::vehicles::PALETTE.len()] };
            let mut name = format!("RACER {}", k + 1);
            let mut driver = String::new();
            let mut plate = String::new();
            let mut vinyl = String::new();
            if let Some(r) = rivals.get(k) {
                if let Some(m) = vehicles.models.iter().find(|m| m.name.eq_ignore_ascii_case(&r.car)) {
                    model = m.clone();
                    name = names(&r.id).to_uppercase();
                    driver = r.id.clone();
                    plate = r.plate.clone();
                    if !teams {
                        color = r.color;
                        vinyl = r.config.clone();
                    }
                }
            }
            let top = model.params.top_speed;
            let idx = vehicles.spawn(gpu, model, p + Vec3::Y * 0.9, yaw, color, false);
            vehicles.cars[idx].pinned = true;
            vehicles.cars[idx].driver = driver;
            vehicles.cars[idx].plate_text = plate;
            vehicles.cars[idx].vinyl_name = vinyl;
            let mut d = AiDriver::new(0x4321 + k as u32 * 131);
            d.target_speed = (top * 0.72).clamp(24.0, 52.0);
            d.aggression = 0.9;
            vehicles.cars[idx].ai = Some(d);
            g.cars.push(idx);
            g.side.push(side);
            g.names.push(name);
        }
        // the points: bases are the ones next to the grids
        let mut pts: Vec<Vec3> = def.checkpoints.clone();
        pts.dedup_by(|a, b| a.distance(*b) < 3.0);
        if teams {
            for start in [def.grid[0].0, def.grid2[0].0] {
                let near = pts.iter().enumerate().min_by(|a, b| a.1.distance(start).partial_cmp(&b.1.distance(start)).unwrap()).filter(|(_, p)| p.distance(start) < 80.0).map(|(i, _)| i);
                match near {
                    Some(i) => g.bases.push(pts.remove(i)),
                    None => g.bases.push(start),
                }
            }
        }
        g.points = pts;
        let start = def.grid[0].0;
        match kind {
            Kind::BaseWar => {
                for t in 0..2 {
                    let home = g.bases[t];
                    g.flags.push(Flag { home, pos: home, carrier: None, owner: Some(t), done: false, immune: 0.0 });
                }
            }
            Kind::SplitBase => {
                let mid = (g.bases[0] + g.bases[1]) * 0.5;
                if g.points.is_empty() {
                    g.points.push(mid);
                }
                let k = (g.rand() * g.points.len() as f32) as usize % g.points.len();
                let p = g.points[k];
                g.flags.push(Flag { home: p, pos: p, carrier: None, owner: None, done: false, immune: 0.0 });
            }
            Kind::Stockpile => {
                if g.points.is_empty() {
                    g.points.push((g.bases[0] + g.bases[1]) * 0.5);
                }
                for p in g.points.iter().take(9) {
                    g.flags.push(Flag { home: *p, pos: *p, carrier: None, owner: None, done: false, immune: 0.0 });
                }
                g.goal = g.flags.len() as f32;
            }
            Kind::Ffa | Kind::StockpileFfa => {
                // play around the start: the nearest points only
                g.points.sort_by(|a, b| a.distance(start).partial_cmp(&b.distance(start)).unwrap());
                g.points.truncate(14);
                if g.points.len() < 2 {
                    return None;
                }
                if kind == Kind::Ffa {
                    g.bases.push(start);
                    g.flags.push(Flag { home: start, pos: start, carrier: None, owner: None, done: false, immune: 0.0 });
                    g.respawn(0);
                } else {
                    g.bases.push(g.points[0]);
                    for p in g.points.iter().skip(1).take(7) {
                        g.flags.push(Flag { home: *p, pos: *p, carrier: None, owner: None, done: false, immune: 0.0 });
                    }
                    g.goal = g.flags.len() as f32;
                }
            }
            Kind::KeepAway => {
                let p = g.points[0];
                g.flags.push(Flag { home: p, pos: p, carrier: None, owner: None, done: false, immune: 0.0 });
                g.goal = 60.0;
            }
        }
        g.score = vec![0.0; if teams { 2 } else { g.cars.len() }];
        log::info!("flag game {} ({:?}): {} cars, {} flags, {} bases, {} points", def.name, kind, g.cars.len(), g.flags.len(), g.bases.len(), g.points.len());
        Some(g)
    }

    /// Put flag `fi` on a fresh point (and, everyone for themselves, pick a new drop point).
    fn respawn(&mut self, fi: usize) {
        if self.points.is_empty() {
            return;
        }
        let old = self.flags[fi].home;
        let n = self.points.len();
        let mut p = old;
        for _ in 0..8 {
            let k = (self.rand() * n as f32) as usize % n;
            p = self.points[k];
            if p.distance(old) > 40.0 {
                break;
            }
        }
        if self.kind == Kind::Ffa {
            let mut d = p;
            for _ in 0..12 {
                let k = (self.rand() * n as f32) as usize % n;
                d = self.points[k];
                if d.distance(p) > 150.0 {
                    break;
                }
            }
            self.bases[0] = d;
        }
        let f = &mut self.flags[fi];
        f.home = p;
        f.pos = p;
        f.carrier = None;
        f.immune = 0.0;
    }

    fn base_of(&self, who: usize) -> Vec3 {
        if self.teams {
            self.bases[self.side[who].min(1)]
        } else {
            self.bases.first().copied().unwrap_or(Vec3::ZERO)
        }
    }

    /// A team game's scores for the end-of-battle panel (F-34603): (red, blue) - the player's side is blue (team 1),
    /// the other red (team 2), as `marker_looks` colours them
    pub fn team_scores(&self) -> Option<(i32, i32)> {
        self.teams.then(|| (self.score.get(1).map_or(0, |s| s.floor() as i32), self.score.first().map_or(0, |s| s.floor() as i32)))
    }

    fn side_name(&self, side: usize) -> String {
        if self.teams {
            if side == 0 { "YOUR TEAM".into() } else { "RIVAL TEAM".into() }
        } else {
            self.names.get(side).cloned().unwrap_or_default()
        }
    }

    /// One frame. Returns the final standings (name, score) when the game is over.
    pub fn update(&mut self, vehicles: &mut Vehicles, dt: f32) -> Option<Vec<(String, Option<f32>)>> {
        if self.cars.iter().any(|&c| c >= vehicles.cars.len()) {
            return Some(self.standings());
        }
        if self.countdown > 0.0 {
            self.countdown -= dt;
            vehicles.frozen = self.countdown > 0.0;
            return None;
        }
        vehicles.frozen = false;
        self.time += dt;
        let pos: Vec<Vec3> = self.cars.iter().map(|&c| vehicles.cars[c].state.pos).collect();
        let vel: Vec<Vec3> = self.cars.iter().map(|&c| vehicles.cars[c].state.vel).collect();
        let n = self.cars.len();
        let mut banner: Option<String> = None;
        let mut respawn: Vec<usize> = Vec::new();
        for fi in 0..self.flags.len() {
            if self.flags[fi].done {
                continue;
            }
            self.flags[fi].immune = (self.flags[fi].immune - dt).max(0.0);
            match self.flags[fi].carrier {
                Some(c) => {
                    self.flags[fi].pos = pos[c];
                    // someone of another side touches the carrier
                    let thief = (0..n).find(|&j| self.side[j] != self.side[c] && pos[j].distance(pos[c]) < TOUCH);
                    if let (Some(j), true) = (thief, self.flags[fi].immune <= 0.0) {
                        if self.flags[fi].owner == Some(self.side[j]) {
                            let home = self.flags[fi].home;
                            self.flags[fi].carrier = None;
                            self.flags[fi].pos = home;
                            banner = Some(format!("{} SENT THE FLAG BACK", self.names[j]));
                        } else if !self.flags.iter().any(|f| f.carrier == Some(j)) {
                            self.flags[fi].carrier = Some(j);
                            self.flags[fi].immune = 2.5;
                            banner = Some(format!("{} TOOK THE FLAG FROM {}", self.names[j], self.names[c]));
                        }
                        continue;
                    }
                    if self.kind == Kind::KeepAway {
                        self.score[self.side[c]] += dt;
                    } else if dxz(pos[c], self.base_of(c)) < DELIVER && (pos[c].y - self.base_of(c).y).abs() < 6.0 {
                        self.score[self.side[c]] += 1.0;
                        banner = Some(format!("{} SCORED", self.names[c]));
                        match self.kind {
                            Kind::BaseWar => {
                                let home = self.flags[fi].home;
                                self.flags[fi].carrier = None;
                                self.flags[fi].pos = home;
                            }
                            Kind::Stockpile | Kind::StockpileFfa => {
                                self.flags[fi].carrier = None;
                                self.flags[fi].done = true;
                            }
                            _ => respawn.push(fi),
                        }
                    }
                }
                None => {
                    let fp = self.flags[fi].pos;
                    let taker = (0..n).find(|&j| dxz(pos[j], fp) < PICKUP && (pos[j].y - fp.y).abs() < 5.0 && !self.flags.iter().any(|f| f.carrier == Some(j)));
                    if let Some(j) = taker {
                        if self.flags[fi].owner == Some(self.side[j]) {
                            // the own flag: back home if it lies somewhere else
                            if fp.distance(self.flags[fi].home) > 2.0 {
                                self.flags[fi].pos = self.flags[fi].home;
                                banner = Some(format!("{} SENT THE FLAG BACK", self.names[j]));
                            }
                        } else {
                            self.flags[fi].carrier = Some(j);
                            self.flags[fi].immune = 1.5;
                            banner = Some(format!("{} {} THE FLAG", self.names[j], if j == 0 { "HAVE" } else { "HAS" }));
                        }
                    }
                }
            }
        }
        for fi in respawn {
            self.respawn(fi);
        }
        if let Some(b) = banner {
            log::info!("flag game t {:.0}: {} (score {:?})", self.time, b, self.score);
            self.banner = Some((b, self.time + 4.0));
        }
        // ---- the others decide where to go ----
        self.think -= dt;
        if self.think <= 0.0 {
            self.think = 0.35;
            // (the player's car too when it drives itself: test runs)
            for i in 0..n {
                if vehicles.cars[self.cars[i]].ai.is_none() {
                    continue;
                }
                let mine = self.flags.iter().position(|f| f.carrier == Some(i));
                let hold = Vec3::Y * 3.1;
                let target: Option<(Vec3, Vec3)> = match mine {
                    Some(_) if self.kind == Kind::KeepAway => {
                        // run from whoever is closest
                        let threat = (0..n).filter(|&j| j != i).min_by(|&a, &b| pos[a].distance(pos[i]).partial_cmp(&pos[b].distance(pos[i])).unwrap());
                        match threat.filter(|&j| pos[j].distance(pos[i]) < 90.0) {
                            Some(j) => {
                                let away = (pos[i] - pos[j]).normalize_or(Vec3::X);
                                Some((pos[i] + away * 220.0, hold))
                            }
                            None => None,
                        }
                    }
                    Some(_) => Some((self.base_of(i), hold)),
                    None => {
                        let mut best: Option<(f32, Vec3, Vec3)> = None;
                        let mut consider = |d: f32, p: Vec3, v: Vec3| {
                            if best.map_or(true, |b| d < b.0) {
                                best = Some((d, p, v));
                            }
                        };
                        for f in self.flags.iter().filter(|f| !f.done) {
                            match f.carrier {
                                Some(c) if self.side[c] != self.side[i] => consider(pos[c].distance(pos[i]) * 0.8, pos[c], vel[c]),
                                // ride along with a team mate who has it
                                Some(c) => consider(pos[c].distance(pos[i]) + 400.0, pos[c] - vel[c].normalize_or_zero() * 9.0, vel[c]),
                                None if f.owner == Some(self.side[i]) => {
                                    if f.pos.distance(f.home) > 2.0 {
                                        consider(f.pos.distance(pos[i]), f.pos, hold);
                                    }
                                }
                                None => consider(f.pos.distance(pos[i]), f.pos, hold),
                            }
                        }
                        best.map(|b| (b.1, b.2))
                    }
                };
                if let Some(ai) = vehicles.cars[self.cars[i]].ai.as_mut() {
                    match target {
                        Some(t) => ai.chase = Some(t),
                        None => {
                            // nothing to do: cruise
                            if ai.chase.take().is_some() {
                                ai.path = None;
                            }
                        }
                    }
                }
            }
        }
        // ---- over? ----
        let top = self.score.iter().cloned().fold(0.0f32, f32::max);
        let over = match self.kind {
            Kind::Stockpile | Kind::StockpileFfa => self.flags.iter().all(|f| f.done),
            _ => top >= self.goal,
        };
        if over || self.time > TIME_LIMIT {
            return Some(self.standings());
        }
        None
    }

    /// Sides by score, the winner first; the player's side starts with "YOU".
    pub fn standings(&self) -> Vec<(String, Option<f32>)> {
        let mut v: Vec<(String, f32)> = (0..self.score.len()).map(|s| (self.side_name(s), self.score[s])).collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        v.into_iter().map(|(n, s)| (n, Some(s.floor()))).collect()
    }

    pub fn player_won(res: &[(String, Option<f32>)]) -> bool {
        res.first().map_or(false, |r| r.0.starts_with("YOU")) && res.get(1).map_or(true, |s| s.1 != res[0].1)
    }

    /// The markers as the original's flag looks (`checkpoints::NAMES` 5..10: pick-up / drop-off
    /// x no team / blue / red - `Checkpoint_SetTypeFlagPickup` / `Dropoff` add the team to 5 / 8,
    /// sub_82265060 / sub_82265088). The port's choice for its offline game: the free-for-all
    /// games take "no team"; in the team games the player's side is blue, the other red.
    pub fn marker_looks(&self, vehicles: &Vehicles) -> Vec<(Vec3, u8)> {
        let side = |owner: Option<usize>| -> u8 {
            match (self.teams, owner) {
                (false, _) | (true, None) => 0,
                (true, Some(0)) => 1,
                (true, Some(_)) => 2,
            }
        };
        let carrying = self.flags.iter().any(|f| f.carrier == Some(0));
        self.markers(vehicles)
            .into_iter()
            .map(|(p, li, _)| {
                if li == 1 {
                    (p, 8 + side(Some(0)))
                } else {
                    let owner = self.flags.iter().filter(|f| f.pos.distance(p) < 0.01).map(|f| f.owner).next().flatten();
                    let _ = carrying;
                    (p, 5 + side(owner))
                }
            })
            .collect()
    }

    /// World markers: (position, look 0 yellow / 1 red, size).
    pub fn markers(&self, vehicles: &Vehicles) -> Vec<(Vec3, usize, f32)> {
        let mut out = Vec::new();
        let carrying = self.flags.iter().any(|f| f.carrier == Some(0));
        if carrying && self.kind != Kind::KeepAway {
            out.push((self.base_of(0), 1, 1.0));
        }
        let pp = vehicles.cars.get(self.cars[0]).map(|c| c.state.pos).unwrap_or_default();
        let mut fl: Vec<&Flag> = self.flags.iter().filter(|f| !f.done && f.carrier != Some(0)).collect();
        fl.sort_by(|a, b| a.pos.distance(pp).partial_cmp(&b.pos.distance(pp)).unwrap());
        for f in fl {
            if out.len() >= 3 {
                break;
            }
            // (the own flag at home is no destination)
            if f.owner == Some(0) && f.carrier.is_none() && f.pos.distance(f.home) < 2.0 {
                continue;
            }
            out.push((f.pos, 0, if f.carrier.is_some() { 0.5 } else { 0.9 }));
        }
        out
    }

    pub fn hud(&self) -> FlagHud {
        let title = match self.kind {
            Kind::BaseWar => "CAPTURE THE FLAG - BASE WAR",
            Kind::SplitBase => "CAPTURE THE FLAG - SPLIT BASE",
            Kind::Stockpile => "STOCKPILE",
            Kind::Ffa => "CAPTURE THE FLAG",
            Kind::StockpileFfa => "STOCKPILE",
            Kind::KeepAway => "KEEP AWAY",
        };
        let score = if self.teams {
            format!("YOUR TEAM {:.0}  -  {:.0} RIVAL TEAM", self.score[0], self.score[1])
        } else {
            let best = (1..self.score.len()).max_by(|&a, &b| self.score[a].partial_cmp(&self.score[b]).unwrap()).unwrap_or(0);
            format!("YOU {:.0}  -  {:.0} {}", self.score[0], self.score[best], self.names.get(best).cloned().unwrap_or_default())
        };
        let mine = self.flags.iter().any(|f| f.carrier == Some(0));
        let (objective, target) = if mine {
            if self.kind == Kind::KeepAway {
                ("KEEP THE FLAG".to_string(), None)
            } else {
                (if self.teams { "TAKE THE FLAG TO YOUR BASE".to_string() } else { "TAKE THE FLAG TO THE DROP POINT".to_string() }, Some(self.base_of(0)))
            }
        } else {
            // the nearest flag that matters
            let p0 = self.flags.first().map(|f| f.pos).unwrap_or_default();
            let mut best: Option<(String, Vec3)> = None;
            for f in self.flags.iter().filter(|f| !f.done) {
                let o = match f.carrier {
                    Some(c) if self.side[c] != self.side[0] => (format!("STOP {}", self.names[c]), f.pos),
                    Some(c) => (format!("PROTECT {}", self.names[c]), f.pos),
                    None if f.owner == Some(0) => continue,
                    None => ("GET THE FLAG".to_string(), f.pos),
                };
                if best.is_none() || o.0.starts_with("GET") || o.0.starts_with("STOP") && !best.as_ref().unwrap().0.starts_with("GET") {
                    best = Some(o);
                }
            }
            match best {
                Some((o, p)) => (o, Some(p)),
                None => ("DEFEND YOUR FLAG".to_string(), Some(p0)),
            }
        };
        let mut blips = Vec::new();
        for f in self.flags.iter().filter(|f| !f.done) {
            blips.push((f.pos, [255, 220, 40]));
        }
        for (s, b) in self.bases.iter().enumerate() {
            blips.push((*b, if !self.teams || s == 0 { [80, 200, 255] } else { [255, 80, 70] }));
        }
        let objective = match self.banner.as_ref() {
            Some((b, until)) if self.time < *until => b.clone(),
            _ => objective,
        };
        FlagHud { title: title.to_string(), score, objective, target, blips, countdown: self.countdown }
    }
}
