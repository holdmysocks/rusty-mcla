//! Rusty MCLA mod loader core (docs/MODS.md, M-5 stage S1; a user-approved deviation, DECISIONS.md D4 / D28).
//!
//! - Mods live in `<data root>/mods/`: one folder or one `.zip` per mod, with `mod.toml` (format:
//!   docs/reference/mods/manifest.md) at its root or in its one top folder. The disc data is never written.
//! - [`ModSet::load`] reads them all, checks every car against the minimum of docs/reference/mods/vehicle_inventory.md
//!   section 5, settles duplicates (same mod id twice: the newest is used) and conflicts (two mods defining one car:
//!   the first by mod id is used), honours the launcher's on / off flags (`[mods] disabled` in settings.toml) and
//!   builds the file overlay and the extra vehicle list (`vehicle0003.lst` format; mod cars take table indices after
//!   the disc's 69, in name order).
//! - [`install_package`] is the update rule: the same mod id replaces in place; an older version over a newer one
//!   returns [`InstallOutcome::NeedsConfirmation`] until confirmed.

pub mod disc;
pub mod manifest;
pub mod overlay;
pub mod package;
pub mod runtime;
pub mod validate;

use disc::DiscInfo;
use manifest::{CarEntry, Donors, Opponent, Row};
use overlay::Priority;
use package::Package;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub use disc::{DISC_VEHICLE_COUNT, EXTRA_LIST_NAME};
pub use runtime::{Donor, GAME_DAT, SOUNDS_DAT, WAVE_ROOT};

/// The mods folder in the data root (next to `tune`, `resources`).
pub fn mods_dir(data_root: &Path) -> PathBuf {
    data_root.join("mods")
}

/// Where zip mods are unpacked for readers that need real files (inside the mods folder; the scan skips names
/// starting with a dot).
pub fn cache_dir(mods_dir: &Path) -> PathBuf {
    mods_dir.join(".cache")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModState {
    /// valid, switched on: its cars and files are in use
    Loaded,
    /// valid but switched off in the launcher
    Disabled,
    /// not usable: see `reasons`
    Invalid,
    /// defines a car another mod (earlier by mod id) already defines: not loaded
    Conflict,
    /// another copy of the same mod id with a newer (or equal) version is used instead
    Duplicate,
}

#[derive(Clone, Debug)]
pub struct CarStatus {
    pub name: String,
    pub long_name: String,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}

/// One package found in the mods folder, as the launcher's Mods tab shows it.
#[derive(Clone, Debug)]
pub struct ModInfo {
    /// mod id; for a package whose manifest could not be read, its file name
    pub id: String,
    pub version: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub path: PathBuf,
    pub is_zip: bool,
    pub state: ModState,
    /// why the mod is not loaded (empty when Loaded / Disabled-but-valid)
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
    pub cars: Vec<CarStatus>,
}

/// A loaded mod car (keyed by name everywhere; its table index follows the disc cars).
#[derive(Clone, Debug)]
pub struct ModCar {
    pub name: String,
    pub mod_id: String,
    /// vehicle-table index: disc count + place in name order
    pub index: usize,
    pub long_name: String,
    pub short_name: String,
    pub row: Row,
    /// the class as the lists spell it
    pub class: String,
    /// the manufacturer as vehicle.lst spells it
    pub manufacturer: String,
    /// figures, resolved from `figures_from` when given
    pub figures: manifest::Figures,
    pub donors: Donors,
    pub opponent: Option<Opponent>,
    /// resolved opponent samples (performance, speed) at mod 0 / 50 / 100
    pub samples: Option<([f32; 3], [f32; 3])>,
}

#[derive(Default)]
pub struct LoadOptions {
    /// mod ids switched off (see [`disabled_ids`])
    pub disabled: HashSet<String>,
    /// car names the port adds itself (e.g. `catalog::CUT_GTR`): a mod may not define them
    pub reserved: Vec<String>,
}

pub struct ModSet {
    pub mods_dir: PathBuf,
    mods: Vec<ModInfo>,
    /// the packages of the loaded mods, by mod id order
    packages: Vec<(String, Package)>,
    cars: Vec<ModCar>,
    disc_count: usize,
    /// overlay key -> (package, file)
    overlay: HashMap<String, (usize, usize)>,
}

/// A file of a loaded mod.
#[derive(Clone, Copy)]
pub struct ModFile<'a> {
    set: &'a ModSet,
    pkg: usize,
    file: usize,
}

impl<'a> ModFile<'a> {
    pub fn mod_id(&self) -> &'a str {
        &self.set.packages[self.pkg].0
    }
    /// the path inside the package (= in the data tree), as the author wrote it
    pub fn rel(&self) -> &'a str {
        &self.set.packages[self.pkg].1.files[self.file].rel
    }
    pub fn size(&self) -> u64 {
        self.set.packages[self.pkg].1.files[self.file].size
    }
    pub fn read(&self) -> std::io::Result<Vec<u8>> {
        let p = &self.set.packages[self.pkg].1;
        p.read(&p.files[self.file])
    }
    /// a real path (folder mods, or zip mods after [`ModSet::unpack_zips`])
    pub fn disk_path(&self) -> Option<PathBuf> {
        let p = &self.set.packages[self.pkg].1;
        p.disk_path(&p.files[self.file])
    }
}

/// Where a data path is read from.
pub enum Located<'a> {
    Disc(PathBuf),
    Mod(ModFile<'a>),
}

impl ModSet {
    /// An empty set (no mods folder, or mods not wanted).
    pub fn empty(mods_dir: PathBuf) -> ModSet {
        ModSet { mods_dir, mods: Vec::new(), packages: Vec::new(), cars: Vec::new(), disc_count: DISC_VEHICLE_COUNT, overlay: HashMap::new() }
    }

    /// Read `<data root>/mods`. Never fails: problems are in each [`ModInfo`] (and the log).
    pub fn load(data_root: &Path, opts: &LoadOptions) -> ModSet {
        let disc = DiscInfo::load(data_root);
        Self::load_from(&mods_dir(data_root), &disc, opts)
    }

    pub fn load_from(dir: &Path, disc: &DiscInfo, opts: &LoadOptions) -> ModSet {
        let mut set = ModSet::empty(dir.to_path_buf());
        set.disc_count = disc.count();
        // 1. open and parse every package
        let mut found: Vec<(ModInfo, Option<(Package, manifest::Manifest)>)> = Vec::new();
        for path in package_paths(dir) {
            let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mut info = ModInfo { id: file_name, version: String::new(), name: String::new(), author: String::new(), description: String::new(), path: path.clone(), is_zip: !path.is_dir(), state: ModState::Invalid, reasons: Vec::new(), warnings: Vec::new(), cars: Vec::new() };
            let pkg = match Package::open(&path) {
                Ok(p) => p,
                Err(e) => {
                    info.reasons.push(e);
                    found.push((info, None));
                    continue;
                }
            };
            let m = match manifest::parse(&pkg.manifest_text) {
                Ok(m) => m,
                Err(e) => {
                    info.reasons.push(e);
                    found.push((info, None));
                    continue;
                }
            };
            info.id = m.info.id.clone();
            info.version = m.info.version.clone();
            info.name = m.info.name.clone();
            info.author = m.info.author.clone();
            info.description = m.info.description.clone();
            info.reasons.extend(header_problems(&m));
            info.warnings.extend(m.unknown.iter().map(|k| format!("mod.toml: unknown entry \"{k}\" ignored (a newer content type?)")));
            found.push((info, Some((pkg, m))));
        }
        // 2. the same mod id more than once: the newest version is used (ties: the first by path)
        let mut best: HashMap<String, usize> = HashMap::new();
        for (i, (info, p)) in found.iter().enumerate() {
            if p.is_none() || !info.reasons.is_empty() {
                continue;
            }
            match best.get(&info.id) {
                Some(&j) if manifest::cmp_version(&info.version, &found[j].0.version) != Ordering::Greater => {}
                _ => {
                    best.insert(info.id.clone(), i);
                }
            }
        }
        for i in 0..found.len() {
            let (info, p) = &found[i];
            if p.is_none() || !info.reasons.is_empty() {
                continue;
            }
            let j = best[&info.id];
            if j != i {
                let (v, at) = (found[j].0.version.clone(), found[j].0.path.display().to_string());
                let info = &mut found[i].0;
                info.state = ModState::Duplicate;
                info.reasons.push(format!("another copy of mod {} (version {v}, {at}) is used instead", info.id));
            }
        }
        // 3. check the cars; 4. conflicts by mod id order
        let mut order: Vec<usize> = (0..found.len()).collect();
        order.sort_by(|&a, &b| found[a].0.id.cmp(&found[b].0.id).then(found[a].0.path.cmp(&found[b].0.path)));
        let mut claimed: HashMap<String, String> = HashMap::new();
        let mut cars: Vec<ModCar> = Vec::new();
        let mut packages: Vec<(String, Package)> = Vec::new();
        for i in order {
            let (info, p) = &mut found[i];
            let Some((pkg, m)) = p.take() else { continue };
            if info.state == ModState::Duplicate || !info.reasons.is_empty() {
                continue;
            }
            let keys = validate::FileKeys::new(&pkg);
            let mut ok_cars = Vec::new();
            for c in &m.cars {
                let chk = validate::check_car(c, &pkg, &keys, disc, &opts.reserved);
                info.cars.push(CarStatus { name: c.name.clone(), long_name: c.long_name.clone(), reasons: chk.reasons.clone(), warnings: chk.warnings.clone() });
                if !chk.reasons.is_empty() {
                    info.reasons.push(format!("car {}: {}", c.name, chk.reasons.join("; ")));
                }
                if let Some(k) = chk.checked {
                    ok_cars.push(mod_car(c, &info.id, k));
                }
            }
            if !info.reasons.is_empty() {
                info.state = ModState::Invalid;
                continue;
            }
            if opts.disabled.contains(&info.id) {
                info.state = ModState::Disabled;
                continue;
            }
            let taken: Vec<String> = ok_cars.iter().filter_map(|c| claimed.get(&c.name).map(|o| format!("car {} is already defined by mod {o}", c.name))).collect();
            if !taken.is_empty() {
                info.state = ModState::Conflict;
                info.reasons.extend(taken);
                continue;
            }
            for c in &ok_cars {
                claimed.insert(c.name.clone(), info.id.clone());
            }
            info.state = ModState::Loaded;
            cars.extend(ok_cars);
            packages.push((info.id.clone(), pkg));
        }
        // 5. the extra list: mod cars in name order after the disc cars
        cars.sort_by(|a, b| a.name.cmp(&b.name));
        for (k, c) in cars.iter_mut().enumerate() {
            c.index = set.disc_count + k;
        }
        // 6. the overlay (first loaded mod by id wins a path)
        let mut later: Vec<(String, String)> = Vec::new();
        for (pi, (id, pkg)) in packages.iter().enumerate() {
            for (fi, f) in pkg.files.iter().enumerate() {
                for key in overlay::keys(&f.rel) {
                    match set.overlay.get(&key) {
                        Some(&(op, _)) if op != pi => {
                            // (the sound object files are merged from every mod, not replaced: runtime::read_each)
                            if key == overlay::norm(&f.rel) && ![SOUNDS_DAT, GAME_DAT].iter().any(|m| overlay::norm(m) == key) {
                                later.push((id.clone(), format!("file {} is also in mod {} (that one is used)", f.rel, packages[op].0)));
                            }
                        }
                        Some(_) => {}
                        None => {
                            set.overlay.insert(key, (pi, fi));
                        }
                    }
                }
            }
        }
        let mut mods: Vec<ModInfo> = found.into_iter().map(|(i, _)| i).collect();
        for (id, w) in later {
            if let Some(m) = mods.iter_mut().find(|m| m.id == id && m.state == ModState::Loaded) {
                m.warnings.push(w);
            }
        }
        mods.sort_by(|a, b| a.id.cmp(&b.id).then(a.path.cmp(&b.path)));
        for m in &mods {
            match m.state {
                ModState::Loaded => log::info!("mod {} {} loaded ({} car(s)) from {}", m.id, m.version, m.cars.len(), m.path.display()),
                ModState::Disabled => log::info!("mod {} {} switched off", m.id, m.version),
                _ => log::warn!("mod {} not loaded ({:?}): {}", m.id, m.state, m.reasons.join(" | ")),
            }
            for w in &m.warnings {
                log::warn!("mod {}: {w}", m.id);
            }
        }
        set.mods = mods;
        set.packages = packages;
        set.cars = cars;
        set
    }

    /// Every package found, valid or not (the Mods tab's list).
    pub fn mods(&self) -> &[ModInfo] {
        &self.mods
    }

    /// The loaded mod cars, in table order.
    pub fn cars(&self) -> &[ModCar] {
        &self.cars
    }

    pub fn car(&self, name: &str) -> Option<&ModCar> {
        self.cars.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// The vehicle-table index of a mod car (disc count + its place in name order).
    pub fn table_index(&self, name: &str) -> Option<usize> {
        self.car(name).map(|c| c.index)
    }

    /// The number of disc vehicle-table records the mod cars follow.
    pub fn disc_count(&self) -> usize {
        self.disc_count
    }

    /// The extra vehicle list (the `Vehicle` blocks of the mod cars in the format of `vehicle.lst`, as the
    /// original's `vehicle0003.lst` would hold them). Built in memory; never written to the data folder.
    pub fn extra_list(&self) -> String {
        self.cars.iter().map(vehicle_block).collect()
    }

    /// String-table entries of the mod cars: `<car>` = long name, `<car>_shrt` = short name.
    pub fn strings(&self) -> Vec<(String, String)> {
        self.cars.iter().flat_map(|c| [(c.name.clone(), c.long_name.clone()), (format!("{}_shrt", c.name), c.short_name.clone())]).collect()
    }

    /// The mod file answering to a data path (plain or hashed form; see [`overlay`]).
    pub fn find(&self, rel: &str) -> Option<ModFile<'_>> {
        self.overlay.get(&overlay::lookup_key(rel)).map(|&(pkg, file)| ModFile { set: self, pkg, file })
    }

    /// Read a data path through the overlay alone.
    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        self.find(rel).and_then(|f| f.read().ok())
    }

    /// Where `rel` is read from: the mod's file or the disc's (`data_root/rel`), by `prio`. `None`: neither has it.
    pub fn locate(&self, data_root: &Path, rel: &str, prio: Priority) -> Option<Located<'_>> {
        let disc = data_root.join(rel);
        let on_disc = disc.is_file();
        match (self.find(rel), prio) {
            (Some(f), Priority::ModFirst) => Some(Located::Mod(f)),
            (Some(_), Priority::DiscFirst) if on_disc => Some(Located::Disc(disc)),
            (Some(f), Priority::DiscFirst) => Some(Located::Mod(f)),
            (None, _) if on_disc => Some(Located::Disc(disc)),
            (None, _) => None,
        }
    }

    /// Read `rel` from a mod or the disc, by `prio`.
    pub fn read_layered(&self, data_root: &Path, rel: &str, prio: Priority) -> Option<Vec<u8>> {
        match self.locate(data_root, rel, prio)? {
            Located::Disc(p) => std::fs::read(p).ok(),
            Located::Mod(f) => f.read().ok(),
        }
    }

    /// The mod files directly in a data folder (e.g. `resources/vehicle/<car>`), by plain path.
    pub fn list_dir(&self, rel_dir: &str) -> Vec<ModFile<'_>> {
        let d = overlay::norm(rel_dir);
        let mut out: Vec<ModFile> = Vec::new();
        for (pi, (_, p)) in self.packages.iter().enumerate() {
            for (fi, f) in p.files.iter().enumerate() {
                let n = overlay::norm(&f.rel);
                let Some(rest) = n.strip_prefix(&d).and_then(|r| r.strip_prefix('/')) else { continue };
                // (the file must be the one the overlay uses for its plain path)
                if !rest.contains('/') && self.overlay.get(&n) == Some(&(pi, fi)) {
                    out.push(ModFile { set: self, pkg: pi, file: fi });
                }
            }
        }
        out
    }

    /// Unpack the loaded zip mods into `cache_root/<mod id>/` (skipped when a stamp says the same zip is already
    /// there), so readers that need real files (folder listings, memory maps) get [`ModFile::disk_path`].
    pub fn unpack_zips(&mut self, cache_root: &Path) -> std::io::Result<()> {
        for (id, pkg) in &mut self.packages {
            if !pkg.is_zip() {
                continue;
            }
            let meta = std::fs::metadata(&pkg.path)?;
            let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            let stamp_text = format!("{} {} {}\n", pkg.path.display(), meta.len(), mtime);
            let dir = cache_root.join(id.as_str());
            let stamp = cache_root.join(format!("{id}.stamp"));
            if dir.is_dir() && std::fs::read_to_string(&stamp).ok().as_deref() == Some(stamp_text.as_str()) {
                pkg.use_folder(&dir);
                continue;
            }
            let _ = std::fs::remove_file(&stamp);
            pkg.unpack_to(&dir)?;
            std::fs::write(&stamp, stamp_text)?;
        }
        Ok(())
    }
}

fn header_problems(m: &manifest::Manifest) -> Vec<String> {
    let mut r = Vec::new();
    if m.schema > manifest::SCHEMA {
        r.push(format!("mod.toml schema {} needs a newer Rusty MCLA (this one reads {})", m.schema, manifest::SCHEMA));
    }
    if !manifest::valid_id(&m.info.id) {
        r.push(format!("mod id \"{}\": use 1..64 of a-z 0-9 . _ - (not starting with a dot)", m.info.id));
    }
    if manifest::parse_version(&m.info.version).is_none() {
        r.push(format!("version \"{}\": use dotted numbers like 1.0.2", m.info.version));
    }
    if m.info.name.trim().is_empty() {
        r.push("name is empty".into());
    }
    if m.cars.is_empty() {
        r.push("the mod defines no car (cars are the only content type so far)".into());
    }
    let mut seen = HashSet::new();
    for c in &m.cars {
        if !seen.insert(c.name.to_ascii_lowercase()) {
            r.push(format!("car {} is defined twice", c.name));
        }
    }
    r
}

fn mod_car(c: &CarEntry, id: &str, k: validate::Checked) -> ModCar {
    ModCar {
        name: c.name.clone(),
        mod_id: id.to_string(),
        index: 0,
        long_name: c.long_name.clone(),
        short_name: c.short_name.clone(),
        row: c.row.clone(),
        class: k.class,
        manufacturer: k.manufacturer,
        figures: k.figures,
        donors: c.donors.clone(),
        opponent: c.opponent.clone(),
        samples: k.samples,
    }
}

/// One `Vehicle` block as the disc lists write it (keys and order of the R8's block in `vehicle.lst`; no content
/// keys, so the defaults apply: ContentDownloadFlags 1, ContentFlags 1 = in the mask, PortalRewardIdx -1).
pub fn vehicle_block(c: &ModCar) -> String {
    let r = &c.row;
    let g = |v: &str| validate::GROUPS.iter().find(|x| x.eq_ignore_ascii_case(v)).copied().unwrap_or("D0");
    let mut s = String::new();
    let mut line = |k: &str, v: String| s.push_str(&format!("\t{k}\t{v}\r\n"));
    line("Name", c.name.clone());
    line("PoliceDesc", r.police_desc.clone().unwrap_or_else(|| c.name.clone()));
    line("Class", c.class.clone());
    line("Manufacturer", c.manufacturer.clone());
    line("Group", g(&r.group).to_string());
    line("UnlockingGroup", g(&r.unlocking_group).to_string());
    for d in r.districts.iter().filter_map(|d| validate::DISTRICTS.iter().find(|x| x.eq_ignore_ascii_case(d))) {
        line("DistrictFilter", d.to_string());
    }
    line("Roof", r.roof.to_string());
    if let Some(t) = r.transmission {
        line("Transmission", t.to_string());
    }
    line("PlayerOnly", (r.player_only as u8).to_string());
    line("Price", r.price.to_string());
    let f = &c.figures;
    line("BaseAccel", f.base_accel.to_string());
    line("ModsAccel", f.mods_accel.to_string());
    line("BaseSpeed", f.base_speed.to_string());
    line("ModsSpeed", f.mods_speed.to_string());
    line("BaseHndlg", f.base_handling.to_string());
    line("ModsHndlg", f.mods_handling.to_string());
    if let Some(sr) = r.showroom {
        line("ShowroomAccel", sr.accel.to_string());
        line("ShowroomSpeed", sr.speed.to_string());
        line("ShowroomHndlg", sr.handling.to_string());
    }
    format!("Vehicle\r\n{{\r\n{s}}}\r\n")
}

/// The packages in a mods folder: sub-folders and `.zip` files, by name; names starting with a dot (the cache) and
/// other files are skipped.
pub fn package_paths(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .filter(|p| p.is_dir() || p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")))
        .collect();
    v.sort();
    v
}

// ---- on / off (the launcher's settings file) ----

/// settings.toml table and key of the switched-off mod ids: `[mods] disabled = ["id", ...]`.
pub const SETTINGS_TABLE: &str = "mods";
pub const SETTINGS_KEY: &str = "disabled";

/// The mod ids switched off in a settings file (`mcla_config::paths::settings_path`). A missing or broken file: none.
pub fn disabled_ids(settings_file: &Path) -> HashSet<String> {
    let Ok(text) = std::fs::read_to_string(settings_file) else { return HashSet::new() };
    let Ok(t) = text.trim_start_matches('\u{feff}').parse::<toml::Table>() else { return HashSet::new() };
    t.get(SETTINGS_TABLE)
        .and_then(|m| m.get(SETTINGS_KEY))
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Switch a mod on or off (only that key of the file changes; `mcla_config::set_key`).
pub fn set_enabled(settings_file: &Path, id: &str, on: bool) -> std::io::Result<()> {
    let mut ids: Vec<String> = disabled_ids(settings_file).into_iter().filter(|x| x != id).collect();
    if !on {
        ids.push(id.to_string());
    }
    ids.sort();
    mcla_config::set_key(settings_file, SETTINGS_TABLE, SETTINGS_KEY, toml::Value::Array(ids.into_iter().map(toml::Value::String).collect()))
}

// ---- install / update ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateDecision {
    /// no copy of this mod id is installed
    Install,
    /// an older or equal version is installed: replace it in place
    Replace { installed: String },
    /// a newer version is installed: ask the player first (the launcher's question, stage S5)
    NeedsConfirmation { installed: String },
}

/// The update rule (DECISIONS.md D28): same mod id = the same mod, replaced in place; a newer or equal version
/// replaces without asking; an older one asks first.
pub fn update_decision(installed: Option<&str>, incoming: &str) -> UpdateDecision {
    match installed {
        None => UpdateDecision::Install,
        Some(v) if manifest::cmp_version(incoming, v) == Ordering::Less => UpdateDecision::NeedsConfirmation { installed: v.to_string() },
        Some(v) => UpdateDecision::Replace { installed: v.to_string() },
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed { id: String, version: String, path: PathBuf, replaced: Vec<PathBuf> },
    /// nothing changed: an older version over a newer one, call again with `confirm_older`
    NeedsConfirmation { id: String, installed: String, incoming: String },
}

/// Install a mod zip into the mods folder as `<mod id>.zip`, replacing every installed copy of the same mod id
/// (folder or zip). Only the manifest's header is checked here; the full check happens on load.
pub fn install_package(mods_dir: &Path, zip: &Path, confirm_older: bool) -> Result<InstallOutcome, String> {
    if zip.is_dir() || !zip.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
        return Err(format!("{}: a mod is installed from a .zip file", zip.display()));
    }
    let pkg = Package::open(zip)?;
    let m = manifest::parse(&pkg.manifest_text)?;
    if !manifest::valid_id(&m.info.id) || manifest::parse_version(&m.info.version).is_none() {
        return Err(header_problems(&m).join("; "));
    }
    drop(pkg);
    let id = m.info.id.clone();
    let installed: Vec<(PathBuf, String)> = package_paths(mods_dir)
        .into_iter()
        .filter_map(|p| {
            let pk = Package::open(&p).ok()?;
            let mm = manifest::parse(&pk.manifest_text).ok()?;
            (mm.info.id == id).then_some((p, mm.info.version))
        })
        .collect();
    let newest = installed.iter().map(|(_, v)| v.as_str()).max_by(|a, b| manifest::cmp_version(a, b));
    if let UpdateDecision::NeedsConfirmation { installed } = update_decision(newest, &m.info.version) {
        if !confirm_older {
            return Ok(InstallOutcome::NeedsConfirmation { id, installed, incoming: m.info.version });
        }
    }
    std::fs::create_dir_all(mods_dir).map_err(|e| format!("cannot create {}: {e}", mods_dir.display()))?;
    let target = mods_dir.join(format!("{id}.zip"));
    let same = |a: &Path, b: &Path| std::fs::canonicalize(a).ok().zip(std::fs::canonicalize(b).ok()).is_some_and(|(x, y)| x == y);
    if !same(zip, &target) {
        let tmp = mods_dir.join(format!(".{id}.zip.tmp"));
        std::fs::copy(zip, &tmp).map_err(|e| format!("cannot copy {}: {e}", zip.display()))?;
        std::fs::rename(&tmp, &target).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("cannot write {}: {e}", target.display())
        })?;
    }
    let mut replaced = Vec::new();
    for (p, _) in installed {
        if same(&p, &target) {
            continue;
        }
        let r = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        r.map_err(|e| format!("installed, but the old copy {} could not be removed: {e}", p.display()))?;
        replaced.push(p);
    }
    Ok(InstallOutcome::Installed { id, version: m.info.version, path: target, replaced })
}

#[cfg(test)]
mod tests;
