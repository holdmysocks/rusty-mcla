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

/// mcVehicleInfo Class (index in 0x827E9ABC): Tuner, Muscle, Exotic, SportBike, Luxury, Cop, SUV, LowRider, Misc1.
pub const CLASSES: [&str; 9] = ["TUNER", "MUSCLE", "EXOTIC", "SPORTBIKE", "LUXURY", "COP", "SUV", "LOWRIDER", "MISC1"];
pub const CLASS_COP: u32 = 5;
/// DistrictFilter names (table 0x827E91DC; "SC" / "South Central" also mean bit 4).
const DISTRICTS: [&str; 7] = ["HOLLYWOOD", "BEACHES", "HILLS", "DOWNTOWN", "SOUTHCENTRAL", "UNKNOWN", "ALL"];

/// The race script's masks: the five districts, classes 0..7 (RaceWrapper 0x135, TournamentCore 0x1f9).
pub const MASKS_RACE: (i32, i32) = (31, 255);
/// The LA story scripts that launch OrderedSeriesCore: the four LA districts, Tuner .. Luxury.
pub const MASKS_LA_SERIES: (i32, i32) = (15, 31);

/// A vehicle record's fields the pick reads (sub_823964C0: +24 Class, +40 DEVONLY, +42 PlayerOnly, +60 DistrictFilter).
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
}

/// rage::atMap's iteration order for keys inserted in this order: buckets 0..n-1, each chain from its
/// head. The first insert makes 11 buckets; when the count reaches the bucket count the map grows to
/// the next prime of the table (sub_821C9438) and the old nodes move, bucket by bucket from each chain's
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
    /// Start-up loads (sub_822D4878): the vehicle lists `vehicle.lst`, `vehicle0001..0008.lst` (+ `extra`:
    /// vehicle blocks the port adds itself, in the same format), the samples `car_performance_sample_0..4.lst` (sub_8239C6F0: a car
    /// not in the vehicle lists or DEVONLY is skipped) and `tune/Career/CarCfg/Opponent.lst` (sub_822B89C8).
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
            for s in t.split("sample").skip(1) {
                let tok: Vec<&str> = s.split_whitespace().collect();
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
        OpPick { cars, drivers }
    }

    pub fn car(&self, name: &str) -> Option<&PickCar> {
        self.cars.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// mcCarArray_Fill (sub_8239D598 -> sub_8239D220): indices of the cars a race can draw for a player
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
    pub fn pick(&self, n: usize, p: f32, d: i32, masks: (i32, i32), used: &[String], rng: &mut Mwc, usable: &dyn Fn(&str) -> bool) -> Vec<Pick> {
        let arr = self.fill(p, d, masks.0, masks.1, -1, usable);
        let mut taken: Vec<bool> = self.drivers.iter().map(|c| used.iter().any(|u| u.eq_ignore_ascii_case(&c.id))).collect();
        let mut out = Vec::new();
        if arr.is_empty() {
            return out;
        }
        for _ in 0..n {
            let drawn = self.cars[arr[((rng.next() & 0x7FFF_FFFF) as usize) % arr.len()]].name.clone();
            // OpponentManager_FindRandomAvailableCharacterByVehicleName (sub_822B8608): not random
            let free = |k: usize| !taken[k];
            let chr = (0..self.drivers.len()).find(|&k| free(k) && self.drivers[k].cars.iter().any(|c| joaat(&c.0) == joaat(&drawn))).or_else(|| (0..self.drivers.len()).find(|&k| free(k)));
            let Some(ci) = chr else {
                // (no character left: no config, mod 100, the set-up vp_vw_golf_83)
                out.push(Pick { driver: String::new(), car: "vp_vw_golf_83".into(), setup: "vp_vw_golf_83".into(), drawn });
                continue;
            };
            taken[ci] = true;
            let ch = &self.drivers[ci];
            // mcCharacterConfigData_GetCarConfigDataByVehicleName (sub_823989B0, class mask 255)
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
            let setup = if cfg.1.is_empty() { "vp_vw_golf_83".to_string() } else { cfg.1[((rng.next() & 0x7FFF_FFFF) as usize) % cfg.1.len()].clone() };
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
    for ch in text.split("Character").skip(1) {
        let mut words = ch.split_whitespace();
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
        let mut rng = Mwc { lo: 1, carry: 1 };
        let picks = p.pick(7, 0.790155, 1, MASKS_RACE, &[], &mut rng, &all);
        assert_eq!(picks.len(), 7);
        for k in 0..picks.len() {
            assert!(!picks[..k].iter().any(|q| q.driver == picks[k].driver));
        }
        let r8 = p.pick(1, 0.790155, 1, MASKS_RACE, &[], &mut Mwc { lo: 1, carry: 1 }, &|c: &str| c == "vp_aud_r8_08");
        // (the list holds one car: fallback adds vp_pon_transam_79 - only usable cars pass, so check the R8's driver)
        if r8[0].drawn == "vp_aud_r8_08" {
            assert_eq!(r8[0].driver, "GH_26");
        }
        let two = p.pick(2, 0.790155, 1, MASKS_RACE, &["GH_26".into()], &mut Mwc { lo: 1, carry: 1 }, &all);
        assert!(two.iter().all(|q| q.driver != "GH_26"));
    }

    /// The cut GT-R through `extra` (DECISIONS.md D21): a car of the table with the R8's samples,
    /// drawn where the R8 is, raced by GH_12 in its set-up; the disc's cars keep their places.
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
        let one = p.pick(1, 0.790155, 1, MASKS_RACE, &[], &mut Mwc { lo: 1, carry: 1 }, &|c: &str| c == crate::catalog::CUT_GTR || c == "vp_aud_r8_08");
        if one[0].drawn == crate::catalog::CUT_GTR {
            assert_eq!((one[0].driver.as_str(), one[0].setup.as_str()), ("GH_12", crate::catalog::CUT_SETUP));
        }
    }
}
