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

/// `tune/camera/<hash of name>` as text. A few files of the folder are on the disc under the hash of their name only,
/// and the installer's name list recovered a wrong name for them (another string of the same hash: the GTO 66's boom2
/// as `48.5115.bmp`, the Challenger 06 DUB's POV as `545.489258.ped`, the CL65's boom3, the Golf 83's and the Range
/// Rover Sport's reverse). The original asks for the hash and reads them; so a file not under `0x<hash>` is looked
/// for among the folder's other names by their hash.
pub fn camera_text(data_dir: &str, name: &str) -> Option<String> {
    let h = mcla_formats::hash::joaat(name);
    read_text(data_dir, &format!("tune/camera/0x{h:08x}")).or_else(|| camera_alias(data_dir, h).and_then(|n| read_text(data_dir, &format!("tune/camera/{n}"))))
}

/// The file of `<data>/tune/camera` stored under a name (not `0x<hash>`) whose hash is `h`.
fn camera_alias(data_dir: &str, h: u32) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static NAMES: Mutex<Option<(String, HashMap<u32, String>)>> = Mutex::new(None);
    let mut g = NAMES.lock().ok()?;
    if g.as_ref().map_or(true, |(d, _)| d != data_dir) {
        let mut m = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(Path::new(data_dir).join("tune/camera")) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if !n.starts_with("0x") {
                    m.insert(mcla_formats::hash::joaat(&n), n);
                }
            }
        }
        *g = Some((data_dir.to_string(), m));
    }
    g.as_ref()?.1.get(&h).cloned()
}

/// The first car of [`car_names`] (kind `k`) whose file `<car><suffix>` in `tune/camera` exists: the mod car's own
/// file, then its donor's. A disc car: its own alone, as before.
pub fn car_camera_text(data_dir: &str, car: &str, suffix: &str, k: Donor) -> Option<String> {
    car_names(car, k).iter().find_map(|n| camera_text(data_dir, &format!("{n}{suffix}")))
}
