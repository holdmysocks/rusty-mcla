//! Checks a car of a mod against the minimum of docs/reference/mods/vehicle_inventory.md section 5 ("must provide",
//! or a donor named in the manifest). Each reason names the section-5 row it comes from.

use crate::disc::DiscInfo;
use crate::manifest::{car_name_problem, CarEntry, Figures};
use crate::overlay;
use crate::package::Package;
use crate::runtime;
use mcla_formats::audcfg::Dat;
use std::collections::HashSet;

/// mcVehicleInfo Class names (index in 0x827E9ABC; `oppick::CLASSES`), written as the disc lists write them.
pub const CLASSES: [&str; 9] = ["TUNER", "MUSCLE", "EXOTIC", "SPORTBIKE", "LUXURY", "COP", "SUV", "LowRider", "MISC1"];
/// The race groups / unlock groups of the disc rows.
pub const GROUPS: [&str; 4] = ["D0", "C0", "B0", "A0"];
/// `DistrictFilter` values a car row uses on the disc (the parser also knows Unknown and All, `car_pick.md` 1.2).
pub const DISTRICTS: [&str; 5] = ["Hollywood", "Beaches", "Hills", "Downtown", "SouthCentral"];
/// Size of a set-up record (`CarCfg`, vehicle_inventory.md 2.6) and its TypeName field.
pub const XCC_SIZE: usize = 8176;
pub const XCC_TYPENAME: usize = 0x1FAD;

/// The files of one package, by overlay key (plain and hashed forms).
pub struct FileKeys(HashSet<String>);

impl FileKeys {
    pub fn new(p: &Package) -> FileKeys {
        FileKeys(p.files.iter().flat_map(|f| overlay::keys(&f.rel)).collect())
    }
    pub fn has(&self, rel: &str) -> bool {
        self.0.contains(&overlay::lookup_key(rel))
    }
}

/// A checked car: what the rest of the loader uses.
#[derive(Clone, Debug)]
pub struct Checked {
    pub class: String,
    pub manufacturer: String,
    pub figures: Figures,
    pub districts: Vec<String>,
    /// opponent samples (performance, speed) when the car is opponent-enabled and they could be resolved
    pub samples: Option<([f32; 3], [f32; 3])>,
}

pub struct CarCheck {
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
    pub checked: Option<Checked>,
}

fn canon<'a>(list: &[&'a str], v: &str) -> Option<&'a str> {
    list.iter().copied().find(|x| x.eq_ignore_ascii_case(v))
}

pub fn check_car(car: &CarEntry, pkg: &Package, keys: &FileKeys, disc: &DiscInfo, reserved: &[String]) -> CarCheck {
    let mut r: Vec<String> = Vec::new();
    let mut w: Vec<String> = Vec::new();
    let name = car.name.as_str();
    if let Some(p) = car_name_problem(name) {
        r.push(format!("name: {p}"));
        return CarCheck { reasons: r, warnings: w, checked: None };
    }
    if disc.has_car(name) {
        r.push(format!("name: {name} is a car of the game (two cars cannot share a name; LookupCar takes the first)"));
    }
    if reserved.iter().any(|x| x.eq_ignore_ascii_case(name)) {
        r.push(format!("name: {name} is a car the port adds itself"));
    }
    // a donor must be a car of the game's table
    let donor = |what: &str, d: &Option<String>, r: &mut Vec<String>| -> bool {
        match d {
            None => false,
            Some(d) if disc.found && !disc.has_car(d) => {
                r.push(format!("{what}: donor {d} is not a car of the game"));
                true
            }
            Some(_) => true,
        }
    };

    // row 1: the resource folder (its geometry is the car)
    let vdir = format!("resources/vehicle/{name}/");
    let in_dir: Vec<&str> = pkg.files.iter().map(|f| f.rel.as_str()).filter(|f| f.to_ascii_lowercase().starts_with(&vdir) && !f[vdir.len()..].contains('/')).collect();
    let count = |ext: &str| in_dir.iter().filter(|f| f.to_ascii_lowercase().ends_with(ext)).count();
    if count(".r63") == 0 {
        r.push(format!("row 1: no part models (.r63) in {vdir}"));
    }
    // row 2: two .r83 (shaders, textures, hull)
    if count(".r83") != 2 {
        r.push(format!("row 2: {vdir} must hold two .r83 files (found {})", count(".r83")));
    }
    // row 3: one .r19
    if count(".r19") != 1 {
        r.push(format!("row 3: {vdir} must hold one .r19 file (found {})", count(".r19")));
    }
    // row 4: tuning
    let tune_donor = donor("row 4 (tuning)", &car.donors.tune, &mut r);
    match (count(".r50"), tune_donor) {
        (0, false) => r.push(format!("row 4: no .r50 tuning in {vdir} and no tune donor")),
        (n, _) if n > 1 => r.push(format!("row 4: {vdir} holds {n} .r50 files (one expected)")),
        (1, true) => w.push("row 4: own .r50 and a tune donor: the donor's tuning is used".into()),
        _ => {}
    }
    // row 5: stock set-up
    let xcc = format!("resources/vehicle/configs/{name}.xcc");
    let setup_donor = donor("row 5 (set-up)", &car.donors.setup, &mut r);
    if let Some(f) = pkg.files.iter().find(|f| f.rel.eq_ignore_ascii_case(&xcc)) {
        match pkg.read(f) {
            Ok(b) if b.len() != XCC_SIZE => r.push(format!("row 5: {xcc} is {} bytes ({XCC_SIZE} expected)", b.len())),
            Ok(b) => {
                let tn = &b[XCC_TYPENAME..XCC_TYPENAME + 32];
                let tn = String::from_utf8_lossy(&tn[..tn.iter().position(|&c| c == 0).unwrap_or(32)]).into_owned();
                if !tn.eq_ignore_ascii_case(name) {
                    r.push(format!("row 5: {xcc} TypeName is \"{tn}\", not {name}"));
                }
            }
            Err(e) => r.push(format!("row 5: cannot read {xcc}: {e}")),
        }
    } else if !setup_donor {
        r.push(format!("row 5: no stock set-up {xcc} and no set-up donor"));
    }
    // row 6: the vehicle row
    let row = &car.row;
    let class = canon(&CLASSES, &row.class);
    if class.is_none() {
        r.push(format!("row 6: class \"{}\" is not one of {}", row.class, CLASSES.join(" ")));
    }
    let manufacturer = if disc.found {
        match disc.manufacturer(&row.manufacturer) {
            Some(m) => m.to_string(),
            None => {
                r.push(format!("row 6: manufacturer \"{}\" is not in vehicle.lst ({})", row.manufacturer, disc.manufacturers.join(" ")));
                row.manufacturer.clone()
            }
        }
    } else {
        w.push("row 6: vehicle.lst not found; manufacturer not checked".into());
        row.manufacturer.clone()
    };
    for (k, v) in [("group", &row.group), ("unlocking_group", &row.unlocking_group)] {
        if canon(&GROUPS, v).is_none() {
            r.push(format!("row 6: {k} \"{v}\" is not one of {}", GROUPS.join(" ")));
        }
    }
    if row.price <= 0 {
        r.push("row 6: price must be above 0 (the showroom lists cars with a price)".into());
    }
    let mut districts = Vec::new();
    for d in &row.districts {
        match canon(&DISTRICTS, d) {
            Some(c) => districts.push(c.to_string()),
            None => r.push(format!("row 6: district \"{d}\" is not one of {}", DISTRICTS.join(" "))),
        }
    }
    if row.roof > 2 {
        r.push(format!("row 6: roof {} (the disc uses 0..2)", row.roof));
    }
    let figures = match (&row.figures, &row.figures_from) {
        (Some(f), None) => Some(*f),
        (None, Some(d)) => match disc.row(d) {
            Some(x) => Some(x.figures),
            None if !disc.found => {
                r.push(format!("row 6: figures_from {d}: vehicle.lst not found"));
                None
            }
            None => {
                r.push(format!("row 6: figures_from {d} is not a car of the game"));
                None
            }
        },
        (Some(_), Some(_)) => {
            r.push("row 6: give figures or figures_from, not both".into());
            None
        }
        (None, None) => {
            r.push("row 6: no figures and no figures_from".into());
            None
        }
    };
    if let Some(f) = figures {
        if [f.base_accel, f.mods_accel, f.base_speed, f.mods_speed, f.base_handling, f.mods_handling].iter().any(|x| !x.is_finite() || *x < 0.0) {
            r.push("row 6: figures must be numbers >= 0".into());
        }
    }
    // row 7: names
    if car.long_name.trim().is_empty() || car.short_name.trim().is_empty() {
        r.push("row 7: long_name and short_name are required".into());
    }
    // row 8: stock rims
    let own_rims = ["whl_stk_", "whld_stk_"].iter().any(|p| {
        let d = format!("resources/rims/{p}{name}/");
        pkg.files.iter().any(|f| f.rel.to_ascii_lowercase().starts_with(&d))
    });
    if !own_rims {
        match &car.donors.rims {
            None => r.push(format!("row 8: no stock rims resources/rims/whl_stk_{name}/ and no rims donor")),
            Some(d) if !(d.starts_with("whl_") || d.starts_with("whld_")) => r.push(format!("row 8: rims donor {d} is not a rim folder name (whl_... / whld_...)")),
            Some(_) => {}
        }
    }
    // row 10: garage camera
    let own_garage = keys.has(&format!("tune/garage/{name}.garage"));
    if !donor("row 10 (garage)", &car.donors.garage, &mut r) && !own_garage {
        r.push(format!("row 10: no tune/garage/{name}.garage and no garage donor"));
    }
    // row 11: part price list
    let own_prices = keys.has(&format!("tune/garage/{name}_parts.gtun"));
    if !donor("row 11 (prices)", &car.donors.prices, &mut r) && !own_prices {
        r.push(format!("row 11: no tune/garage/{name}_parts.gtun and no prices donor"));
    }
    // row 16: sound - the car's own <CAR>_PROFILE in the mod's game.dat, else the sound donor's, else the game's
    // default car / bike profile (manifest.md section 7)
    let sound_donor = donor("row 16 (sound)", &car.donors.sound, &mut r);
    let (brings, own_profile) = check_sound(name, pkg, keys, disc, &mut r, &mut w);
    match (own_profile, sound_donor) {
        (true, true) => w.push(format!("row 16: own {} and a sound donor: the donor is never used", runtime::profile_name(name))),
        (false, false) if brings => w.push(format!(
            "row 16: the mod's game.dat has no {} and there is no sound donor: the game's default car profile plays (the bike one for a bike), as the original does for a car without a profile",
            runtime::profile_name(name)
        )),
        (false, false) => r.push(format!("row 16: no sound objects ({} with {}) and no sound donor", runtime::GAME_DAT, runtime::profile_name(name))),
        _ => {}
    }
    // row 17: cameras (original behaviour without them unverified: a warning only)
    // (each file is looked up under the car, then the donor: runtime::car_names; any own driving view counts)
    let own_cams = ["boom1", "boom2", "boom3", "POV", "bumper"].iter().any(|v| keys.has(&format!("tune/camera/{name}_{v}.camboomcs")));
    if !donor("row 17 (cameras)", &car.donors.cameras, &mut r) && !own_cams {
        w.push("row 17: no camera files and no cameras donor; the port's fallback cameras are used".into());
    }
    // row 18: driver animation (optional)
    let _ = &car.donors.anim;
    // row 19: pictures
    let pics = ["managerides/mr_", "showroom/sh_", "mini_showroom/msh_"];
    let missing: Vec<&str> = pics.iter().copied().filter(|p| !keys.has(&format!("resources/ui/textures/garage/dyntextures/{p}{name}.r9"))).collect();
    if !donor("row 19 (pictures)", &car.donors.pictures, &mut r) && !missing.is_empty() {
        r.push(format!("row 19: pictures missing ({}) and no pictures donor", missing.iter().map(|p| format!("{p}{name}.r9")).collect::<Vec<_>>().join(", ")));
    }
    // row 22: bikes
    if class == Some("SPORTBIKE") {
        w.push("row 22: a bike's geometry (centre-line hubs, bike parts) is not checked by the loader".into());
    }
    // row 14 / 15: opponent use (stub: completeness only; wiring is stage S4)
    let mut samples = None;
    if let Some(o) = car.opponent.as_ref().filter(|o| o.enabled) {
        if row.player_only {
            r.push("opponent: enabled but the row says player_only".into());
        }
        if class == Some("COP") {
            r.push("opponent: a COP class car never races (opponent pick)".into());
        }
        for d in &o.districts {
            if canon(&DISTRICTS, d).is_none() {
                r.push(format!("opponent: district \"{d}\" is not one of {}", DISTRICTS.join(" ")));
            }
        }
        if o.districts.is_empty() && districts.is_empty() {
            r.push("opponent: no districts (row or opponent block)".into());
        }
        samples = match (&o.performance, &o.samples_from) {
            (Some(p), None) => Some((*p, o.speed.unwrap_or([f32::NAN; 3]))),
            (None, Some(d)) => match disc.samples.get(&d.to_ascii_lowercase()) {
                Some(s) => Some(*s),
                None => {
                    r.push(format!("row 14: samples_from {d} has no performance samples on the disc"));
                    None
                }
            },
            (Some(_), Some(_)) => {
                r.push("row 14: give performance or samples_from, not both".into());
                None
            }
            (None, None) => {
                r.push("row 14: an opponent car needs performance samples (performance or samples_from)".into());
                None
            }
        };
        match (&o.setup, &o.setup_from) {
            (Some(s), None) if !keys.has(&format!("resources/vehicle/configs/{s}.xcc")) => r.push(format!("row 15: opponent set-up {s} has no resources/vehicle/configs/{s}.xcc and no setup_from")),
            (None, Some(d)) => r.push(format!("row 15: setup_from {d} without a setup name")),
            _ => {}
        }
        if o.driver.is_none() && o.setup.is_some() {
            r.push("row 15: an opponent set-up needs a driver".into());
        }
    }
    let checked = r.is_empty().then(|| Checked {
        class: class.unwrap_or("").to_string(),
        manufacturer,
        figures: figures.unwrap_or_default(),
        districts: if districts.is_empty() { Vec::new() } else { districts },
        samples,
    });
    CarCheck { reasons: r, warnings: w, checked }
}

fn be_u32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_pkg(pkg: &Package, rel: &str) -> Option<Result<Vec<u8>, String>> {
    let want = overlay::norm(rel);
    let f = pkg.files.iter().find(|f| overlay::norm(&f.rel) == want)?;
    Some(pkg.read(f).map_err(|e| e.to_string()))
}

/// Up to five names, then the count of the rest.
fn few(names: &[String]) -> String {
    let mut s = names.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > 5 {
        s.push_str(&format!(" and {} more", names.len() - 5));
    }
    s
}

/// Row 16 for a mod that brings sound objects: its game.dat / sounds.dat must read, the car's profile must be a
/// profile whose player and opponent `_DEFAULT` objects exist (in the mod or the game), every wave its banks' indexes
/// list must be there. Returns (the mod brings sound objects, the car has its own profile).
fn check_sound(name: &str, pkg: &Package, keys: &FileKeys, disc: &DiscInfo, r: &mut Vec<String>, w: &mut Vec<String>) -> (bool, bool) {
    let mut brings = false;
    let mut own = false;
    let profile = runtime::profile_name(name);
    let mut load = |rel: &str, r: &mut Vec<String>| -> Option<Dat> {
        let b = match read_pkg(pkg, rel)? {
            Ok(b) => b,
            Err(e) => {
                r.push(format!("row 16: cannot read {rel}: {e}"));
                return None;
            }
        };
        brings = true;
        let d = Dat::parse(&b);
        if d.is_none() {
            r.push(format!("row 16: {rel} is not a readable .dat file (the layout of the game's own)"));
        }
        d
    };
    let game = load(runtime::GAME_DAT, r);
    let sounds = load(runtime::SOUNDS_DAT, r);
    let known = |d: &Option<Dat>, h: u32, disc_set: &std::collections::HashSet<u32>| d.as_ref().is_some_and(|d| d.contains(h)) || disc_set.contains(&h);
    if let Some(g) = &game {
        if let Some(o) = g.get_name(&profile) {
            own = true;
            if o.first() != Some(&12) || o.len() < 18 {
                r.push(format!("row 16: {profile} is not a profile object (type 12, as the game's 22-byte profiles)"));
            } else {
                for (at, who) in [(10, "player"), (14, "opponent")] {
                    let h = be_u32(o, at);
                    if h == 0 {
                        r.push(format!("row 16: {profile} +{at} ({who} DEFAULT) is empty"));
                    } else if !known(&game, h, &disc.game_objects) {
                        if disc.audio_found {
                            r.push(format!("row 16: {profile} +{at} ({who} DEFAULT, hash {h:08x}) is neither in the mod's game.dat nor in the game's"));
                        } else {
                            w.push(format!("row 16: {profile} +{at}: the game's game.dat not found, not checked"));
                        }
                    }
                }
            }
        }
        let taken: Vec<String> = g.objects.iter().filter(|(n, _, _)| disc.game_objects.contains(&mcla_formats::hash::joaat(n))).map(|(n, _, _)| n.clone()).collect();
        if !taken.is_empty() {
            w.push(format!("row 16: game.dat objects already in the game's (the game's are used): {}", few(&taken)));
        }
    }
    if let Some(s) = &sounds {
        let taken: Vec<String> = s.objects.iter().filter(|(n, _, _)| disc.sound_objects.contains(&mcla_formats::hash::joaat(n))).map(|(n, _, _)| n.clone()).collect();
        if !taken.is_empty() {
            w.push(format!("row 16: sounds.dat sounds already in the game's (the game's are used): {}", few(&taken)));
        }
        for bank in &s.banks {
            let dir = runtime::bank_folder(bank);
            match read_pkg(pkg, &format!("{dir}/_index.tsv")) {
                None => w.push(format!("row 16: bank {bank} has no {dir}/_index.tsv in the mod: its waves must be the game's")),
                Some(Err(e)) => r.push(format!("row 16: cannot read {dir}/_index.tsv: {e}")),
                Some(Ok(t)) => {
                    let t = String::from_utf8_lossy(&t);
                    let mut rows = 0;
                    for l in t.lines().filter(|l| !l.trim().is_empty()) {
                        let c: Vec<&str> = l.split('\t').collect();
                        let ok_hash = c.get(1).is_some_and(|h| u32::from_str_radix(h.trim(), 16).is_ok());
                        if !ok_hash {
                            r.push(format!("row 16: {dir}/_index.tsv: \"{l}\" is not <wave name> TAB <hash hex> ..."));
                            continue;
                        }
                        rows += 1;
                        if !keys.has(&format!("{dir}/{}.wav", c[0])) {
                            r.push(format!("row 16: {dir}/{}.wav (listed in _index.tsv) is missing", c[0]));
                        }
                    }
                    if rows == 0 {
                        w.push(format!("row 16: {dir}/_index.tsv lists no wave"));
                    }
                }
            }
        }
    }
    (brings, own)
}
