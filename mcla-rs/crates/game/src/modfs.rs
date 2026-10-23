//! Data reads through the loaded mods (docs/MODS.md M-5 stage S1b; a user-approved deviation, DECISIONS.md D4 / D28):
//! a mod brings its own camera files, garage file and sound objects, its manifest's donors stand in for what it does
//! not bring. The set is the one `modcars::init` read; without it (unit tests, tools) every read is the disc's own.

use mcla_mods::runtime;
pub use mcla_mods::Donor;
use std::path::Path;

/// The run's mod set, when `modcars::init` ran.
pub fn set() -> Option<&'static mcla_mods::ModSet> {
    crate::modcars::get().map(|r| &r.set)
}

/// The names a car's file of kind `k` is looked up under: the car, then (a mod car) its manifest donor.
pub fn car_names(car: &str, k: Donor) -> Vec<String> {
    runtime::car_names(set(), car, k)
}

/// A data file as text: the disc's when it has one, else a mod's (plain or hashed name, `docs/reference/mods/manifest.md`).
pub fn read_text(data_dir: &str, rel: &str) -> Option<String> {
    runtime::read_text(set(), Path::new(data_dir), rel)
}

/// `tune/camera/<hash of name>` as text.
pub fn camera_text(data_dir: &str, name: &str) -> Option<String> {
    read_text(data_dir, &format!("tune/camera/0x{:08x}", mcla_formats::hash::joaat(name)))
}

/// The first car of [`car_names`] (kind `k`) whose file `<car><suffix>` in `tune/camera` exists: the mod car's own
/// file, then its donor's. A disc car: its own alone, as before.
pub fn car_camera_text(data_dir: &str, car: &str, suffix: &str, k: Donor) -> Option<String> {
    car_names(car, k).iter().find_map(|n| camera_text(data_dir, &format!("{n}{suffix}")))
}
