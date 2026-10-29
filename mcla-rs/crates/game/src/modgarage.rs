//! The port's garage beyond the original's (docs/MODS.md M-5 stage S3; a user-approved deviation, DECISIONS.md D27 /
//! D28).
//!
//! The port's garage is `Game::owned` (car names in garage order) and the save's `mc4` block, whose `garage` and
//! `vehicle_records` are lists keyed by the car's name in the port's JSON: neither has the original's fixed sizes (30
//! slots, 77 purchase records, 100 statistics entries; docs/reference/mods/vehicle_lookup.md), and the port's
//! statistics are keyed by name (`vWIN/<car>`). Nothing here caps the garage: it holds every car there is.
//!
//! What this module adds is the **placeholder** of a mod car whose mod is not installed (D27 / D28): the original
//! deletes such a slot on load; the port keeps it. On load the save's garage order is split into the cars the game
//! has (`owned`, everything plays with these) and the placeholders ([`Missing`]: the car's place in the order and its
//! name); on save the two are merged back in their places, and the `mc4` records, the per-car maps of the save and
//! the statistics of a placeholder are never touched. The save keeps per mod car the mod it came from and its names
//! ([`ModCarSave`], `SaveData::mod_cars`, outside the `mc4` block like the keyboard bindings), so a placeholder shows
//! the car's last known name. MANAGE VEHICLES lists the placeholders greyed with a "MOD MISSING" note; they cannot be
//! taken out, sold or staked, and count for no rule (`owned.len()`). Reinstalling the mod brings the car back as it was.

use crate::game::Game;
use std::collections::BTreeMap;

/// What the save keeps of a mod car of the garage (`SaveData::mod_cars`, keyed by the car's name in lower case).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ModCarSave {
    /// the mod (manifest `[mod] id`) the car came from
    pub mod_id: String,
    /// its names when it was last loaded (string keys `<car>` / `<car>_shrt`)
    pub long_name: String,
    pub short_name: String,
}

/// A garage car whose mod is not installed: its place in the save's garage order and its name as the save has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Missing {
    pub at: usize,
    pub name: String,
}

/// The vehicle lists hand placeholders out as indices from here on (never a `car_dirs` index).
pub const PLACEHOLDER: usize = usize::MAX / 2;

/// The placeholder a list index stands for.
pub fn placeholder(i: usize) -> Option<usize> {
    i.checked_sub(PLACEHOLDER)
}

/// A car of the game's own table or one the port adds itself (never a placeholder, even when its folder is not in
/// the garage list of this run).
pub fn is_game_car(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    crate::mc4::VEHICLES.contains(&n.as_str()) || n == crate::catalog::CUT_GTR
}

/// The save's garage order split into the cars present (`present(name)`) and the placeholders: a name that is neither
/// present nor a car of the game.
pub fn split(order: Vec<String>, present: impl Fn(&str) -> bool) -> (Vec<String>, Vec<Missing>) {
    let mut owned = Vec::with_capacity(order.len());
    let mut missing = Vec::new();
    for (at, name) in order.into_iter().enumerate() {
        if present(&name) || is_game_car(&name) {
            owned.push(name);
        } else {
            missing.push(Missing { at, name });
        }
    }
    (owned, missing)
}

/// The garage order for the save: `owned` with the placeholders back in their places (a place past the end, after a
/// car before it was sold, puts it last).
pub fn merge(owned: &[String], missing: &[Missing]) -> Vec<String> {
    let mut out = owned.to_vec();
    let mut m: Vec<&Missing> = missing.iter().collect();
    m.sort_by_key(|x| x.at);
    for x in m {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&x.name)) {
            out.insert(x.at.min(out.len()), x.name.clone());
        }
    }
    out
}

impl Game {
    /// The save's garage (`order`) and mod-car entries, read: the placeholders apart, the vehicle in use one the
    /// game has (a placeholder cannot be driven: the first car of the garage that is here).
    pub fn load_mod_garage(&mut self, order: Vec<String>, mod_cars: BTreeMap<String, ModCarSave>, current: &str) {
        let dirs: Vec<String> = self.car_dirs.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().to_ascii_lowercase()).collect();
        let present = |n: &str| dirs.iter().any(|d| d.eq_ignore_ascii_case(n)) || crate::modcars::is_mod_car(n);
        let (owned, missing) = split(order, present);
        if !missing.is_empty() {
            log::info!("garage: {} car(s) of mods that are not installed kept as placeholders: {}", missing.len(), missing.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", "));
        }
        if missing.iter().any(|m| m.name.eq_ignore_ascii_case(current)) {
            if let Some(i) = owned.iter().find_map(|o| dirs.iter().position(|d| d.eq_ignore_ascii_case(o))) {
                self.current_car = i;
                self.selected_car = i;
            }
        }
        self.owned = owned;
        self.mod_missing = missing;
        self.mod_cars = mod_cars;
    }

    /// The garage order the save holds: the cars (as the save keeps them, `mc4_owned_out`) and the placeholders.
    pub fn garage_order(&self) -> Vec<String> {
        merge(&self.mc4_owned_out(), &self.mod_missing)
    }

    /// The save's mod-car entries: a loaded mod car's from its mod now, a placeholder's (or a car's whose entry
    /// cannot be renewed) as read; cars no longer in the garage have none.
    pub fn mod_cars_out(&self) -> BTreeMap<String, ModCarSave> {
        let mut out = BTreeMap::new();
        for name in self.garage_order() {
            let key = name.to_ascii_lowercase();
            let e = match crate::modcars::car(&key) {
                Some(c) => Some(ModCarSave { mod_id: c.mod_id.clone(), long_name: c.long_name.clone(), short_name: c.short_name.clone() }),
                None => self.mod_cars.get(&key).cloned(),
            };
            if let Some(e) = e {
                out.insert(key, e);
            }
        }
        out
    }

    /// The car name behind a vehicle-list index (a `car_dirs` index or a placeholder).
    pub fn list_car_name(&self, i: usize) -> String {
        match placeholder(i) {
            Some(k) => self.mod_missing.get(k).map(|m| m.name.clone()).unwrap_or_default(),
            None => self.car_dirs.get(i).and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        }
    }

    /// A placeholder's line in the vehicle list: the car's last known short name (the list shows short names), else
    /// its long name, else the car's name.
    pub fn mod_missing_label(&self, k: usize) -> String {
        let Some(m) = self.mod_missing.get(k) else { return String::new() };
        let e = self.mod_cars.get(&m.name.to_ascii_lowercase());
        e.map(|e| e.short_name.trim()).filter(|s| !s.is_empty()).or_else(|| e.map(|e| e.long_name.trim()).filter(|s| !s.is_empty())).unwrap_or(&m.name).to_uppercase()
    }

    /// The vehicle list's MANAGE VEHICLES order: the cars `list` (indices of `car_dirs`) and the placeholders, in the
    /// garage's order.
    pub fn manage_list(&self, list: Vec<usize>) -> Vec<usize> {
        let mut l = list;
        l.extend((0..self.mod_missing.len()).map(|k| PLACEHOLDER + k));
        let order = merge(&self.owned, &self.mod_missing);
        l.sort_by_key(|i| {
            let n = self.list_car_name(*i);
            order.iter().position(|o| o.eq_ignore_ascii_case(&n)).unwrap_or(usize::MAX)
        });
        l
    }
}

/// The note a placeholder's class line shows in MANAGE VEHICLES.
pub const MOD_MISSING: &str = "MOD MISSING";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;

    /// A folder of its own for one test's save (never the player's).
    fn dir(tag: &str) -> String {
        std::env::remove_var("MCLA_SAVE_FILE");
        std::env::remove_var("MCLA_NEW_GAME");
        let d = std::env::temp_dir().join(format!("mcla_modgarage_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().to_string()
    }

    fn game(cars: &[&str], dir: &str) -> Game {
        let mut g = Game::new(cars.iter().map(PathBuf::from).collect(), 0);
        g.data_dir = dir.to_string();
        g.load_save(dir);
        g.ensure_owned();
        g
    }

    fn file(dir: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(dir).join("mcla_save.json")).unwrap()).unwrap()
    }

    fn names(v: &Value) -> Vec<String> {
        v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
    }

    fn record<'a>(save: &'a Value, car: &str) -> &'a Value {
        save["mc4"]["garage"].as_array().unwrap().iter().find(|c| c["vehicle"].as_str() == Some(car)).unwrap_or_else(|| panic!("no record of {car}"))
    }

    #[test]
    fn split_and_merge_keep_the_order() {
        let order: Vec<String> = ["vp_vw_golf_83", "vp_mod_a_08", "vp_nsn_350z_06", "vp_nsn_gtr_08", "vp_mod_b_08"].iter().map(|s| s.to_string()).collect();
        let (owned, missing) = split(order.clone(), |n| n == "vp_vw_golf_83");
        // (a disc car or the cut GT-R is never a placeholder, even without its folder in this run)
        assert_eq!(owned, ["vp_vw_golf_83", "vp_nsn_350z_06", "vp_nsn_gtr_08"]);
        assert_eq!(missing, [Missing { at: 1, name: "vp_mod_a_08".into() }, Missing { at: 4, name: "vp_mod_b_08".into() }]);
        assert_eq!(merge(&owned, &missing), order);
        // a car before them sold: the places hold as far as they can, the last goes last
        assert_eq!(merge(&owned[1..], &missing), ["vp_nsn_350z_06", "vp_mod_a_08", "vp_nsn_gtr_08", "vp_mod_b_08"]);
        assert_eq!(placeholder(PLACEHOLDER + 3), Some(3));
        assert_eq!(placeholder(68), None);
    }

    /// A save written before S3 (no `mod_cars`) loads unchanged and is written back without the new member.
    #[test]
    fn old_format_save_loads_unchanged() {
        let d = dir("old");
        let old = serde_json::json!({
            "car": "vp_nsn_350z_06", "color": 3, "cfg": serde_json::to_value(crate::vehicle_model::CarConfig::default()).unwrap(),
            "best_time": null, "money": 12345, "rep": 77, "owned": ["vp_vw_golf_83", "vp_nsn_350z_06"],
            "perf": {"vp_nsn_350z_06": 2}
        });
        std::fs::write(std::path::Path::new(&d).join("mcla_save.json"), old.to_string()).unwrap();
        let sd: crate::game::SaveData = serde_json::from_value(old.clone()).unwrap();
        assert!(sd.mod_cars.is_empty());
        let g = game(&["vp_vw_golf_83", "vp_nsn_350z_06"], &d);
        assert_eq!(g.owned, ["vp_vw_golf_83", "vp_nsn_350z_06"]);
        assert!(g.mod_missing.is_empty());
        assert_eq!((g.current_car, g.money, g.rep), (1, 12345, 77));
        g.write_save(&d);
        let back = file(&d);
        assert!(back.get("mod_cars").is_none(), "no mod cars: no new member");
        assert_eq!(names(&back["owned"]), ["vp_vw_golf_83", "vp_nsn_350z_06"]);
        assert_eq!(back["car"], "vp_nsn_350z_06");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 31+ cars (40 of the disc's and three more by name): the garage, its records and their order go through save
    /// and load whole; nothing is capped at the original's 30 slots or 77 records.
    #[test]
    fn a_garage_past_thirty_round_trips() {
        let d = dir("big");
        let mut cars: Vec<&str> = crate::mc4::VEHICLES[..40].to_vec();
        cars.extend(["vp_big_mod_a_08", "vp_big_mod_b_08", "vp_big_mod_c_08"]);
        let mut g = Game::new(cars.iter().map(PathBuf::from).collect(), 0);
        g.data_dir = d.clone();
        g.owned = cars.iter().rev().map(|s| s.to_string()).collect();
        for c in &cars {
            g.mc4_car_added(c, None);
        }
        g.write_save(&d);
        let first = file(&d);
        assert_eq!(names(&first["owned"]).len(), 43);
        assert_eq!(first["mc4"]["garage"].as_array().unwrap().len(), 43);
        assert!(first["mc4"]["vehicle_records"].as_array().unwrap().len() >= 43);
        let g2 = game(&cars, &d);
        assert_eq!(g2.owned, g.owned);
        assert!(g2.mod_missing.is_empty());
        g2.write_save(&d);
        let second = file(&d);
        assert_eq!(second["owned"], first["owned"]);
        assert_eq!(second["mc4"]["garage"], first["mc4"]["garage"]);
        assert_eq!(second["mc4"]["vehicle_records"], first["mc4"]["vehicle_records"]);
        // (slots follow the garage's order past 30)
        let slots: Vec<u64> = second["mc4"]["garage"].as_array().unwrap().iter().map(|c| c["slot"].as_u64().unwrap()).collect();
        assert_eq!(slots, (0..43).collect::<Vec<u64>>());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// D27 / D28: mods gone - their cars stay in the save untouched as placeholders (not drivable, not counted); the
    /// vehicle in use moves to a car that is here; with the mods back the cars return as they were.
    #[test]
    fn a_missing_mod_round_trips() {
        let d = dir("missing");
        let all = ["vp_vw_golf_83", "vp_test_mod_01", "vp_nsn_350z_06", "vp_gone_car_08"];
        // a garage with two mod cars (here, their folders present), the first mod car in use
        let mut g = Game::new(all.iter().map(PathBuf::from).collect(), 1);
        g.data_dir = d.clone();
        g.owned = all.iter().map(|s| s.to_string()).collect();
        for c in all {
            g.mc4_car_added(c, None);
        }
        g.perf.insert("vp_test_mod_01".into(), 2);
        g.parts_owned.insert("vp_gone_car_08".into(), vec!["spoiler:2".into()]);
        g.mod_cars.insert("vp_test_mod_01".into(), ModCarSave { mod_id: "test.mod".into(), long_name: "2006 TEST MOD".into(), short_name: "06 TEST".into() });
        g.mod_cars.insert("vp_gone_car_08".into(), ModCarSave { mod_id: "gone".into(), long_name: "2008 GONE".into(), short_name: String::new() });
        g.write_save(&d);
        // (one load and save with every car here first: the purchase records take the port's part tags on load)
        game(&all, &d).write_save(&d);
        let before = file(&d);
        assert_eq!(before["car"], "vp_test_mod_01");
        assert_eq!(before["mc4"]["vehicle_records"].as_array().unwrap().len(), 4);
        // the mods are not installed: only the disc cars are in the garage list
        let g2 = game(&["vp_vw_golf_83", "vp_nsn_350z_06"], &d);
        assert_eq!(g2.owned, ["vp_vw_golf_83", "vp_nsn_350z_06"], "placeholders count for no rule");
        assert_eq!(g2.mod_missing, [Missing { at: 1, name: "vp_test_mod_01".into() }, Missing { at: 3, name: "vp_gone_car_08".into() }]);
        assert_eq!(g2.current_car, 0, "the vehicle in use is one that is here");
        assert_eq!((g2.mod_missing_label(0), g2.mod_missing_label(1)), ("06 TEST".to_string(), "2008 GONE".to_string()));
        // MANAGE VEHICLES: the two cars and the two placeholders in the garage's order
        let list = g2.manage_list(vec![1, 0]);
        assert_eq!(list, [0, PLACEHOLDER, 1, PLACEHOLDER + 1]);
        assert_eq!(g2.list_car_name(list[3]), "vp_gone_car_08");
        g2.write_save(&d);
        let without = file(&d);
        assert_eq!(without["owned"], before["owned"], "the garage order keeps the placeholders");
        assert_eq!(without["mod_cars"], before["mod_cars"]);
        for car in ["vp_test_mod_01", "vp_gone_car_08"] {
            let (a, b) = (record(&before, car), record(&without, car));
            let strip = |v: &Value| { let mut v = v.clone(); v.as_object_mut().unwrap().remove("current"); v };
            assert_eq!(strip(a), strip(b), "{car}: record untouched");
        }
        assert_eq!(without["perf"]["vp_test_mod_01"], 2);
        assert_eq!(without["parts_owned"]["vp_gone_car_08"], before["parts_owned"]["vp_gone_car_08"]);
        // (statistics are `<code>/<car>` in the career's state, which this module never touches; a unit test's career has
        // no missions and restarts on load, so they are not compared here)
        assert_eq!(without["mc4"]["vehicle_records"], before["mc4"]["vehicle_records"]);
        // the mods back: everything as it was
        let g3 = game(&all, &d);
        assert_eq!(g3.owned, all);
        assert!(g3.mod_missing.is_empty());
        g3.write_save(&d);
        let back = file(&d);
        for car in ["vp_test_mod_01", "vp_gone_car_08"] {
            let strip = |v: &Value| { let mut v = v.clone(); v.as_object_mut().unwrap().remove("current"); v };
            assert_eq!(strip(record(&back, car)), strip(record(&before, car)), "{car}: back unchanged");
        }
        assert_eq!(back["owned"], before["owned"]);
        assert_eq!(back["mod_cars"], before["mod_cars"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
