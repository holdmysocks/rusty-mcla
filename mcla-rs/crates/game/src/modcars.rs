//! Mod cars in the running game (docs/MODS.md M-5 stage S2; a user-approved deviation, DECISIONS.md D4 / D28).
//!
//! At start-up [`init`] reads the mods folder with `mcla_mods::ModSet` and keeps, per loaded mod car, what the game's
//! readers need: its resource folder on disk (folder mods directly, zip mods unpacked to `mods/.cache`), its stock
//! set-up (the mod's own `.xcc`, or one made from the donor's in `mods/.cache/setups` - never in the disc folder,
//! the way `catalog::write_cut_setups` makes the GT-R's), its stock rims, pictures, price list and the donors named
//! in its manifest. The game joins them to its own tables keyed by name: the garage list (`car_dirs`, after the disc
//! cars), the catalogue (`catalog::Catalog::load` reads [`extra_list`]), the string table (`career.rs`), the price
//! lists (`Game::load_perf_prices`) and the records (`mc4::vehicle_index`: table index 69 + in name order).
//! Without a mods folder (and in unit tests, where [`init`] is never called) every function answers "not a mod car".

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A loaded mod car, as the game's readers use it.
#[derive(Clone, Debug, Default)]
pub struct ModCarRt {
    pub name: String,
    pub mod_id: String,
    /// vehicle-table index (disc count + place in name order)
    pub index: usize,
    /// the car's resource folder on disk (`resources/vehicle/<car>` of the mod)
    pub dir: PathBuf,
    /// its stock set-up `<car>.xcc` (the mod's own or made from `donors.setup`)
    pub setup: Option<PathBuf>,
    /// its stock rim folder (the mod's own `whl_stk_<car>` / `whld_stk_<car>`, or the disc's `donors.rims`)
    pub rims: Option<PathBuf>,
    /// the brake (index of `mc4::BRAKES`) from `donors.brake`
    pub brake: Option<u8>,
    /// the car whose `.r50` it drives on (`donors.tune`; the loader's rule: the donor's wins over the mod's own)
    pub tune: Option<String>,
    /// sound profile donor (`donors.sound`)
    pub sound: Option<String>,
    /// driving-camera donor (`donors.cameras`)
    pub cameras: Option<String>,
    /// `.garage` donor (`donors.garage`)
    pub garage: Option<String>,
    /// the mod's own `<car>_parts.gtun` text
    pub prices_text: Option<String>,
    /// the price list donor (`donors.prices`), when the mod has no list of its own
    pub prices: Option<String>,
    /// how the donor's list is scaled (`donors.prices_level` / `prices_kit` / `prices_maker`; DECISIONS.md D28
    /// "Prices"): only with `prices`, never over an own list
    pub prices_scale: Option<PriceScale>,
    /// own pictures `mr_`, `sh_`, `msh_`
    pub pictures: [Option<PathBuf>; 3],
    /// picture donor (`donors.pictures`)
    pub picture_donor: Option<String>,
    pub long_name: String,
    pub short_name: String,
}

/// The scaling of a donor's price list (`catalog::scale_price_list`, D21's recipe): its car-made rows to a disc car's
/// body-kit level or to a body-kit price, the stock rows' maker logo (None: the donor's own).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PriceScale {
    pub level: Option<String>,
    pub kit: Option<i64>,
    pub maker: Option<i32>,
}

impl PriceScale {
    /// The donor's list `rows` scaled; `list_of` gives a disc car's list (for `level`). None: nothing to scale to.
    pub fn apply<'a>(&self, rows: &crate::catalog::PartList, list_of: impl Fn(&str) -> Option<&'a crate::catalog::PartList>) -> Option<crate::catalog::PartList> {
        match (&self.level, self.kit) {
            (Some(l), _) => {
                let level = list_of(l)?;
                Some(crate::catalog::scale_price_list_to(rows, crate::catalog::body_kit_price(level), Some(level), self.maker))
            }
            (None, Some(k)) => Some(crate::catalog::scale_price_list_to(rows, k, None, self.maker)),
            // (only a maker: the donor's prices, its stock rows under the mod's logo)
            (None, None) => Some(crate::catalog::scale_price_list_to(rows, crate::catalog::body_kit_price(rows), None, self.maker)),
        }
    }
}

pub struct Registry {
    pub data_root: PathBuf,
    pub mods_dir: PathBuf,
    pub cars: Vec<ModCarRt>,
    /// the `Vehicle` blocks of the mod cars (vehicle.lst format, built in memory)
    pub extra_list: String,
    /// `<car>` / `<car>_shrt` string entries
    pub strings: Vec<(String, String)>,
    /// what the opponent pick gets for the opponent-marked mod cars (`OpPick::load`'s `extra` after the cut GT-R's;
    /// MODS.md design 7b, stage S4): per car in name order its `Vehicle` block, `Character` blocks and samples
    pub opponent_extra: String,
    /// the opponents' set-ups of the mod cars: (set-up name, file: the mod's own or made in `mods/.cache/setups`)
    pub opponent_setups: Vec<(String, PathBuf)>,
    /// the opponent-marked cars without a driver of their own (stage S4b): the pick gives each draw a free driver of the
    /// race's performance band, in a set-up [`free_setup_name`] made from the car's stock set-up
    pub free_driver_cars: Vec<String>,
    /// the opponent-marked cars with `rims = "random"` (stage S4b)
    pub rims_random: Vec<String>,
    /// the loader's full answer (the Mods tab's list, the file overlay)
    pub set: mcla_mods::ModSet,
}

static REG: OnceLock<Registry> = OnceLock::new();

/// Picture kinds: folder under `resources/ui/textures/garage/dyntextures` and file prefix.
pub const PICTURES: [(&str, &str); 3] = [("managerides", "mr_"), ("showroom", "sh_"), ("mini_showroom", "msh_")];

/// The mods folder of this run: `<data root>/mods`, or the test hook `MCLA_MODS_DIR` (a folder of mods elsewhere,
/// so a test never writes into the data folder).
pub fn mods_dir(data_root: &Path) -> PathBuf {
    match std::env::var_os("MCLA_MODS_DIR").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => mcla_mods::mods_dir(data_root),
    }
}

/// Read the mods (once per run; later calls return the first answer).
pub fn init(data_root: &Path) -> &'static Registry {
    REG.get_or_init(|| build(data_root, &mods_dir(data_root)))
}

/// The registry, when [`init`] ran.
pub fn get() -> Option<&'static Registry> {
    REG.get()
}

/// A loaded mod car by name (any case).
pub fn car(name: &str) -> Option<&'static ModCarRt> {
    get()?.cars.iter().find(|c| c.name.eq_ignore_ascii_case(name))
}

pub fn is_mod_car(name: &str) -> bool {
    car(name).is_some()
}

/// The vehicle-table index of a mod car (69 + its place in name order).
pub fn table_index(name: &str) -> Option<usize> {
    car(name).map(|c| c.index)
}

/// The resource folders of the mod cars, in table order (the garage list appends them after the disc's).
pub fn car_dirs() -> Vec<PathBuf> {
    get().map(|r| r.cars.iter().map(|c| c.dir.clone()).collect()).unwrap_or_default()
}

/// The `Vehicle` blocks of the mod cars ("" without mods).
pub fn extra_list() -> &'static str {
    get().map_or("", |r| r.extra_list.as_str())
}

/// `<car>` / `<car>_shrt` names of the mod cars.
pub fn strings() -> &'static [(String, String)] {
    get().map_or(&[], |r| r.strings.as_slice())
}

/// The stock set-up file of a mod car (`name` = the car's name, as a bought car's record is made from), or an opponent
/// set-up of a mod car (`GH_nn_k_<car>`, stage S4).
pub fn setup_file(name: &str) -> Option<PathBuf> {
    car(name).and_then(|c| c.setup.clone()).or_else(|| get()?.opponent_setups.iter().find(|s| s.0.eq_ignore_ascii_case(name)).map(|s| s.1.clone()))
}

/// The opponent pick's text for the opponent-marked mod cars ("" without any): `OpPick::load`'s `extra` takes it after
/// the cut GT-R's `catalog::CUT_CATALOG` / `CUT_OPPONENT`.
pub fn opponent_extra() -> &'static str {
    get().map_or("", |r| r.opponent_extra.as_str())
}

/// The opponent-marked mod cars (lower case) the pick gives a free driver of the race's performance band (stage S4b).
pub fn free_driver_cars() -> Vec<String> {
    get().map(|r| r.free_driver_cars.clone()).unwrap_or_default()
}

/// Is this opponent car one with `rims = "random"` (stage S4b)?
pub fn rims_random(car: &str) -> bool {
    get().is_some_and(|r| r.rims_random.iter().any(|c| c.eq_ignore_ascii_case(car)))
}

/// The set-up a free driver `driver` races the mod car `car` in (stage S4b): `<driver>_m_<car>`, made from the car's
/// stock set-up in `mods/.cache/setups`. A name over the set-up field's 31 characters is cut to 31 for the file's own
/// name field only (the key stays whole, it is never read from the file).
pub fn free_setup_name(driver: &str, car: &str) -> String {
    format!("{driver}_m_{car}")
}

/// Make the set-up `setup` of a free driver from the mod car's stock set-up file `stock` (stage S4b): the car's hash and
/// TypeName, the set-up name; the stock set-up's rim, brake and plate stay (it is the car's own).
fn make_free_setup(mods: &Path, car: &str, setup: &str, stock: &Path) -> Result<PathBuf, String> {
    let base = std::fs::read(stock).map_err(|e| format!("cannot read {}: {e}", stock.display()))?;
    let inner = &setup[..setup.len().min(31)];
    let b = opponent_setup_bytes(&base, car, inner, None, None).ok_or_else(|| format!("{} is not a set-up file (8176 bytes)", stock.display()))?;
    let out_dir = mcla_mods::cache_dir(mods).join("setups");
    let out = out_dir.join(format!("{setup}.xcc"));
    if std::fs::read(&out).ok().as_deref() != Some(b.as_slice()) {
        std::fs::create_dir_all(&out_dir).map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
        std::fs::write(&out, &b).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    }
    Ok(out)
}

/// The rims drawn for this race's opponent set-ups of cars with `rims = "random"`: set-up name -> the rim table index
/// (+0x844 in place of the set-up's own; stage S4b). Disc set-ups are never in it.
static RIM_DRAWS: std::sync::Mutex<Vec<(String, i32)>> = std::sync::Mutex::new(Vec::new());

/// Record the rims drawn for the grid about to be made (a set-up drawn again takes its new rim; a kept grid of a series
/// keeps the ones it has).
pub fn set_rim_draws(draws: Vec<(String, i32)>) {
    let mut all = RIM_DRAWS.lock().unwrap_or_else(|e| e.into_inner());
    all.retain(|o| !draws.iter().any(|n| n.0.eq_ignore_ascii_case(&o.0)));
    all.extend(draws);
}

/// The rim table index drawn for the opponent set-up `setup` of this race, if any.
pub fn rim_draw(setup: &str) -> Option<i32> {
    RIM_DRAWS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|d| d.0.eq_ignore_ascii_case(setup)).map(|d| d.1)
}

/// One rim out of `candidates` (rim table indices) for the opponent in grid place `slot`, from the race's seed: the
/// generator of the game's pick (MWC, seeded as the career's: lo = s + (s == 0), carry = s ^ rot16(s)), two steps in.
/// None without candidates.
pub fn draw_rim(candidates: &[i32], seed: u32, slot: usize) -> Option<i32> {
    if candidates.is_empty() {
        return None;
    }
    let mut rng = crate::oppick::seeded(seed ^ (slot as u32 + 1).wrapping_mul(0x9E37_79B9));
    rng.next();
    Some(candidates[((rng.next() & 0x7FFF_FFFF) as usize) % candidates.len()])
}

/// A disc car's resource folder (a donor's), for a reader that holds a mod car's folder.
pub fn disc_vehicle_dir(name: &str) -> Option<PathBuf> {
    Some(get()?.data_root.join("resources/vehicle").join(name))
}

/// The tuning donor of a mod car.
pub fn tune_donor(name: &str) -> Option<&'static str> {
    car(name).and_then(|c| c.tune.as_deref())
}

/// A garage picture of a mod car: its own file, or the donor's picture on the disc. `kind` 0 `mr_`, 1 `sh_`, 2 `msh_`.
pub fn picture(kind: usize, name: &str) -> Option<PathBuf> {
    let c = car(name)?;
    let (dir, prefix) = PICTURES[kind.min(2)];
    if let Some(p) = c.pictures[kind.min(2)].clone() {
        return Some(p);
    }
    let donor = c.picture_donor.as_deref()?;
    Some(get()?.data_root.join("resources/ui/textures/garage/dyntextures").join(dir).join(format!("{prefix}{donor}.r9")))
}

/// The brake folder of a mod car (`donors.brake`: an index of the brakes list).
pub fn brake_dir(name: &str) -> Option<PathBuf> {
    let b = crate::mc4::BRAKES.get(car(name)?.brake? as usize)?;
    Some(get()?.data_root.join("resources/brakes").join(b))
}

/// The mark the vehicle lists show beside a mod car's class (MODS.md design 7: the showroom's purchase view and the
/// garage's MANAGE VEHICLES, nowhere else).
pub const MOD_MARK: &str = "MOD";

fn build(data_root: &Path, dir: &Path) -> Registry {
    let disabled = crate::hooks::get().map(|r| mcla_mods::disabled_ids(&r.settings_path)).unwrap_or_default();
    let opts = mcla_mods::LoadOptions { disabled, reserved: vec![crate::catalog::CUT_GTR.to_string()] };
    let disc = mcla_mods::disc::DiscInfo::load(data_root);
    let mut set = if dir.is_dir() { mcla_mods::ModSet::load_from(dir, &disc, &opts) } else { mcla_mods::ModSet::empty(dir.to_path_buf()) };
    if !set.cars().is_empty() {
        if let Err(e) = set.unpack_zips(&mcla_mods::cache_dir(dir)) {
            log::warn!("mods: cannot unpack the zip mods into {}: {e}", mcla_mods::cache_dir(dir).display());
        }
    }
    let mut cars = Vec::new();
    for c in set.cars() {
        match car_rt(&set, c, data_root, dir) {
            Ok(rt) => {
                log::info!("mods: car {} (mod {}) as table index {} from {}", rt.name, rt.mod_id, rt.index, rt.dir.display());
                cars.push(rt);
            }
            Err(e) => log::warn!("mods: car {} (mod {}) left out: {e}", c.name, c.mod_id),
        }
    }
    // (only the cars that could be put together: the list, names and indices follow them)
    let extra_list = set.cars().iter().filter(|c| cars.iter().any(|r: &ModCarRt| r.name == c.name)).map(mcla_mods::vehicle_block).collect();
    let strings = set.strings().into_iter().filter(|(k, _)| cars.iter().any(|r| k == &r.name || k == &format!("{}_shrt", r.name))).collect();
    // (opponent use, stage S4: only the cars that could be put together and are marked; a driver whose set-up cannot be
    // had is left out, a car left without a driver is not given to the pick)
    let mut opponent_extra = String::new();
    let mut opponent_setups = Vec::new();
    let mut free_driver_cars = Vec::new();
    let mut rims_random = Vec::new();
    for c in set.cars().iter().filter(|c| cars.iter().any(|r| r.name == c.name)) {
        let Some(u) = c.opponent_use.as_ref() else { continue };
        let mut pairs: Vec<(String, String)> = Vec::new();
        for d in &u.drivers {
            let file = match disk(set.find(&format!("resources/vehicle/configs/{}.xcc", d.setup))) {
                Some(p) => Ok(p),
                None => match d.setup_from.as_deref() {
                    Some(from) => make_opponent_setup(data_root, dir, &c.name, &d.setup, from, c.donors.brake, d.plate.as_deref()),
                    None => Err("no set-up file".to_string()),
                },
            };
            match file {
                Ok(p) => {
                    opponent_setups.push((d.setup.clone(), p));
                    pairs.push((d.driver.clone(), d.setup.clone()));
                }
                Err(e) => log::warn!("mods: opponent {} of car {} left out: {e}", d.driver, c.name),
            }
        }
        if u.rims_random {
            rims_random.push(c.name.to_ascii_lowercase());
        }
        // (no driver declared, stage S4b: one set-up per character of Opponent.lst, made from the car's stock set-up)
        if u.drivers.is_empty() {
            let Some(stock) = cars.iter().find(|r| r.name == c.name).and_then(|r| r.setup.clone()) else {
                log::warn!("mods: car {} is marked for opponents without a driver but has no stock set-up: not on any grid", c.name);
                continue;
            };
            let mut made = 0;
            for id in &disc.drivers {
                let name = free_setup_name(id, &c.name);
                match make_free_setup(dir, &c.name, &name, &stock) {
                    Ok(p) => {
                        opponent_setups.push((name, p));
                        made += 1;
                    }
                    Err(e) => log::warn!("mods: free-driver set-up {name} of car {} not made: {e}", c.name),
                }
            }
            if made == 0 {
                log::warn!("mods: car {} is marked for opponents without a driver but no set-up could be made: not on any grid", c.name);
                continue;
            }
            free_driver_cars.push(c.name.to_ascii_lowercase());
        }
        match mcla_mods::opponent_text(c, Some(&pairs)) {
            Some(t) => {
                log::info!("mods: car {} races as an opponent ({})", c.name, if pairs.is_empty() { "a free driver of the band".to_string() } else { pairs.iter().map(|p| format!("{} in {}", p.0, p.1)).collect::<Vec<_>>().join(", ") });
                opponent_extra.push_str(&t);
            }
            None => log::warn!("mods: car {} is marked for opponents but has no driver with a set-up: not on any grid", c.name),
        }
    }
    if !cars.is_empty() {
        log::info!("mods: {} car(s) from {}", cars.len(), dir.display());
    }
    Registry { data_root: data_root.to_path_buf(), mods_dir: dir.to_path_buf(), cars, extra_list, strings, opponent_extra, opponent_setups, free_driver_cars, rims_random, set }
}

fn disk(f: Option<mcla_mods::ModFile<'_>>) -> Option<PathBuf> {
    f.and_then(|f| f.disk_path()).filter(|p| p.is_file())
}

fn car_rt(set: &mcla_mods::ModSet, c: &mcla_mods::ModCar, data_root: &Path, mods: &Path) -> Result<ModCarRt, String> {
    let name = c.name.as_str();
    let d = &c.donors;
    // the resource folder: where its files lie on disk
    let files = set.list_dir(&format!("resources/vehicle/{name}"));
    let dir = files.iter().find_map(|f| f.disk_path()).and_then(|p| p.parent().map(Path::to_path_buf)).ok_or("its resource folder is not on disk")?;
    // stock rims: the mod's own folder first, else the donor's on the disc
    let own_rims = ["whl_stk_", "whld_stk_"].iter().find_map(|p| {
        let leaf = format!("{p}{}", name.trim_start_matches("vpd_").trim_start_matches("vp_"));
        set.list_dir(&format!("resources/rims/{leaf}")).iter().find_map(|f| f.disk_path()).and_then(|p| p.parent().map(Path::to_path_buf))
    });
    let rims = own_rims.or_else(|| d.rims.as_ref().map(|r| data_root.join("resources/rims").join(r))).filter(|p| p.is_dir());
    // stock set-up: the mod's own, else made from the donor's (8176 bytes, the GT-R's recipe)
    let setup = match disk(set.find(&format!("resources/vehicle/configs/{name}.xcc"))) {
        Some(p) => Some(p),
        None => match d.setup.as_deref() {
            Some(donor) => Some(make_setup(data_root, mods, name, donor, d.brake)?),
            None => None,
        },
    };
    let pictures = PICTURES.map(|(sub, prefix)| {
        let base = format!("resources/ui/textures/garage/dyntextures/{sub}");
        disk(set.find(&format!("{base}/{prefix}{name}.r9"))).or_else(|| disk(set.find(&format!("{base}/0x{:08x}.r9", mcla_formats::hash::joaat(&format!("{prefix}{name}.xtd"))))))
    });
    let prices_text = set.find(&format!("tune/garage/{name}_parts.gtun")).and_then(|f| f.read().ok()).map(|b| String::from_utf8_lossy(&b).into_owned());
    Ok(ModCarRt {
        name: name.to_string(),
        mod_id: c.mod_id.clone(),
        index: c.index,
        dir,
        setup,
        rims,
        brake: d.brake,
        tune: d.tune.clone(),
        sound: d.sound.clone(),
        cameras: d.cameras.clone(),
        garage: d.garage.clone(),
        prices: if prices_text.is_none() { d.prices.clone() } else { None },
        prices_scale: (prices_text.is_none() && d.prices.is_some() && (d.prices_level.is_some() || d.prices_kit.is_some() || d.prices_maker.is_some()))
            .then(|| PriceScale { level: d.prices_level.clone(), kit: d.prices_kit, maker: d.prices_maker }),
        prices_text,
        pictures,
        picture_donor: d.pictures.clone(),
        long_name: c.long_name.clone(),
        short_name: c.short_name.clone(),
    })
}

/// Make a mod car's stock set-up from the donor's (`resources/vehicle/configs/<donor>.xcc` on the disc) into
/// `<mods>/.cache/setups/<car>.xcc`: the vehicle hash (+0x1FA8) and names (+0x1FAD TypeName, +0x1FCD set-up name)
/// are the car's, no rim of the rim table (+0x844 = 255: the car keeps its stock rim folder) and, when the manifest
/// names one, its brake (+0x846). The same recipe as `catalog::write_cut_setups` (DECISIONS.md D21).
pub fn make_setup(data_root: &Path, mods: &Path, car: &str, donor: &str, brake: Option<u8>) -> Result<PathBuf, String> {
    let src = crate::render::car_lamps::setup_file(data_root, donor).ok_or_else(|| format!("the set-up donor {donor} has no set-up file"))?;
    let base = std::fs::read(&src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    let b = setup_bytes(&base, car, brake).ok_or_else(|| format!("{} is not a set-up file (8176 bytes)", src.display()))?;
    let out_dir = mcla_mods::cache_dir(mods).join("setups");
    let out = out_dir.join(format!("{car}.xcc"));
    if std::fs::read(&out).ok().as_deref() != Some(b.as_slice()) {
        std::fs::create_dir_all(&out_dir).map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
        std::fs::write(&out, &b).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        log::info!("mods: wrote the set-up {} (from {donor})", out.display());
    }
    Ok(out)
}

/// Make an opponent set-up of a mod car (`GH_nn_k_<car>`, stage S4) from the disc set-up `from` into
/// `<mods>/.cache/setups/<setup>.xcc`, as `catalog::write_cut_setups` makes the GT-R's `GH_12_2_vp_nsn_gtr_08`: the
/// vehicle hash (+0x1FA8) and TypeName (+0x1FAD) the car's, the set-up name (+0x1FCD) its own, the plate (+0x847, 7
/// characters, padded with spaces) when given. Made from a car's stock set-up (`from` a car of the vehicle table) it
/// also gets the stock set-up's rules of [`setup_bytes`] (+0x844 = 255, the manifest's brake); from an opponent's
/// set-up it keeps that set-up's rim and brake.
pub fn make_opponent_setup(data_root: &Path, mods: &Path, car: &str, setup: &str, from: &str, brake: Option<u8>, plate: Option<&str>) -> Result<PathBuf, String> {
    let src = crate::render::car_lamps::setup_file(data_root, from).ok_or_else(|| format!("the set-up {from} is not in the data"))?;
    let base = std::fs::read(&src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    let stock = crate::mc4::VEHICLES.iter().any(|v| v.eq_ignore_ascii_case(from));
    let b = opponent_setup_bytes(&base, car, setup, stock.then_some(brake), plate).ok_or_else(|| format!("{} is not a set-up file (8176 bytes)", src.display()))?;
    let out_dir = mcla_mods::cache_dir(mods).join("setups");
    let out = out_dir.join(format!("{setup}.xcc"));
    if std::fs::read(&out).ok().as_deref() != Some(b.as_slice()) {
        std::fs::create_dir_all(&out_dir).map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
        std::fs::write(&out, &b).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        log::info!("mods: wrote the opponent set-up {} (from {from})", out.display());
    }
    Ok(out)
}

/// The bytes of [`make_opponent_setup`]; `stock` = Some(brake) when the base is a car's stock set-up.
pub fn opponent_setup_bytes(base: &[u8], car: &str, setup: &str, stock: Option<Option<u8>>, plate: Option<&str>) -> Option<Vec<u8>> {
    if setup.len() > 31 {
        return None;
    }
    let mut b = match stock {
        Some(brake) => setup_bytes(base, car, brake)?,
        None => {
            if base.len() != 8176 || car.len() > 31 {
                return None;
            }
            let mut b = base.to_vec();
            b[0x1FA8..0x1FAC].copy_from_slice(&mcla_formats::hash::joaat(car).to_be_bytes());
            b[0x1FAD..0x1FAD + 32].fill(0);
            b[0x1FAD..0x1FAD + car.len()].copy_from_slice(car.as_bytes());
            b
        }
    };
    b[0x1FCD..0x1FCD + 32].fill(0);
    b[0x1FCD..0x1FCD + setup.len()].copy_from_slice(setup.as_bytes());
    if let Some(p) = plate {
        let mut raw = p.as_bytes().iter().copied().take(7).collect::<Vec<u8>>();
        raw.resize(7, b' ');
        b[0x847..0x84E].copy_from_slice(&raw);
    }
    Some(b)
}

/// The bytes of [`make_setup`].
pub fn setup_bytes(base: &[u8], car: &str, brake: Option<u8>) -> Option<Vec<u8>> {
    if base.len() != 8176 || car.len() > 31 {
        return None;
    }
    let mut b = base.to_vec();
    b[0x1FA8..0x1FAC].copy_from_slice(&mcla_formats::hash::joaat(car).to_be_bytes());
    for at in [0x1FAD, 0x1FCD] {
        b[at..at + 32].fill(0);
        b[at..at + car.len()].copy_from_slice(car.as_bytes());
    }
    b[0x844] = 255;
    if let Some(k) = brake {
        b[0x846] = k;
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    /// A mod car's set-up: the donor's with the car's hash and names, no table rim, the manifest's brake.
    #[test]
    fn setup_from_donor() {
        let mut base = vec![0u8; 8176];
        base[0x844] = 7;
        base[0x846] = 3;
        base[0x1FAD..0x1FAD + 14].copy_from_slice(b"vp_nsn_350z_06");
        let b = super::setup_bytes(&base, "vp_test_mod_01", Some(9)).unwrap();
        assert_eq!(&b[0x1FA8..0x1FAC], &mcla_formats::hash::joaat("vp_test_mod_01").to_be_bytes());
        assert_eq!(&b[0x1FAD..0x1FAD + 15], b"vp_test_mod_01\0");
        assert_eq!(&b[0x1FCD..0x1FCD + 15], b"vp_test_mod_01\0");
        assert_eq!((b[0x844], b[0x846]), (255, 9));
        assert_eq!(super::setup_bytes(&base, "vp_test_mod_01", None).unwrap()[0x846], 3);
        assert!(super::setup_bytes(&base[..100], "vp_x", None).is_none());
        // (the car record the game makes from it names the car and has the table index of a mod car: none here)
        let car = crate::mc4::Car::from_xcc(&b, "vp_test_mod_01", "vp_test_mod_01", 0).unwrap();
        assert_eq!((car.TypeName.as_str(), car.vehicle_index), ("vp_test_mod_01", -1));
    }

    /// A mod car's own price list is read as the disc's lists are (the 350Z's file, parsed both ways).
    #[test]
    fn own_price_list_reads_as_the_disc_lists() {
        let data = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let file = std::path::Path::new(&data).join("tune/garage").join(format!("0x{:08x}", mcla_formats::hash::joaat("vp_nsn_350z_06_parts.gtun")));
        let Ok(text) = std::fs::read_to_string(&file) else { return };
        let mut g = crate::game::Game::new(vec![std::path::PathBuf::from(&data).join("resources/vehicle/vp_nsn_350z_06")], 0);
        g.load_perf_prices(&data);
        let rows = crate::game::parse_part_rows(&text);
        assert!(rows.len() > 10, "{} sections", rows.len());
        for (sec, list) in &rows {
            assert_eq!(g.part_entries_sec(0, sec), Some(list), "{sec}");
        }
    }

    /// Price scaling of a donor's list (DECISIONS.md D28 "Prices", D21's recipe): car-made rows (brand 0) by the ratio of
    /// the body kits rounded to $5, branded rows as they are, scissor doors from the level's list, the stock rows'
    /// logo the manifest's maker; a body-kit price works the same without a level list.
    #[test]
    fn price_scaling_of_a_donor_list() {
        use crate::catalog::PartList;
        let list = |rows: &[(&str, Vec<(i32, i64, usize)>)]| -> PartList { rows.iter().map(|(s, r)| (s.to_string(), r.clone())).collect() };
        let donor = list(&[
            ("body", vec![(0, 0, 0), (0, 13030, 1)]),
            ("bumper_front", vec![(35, 0, 0), (0, 1500, 1), (0, 2210, 2)]),
            ("exhaust", vec![(12, 900, 1)]),
            ("scissor_door", vec![(0, 4000, 1)]),
        ]);
        let level = list(&[("body", vec![(0, 30600, 1)]), ("scissor_door", vec![(0, 9000, 1)])]);
        let by_level = super::PriceScale { level: Some("vp_aud_r8_08".into()), kit: None, maker: Some(52) };
        let out = by_level.apply(&donor, |c| (c == "vp_aud_r8_08").then_some(&level)).unwrap();
        // 1500 x 30600 / 13030 = 3522.6 -> $3,525; 2210 -> 5190.0 -> $5,190; the kit itself -> $30,600
        assert_eq!(out["bumper_front"], vec![(52, 0, 0), (0, 3525, 1), (0, 5190, 2)]);
        assert_eq!(out["body"][1], (0, 30600, 1));
        assert_eq!(out["exhaust"], vec![(12, 900, 1)], "branded rows at the list's own price");
        assert_eq!(out["scissor_door"], vec![(0, 9000, 1)], "the level's scissor doors");
        // the same as the cut GT-R's function
        assert_eq!(out, crate::catalog::scale_price_list(&donor, &level, 52));
        // a body-kit price: same rows, the donor's own scissor doors (scaled), its own logo without a maker
        let by_kit = super::PriceScale { level: None, kit: Some(30600), maker: None };
        let out = by_kit.apply(&donor, |_| None).unwrap();
        assert_eq!(out["bumper_front"], vec![(35, 0, 0), (0, 3525, 1), (0, 5190, 2)]);
        assert_eq!(out["scissor_door"], vec![(0, 9395, 1)]);
        // a level car without a list: nothing to scale to (the game keeps the donor's list then: game.rs)
        assert!(by_level.apply(&donor, |_| None).is_none());
    }

    /// An opponent set-up of a mod car (stage S4): from an opponent's set-up only the car's hash, TypeName, the set-up's
    /// own name and the plate change; from a stock set-up also the stock rules (no table rim, the manifest's brake).
    #[test]
    fn opponent_setup_from_donor() {
        let mut base = vec![0u8; 8176];
        base[0x844] = 7;
        base[0x846] = 3;
        base[0x847..0x84E].copy_from_slice(b"5HZX106");
        base[0x1FAD..0x1FAD + 14].copy_from_slice(b"vp_nsn_350z_06");
        base[0x1FCD..0x1FCD + 22].copy_from_slice(b"GH_06_2_vp_nsn_350z_06");
        let b = super::opponent_setup_bytes(&base, "vp_test_mod_01", "GH_06_2_vp_test_mod_01", None, Some("TEST 1")).unwrap();
        assert_eq!(&b[0x1FA8..0x1FAC], &mcla_formats::hash::joaat("vp_test_mod_01").to_be_bytes());
        assert_eq!(&b[0x1FAD..0x1FAD + 15], b"vp_test_mod_01\0");
        assert_eq!(&b[0x1FCD..0x1FCD + 23], b"GH_06_2_vp_test_mod_01\0");
        assert_eq!((b[0x844], b[0x846], &b[0x847..0x84E]), (7, 3, &b"TEST 1 "[..]));
        // nothing else changes
        assert_eq!(b.iter().zip(&base).enumerate().filter(|(_, (x, y))| x != y).filter(|(i, _)| !(0x847..0x84E).contains(i) && *i < 0x1FA8).count(), 0);
        let s = super::opponent_setup_bytes(&base, "vp_test_mod_01", "GH_06_2_vp_test_mod_01", Some(Some(9)), None).unwrap();
        assert_eq!((s[0x844], s[0x846], &s[0x847..0x84E]), (255, 9, &b"5HZX106"[..]));
        assert!(super::opponent_setup_bytes(&base, "vp_test_mod_01", "GH_06_2_vp_test_mod_01_and_a_long_name", None, None).is_none());
        // the record the game makes from it names the car; its plate reads as written
        let car = crate::mc4::Car::from_xcc(&b, "vp_test_mod_01", "GH_06_2_vp_test_mod_01", 0).unwrap();
        assert_eq!(car.TypeName.as_str(), "vp_test_mod_01");
    }

    /// Without `init` nothing is a mod car (unit tests and runs without mods behave as before).
    #[test]
    fn no_registry_no_mod_cars() {
        assert!(!super::is_mod_car("vp_nsn_350z_06"));
        assert_eq!(super::extra_list(), "");
        assert!(super::car_dirs().is_empty());
        assert_eq!(super::opponent_extra(), "");
        assert!(super::setup_file("GH_12_2_vp_test_mod_01").is_none());
    }

    /// `rims = "random"` (stage S4b): a rim of the shop's car brands (Car and Dlc, not Bike), the same for the same seed
    /// and place, different over places and seeds, and it replaces only the set-up it was drawn for.
    #[test]
    fn random_opponent_rims() {
        let dir = "D:/MCLA/extracted/cache";
        if !std::path::Path::new(dir).join("tune/ui/garage/elements").is_dir() {
            return;
        }
        let shop = crate::rimshop::load(dir);
        let rims = shop.car_rims();
        assert!(rims.len() > 80 && rims.iter().all(|r| shop.entry(r.0).is_some()));
        let bike: Vec<i32> = shop.brands.iter().filter(|b| b.shop == crate::rimshop::Shop::Bike).flat_map(|b| b.rims.iter().map(|r| r.index)).collect();
        let cands: Vec<i32> = rims.iter().map(|r| r.0).collect();
        assert!(cands.iter().all(|i| !bike.contains(i)) && !bike.is_empty());
        let mut seen = std::collections::BTreeSet::new();
        for seed in 1..40u32 {
            for slot in 0..3 {
                let a = super::draw_rim(&cands, seed, slot).unwrap();
                assert_eq!(Some(a), super::draw_rim(&cands, seed, slot), "reproducible");
                assert!(cands.contains(&a) && shop.entry(a).is_some());
                seen.insert(a);
            }
        }
        assert!(seen.len() > 30, "{}", seen.len());
        assert_eq!(super::draw_rim(&[], 1, 0), None);
        super::set_rim_draws(vec![("GH_10_m_vp_x".into(), 77)]);
        super::set_rim_draws(vec![("GH_11_m_vp_x".into(), 88), ("GH_10_m_vp_x".into(), 99)]);
        assert_eq!((super::rim_draw("gh_10_m_vp_x"), super::rim_draw("GH_11_m_vp_x"), super::rim_draw("GH_12_2_vp_aud_r8_08")), (Some(99), Some(88), None));
    }

    /// A free driver's set-up (stage S4b): the stock set-up with the set-up name (cut to the 31-character field), rim, brake
    /// and plate of the stock set-up kept.
    #[test]
    fn free_driver_setup_from_stock() {
        let mut stock = vec![0u8; 8176];
        stock[0x844] = 255;
        stock[0x846] = 4;
        stock[0x847..0x84E].copy_from_slice(b"STOCK 1");
        stock[0x1FAD..0x1FAD + 14].copy_from_slice(b"vp_test_mod_01");
        let mods = std::env::temp_dir().join(format!("mcla-free-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&mods);
        std::fs::create_dir_all(&mods).unwrap();
        std::fs::write(mods.join("stock.xcc"), &stock).unwrap();
        let car = "vp_test_a_rather_long_car_name";
        let name = super::free_setup_name("GH_10", car);
        assert_eq!(name, "GH_10_m_vp_test_a_rather_long_car_name");
        let out = super::make_free_setup(&mods, car, &name, &mods.join("stock.xcc")).unwrap();
        let b = std::fs::read(&out).unwrap();
        let _ = std::fs::remove_dir_all(&mods);
        assert_eq!(b.len(), 8176);
        assert_eq!(&b[0x1FA8..0x1FAC], &mcla_formats::hash::joaat(car).to_be_bytes());
        assert_eq!(&b[0x1FAD..0x1FAD + car.len()], car.as_bytes());
        assert_eq!(&b[0x1FCD..0x1FCD + 31], &name.as_bytes()[..31]);
        assert_eq!((b[0x844], b[0x846], &b[0x847..0x84E]), (255, 4, &b"STOCK 1"[..]));
    }
}
