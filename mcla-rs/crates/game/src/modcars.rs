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
    /// own pictures `mr_`, `sh_`, `msh_`
    pub pictures: [Option<PathBuf>; 3],
    /// picture donor (`donors.pictures`)
    pub picture_donor: Option<String>,
    pub long_name: String,
    pub short_name: String,
}

pub struct Registry {
    pub data_root: PathBuf,
    pub mods_dir: PathBuf,
    pub cars: Vec<ModCarRt>,
    /// the `Vehicle` blocks of the mod cars (vehicle.lst format, built in memory)
    pub extra_list: String,
    /// `<car>` / `<car>_shrt` string entries
    pub strings: Vec<(String, String)>,
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

/// The stock set-up file of a mod car (`name` = the car's name, as a bought car's record is made from).
pub fn setup_file(name: &str) -> Option<PathBuf> {
    car(name).and_then(|c| c.setup.clone())
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
    if !cars.is_empty() {
        log::info!("mods: {} car(s) from {}", cars.len(), dir.display());
    }
    Registry { data_root: data_root.to_path_buf(), mods_dir: dir.to_path_buf(), cars, extra_list, strings, set }
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

    /// Without `init` nothing is a mod car (unit tests and runs without mods behave as before).
    #[test]
    fn no_registry_no_mod_cars() {
        assert!(!super::is_mod_car("vp_nsn_350z_06"));
        assert_eq!(super::extra_list(), "");
        assert!(super::car_dirs().is_empty());
    }
}
