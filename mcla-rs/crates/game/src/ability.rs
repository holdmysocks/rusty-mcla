//! Special abilities: Zone (slow motion), Agro (plough through traffic), Roar (clear
//! the road ahead) and EMP (kill the engines around you). A meter fills through
//! skilful driving - speed, drifting, air time, slipstreaming, near misses - and one
//! full meter buys one use.

use crate::vehicles::Vehicles;
use glam::Vec3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Zone,
    Agro,
    Roar,
    Emp,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Zone, Kind::Agro, Kind::Roar, Kind::Emp];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Zone => "ZONE",
            Kind::Agro => "AGRO",
            Kind::Roar => "ROAR",
            Kind::Emp => "EMP",
        }
    }
    /// How long the effect lasts once triggered (s).
    fn duration(self) -> f32 {
        match self {
            Kind::Zone => 5.0,
            Kind::Agro => 8.0,
            Kind::Roar => 1.2,
            Kind::Emp => 0.8,
        }
    }
    /// The career's name of the ability (the "Pulse" system is the EMP) and its use counter.
    pub fn system(self) -> (&'static str, &'static str) {
        match self {
            Kind::Zone => ("Zone", "gZON"),
            Kind::Agro => ("Agro", "gAGR"),
            Kind::Roar => ("Roar", "gROR"),
            Kind::Emp => ("Pulse", "gPUL"),
        }
    }
}

/// What the disc says of one ability (`tune/vehicle/<zone|agro|roar|pulse>.specialability`).
#[derive(Clone, Copy, Debug)]
pub struct AbilityTune {
    /// gauge per second lost while the ability runs (in game time)
    pub drain: f32,
    /// seconds for the camera / time warp to come in and to go out
    pub in_time: f32,
    pub out_time: f32,
    /// degrees added to the lens and scale of the camera's distance while it runs
    pub fov_offset: f32,
    pub dist_scale: f32,
    /// world time scale while it runs (Zone)
    pub time_warp: f32,
    /// the gauge: per second gained above ThresholdSpeed (times 1 + ThresholdSpeedFactor per
    /// 100 m/s over it), per second lost below it, and lost for a moment (the timer) after
    /// hitting a car, a prop or the city; the least gauge the ability can be fired with
    pub increase: f32,
    pub decrease: f32,
    pub threshold_speed: f32,
    pub threshold_factor: f32,
    pub strike: [f32; 3],
    pub strike_timer: f32,
    pub min_gauge: f32,
    /// how far the blast reaches: ExplosionVelocity x ExplosionDuration (m)
    pub reach: f32,
}

impl AbilityTune {
    fn fallback(kind: Kind) -> AbilityTune {
        let (drain, fov_offset, dist_scale, time_warp, in_time, out_time) = match kind {
            Kind::Zone => (0.8, -5.0, 1.0, 0.239, 0.4, 2.0),
            Kind::Agro => (0.25, 20.0, 0.65, 1.0, 0.8, 0.6),
            Kind::Roar => (0.225, 10.0, 0.75, 1.0, 0.5, 0.8),
            Kind::Emp => (0.225, 0.0, 1.0, 1.0, 1.0, 1.0),
        };
        AbilityTune { drain, in_time, out_time, fov_offset, dist_scale, time_warp, increase: 0.01, decrease: 0.15, threshold_speed: 20.0, threshold_factor: 10.0, strike: [0.2, 0.15, 0.05], strike_timer: 0.333, min_gauge: if matches!(kind, Kind::Roar | Kind::Emp) { 0.1 } else { 0.0 }, reach: match kind { Kind::Roar => 180.0, Kind::Emp => 60.0, _ => 0.0 } }
    }

    fn load(data_dir: &str, kind: Kind) -> AbilityTune {
        let mut t = AbilityTune::fallback(kind);
        let name = match kind {
            Kind::Zone => "zone",
            Kind::Agro => "agro",
            Kind::Roar => "roar",
            Kind::Emp => "pulse",
        };
        let h = mcla_formats::hash::joaat(&format!("{name}.specialability"));
        let Ok(text) = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/vehicle").join(format!("0x{h:08x}"))) else { return t };
        let (mut ev, mut ed) = (0.0f32, 0.0f32);
        for l in text.lines() {
            let mut it = l.split_whitespace();
            let (Some(k), Some(v)) = (it.next(), it.next().and_then(|x| x.parse::<f32>().ok())) else { continue };
            match k {
                "SpecialAbilityDecreaseRateWhenActive" if v < 0.0 => t.drain = -v,
                "InTime" if v > 0.0 => t.in_time = v,
                "OutTime" if v > 0.0 => t.out_time = v,
                "FovOffset" => t.fov_offset = v,
                "DistScale" if v > 0.0 => t.dist_scale = v,
                "m_rSlowMotionTimeWarp" if v > 0.0 => t.time_warp = v,
                "SpecialAbilityIncreaseRate" if v > 0.0 => t.increase = v,
                "SpecialAbilityDecreaseRate" if v < 0.0 => t.decrease = -v,
                "ThresholdSpeed" if v > 0.0 => t.threshold_speed = v,
                "ThresholdSpeedFactor" => t.threshold_factor = v,
                "VehicleStrikeDecreaseRate" if v < 0.0 => t.strike[0] = -v,
                "PropStrikeDecreaseRate" if v < 0.0 => t.strike[1] = -v,
                "CityStrikeDecreaseRate" if v < 0.0 => t.strike[2] = -v,
                "m_rSpecialAbilityDecreaseTimer" if v > 0.0 => t.strike_timer = v,
                "MinGaugeThreshold" => t.min_gauge = v.max(0.0),
                "ExplosionVelocity" => ev = v,
                "ExplosionDuration" => ed = v,
                _ => {}
            }
        }
        if ev * ed > 1.0 && matches!(kind, Kind::Roar | Kind::Emp) {
            t.reach = ev * ed;
        }
        t
    }
}

pub struct Abilities {
    pub kind: Kind,
    /// the four abilities' tuning, in the order of `Kind::ALL`
    pub tunes: [AbilityTune; 4],
    /// seconds since the running ability was triggered
    since: f32,
    running: bool,
    /// after a collision: seconds left of losing gauge, and at what rate
    strike_left: f32,
    strike_rate: f32,
    last_impact: f32,
    /// the screen tints of Zone and Agro (inner, outer RGBA; `screenTint`)
    pub tints: [[[f32; 4]; 2]; 2],
    /// 0..1, full = ready
    pub meter: f32,
    /// seconds of the running effect left
    pub active: f32,
    near_miss_cool: f32,
    /// level of each ability (0..2; they grow with use), in the order of `Kind::ALL`
    pub levels: [u8; 4],
}

impl Abilities {
    pub fn new() -> Abilities {
        Abilities {
            kind: Kind::Zone,
            tunes: Kind::ALL.map(AbilityTune::fallback),
            since: 0.0,
            running: false,
            strike_left: 0.0,
            strike_rate: 0.0,
            last_impact: 0.0,
            tints: [[[0.0, 0.333, 0.8, 0.0], [0.204, 0.294, 0.659, 0.118]], [[0.012, 0.0, 0.0, 0.49], [0.051, 0.0, 0.0, 1.0]]],
            meter: 0.0,
            active: 0.0,
            near_miss_cool: 0.0,
            levels: [0; 4],
        }
    }

    /// Read the abilities' tuning from the disc.
    pub fn load(&mut self, data_dir: &str) {
        self.tunes = Kind::ALL.map(|k| AbilityTune::load(data_dir, k));
        // the tint file's name is not known: it is the one screenTint in the folder
        if let Ok(rd) = std::fs::read_dir(std::path::Path::new(data_dir).join("tune/vehicle")) {
            for e in rd.flatten() {
                let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
                if !text.contains("screenTint") {
                    continue;
                }
                for l in text.lines() {
                    let mut it = l.split_whitespace();
                    let Some(k) = it.next() else { continue };
                    let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
                    if v.len() == 4 {
                        let c = [v[0], v[1], v[2], v[3]];
                        match k {
                            "m_vZoneColorA" => self.tints[0][0] = c,
                            "m_vZoneColorB" => self.tints[0][1] = c,
                            "m_vAgroColorA" => self.tints[1][0] = c,
                            "m_vAgroColorB" => self.tints[1][1] = c,
                            _ => {}
                        }
                    }
                }
            }
        }
        log::info!("abilities: {:?}", self.tunes.map(|t| (t.drain, t.fov_offset, t.dist_scale, t.time_warp)));
    }

    fn tune(&self) -> &AbilityTune {
        &self.tunes[Kind::ALL.iter().position(|k| *k == self.kind).unwrap_or(0)]
    }

    /// How far in the running ability's look is (0..1): in over InTime, out over OutTime.
    pub fn blend(&self) -> f32 {
        if self.active <= 0.0 {
            return 0.0;
        }
        let t = self.tune();
        (self.since / t.in_time.max(0.05)).min(1.0).min((self.active / t.out_time.max(0.05)).min(1.0))
    }

    /// What the running ability does to the driving camera: degrees added to the lens, and
    /// the scale of its distance.
    pub fn camera(&self) -> (f32, f32) {
        let (b, t) = (self.blend(), self.tune());
        (t.fov_offset * b, 1.0 + (t.dist_scale - 1.0) * b)
    }

    /// Screen tint of the running ability (inner, outer RGBA), if it has one.
    pub fn tint(&self) -> Option<[[f32; 4]; 2]> {
        if self.active <= 0.0 {
            return None;
        }
        match self.kind {
            Kind::Zone => Some(self.tints[0]),
            Kind::Agro => Some(self.tints[1]),
            _ => None,
        }
    }

    pub fn cycle(&mut self) {
        if self.active > 0.0 {
            return;
        }
        let i = Kind::ALL.iter().position(|k| *k == self.kind).unwrap_or(0);
        self.kind = Kind::ALL[(i + 1) % Kind::ALL.len()];
    }

    /// Length of the current ability's effect: a level adds three tenths.
    fn duration(&self) -> f32 {
        let i = Kind::ALL.iter().position(|k| *k == self.kind).unwrap_or(0);
        // a full gauge drains at the file's rate - in game time, so Zone's slow motion
        // stretches it; the blasts (Roar, the pulse) are over in a moment on screen
        let t = &self.tunes[i];
        let full = match self.kind {
            Kind::Zone => 1.0 / t.drain.max(0.05) / t.time_warp.clamp(0.05, 1.0),
            Kind::Agro => 1.0 / t.drain.max(0.05),
            _ => self.kind.duration(),
        };
        full * (1.0 + 0.3 * self.levels[i] as f32)
    }

    /// The ability can be fired with any gauge above its MinGaugeThreshold; it then runs
    /// until the gauge is empty.
    pub fn ready(&self) -> bool {
        !self.running && self.meter > self.tune().min_gauge.max(0.02)
    }

    /// World time scale: Zone slows everything down.
    pub fn time_scale(&self) -> f32 {
        if self.kind == Kind::Zone && self.active > 0.0 {
            // (m_rSlowMotionTimeWarp, eased in and out over InTime / OutTime)
            1.0 + (self.tune().time_warp - 1.0) * self.blend()
        } else {
            1.0
        }
    }

    /// Step 19e, the Agro power-up (sub_8237E638): the car's ability is made `kind`
    /// (sub_82338A48), its gauge full and running (sub_823383C8). Returns the one it was.
    pub fn powerup_run(&mut self, kind: Kind) -> Kind {
        let prev = self.kind;
        self.kind = kind;
        self.meter = 1.0;
        self.running = true;
        self.since = 0.0;
        prev
    }

    /// Its end (sub_8237E690): stopped (sub_82338388) and the car's own ability back.
    pub fn powerup_stop(&mut self, prev: Kind) {
        self.running = false;
        self.active = 0.0;
        self.kind = prev;
    }

    pub fn agro(&self) -> bool {
        self.kind == Kind::Agro && self.active > 0.0
    }

    /// 0..1 strength of the on-screen effect.
    pub fn fx(&self) -> f32 {
        if self.active <= 0.0 {
            0.0
        } else {
            (self.active / self.duration()).clamp(0.0, 1.0).sqrt()
        }
    }

    /// Advance: fill the meter from the player's driving, trigger on `fire`, and apply
    /// the instant effects to the cars around. `dt` is real time.
    /// `prop_hit`: a prop was knocked this frame (its strike costs less than a car's, the
    /// city's walls least).
    pub fn update(&mut self, vehicles: &mut Vehicles, fire: bool, prop_hit: bool, dt: f32) -> Option<Kind> {
        self.since += dt;
        self.near_miss_cool = (self.near_miss_cool - dt).max(0.0);
        let Some(pi) = vehicles.cars.iter().position(|c| c.is_player) else { return None };
        let (ppos, pvel, pfwd, impact) = {
            let c = &vehicles.cars[pi];
            (c.state.pos, c.state.vel, c.state.forward(), c.state.last_impact)
        };
        let speed = pvel.length();

        let near_miss = false;
        let dtg = dt * self.time_scale();
        // ---- the gauge (specialAbility, 0x82338798): it only grows with speed - above
        // ThresholdSpeed, faster the further above; below it, and for a moment after every
        // collision, it runs down; while the ability runs it drains at its own rate, in game
        // time, and the ability ends when it is empty ----
        let _ = near_miss;
        let tune = *self.tune();
        let level = self.levels[Kind::ALL.iter().position(|k| *k == self.kind).unwrap_or(0)] as f32;
        let warp = if self.kind == Kind::Zone { tune.time_warp.clamp(0.05, 1.0) } else { 1.0 };
        let hit = impact > self.last_impact + 0.05 || prop_hit;
        self.last_impact = impact;
        if self.running {
            let drain = tune.drain.max(0.02) / (1.0 + 0.3 * level);
            self.meter -= drain * dtg;
            if self.meter <= 0.0 {
                self.meter = 0.0;
                self.running = false;
                self.active = 0.0;
            } else {
                // (what is left on the clock)
                self.active = self.meter / drain / warp;
            }
        } else {
            self.active = 0.0;
            if hit {
                // a car, a prop, or the city itself
                let car = vehicles.cars.iter().enumerate().any(|(i, c)| i != pi && c.state.pos.distance(ppos) < 5.5);
                self.strike_rate = if car { tune.strike[0] } else if prop_hit { tune.strike[1] } else { tune.strike[2] };
                self.strike_left = tune.strike_timer;
            }
            if self.strike_left > 0.0 {
                self.strike_left -= dtg;
                self.meter -= self.strike_rate * dtg;
            } else if speed > tune.threshold_speed {
                self.meter += ((speed - tune.threshold_speed).clamp(0.0, 100.0) * tune.threshold_factor * 0.01 + 1.0) * tune.increase * dtg;
            } else {
                self.meter -= tune.decrease * dtg;
            }
            self.meter = self.meter.clamp(0.0, 1.0);
        }

        // ---- trigger ----
        let mut fired = None;
        if fire && self.ready() {
            self.running = true;
            self.since = 0.0;
            self.active = self.meter / (tune.drain.max(0.02) / (1.0 + 0.3 * level)) / warp;
            fired = Some(self.kind);
            match self.kind {
                Kind::Roar => {
                    // traffic ahead dives out of the way
                    for (i, c) in vehicles.cars.iter_mut().enumerate() {
                        if i == pi || c.parked.is_some() {
                            continue;
                        }
                        let rel = c.state.pos - ppos;
                        let ahead = rel.dot(pfwd);
                        let lat = rel - pfwd * ahead;
                        if ahead > -5.0 && ahead < tune.reach && lat.length() < 14.0 {
                            let side = Vec3::new(-pfwd.z, 0.0, pfwd.x);
                            let dir = if lat.dot(side) >= 0.0 { side } else { -side };
                            let k = 1.0 - (ahead.max(0.0) / tune.reach.max(1.0)) * 0.6;
                            c.state.vel += dir * 9.0 * k + Vec3::Y * 1.5 * k;
                            c.state.ang_vel.y += if lat.dot(side) >= 0.0 { 1.2 } else { -1.2 } * k;
                            c.stunned = c.stunned.max(2.5);
                        }
                    }
                }
                Kind::Emp => {
                    // every engine around dies for a few seconds
                    for (i, c) in vehicles.cars.iter_mut().enumerate() {
                        if i == pi || c.parked.is_some() {
                            continue;
                        }
                        if c.state.pos.distance(ppos) < tune.reach {
                            c.stunned = c.stunned.max(4.5);
                        }
                    }
                }
                Kind::Zone | Kind::Agro => {}
            }
        }
        fired
    }
}
