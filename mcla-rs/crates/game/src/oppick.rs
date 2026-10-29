//! Who races against the player: the race scripts' opponent pick (RaceWrapper `game/race/0xc4e26a20`
//! 0x107..0x546; TournamentCore, OrderedSeriesCore and the series scripts run the same steps).
//! Decode: docs/reference/ai/car_pick.md.
//!
//! ```text
//! OpponentManager_ClearUsedCharacters()
//! arr = mcCarArray_Fill(player, d = +3100, district mask, class mask, -1)
//! per opponent: car = arr[rand % n]                (with replacement)
//!               driver = the first unused character (map order) who has that car, else the first unused
//!               config = his entry of that car, else his car nearest the difficulty
//!               MarkCharacterAsUsed(driver); set-up = config's (one per car on the disc, still a draw)
//!               Racer_SetModValue(mod value of the band - mc4::setup_mod_value)
//! ```
//! Everything comes from data, so a car added as data races when it has a vehicle record (a class that
//! is not Cop, a DistrictFilter the race's mask takes, not PlayerOnly / DEVONLY), performance samples at
//! mod 0 / 50 / 100 (`car_performance_sample_0..4.lst`) and a driver in `Opponent.lst` whose Config loads.

use crate::mc4::{diff_window, gap_difficulty, joaat, setup_mod_value, setup_perf};
use crate::vehicles::parked::Mwc;
use std::path::Path;

/// The career's random numbers (mcCareer +0x16C78): the MWC of the game (a = 0x5CDCFAA7) seeded once at
/// boot with 1 - lo 1, carry 0x10001 (F-77509 -> F-82174 / F-37317: lo = s + (s == 0), carry =
/// s ^ rot16(s)) - and never saved or restored. One state for every draw the original makes from it: the
/// opponents' cars and set-ups here, the slot system's random character (F-32224), the local
/// hookman's beat-me-there race (F-73402) (docs/reference/ai/race_difficulty.md 5).
/// (approx.: the original's free-roam "incoming call" timer, mission events in Random mode and
/// CityDescription_FindRaceByRacerPosition draw from it too; the port's versions of those do not, so the
/// sequence runs ahead less often than the original's.)
pub fn career_rng() -> std::sync::MutexGuard<'static, Mwc> {
    static RNG: std::sync::OnceLock<std::sync::Mutex<Mwc>> = std::sync::OnceLock::new();
    RNG.get_or_init(|| std::sync::Mutex::new(Mwc { lo: 1, carry: 0x0001_0001 })).lock().unwrap_or_else(|e| e.into_inner())
}

/// A generator of the port's own for a mod feature, seeded as the career's (lo = s + (s == 0), carry = s ^ rot16(s)) and
/// warmed up; the same `s` always gives the same numbers (stage S4b).
pub fn seeded(s: u32) -> Mwc {
    let mut g = Mwc { lo: s.wrapping_add((s == 0) as u32), carry: s ^ s.rotate_left(16) };
    for _ in 0..3 {
        g.next();
    }
    g
}

/// The seed a pick takes from the career generator's state without advancing it (stage S4b).
pub fn state_seed(rng: &Mwc) -> u32 {
    rng.lo.wrapping_mul(0x9E37_79B1) ^ rng.carry.rotate_left(13)
}

/// An integer draw of the career's generator: `(lo & 0x7FFFFFFF) % n`.
pub fn career_index(n: usize) -> usize {
    ((career_rng().next() & 0x7FFF_FFFF) as usize) % n.max(1)
}

/// mcVehicleInfo Class (index in 0x827E9ABC): Tuner, Muscle, Exotic, SportBike, Luxury, Cop, SUV, LowRider, Misc1.
pub const CLASSES: [&str; 9] = ["TUNER", "MUSCLE", "EXOTIC", "SPORTBIKE", "LUXURY", "COP", "SUV", "LOWRIDER", "MISC1"];
pub const CLASS_COP: u32 = 5;
/// DistrictFilter names (table 0x827E91DC; "SC" / "South Central" also mean bit 4).
const DISTRICTS: [&str; 7] = ["HOLLYWOOD", "BEACHES", "HILLS", "DOWNTOWN", "SOUTHCENTRAL", "UNKNOWN", "ALL"];

/// The race script's masks: the five districts, classes 0..7 (RaceWrapper 0x135, TournamentCore 0x1f9).
pub const MASKS_RACE: (i32, i32) = (31, 255);
/// The LA story scripts that launch OrderedSeriesCore: the four LA districts, Tuner .. Luxury.
pub const MASKS_LA_SERIES: (i32, i32) = (15, 31);

/// A vehicle record's fields the pick reads (F-87336: +24 Class, +40 DEVONLY, +42 PlayerOnly, +60 DistrictFilter).
#[derive(Clone, Debug, Default)]
pub struct VehInfo {
    pub class: u32,
    pub dev_only: bool,
    pub player_only: bool,
    pub districts: i32,
}

/// One node of the sample table 0x828CD0FC.
#[derive(Clone, Debug)]
pub struct PickCar {
    /// as the sample file spells it (sample +40)
    pub name: String,
    pub samples: [f32; 3],
    pub info: VehInfo,
}

#[derive(Clone, Debug)]
pub struct PickDriver {
    pub id: String,
    /// (vehicle as Opponent.lst spells it, set-ups) in his car map's order
    pub cars: Vec<(String, Vec<String>)>,
}

/// One opponent of the pick.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub driver: String,
    pub car: String,
    pub setup: String,
    /// the car drawn from the list (differs from `car` when no free driver has it)
    pub drawn: String,
}

#[derive(Default)]
pub struct OpPick {
    /// the sample table in its iteration order
    pub cars: Vec<PickCar>,
    /// the opponent manager's characters (mgr+36) in their map order
    pub drivers: Vec<PickDriver>,
    /// opponent-marked mod cars without a driver of their own (lower case; stage S4b, `modcars::free_driver_cars`): a draw of
    /// one gets a free driver of the race's performance band, picked by a generator seeded from the career generator's
    /// state (which it does not advance), in the set-up `modcars::free_setup_name`
    pub free_cars: Vec<String>,
}

/// rage::atMap's iteration order for keys inserted in this order: buckets 0..n-1, each chain from its
/// head. The first insert makes 11 buckets; when the count reaches the bucket count the map grows to
/// the next prime of the table (F-82002) and the old nodes move, bucket by bucket from each chain's
/// head, to the heads of their new buckets; a new node goes to the head of `key % buckets`. A key
/// already in the map is not inserted again. Returns indices into `keys`.
pub fn atmap_order(keys: &[u32]) -> Vec<usize> {
    const PRIMES: [usize; 15] = [11, 29, 59, 107, 191, 331, 563, 953, 1609, 2729, 4621, 7841, 13297, 22571, 38351];
    let prime = |n: usize| PRIMES.iter().copied().find(|&p| p > n).unwrap_or(65167);
    let mut buckets: Vec<Vec<usize>> = Vec::new();
    let mut count = 0usize;
    for (i, &k) in keys.iter().enumerate() {
        if buckets.iter().flatten().any(|&j| keys[j] == k) {
            continue;
        }
        if buckets.is_empty() {
            buckets = vec![Vec::new(); prime(10)];
        }
        count += 1;
        if count == buckets.len() {
            let mut grown: Vec<Vec<usize>> = vec![Vec::new(); prime(buckets.len())];
            for chain in &buckets {
                for &j in chain {
                    let n = grown.len();
                    grown[(keys[j] as usize) % n].insert(0, j);
                }
            }
            buckets = grown;
        }
        let n = buckets.len();
        buckets[(k as usize) % n].insert(0, i);
    }
    buckets.into_iter().flatten().collect()
}

/// A data file of the game by its name: the extracted copy is named by the joaat of the name, a
/// file added by hand may keep its own name.
fn read_named(dir: &Path, name: &str) -> Option<String> {
    let hashed = dir.join(format!("0x{:08x}", joaat(name)));
    let b = std::fs::read(&hashed).or_else(|_| std::fs::read(dir.join(name))).ok()?;
    Some(String::from_utf8_lossy(&b).into_owned())
}

/// The `Vehicle { .. }` blocks of a vehicle list: (name, record).
pub fn parse_vehicles(text: &str, out: &mut Vec<(String, VehInfo)>) {
    let mut cur: Option<(String, VehInfo)> = None;
    let mut pending = false;
    for line in text.lines() {
        let l = line.trim();
        if l == "Vehicle" {
            pending = true;
            continue;
        }
        if l == "{" {
            if pending {
                cur = Some((String::new(), VehInfo::default()));
            }
            pending = false;
            continue;
        }
        pending = false;
        if l == "}" {
            if let Some(c) = cur.take().filter(|c| !c.0.is_empty()) {
                out.push(c);
            }
            continue;
        }
        let Some((name, v)) = cur.as_mut() else { continue };
        let mut it = l.split_whitespace();
        let Some(k) = it.next() else { continue };
        let rest: Vec<&str> = it.collect();
        match k {
            "Name" => *name = rest.first().copied().unwrap_or("").to_string(),
            // (an unknown class is 0 with an error)
            "Class" => v.class = rest.first().and_then(|c| CLASSES.iter().position(|n| n.eq_ignore_ascii_case(c))).unwrap_or(0) as u32,
            "DEVONLY" => v.dev_only = true,
            "PlayerOnly" => v.player_only = rest.first().and_then(|x| x.parse::<i32>().ok()).unwrap_or(0) != 0,
            "DistrictFilter" => {
                let d = rest.join("").to_ascii_uppercase();
                let bit = if d == "SC" { Some(4) } else { DISTRICTS.iter().position(|n| *n == d) };
                if let Some(b) = bit {
                    v.districts |= 1 << b;
                }
            }
            _ => {}
        }
    }
}

impl OpPick {
    /// Start-up loads (F-71171): the vehicle lists `vehicle.lst`, `vehicle0001..0008.lst` (+ `extra`:
    /// vehicle blocks the port adds itself, in the same format), the samples `car_performance_sample_0..4.lst` (F-21730: a car
    /// not in the vehicle lists or DEVONLY is skipped) and `tune/Career/CarCfg/Opponent.lst` (F-99400).
    pub fn load(data_dir: &str, extra: &str) -> OpPick {
        let root = Path::new(data_dir);
        let vdir = root.join("tune/vehicle");
        let mut vehicles: Vec<(String, VehInfo)> = Vec::new();
        for i in 0..9 {
            let name = if i == 0 { "vehicle.lst".to_string() } else { format!("vehicle{i:04}.lst") };
            if let Some(t) = read_named(&vdir, &name) {
                parse_vehicles(&t, &mut vehicles);
            }
        }
        parse_vehicles(extra, &mut vehicles);
        let mut found: Vec<PickCar> = Vec::new();
        for i in 0..6 {
            // (after the five files, the samples of `extra`)
            let Some(t) = (if i < 5 { read_named(&vdir, &format!("car_performance_sample_{i}.lst")) } else { Some(extra.to_string()) }) else { continue };
            // (blocks start at the word `sample`: the disc's files hold it only as a word; a mod car's name may contain it)
            let words: Vec<&str> = t.split_whitespace().collect();
            for s in words.split(|w| *w == "sample").skip(1) {
                let tok: Vec<&str> = s.to_vec();
                let field = |k: &str| tok.iter().position(|t| *t == k).and_then(|i| tok.get(i + 1)).copied();
                let (Some(name), Some(m), Some(p)) = (field("name"), field("mod").and_then(|v| v.parse::<i32>().ok()), field("performance").and_then(|v| v.parse::<f32>().ok())) else { continue };
                let Some(info) = vehicles.iter().find(|v| v.0.eq_ignore_ascii_case(name)).map(|v| v.1.clone()) else { continue };
                if info.dev_only {
                    continue;
                }
                let k = match found.iter().position(|c| joaat(&c.name) == joaat(name)) {
                    Some(k) => k,
                    None => {
                        found.push(PickCar { name: name.to_string(), samples: [f32::NAN; 3], info });
                        found.len() - 1
                    }
                };
                match m {
                    0 => found[k].samples[0] = p,
                    50 => found[k].samples[1] = p,
                    100 => found[k].samples[2] = p,
                    _ => {}
                }
            }
        }
        let order = atmap_order(&found.iter().map(|c| joaat(&c.name)).collect::<Vec<_>>());
        let cars: Vec<PickCar> = order.into_iter().map(|i| found[i].clone()).collect();
        let mut drivers = Vec::new();
        // (`extra`'s Character blocks as if written at the end of the file: a character already
        // there gets the cars at the end of his block)
        if let Some(t) = read_named(&root.join("tune/career/carcfg"), "opponent.lst") {
            let mut raw = parse_drivers_raw(&t);
            for m in parse_drivers_raw(extra) {
                match raw.iter_mut().find(|d| d.id.eq_ignore_ascii_case(&m.id)) {
                    Some(d) => {
                        for (car, setups) in m.cars {
                            match d.cars.iter_mut().find(|c| joaat(&c.0) == joaat(&car)) {
                                Some(c) => c.1.extend(setups.into_iter().filter(|s| !c.1.contains(s)).collect::<Vec<_>>()),
                                None => d.cars.push((car, setups)),
                            }
                        }
                    }
                    None => raw.push(m),
                }
            }
            drivers = in_map_order(raw);
        }
        log::info!("opponent pick: {} cars, {} drivers", cars.len(), drivers.len());
        OpPick { cars, drivers, free_cars: Vec::new() }
    }

    pub fn car(&self, name: &str) -> Option<&PickCar> {
        self.cars.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// mcCarArray_Fill (F-77344 -> F-88029): indices of the cars a race can draw for a player
    /// worth `p` at difficulty `d`. When a pass keeps one car or none: d one step up (p < 0.5) or down,
    /// the masks shifted (m1 = m2, m2 = m3, m3 = -1), again; at d -2 or 4 the table's first car is added
    /// to what the pass kept. `usable` (the port's: a car whose model it can load) is one more filter.
    pub fn fill(&self, p: f32, mut d: i32, mut m1: i32, mut m2: i32, mut m3: i32, usable: &dyn Fn(&str) -> bool) -> Vec<usize> {
        loop {
            let arr: Vec<usize> = (0..self.cars.len())
                .filter(|&k| {
                    let c = &self.cars[k];
                    let (lo, hi) = diff_window(p, d);
                    c.info.class != CLASS_COP && !c.info.player_only && c.info.districts & m1 != 0 && (1i64 << c.info.class) as i32 & m2 != 0 && c.samples[2] >= lo && c.samples[0] <= hi && usable(&c.name)
                })
                .collect();
            if arr.len() > 1 {
                return arr;
            }
            d = if p < 0.5 { d + 1 } else { d - 1 };
            if d == -2 || d == 4 {
                let mut arr = arr;
                if !self.cars.is_empty() {
                    arr.push(0);
                }
                return arr;
            }
            (m1, m2, m3) = (m2, m3, -1);
        }
    }

    /// The opponents' cars, drivers and set-ups for `n` opponents (the script's loop, two draws of the
    /// career generator each). `used` = characters already on the grid (OrderedSeriesCore marks its fixed
    /// men before the loop).
    #[allow(clippy::too_many_arguments)]
    pub fn pick(&self, n: usize, p: f32, d: i32, masks: (i32, i32), used: &[String], usable: &dyn Fn(&str) -> bool) -> Vec<Pick> {
        let mut rng = career_rng();
        self.pick_with(&mut rng, n, p, d, masks, used, usable)
    }

    /// `pick` with the generator given (the draws are `(lo & 0x7FFFFFFF) % n`, [`career_index`]).
    pub fn pick_with(&self, rng: &mut Mwc, n: usize, p: f32, d: i32, masks: (i32, i32), used: &[String], usable: &dyn Fn(&str) -> bool) -> Vec<Pick> {
        let index = |rng: &mut Mwc, n: usize| ((rng.next() & 0x7FFF_FFFF) as usize) % n.max(1);
        let arr = self.fill(p, d, masks.0, masks.1, -1, usable);
        // (the generator of the free drivers: its own, seeded from the state before this pick's draws)
        let mut free_rng = seeded(state_seed(rng));
        let mut taken: Vec<bool> = self.drivers.iter().map(|c| used.iter().any(|u| u.eq_ignore_ascii_case(&c.id))).collect();
        let mut out = Vec::new();
        if arr.is_empty() {
            return out;
        }
        for _ in 0..n {
            let drawn = self.cars[arr[index(rng, arr.len())]].name.clone();
            // A mod car with no driver of its own (stage S4b, DECISIONS.md D28): a random free driver among the characters that have
            // a car in this race's band (the cars `fill` kept); none free -> the original's rule below. Two draws of the career
            // generator per opponent stay (the set-up draw is made, as the original's).
            if self.free_cars.iter().any(|c| c.eq_ignore_ascii_case(&drawn)) {
                let in_band = |k: usize| self.drivers[k].cars.iter().any(|c| arr.iter().any(|&a| self.cars[a].name.eq_ignore_ascii_case(&c.0)));
                let cands: Vec<usize> = (0..self.drivers.len()).filter(|&k| !taken[k] && in_band(k)).collect();
                if !cands.is_empty() {
                    let ci = cands[free_rng.index(cands.len() as u32) as usize];
                    taken[ci] = true;
                    let _ = index(rng, 1);
                    let id = self.drivers[ci].id.clone();
                    out.push(Pick { setup: crate::modcars::free_setup_name(&id, &drawn), driver: id, car: drawn.clone(), drawn });
                    continue;
                }
            }
            // OpponentManager_FindRandomAvailableCharacterByVehicleName (F-95899): not random
            let free = |k: usize| !taken[k];
            let chr = (0..self.drivers.len()).find(|&k| free(k) && self.drivers[k].cars.iter().any(|c| joaat(&c.0) == joaat(&drawn))).or_else(|| (0..self.drivers.len()).find(|&k| free(k)));
            let Some(ci) = chr else {
                // (no character left: no config, mod 100, the set-up vp_vw_golf_83)
                out.push(Pick { driver: String::new(), car: "vp_vw_golf_83".into(), setup: "vp_vw_golf_83".into(), drawn });
                continue;
            };
            taken[ci] = true;
            let ch = &self.drivers[ci];
            // mcCharacterConfigData_GetCarConfigDataByVehicleName (F-46809, class mask 255)
            let class_ok = |car: &str| self.car(car).map_or(true, |c| (1i64 << c.info.class) as i32 & 255 != 0);
            let cfg = ch.cars.iter().filter(|c| class_ok(&c.0)).find(|c| c.0 == drawn).or_else(|| {
                // (his car whose difficulty class at its band mod value is nearest d; the first wins a tie)
                let cls = |car: &str| self.car(car).map(|c| gap_difficulty(setup_perf(c.samples, setup_mod_value(c.samples, p, d)) - p));
                let mut best: Option<(&(String, Vec<String>), i32)> = None;
                for c in ch.cars.iter().filter(|c| class_ok(&c.0)) {
                    let Some(k) = cls(&c.0) else { continue };
                    if best.map_or(true, |b| (k - d).abs() < (b.1 - d).abs()) {
                        best = Some((c, k));
                    }
                }
                best.map(|b| b.0)
            });
            let Some(cfg) = cfg else {
                out.push(Pick { driver: ch.id.clone(), car: "vp_vw_golf_83".into(), setup: "vp_vw_golf_83".into(), drawn });
                continue;
            };
            // mcCarConfigData_GetRandomConfigData: a draw even with one set-up
            let setup = if cfg.1.is_empty() { "vp_vw_golf_83".to_string() } else { cfg.1[index(rng, cfg.1.len())].clone() };
            out.push(Pick { driver: ch.id.clone(), car: cfg.0.clone(), setup, drawn });
        }
        out
    }
}

/// `Character { Name <id> Car { Name <vehicle> Config { Name <setup> } .. } .. }` in the map orders
/// (characters by joaat(id), each one's cars by joaat(vehicle); a repeated car adds its set-ups).
pub fn parse_drivers(text: &str) -> Vec<PickDriver> {
    in_map_order(parse_drivers_raw(text))
}

/// The characters and each one's cars in the order the maps were filled.
fn parse_drivers_raw(text: &str) -> Vec<PickDriver> {
    let mut list: Vec<PickDriver> = Vec::new();
    // (blocks start at the word `Character`, as for the samples)
    let all: Vec<&str> = text.split_whitespace().collect();
    for ch in all.split(|w| *w == "Character").skip(1) {
        let mut words = ch.iter().copied();
        let mut id = String::new();
        let mut cars: Vec<(String, Vec<String>)> = Vec::new();
        let mut cur: Option<usize> = None;
        let mut depth = 0;
        while let Some(w) = words.next() {
            match w {
                "{" => depth += 1,
                "}" => {
                    depth -= 1;
                    if depth <= 0 {
                        break;
                    }
                }
                "Name" => {
                    let v = words.next().unwrap_or("").to_string();
                    match depth {
                        1 => id = v,
                        2 => {
                            cur = Some(match cars.iter().position(|c| joaat(&c.0) == joaat(&v)) {
                                Some(k) => k,
                                None => {
                                    cars.push((v, Vec::new()));
                                    cars.len() - 1
                                }
                            })
                        }
                        3 => {
                            if let Some(k) = cur {
                                if !cars[k].1.contains(&v) {
                                    cars[k].1.push(v);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if !id.is_empty() {
            list.push(PickDriver { id, cars });
        }
    }
    list
}

/// The maps' iteration orders: characters by joaat(id), each one's cars by joaat(vehicle).
fn in_map_order(list: Vec<PickDriver>) -> Vec<PickDriver> {
    let list: Vec<PickDriver> = list
        .into_iter()
        .map(|d| {
            let order = atmap_order(&d.cars.iter().map(|c| joaat(&c.0)).collect::<Vec<_>>());
            PickDriver { cars: order.into_iter().map(|i| d.cars[i].clone()).collect(), id: d.id }
        })
        .collect();
    let order = atmap_order(&list.iter().map(|c| joaat(&c.id)).collect::<Vec<_>>());
    order.into_iter().map(|i| list[i].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<OpPick> {
        let dir = "D:/MCLA/extracted/cache";
        Path::new(dir).join("tune/vehicle").is_dir().then(|| OpPick::load(dir, ""))
    }

    #[test]
    fn tables_in_the_originals_orders() {
        let Some(p) = data() else { return };
        // (docs/reference/ai/car_pick.md 1.4 / 3.8: emulated from the insert / rehash code over the disc data)
        assert_eq!(p.cars.len(), 63);
        assert_eq!(p.cars.iter().take(3).map(|c| c.name.as_str()).collect::<Vec<_>>(), ["vp_pon_transam_79", "vpd_mit_evox_08", "vp_chv_camaross_69"]);
        assert_eq!(p.cars.last().unwrap().name, "vp_chv_police_96");
        assert_eq!(p.drivers.len(), 37);
        assert_eq!(p.drivers.iter().take(4).map(|c| c.id.as_str()).collect::<Vec<_>>(), ["GH_26", "GH_11", "GH_30", "GH_06"]);
        assert_eq!(p.drivers.last().unwrap().id, "GH_09");
        assert_eq!(p.drivers[0].cars.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["vpd_chv_malibu_65", "vpd_lr_hse_08", "vp_vw_golf_83", "vp_aud_r8_08", "vp_sln_s7_06"]);
    }

    #[test]
    fn fill_and_drivers() {
        let Some(p) = data() else { return };
        let all = |_: &str| true;
        let names = |v: Vec<usize>| v.into_iter().map(|k| p.cars[k].name.clone()).collect::<Vec<_>>();
        // (a full Saleen S281 on MEDIUM: the ten cars of car_pick.md 1.4, R8 / Murcielago / 999R among them)
        assert_eq!(
            names(p.fill(0.790155, 1, 31, 255, -1, &all)),
            ["vp_kaw_ninja_zx14_06", "vp_chv_corvettez06_07", "vp_lam_gallardo_06", "vp_lam_murcielago_06", "vp_frd_cobra_04", "vp_duc_999r_06", "vp_aud_r8_08", "vpd_duc_1098_07", "vp_sln_s7_06", "vp_frd_gt_06"]
        );
        // (a Golf 83 stock on EASY: six cars)
        assert_eq!(names(p.fill(0.067076, 0, 31, 255, -1, &all)).len(), 6);
        // (a full ZX14 at d 1 falls to d 0 with the masks shifted: nine cars)
        assert_eq!(names(p.fill(1.0, 1, 31, 255, -1, &all)).len(), 9);
        // drivers: the first unused who has the car, in the map order; the 1098 has none
        let picks = p.pick(7, 0.790155, 1, MASKS_RACE, &[], &all);
        assert_eq!(picks.len(), 7);
        for k in 0..picks.len() {
            assert!(!picks[..k].iter().any(|q| q.driver == picks[k].driver));
        }
        let r8 = p.pick(1, 0.790155, 1, MASKS_RACE, &[], &|c: &str| c == "vp_aud_r8_08");
        // (the list holds one car: fallback adds vp_pon_transam_79 - only usable cars pass, so check the R8's driver)
        if r8[0].drawn == "vp_aud_r8_08" {
            assert_eq!(r8[0].driver, "GH_26");
        }
        let two = p.pick(2, 0.790155, 1, MASKS_RACE, &["GH_26".into()], &all);
        assert!(two.iter().all(|q| q.driver != "GH_26"));
    }

    /// The cut GT-R through `extra` (DECISIONS.md D21): a car of the table with the R8's samples,
    /// drawn where the R8 is, raced by GH_12 in its set-up; the disc's cars keep their places.
    /// The original's race start recorded (work/ai-record/orig/pick3.txt, 2026-10-26; the recomp, Saleen S281
    /// fully upgraded, ARCADE AROUND HOLLYWOOD, MEDIUM, 3 opponents): the career generator stood at
    /// (lo 0xe705bc4e, carry 0x2d07775a); seven draws followed (states in pick3.txt), the field was ZX14,
    /// Gallardo, Gallardo. The draws 1 / 3 / 5 are the cars (index % 10 = 0, 2, 2 in the Fill list of
    /// car_pick.md 1.4), 2 / 4 / 6 the set-ups; the seventh is not the pick's (the port has no user for it).
    #[test]
    fn recorded_pick_draws() {
        let Some(p) = data() else { return };
        let mut r = Mwc { lo: 0xe705bc4e, carry: 0x2d07775a };
        let picks = p.pick_with(&mut r, 3, 0.790155, 1, MASKS_RACE, &[], &|_: &str| true);
        assert_eq!(picks.iter().map(|q| q.drawn.as_str()).collect::<Vec<_>>(), ["vp_kaw_ninja_zx14_06", "vp_lam_gallardo_06", "vp_lam_gallardo_06"]);
        // six draws used: the state the recording shows after the sixth
        assert_eq!((r.lo, r.carry), (0xc6269a6f, 0x405fae24));
        // a second recorded start (pick6.txt, the next race): generator (0x943e97cd, 0x496cf02a) before; Murcielago,
        // GT, R8 raced; after the sixth draw (0x1e4b6d15, 0x5566dbd8) (seven draws in all: the seventh is not the pick's)
        let mut r = Mwc { lo: 0x943e97cd, carry: 0x496cf02a };
        let picks = p.pick_with(&mut r, 3, 0.790155, 1, MASKS_RACE, &[], &|_: &str| true);
        assert_eq!(picks.iter().map(|q| q.drawn.as_str()).collect::<Vec<_>>(), ["vp_lam_murcielago_06", "vp_frd_gt_06", "vp_aud_r8_08"]);
        assert_eq!((r.lo, r.carry), (0x1e4b6d15, 0x5566dbd8));
    }

    #[test]
    fn cut_gtr_as_an_opponent() {
        let dir = "D:/MCLA/extracted/cache";
        if !Path::new(dir).join("tune/vehicle").is_dir() {
            return;
        }
        let p = OpPick::load(dir, &format!("{}{}", crate::catalog::CUT_CATALOG, crate::catalog::CUT_OPPONENT));
        let disc = OpPick::load(dir, "");
        assert_eq!(p.cars.len(), disc.cars.len() + 1);
        let gtr = p.car(crate::catalog::CUT_GTR).expect("gt-r");
        assert_eq!(gtr.samples, p.car("vp_aud_r8_08").unwrap().samples);
        assert_eq!(p.drivers.len(), disc.drivers.len());
        let gh12 = p.drivers.iter().find(|d| d.id == "GH_12").unwrap();
        assert!(gh12.cars.iter().any(|c| c.0 == crate::catalog::CUT_GTR && c.1 == [crate::catalog::CUT_SETUP]));
        let all = |_: &str| true;
        let names = |q: &OpPick, v: Vec<usize>| v.into_iter().map(|k| q.cars[k].name.clone()).collect::<Vec<_>>();
        let with = names(&p, p.fill(0.790155, 1, 31, 255, -1, &all));
        assert!(with.iter().any(|n| n == crate::catalog::CUT_GTR));
        let mut without = names(&disc, disc.fill(0.790155, 1, 31, 255, -1, &all));
        without.push(crate::catalog::CUT_GTR.into());
        let mut with_sorted = with.clone();
        with_sorted.sort();
        without.sort();
        assert_eq!(with_sorted, without);
        // (drawn alone, GH_12 takes it in its set-up)
        let one = p.pick(1, 0.790155, 1, MASKS_RACE, &[], &|c: &str| c == crate::catalog::CUT_GTR || c == "vp_aud_r8_08");
        if one[0].drawn == crate::catalog::CUT_GTR {
            assert_eq!((one[0].driver.as_str(), one[0].setup.as_str()), ("GH_12", crate::catalog::CUT_SETUP));
        }
    }

    /// A mod car marked for opponents (MODS.md design 7b, stage S4) through `extra` after the cut GT-R's entries: a
    /// synthetic mod (a 350Z copy under its own name, the 350Z's samples, drivers GH_10 and GH_23) read by the mods
    /// crate from the disc's lists. The disc's tables and the GT-R keep their places and values; the mod car is in the
    /// band exactly where the 350Z is; its drivers take it in their set-ups; an unmarked mod car is in no table.
    #[test]
    fn mod_car_as_an_opponent() {
        let dir = "D:/MCLA/extracted/cache";
        if !Path::new(dir).join("tune/vehicle").is_dir() {
            return;
        }
        let mods = std::env::temp_dir().join(format!("mcla-oppick-mod-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&mods);
        let write = |m: &str, car: &str, opp: &str| {
            let root = mods.join(m);
            for f in ["a.r63", "b.r83", "c.r83", "d.r19"] {
                let p = root.join(format!("resources/vehicle/{car}/{f}"));
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, f).unwrap();
            }
            let d = "vp_nsn_350z_06";
            let toml = format!(
                "[mod]\nid = \"{m}\"\nversion = \"1\"\nname = \"{m}\"\n[[car]]\nname = \"{car}\"\nlong_name = \"T\"\nshort_name = \"T\"\n\
                 [car.row]\nclass = \"TUNER\"\nmanufacturer = \"Nissan\"\ngroup = \"C0\"\nunlocking_group = \"B0\"\nprice = 37000\n\
                 districts = [\"Hollywood\", \"Beaches\", \"Hills\", \"Downtown\"]\nfigures_from = \"{d}\"\n\
                 [car.donors]\ntune = \"{d}\"\nsetup = \"{d}\"\nrims = \"whl_stk_nsn_350z_06\"\ngarage = \"{d}\"\nprices = \"{d}\"\nsound = \"{d}\"\npictures = \"{d}\"\n{opp}"
            );
            std::fs::write(root.join("mod.toml"), toml).unwrap();
        };
        let car = "vp_test_mod_01";
        write(
            "test.mod01",
            car,
            "[car.opponent]\nenabled = true\nsamples_from = \"vp_nsn_350z_06\"\ndriver = \"GH_10\"\nsetup = \"GH_10_2_vp_test_mod_01\"\nsetup_from = \"GH_10_2_vp_nsn_350z_06\"\nplate = \"TESTMOD\"\n\
             [[car.opponent.drivers]]\ndriver = \"GH_23\"\nsetup = \"GH_23_2_vp_test_mod_01\"\nsetup_from = \"GH_23_2_vp_nsn_350z_06\"\n",
        );
        write("test.mod02", "vp_test_mod_02", "");
        let disc_info = mcla_mods::disc::DiscInfo::load(Path::new(dir));
        let set = mcla_mods::ModSet::load_from(&mods, &disc_info, &mcla_mods::LoadOptions { reserved: vec![crate::catalog::CUT_GTR.into()], ..Default::default() });
        let _ = std::fs::remove_dir_all(&mods);
        assert_eq!(set.cars().len(), 2, "{:?}", set.mods().iter().map(|m| (&m.reasons, m.cars.iter().map(|c| &c.reasons).collect::<Vec<_>>())).collect::<Vec<_>>());
        let text: String = set.cars().iter().filter_map(|c| mcla_mods::opponent_text(c, None)).collect();
        assert!(!text.contains("vp_test_mod_02"), "an unmarked mod car gives the pick nothing");
        let cut = format!("{}{}", crate::catalog::CUT_CATALOG, crate::catalog::CUT_OPPONENT);
        let gtr = OpPick::load(dir, &cut);
        let p = OpPick::load(dir, &format!("{cut}{text}"));
        // the disc's and the GT-R's entries: same values, same order
        assert_eq!(p.cars.len(), gtr.cars.len() + 1);
        let rest: Vec<String> = p.cars.iter().filter(|c| c.name != car).map(|c| format!("{c:?}")).collect();
        assert_eq!(rest, gtr.cars.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>());
        assert_eq!(p.drivers.len(), gtr.drivers.len());
        for (a, b) in p.drivers.iter().zip(&gtr.drivers) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.cars.iter().filter(|c| c.0 != car).cloned().collect::<Vec<_>>(), b.cars);
        }
        // the mod car: the 350Z's samples and record, its two drivers in their set-ups
        let m = p.car(car).expect("mod car in the table");
        let z = p.car("vp_nsn_350z_06").unwrap();
        assert_eq!((m.samples, m.info.class, m.info.districts, m.info.player_only), (z.samples, z.info.class, z.info.districts, false));
        for (id, s) in [("GH_10", "GH_10_2_vp_test_mod_01"), ("GH_23", "GH_23_2_vp_test_mod_01")] {
            let d = p.drivers.iter().find(|d| d.id == id).unwrap();
            assert!(d.cars.iter().any(|c| c.0 == car && c.1 == [s]), "{id}");
        }
        assert!(p.car("vp_test_mod_02").is_none());
        // drawn where the 350Z is: in every band that holds the 350Z, nowhere else
        let all = |_: &str| true;
        let names = |q: &OpPick, v: Vec<usize>| {
            let mut n = v.into_iter().map(|k| q.cars[k].name.clone()).collect::<Vec<_>>();
            n.sort();
            n.dedup();
            n
        };
        let mut bands = 0;
        for d in -1..=3 {
            for k in 0..=20 {
                let pl = k as f32 * 0.05;
                let a = names(&gtr, gtr.fill(pl, d, 31, 255, -1, &all));
                let b = names(&p, p.fill(pl, d, 31, 255, -1, &all));
                if a.len() > 1 {
                    let mut want = a.clone();
                    if a.iter().any(|n| n == "vp_nsn_350z_06") {
                        want.push(car.into());
                        want.sort();
                        bands += 1;
                    }
                    assert_eq!(b, want, "p {pl} d {d}");
                }
            }
        }
        assert!(bands > 5, "{bands}");
        // its driver takes it in his set-up (the first free one with the car in map order)
        let first = p.drivers.iter().position(|d| d.cars.iter().any(|c| c.0 == car)).unwrap();
        let (id, setup) = if p.drivers[first].id == "GH_10" { ("GH_10", "GH_10_2_vp_test_mod_01") } else { ("GH_23", "GH_23_2_vp_test_mod_01") };
        let mut hits = 0;
        for seed in 1..200u32 {
            let mut r = Mwc { lo: seed, carry: 0x0001_0001 };
            let one = p.pick_with(&mut r, 1, 0.45, 1, MASKS_RACE, &[], &|c: &str| c == car || c == "vp_nsn_350z_06" || c == "vp_vw_golf_83");
            if one[0].drawn == car {
                assert_eq!((one[0].driver.as_str(), one[0].car.as_str(), one[0].setup.as_str()), (id, car, setup));
                hits += 1;
            }
        }
        assert!(hits > 0);
        // with that driver on the grid already, the other one takes it
        let other = if id == "GH_10" { ("GH_23", "GH_23_2_vp_test_mod_01") } else { ("GH_10", "GH_10_2_vp_test_mod_01") };
        for seed in 1..200u32 {
            let mut r = Mwc { lo: seed, carry: 0x0001_0001 };
            let one = p.pick_with(&mut r, 1, 0.45, 1, MASKS_RACE, &[id.to_string()], &|c: &str| c == car || c == "vp_nsn_350z_06" || c == "vp_vw_golf_83");
            if one[0].drawn == car {
                assert_eq!((one[0].driver.as_str(), one[0].setup.as_str()), other);
            }
        }
    }

    /// A mod car with no driver of its own (stage S4b, DECISIONS.md D28): the pick gives each draw of it a random free
    /// driver among the characters that have a car in the race's band, in `<driver>_m_<car>`; the same career state gives
    /// the same driver; the career generator's draws are the same two per opponent; with the car out of the pool the
    /// picks are the disc's and the GT-R's, bit for bit.
    #[test]
    fn driverless_mod_car_gets_a_driver_of_the_band() {
        let dir = "D:/MCLA/extracted/cache";
        if !Path::new(dir).join("tune/vehicle").is_dir() {
            return;
        }
        let car = "vp_test_free_01";
        let cut = format!("{}{}", crate::catalog::CUT_CATALOG, crate::catalog::CUT_OPPONENT);
        let base = OpPick::load(dir, &cut);
        let z = base.car("vp_nsn_350z_06").unwrap().samples;
        let mut text = format!("Vehicle\r\n{{\r\n\tName\t{car}\r\n\tClass\tTUNER\r\n\tDistrictFilter\tHollywood\r\n\tDistrictFilter\tBeaches\r\n\tDistrictFilter\tHills\r\n\tDistrictFilter\tDowntown\r\n}}\r\n");
        for (m, v) in [0, 50, 100].iter().zip(z) {
            text.push_str(&format!("sample\r\n{{\r\n\tname\t{car}\r\n\tmod\t{m}\r\n\tperformance\t{v}\r\n}}\r\n"));
        }
        let mut p = OpPick::load(dir, &format!("{cut}{text}"));
        p.free_cars = vec![car.to_string()];
        assert_eq!(p.cars.len(), base.cars.len() + 1);
        assert_eq!(p.drivers.len(), base.drivers.len(), "no Character block for it");
        let (pl, d) = (0.45f32, 1);
        let only = |c: &str| c == car || c == "vp_nsn_350z_06" || c == "vp_vw_golf_83" || c == "vp_chv_camaro_06";
        // the band: the cars the fill kept
        let band: Vec<String> = p.fill(pl, d, 31, 255, -1, &only).into_iter().map(|k| p.cars[k].name.clone()).collect();
        assert!(band.iter().any(|n| n == car));
        let in_band = |id: &str| p.drivers.iter().find(|x| x.id == id).unwrap().cars.iter().any(|c| band.iter().any(|b| b.eq_ignore_ascii_case(&c.0)));
        let mut seen = std::collections::BTreeSet::new();
        for seed in 1..300u32 {
            let start = Mwc { lo: seed, carry: 0x0001_0001 };
            let (mut r1, mut r2) = (start, start);
            let a = p.pick_with(&mut r1, 3, pl, d, MASKS_RACE, &[], &only);
            let b = p.pick_with(&mut r2, 3, pl, d, MASKS_RACE, &[], &only);
            assert_eq!(a, b, "the same seed, the same drivers");
            assert_eq!((r1.lo, r1.carry), (r2.lo, r2.carry));
            // (six draws of the career generator: two per opponent)
            let mut c = start;
            for _ in 0..6 {
                c.next();
            }
            assert_eq!((r1.lo, r1.carry), (c.lo, c.carry), "seed {seed}");
            let mut ids = std::collections::BTreeSet::new();
            for pk in &a {
                assert!(ids.insert(pk.driver.clone()), "a driver once per race: {a:?}");
                if pk.drawn == car {
                    assert_eq!(pk.car, car);
                    assert_eq!(pk.setup, format!("{}_m_{car}", pk.driver));
                    assert!(in_band(&pk.driver), "{} is not in the band", pk.driver);
                    seen.insert(pk.driver.clone());
                }
            }
        }
        assert!(seen.len() > 3, "random among the band's drivers: {seen:?}");
        // a driver already on the grid is not drawn
        for seed in 1..100u32 {
            let mut r = Mwc { lo: seed, carry: 0x0001_0001 };
            let used: Vec<String> = seen.iter().take(2).cloned().collect();
            for pk in p.pick_with(&mut r, 2, pl, d, MASKS_RACE, &used, &only) {
                assert!(!used.contains(&pk.driver), "{pk:?}");
            }
        }
        // the car out of the pool: the disc's and the GT-R's picks are the same as without it
        let not_it = |c: &str| c != car;
        for seed in 1..200u32 {
            let start = Mwc { lo: seed, carry: 0x0001_0001 };
            let (mut r1, mut r2) = (start, start);
            let a = p.pick_with(&mut r1, 5, 0.790155, 1, MASKS_RACE, &[], &not_it);
            let b = base.pick_with(&mut r2, 5, 0.790155, 1, MASKS_RACE, &[], &not_it);
            assert_eq!(a, b);
            assert_eq!((r1.lo, r1.carry), (r2.lo, r2.carry));
        }
    }
}
