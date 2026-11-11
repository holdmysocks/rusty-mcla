//! The game's own race definitions (`tune/race/*`, plain text): start grid, checkpoints,
//! laps, type and district for every event.
//!
//! Grid positions are stored un-rotated (columns along x, rows along +z) together with a
//! heading in degrees; the real layout is the stored one rotated about the pole position
//! so that the cars face `yaw = -heading` (verified against the road network).

use glam::Vec3;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct RaceDef {
    /// the layout with more checkpoints that the game uses on the easier difficulty steps
    /// (`EasyRace`: a script of its own; the plain one is the "hard" layout, more open)
    pub easy_id: String,
    pub id: String,
    pub name: String,
    pub race_type: String,
    pub sub_type: String,
    pub district: String,
    pub laps: u32,
    pub beacon: Vec3,
    /// world position + yaw (radians, our convention) per grid slot
    pub grid: Vec<(Vec3, f32)>,
    /// the other team's grid (flag games)
    pub grid2: Vec<(Vec3, f32)>,
    pub checkpoints: Vec<Vec3>,
    pub goal_time: f32,
    pub opponents: usize,
    /// the original's race record +18 (F-95490): with an OpponentsAllowed line the file's number is read and dropped and
    /// the record holds a global byte (7 in the image and never written; read back from all 227 records of the running
    /// original), without one 0. ARCADE's NUMBER OF OPPONENTS row is limited by it.
    pub opp_allowed: u8,
    /// CarConfigName as the original's record +99 keeps it (a copy of at most 31 characters, F-95490): the set-up a
    /// delivery's script loads and puts the player in (Hangouts/0x37c63b19: RaceDescription_GetCarConfigName ->
    /// mcCarConfig_Load -> Racer_ChangeCarByConfig).
    pub car_config: String,
    pub dev_only: bool,
    /// time limits per difficulty (Easy, Medium, Hard, Impossible); 0 = none
    pub target_times: Vec<f32>,
    /// vehicle the event is driven with (deliveries), e.g. "vp_chv_camaross_69"
    pub car: String,
    /// "2x4_Race_Grid", "3x3_Race_Grid", ... (selects the start-line camera moves)
    pub grid_type: String,
    /// `AllowRaceStarter 0`: the race script passes camOnly to the start-scene picker (prerace.rs)
    pub no_starter: bool,
    /// `SafetyCam 1`: the start scene comes from the table's Safety list
    pub safety_cam: bool,
}

impl RaceDef {
    pub fn title(&self) -> String {
        let kind = match self.sub_type.as_str() {
            "Circuit" => "Circuit",
            "Ordered" => "Ordered",
            "Unordered" => "Unordered",
            "Blitz" => "Time Trial",
            "Delivery" => "Delivery",
            "Payback" => "Payback",
            "CTF_BaseWar" => "CTF Base War",
            "CTF_SplitBase" => "CTF Split Base",
            "CTF_FFA" => "Capture the Flag",
            "Stockpile_SplitBase" | "Stockpile_FFA" => "Stockpile",
            "KeepAway" => "Keep Away",
            o => o,
        };
        if self.sub_type == "RedLight" {
            return "Red Light Race".to_string();
        }
        // (the freeway man's race: gps_racename = the string "Freeway" - althk.rs)
        if self.sub_type == "Freeway" {
            return "Freeway Race".to_string();
        }
        format!("{} {}  -  {}", self.district, kind, self.name)
    }
    /// Total length along the checkpoints for one lap (m).
    pub fn lap_len(&self) -> f32 {
        let mut p = self.grid.first().map(|g| g.0).unwrap_or(self.beacon);
        let mut d = 0.0;
        for c in &self.checkpoints {
            d += p.distance(*c);
            p = *c;
        }
        d
    }
}

/// The global the race record loader F-95490 stores at record +18 for an OpponentsAllowed line (its value in the image
/// and in the running original at the title: every one of the 227 records read back holds 7).
pub const OPP_ALLOWED_GLOBAL: u8 = 7;

fn parse(text: &str) -> Vec<RaceDef> {
    let mut out = Vec::new();
    let mut cur: Option<RaceDef> = None;
    let mut raw_grid: Vec<(Vec3, f32)> = Vec::new();
    let mut raw_grid2: Vec<(Vec3, f32)> = Vec::new();
    let mut grids_seen = 0usize;
    let mut pending_id = String::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with(';') {
            continue;
        }
        if l == "{" {
            let name = pending_id.rsplit('/').next().unwrap_or("").to_string();
            cur = Some(RaceDef { id: pending_id.clone(), name, ..Default::default() });
            raw_grid.clear();
            raw_grid2.clear();
            grids_seen = 0;
            continue;
        }
        if l == "}" {
            if let Some(mut r) = cur.take() {
                for (raw, second) in [(&raw_grid, false), (&raw_grid2, true)] {
                    if let Some(&(pivot, heading)) = raw.first() {
                        let yaw = -heading.to_radians();
                        let (s, c) = yaw.sin_cos();
                        for (p, _) in raw.iter() {
                            let dx = p.x - pivot.x;
                            let dz = p.z - pivot.z;
                            // a car's rotation about y is the heading itself (it faces (-sin h, 0, -cos h))
                            let slot = (Vec3::new(pivot.x + dx * c - dz * s, p.y, pivot.z + dx * s + dz * c), -yaw);
                            if second {
                                r.grid2.push(slot);
                            } else {
                                r.grid.push(slot);
                            }
                        }
                    }
                }
                out.push(r);
            }
            continue;
        }
        let Some(r) = cur.as_mut() else {
            pending_id = l.to_string();
            continue;
        };
        let mut it = l.split_whitespace();
        let key = it.next().unwrap_or("");
        let vals: Vec<&str> = it.collect();
        let f = |i: usize| vals.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        match key {
            "EasyRace" => r.easy_id = vals.join(" "),
            "RaceType" => r.race_type = vals.join(" "),
            "SubType" => r.sub_type = vals.join(" "),
            "District" => r.district = vals.join(" "),
            "NumLaps" => r.laps = f(0) as u32,
            "DevOnly" => r.dev_only = f(0) != 0.0,
            "StartBeaconPosition" => r.beacon = Vec3::new(f(0), f(1), f(2)),
            "GoalTargetTime" => r.goal_time = f(0),
            "TargetTime" => r.target_times.push(f(0)),
            "CarConfigName" => {
                r.car_config = vals.first().map_or(String::new(), |v| v.chars().take(31).collect());
                let v = vals.join(" ").to_ascii_lowercase();
                if let Some(i) = v.find("vp") {
                    r.car = v[i..].to_string();
                }
            }
            "OpponentsAllowed" => {
                r.opponents = f(0) as usize;
                r.opp_allowed = OPP_ALLOWED_GLOBAL;
            }
            "AllowRaceStarter" => r.no_starter = f(0) == 0.0,
            "SafetyCam" => r.safety_cam = f(0) != 0.0,
            "GridType" => {
                if r.grid_type.is_empty() {
                    r.grid_type = vals.join(" ");
                }
            }
            "NumGridPositions" => grids_seen += 1,
            "GridPosition" => {
                // the first grid; flag games list the other team's as a second one
                if grids_seen <= 1 {
                    raw_grid.push((Vec3::new(f(0), f(1), f(2)), f(3)));
                } else if grids_seen == 2 {
                    raw_grid2.push((Vec3::new(f(0), f(1), f(2)), f(3)));
                }
            }
            "CheckpointPosition" => r.checkpoints.push(Vec3::new(f(0), f(1), f(2))),
            _ => {}
        }
    }
    out
}

/// Load every drivable race (ordered / circuit / unordered / time trial), grouped by district.
pub fn load(data_dir: &str) -> Vec<RaceDef> {
    let dir = Path::new(data_dir).join("tune/race");
    let mut all = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if let Ok(bytes) = std::fs::read(e.path()) {
                let text = String::from_utf8_lossy(&bytes);
                if text.trim_start().starts_with("game/Race") {
                    all.extend(parse(&text));
                }
            }
        }
    }
    let total = all.len();
    let mut v: Vec<RaceDef> = all
        .into_iter()
        .filter(|r| (matches!(r.race_type.as_str(), "Ordered" | "Unordered" | "Delivery") && !r.checkpoints.is_empty() || r.race_type == "Payback" && r.checkpoints.len() >= 2 || r.race_type == "CaptureTheFlag" && !r.checkpoints.is_empty()) && !r.grid.is_empty() && !r.name.ends_with("_easy") && !r.dev_only && !r.name.to_ascii_lowercase().contains("test") && !r.name.to_ascii_lowercase().starts_with("speedprofile"))
        .collect();
    let order = |d: &str| match d {
        "Hollywood" => 0,
        "Downtown" => 1,
        "Beaches" => 2,
        "Hills" => 3,
        "SouthCentral" => 4,
        _ => 5,
    };
    let sub = |s: &str| match s {
        "Circuit" => 0,
        "Ordered" => 1,
        "Unordered" => 2,
        _ => 3,
    };
    v.sort_by(|a, b| order(&a.district).cmp(&order(&b.district)).then(sub(&a.sub_type).cmp(&sub(&b.sub_type))).then(a.name.cmp(&b.name)));
    log::info!("races: {} drivable of {} definitions", v.len(), total);
    v
}

/// The city's default start position (`tune/city/city.lst`): position + yaw (radians).
pub fn default_start(data_dir: &str) -> Option<(Vec3, f32)> {
    let t = std::fs::read_to_string(Path::new(data_dir).join("tune/city/city.lst")).ok()?;
    for line in t.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("DefaultStartPos") {
            let v: Vec<f32> = rest.split(';').next().unwrap_or("").split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 4 {
                // (the rotation is the car's yaw: it faces (-sin, 0, -cos))
                return Some((Vec3::new(v[0], v[1], v[2]), v[3].to_radians()));
            }
        }
    }
    None
}

#[cfg(test)]
mod opp_allowed_tests {
    /// Record +18 as the original's loader F-95490 leaves it (all 227 records read back from the running original hold 7):
    /// an OpponentsAllowed line gives the global whatever its number, no line gives 0.
    #[test]
    fn opp_allowed_is_the_records_global() {
        let with = |n: &str| super::parse(&format!("game/Race/x/Ordered/a\n{{\nRaceType Ordered\nOpponentsAllowed {n}\n}}\n"));
        for n in ["0", "1", "4", "7"] {
            let r = &with(n)[0];
            assert_eq!(r.opp_allowed, 7);
            assert_eq!(r.opponents, n.parse::<usize>().unwrap());
        }
        let r = &super::parse("game/Race/x/Ordered/b\n{\nRaceType Ordered\n}\n")[0];
        assert_eq!(r.opp_allowed, 0);
        // (CarConfigName: the record keeps the name as it is, at most 31 characters; the port's `car` is the model's part)
        let r = &super::parse("game/Race/x/Delivery/c\n{\nRaceType Delivery\nCarConfigName   Delivery_vp_am_vantage_07\n}\n")[0];
        assert_eq!((r.car_config.as_str(), r.car.as_str()), ("Delivery_vp_am_vantage_07", "vp_am_vantage_07"));
    }
}
