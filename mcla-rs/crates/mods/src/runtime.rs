//! What the game's readers need of a loaded [`ModSet`] (stage S1b, docs/MODS.md): reads of data files through it,
//! the donor chain of a mod car's files, and where a mod's sound objects and wave banks are (format:
//! docs/reference/mods/manifest.md section 7). The game holds the one set of a run (`modcars.rs`) and passes it in.
//!
//! Without a set (`None`: tests, tools, no mods) every read is the disc's own, byte for byte.

use crate::manifest::Donors;
use crate::overlay::{self, Priority};
use crate::ModSet;
use std::path::{Path, PathBuf};

/// A mod's sound objects (`sounds.dat` format of the disc, merged into the game's: the game's objects win a name).
pub const SOUNDS_DAT: &str = "audio/X360/config/sounds.dat";
/// A mod's game objects (`game.dat` format of the disc: `<CAR>_PROFILE`, `_DEFAULT`, cross-faders ...; merged the same way).
pub const GAME_DAT: &str = "audio/X360/config/game.dat";
/// The package folder holding a mod's wave banks, in the installer's converted form (`<bank path>/_index.tsv` + `<wave>.wav`).
pub const WAVE_ROOT: &str = "audio_pc";

/// Which donor of the manifest stands in for a kind of file (`[car.donors]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Donor {
    Tune,
    Setup,
    Garage,
    Prices,
    Sound,
    Cameras,
    Pictures,
}

impl Donors {
    pub fn get(&self, k: Donor) -> Option<&str> {
        match k {
            Donor::Tune => self.tune.as_deref(),
            Donor::Setup => self.setup.as_deref(),
            Donor::Garage => self.garage.as_deref(),
            Donor::Prices => self.prices.as_deref(),
            Donor::Sound => self.sound.as_deref(),
            Donor::Cameras => self.cameras.as_deref(),
            Donor::Pictures => self.pictures.as_deref(),
        }
    }
}

impl ModSet {
    /// The car names whose files a car's file of kind `k` is looked up under, in order: the car itself, then (a
    /// loaded mod car only) its manifest donor. A disc car: itself alone.
    pub fn car_names(&self, car: &str, k: Donor) -> Vec<String> {
        let mut v = vec![car.to_string()];
        if let Some(d) = self.car(car).and_then(|c| c.donors.get(k)) {
            if !d.eq_ignore_ascii_case(car) {
                v.push(d.to_string());
            }
        }
        v
    }

    /// The file at `rel` of every loaded mod that has one (mod id order), not only the first: for files that are
    /// merged rather than replaced ([`SOUNDS_DAT`], [`GAME_DAT`]).
    pub fn read_each(&self, rel: &str) -> Vec<(String, Vec<u8>)> {
        let want = overlay::norm(rel);
        let mut out = Vec::new();
        for (id, p) in &self.packages {
            if let Some(f) = p.files.iter().find(|f| overlay::norm(&f.rel) == want) {
                match p.read(f) {
                    Ok(b) => out.push((id.clone(), b)),
                    Err(e) => log::warn!("mod {id}: cannot read {}: {e}", f.rel),
                }
            }
        }
        out
    }

    /// The folder on disk holding a mod's wave bank `bank` (a bank name of a sounds.dat string block, e.g.
    /// `PLAYER_VEHICLES\VP_X_08`): `audio_pc/<bank path, lower case>/_index.tsv` in the package. Zip mods need
    /// [`ModSet::unpack_zips`] first.
    pub fn wave_bank_dir(&self, bank: &str) -> Option<PathBuf> {
        let f = self.find(&format!("{}/_index.tsv", bank_folder(bank)))?;
        f.disk_path()?.parent().map(Path::to_path_buf)
    }
}

/// The package path of a wave bank's folder: `audio_pc/` + the bank name with `/` for `\`, lower case.
pub fn bank_folder(bank: &str) -> String {
    format!("{WAVE_ROOT}/{}", overlay::norm(bank))
}

/// The game.dat object naming a car's sound: `<CAR>_PROFILE` (the original builds it from the record's name).
pub fn profile_name(car: &str) -> String {
    format!("{}_PROFILE", car.to_ascii_uppercase())
}

/// [`ModSet::car_names`] of the game's set (`None`: no mods loaded - the car alone).
pub fn car_names(set: Option<&ModSet>, car: &str, k: Donor) -> Vec<String> {
    set.map_or_else(|| vec![car.to_string()], |s| s.car_names(car, k))
}

/// Read a data file (`rel` below the data root, plain or hashed name) through the game's mods: the disc's file when
/// it has one, else a mod's ([`Priority::DiscFirst`]: a disc car reads exactly what it read before). `None` for
/// `set`: the disc alone.
pub fn read_data(set: Option<&ModSet>, data_root: &Path, rel: &str) -> Option<Vec<u8>> {
    match set {
        Some(s) => s.read_layered(data_root, rel, Priority::DiscFirst),
        None => std::fs::read(data_root.join(rel)).ok(),
    }
}

/// [`read_data`] as text (`None` when it is not UTF-8, as `std::fs::read_to_string`).
pub fn read_text(set: Option<&ModSet>, data_root: &Path, rel: &str) -> Option<String> {
    String::from_utf8(read_data(set, data_root, rel)?).ok()
}
