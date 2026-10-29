//! What the loader needs from the disc data: the vehicle table (the `Vehicle` blocks of `tune/vehicle/vehicle.lst`
//! and `vehicle0001..0008.lst` in file order, `F-34441`; DEVONLY blocks dropped), the 33 manufacturers, the
//! performance samples (`car_performance_sample_0..4.lst`). Reference: docs/MODS.md "How the original finds a vehicle".

use crate::manifest::Figures;
use mcla_formats::hash::joaat;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The number of vehicle-table records on the disc (48 + 16 + 5; `mc4.rs` `VEHICLES`). Used only when the lists
/// cannot be read.
pub const DISC_VEHICLE_COUNT: usize = 69;

/// The list a mod car's block goes in: the first list file the original tries that the disc does not have
/// (`vehicle0003.lst`, archive hash 0x57872bce; 0003..0008 are loaded when present, MODS.md M-2).
pub const EXTRA_LIST_NAME: &str = "vehicle0003.lst";

#[derive(Clone, Debug, Default)]
pub struct DiscRow {
    pub name: String,
    pub figures: Figures,
}

#[derive(Clone, Debug, Default)]
pub struct DiscInfo {
    /// whether `vehicle.lst` was found (else the checks against the disc are skipped)
    pub found: bool,
    /// table order
    pub rows: Vec<DiscRow>,
    pub manufacturers: Vec<String>,
    /// car (lower case) -> (performance, speed) at mod 0 / 50 / 100
    pub samples: HashMap<String, ([f32; 3], [f32; 3])>,
    /// whether the game's `audio/X360/config/game.dat` and `sounds.dat` were read (else the sound checks against
    /// them are skipped)
    pub audio_found: bool,
    /// name hashes of the game's game.dat objects (car profiles, `_DEFAULT` objects ...) and sounds.dat sounds
    pub game_objects: HashSet<u32>,
    pub sound_objects: HashSet<u32>,
    /// the `Character` ids of `tune/career/carcfg/Opponent.lst` (empty when the file was not found)
    pub drivers: Vec<String>,
    /// the set-up files of `resources/vehicle/configs`: lower-case stems of `<name>.xcc` and the hashes of `0x...` files
    /// (empty when the folder was not found)
    pub setups: HashSet<String>,
    pub setup_hashes: HashSet<u32>,
}

/// A data file by name: the extracted copy is named by the joaat of the name, a file added by hand may keep its own
/// name (the same rule as `oppick::read_named`).
pub fn read_named(dir: &Path, name: &str) -> Option<String> {
    let b = std::fs::read(dir.join(format!("0x{:08x}", joaat(name)))).or_else(|_| std::fs::read(dir.join(name))).ok()?;
    Some(String::from_utf8_lossy(&b).into_owned())
}

impl DiscInfo {
    pub fn load(data_root: &Path) -> DiscInfo {
        let vdir = data_root.join("tune/vehicle");
        let mut d = DiscInfo::default();
        for i in 0..9 {
            let name = if i == 0 { "vehicle.lst".to_string() } else { format!("vehicle{i:04}.lst") };
            if let Some(t) = read_named(&vdir, &name) {
                d.found |= i == 0;
                parse_list(&t, &mut d);
            }
        }
        for i in 0..5 {
            if let Some(t) = read_named(&vdir, &format!("car_performance_sample_{i}.lst")) {
                parse_samples(&t, &mut d.samples);
            }
        }
        let cfg = data_root.join("audio/X360/config");
        let names = |f: &str| mcla_formats::audcfg::Dat::load(&cfg.join(f)).map(|dat| dat.objects.iter().map(|(n, _, _)| joaat(n)).collect::<HashSet<u32>>());
        if let (Some(g), Some(s)) = (names("game.dat"), names("sounds.dat")) {
            d.audio_found = true;
            d.game_objects = g;
            d.sound_objects = s;
        }
        if let Some(t) = read_named(&data_root.join("tune/career/carcfg"), "opponent.lst") {
            d.drivers = character_ids(&t);
        }
        for e in std::fs::read_dir(data_root.join("resources/vehicle/configs")).into_iter().flatten().flatten() {
            let f = e.file_name().to_string_lossy().to_ascii_lowercase();
            match f.strip_prefix("0x").and_then(|x| u32::from_str_radix(x, 16).ok()) {
                Some(h) if f.len() == 10 => {
                    d.setup_hashes.insert(h);
                }
                _ => {
                    if let Some(stem) = f.strip_suffix(".xcc") {
                        d.setups.insert(stem.to_string());
                    }
                }
            }
        }
        d
    }

    /// Whether the disc has the set-up `<name>.xcc` (by its plain or hashed file name).
    pub fn has_setup(&self, name: &str) -> bool {
        let l = name.to_ascii_lowercase();
        self.setups.contains(&l) || self.setup_hashes.contains(&joaat(&format!("{l}.xcc"))) || self.setup_hashes.contains(&joaat(&l))
    }

    /// Whether the disc's set-up folder was read (else set-up checks are skipped).
    pub fn setups_found(&self) -> bool {
        !self.setups.is_empty() || !self.setup_hashes.is_empty()
    }

    /// Table size the mod cars follow.
    pub fn count(&self) -> usize {
        if self.found {
            self.rows.len()
        } else {
            DISC_VEHICLE_COUNT
        }
    }

    pub fn has_car(&self, name: &str) -> bool {
        self.rows.iter().any(|r| r.name.eq_ignore_ascii_case(name))
    }

    pub fn row(&self, name: &str) -> Option<&DiscRow> {
        self.rows.iter().find(|r| r.name.eq_ignore_ascii_case(name))
    }

    /// The manufacturer's spelling in `vehicle.lst` (matched without case).
    pub fn manufacturer(&self, name: &str) -> Option<&str> {
        self.manufacturers.iter().find(|m| m.eq_ignore_ascii_case(name)).map(String::as_str)
    }
}

/// Line-based, as the lists are written (`Vehicle` / `Manufacturer`, then `{` ... `}` on their own lines).
fn parse_list(text: &str, d: &mut DiscInfo) {
    #[derive(PartialEq)]
    enum Kind {
        None,
        Vehicle,
        Manufacturer,
    }
    let (mut pending, mut open) = (Kind::None, Kind::None);
    let mut row = DiscRow::default();
    let mut dev_only = false;
    for line in text.lines() {
        let l = line.trim();
        match l {
            "Vehicle" if open == Kind::None => {
                pending = Kind::Vehicle;
                continue;
            }
            "Manufacturer" if open == Kind::None => {
                pending = Kind::Manufacturer;
                continue;
            }
            "{" if pending != Kind::None => {
                open = std::mem::replace(&mut pending, Kind::None);
                row = DiscRow::default();
                dev_only = false;
                continue;
            }
            "}" if open != Kind::None => {
                if open == Kind::Vehicle && !row.name.is_empty() && !dev_only {
                    d.rows.push(std::mem::take(&mut row));
                }
                open = Kind::None;
                continue;
            }
            _ => {}
        }
        pending = Kind::None;
        let mut it = l.split_whitespace();
        let Some(k) = it.next() else { continue };
        let v = it.next().unwrap_or("");
        let f = || v.parse::<f32>().unwrap_or(0.0);
        match open {
            Kind::Manufacturer if k == "Name" => d.manufacturers.push(v.to_string()),
            Kind::Vehicle => match k {
                "Name" => row.name = v.to_string(),
                "DEVONLY" => dev_only = true,
                "BaseAccel" => row.figures.base_accel = f(),
                "ModsAccel" => row.figures.mods_accel = f(),
                "BaseSpeed" => row.figures.base_speed = f(),
                "ModsSpeed" => row.figures.mods_speed = f(),
                "BaseHndlg" => row.figures.base_handling = f(),
                "ModsHndlg" => row.figures.mods_handling = f(),
                _ => {}
            },
            _ => {}
        }
    }
}

/// The `Character { Name <id> ... }` ids of `Opponent.lst`, in file order (the first `Name` after each `Character`).
fn character_ids(text: &str) -> Vec<String> {
    let w: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    for (i, t) in w.iter().enumerate() {
        if *t == "Character" {
            if let Some(k) = w[i + 1..].iter().position(|x| *x == "Name") {
                if let Some(id) = w.get(i + 1 + k + 1) {
                    out.push(id.to_string());
                }
            }
        }
    }
    out
}

/// `sample { name <car> mod 0|50|100 performance <f> speed <f> }` blocks.
fn parse_samples(text: &str, out: &mut HashMap<String, ([f32; 3], [f32; 3])>) {
    for s in text.split("sample").skip(1) {
        let tok: Vec<&str> = s.split_whitespace().collect();
        let field = |k: &str| tok.iter().position(|t| *t == k).and_then(|i| tok.get(i + 1)).copied();
        let (Some(name), Some(m), Some(p)) = (field("name"), field("mod").and_then(|v| v.parse::<i32>().ok()), field("performance").and_then(|v| v.parse::<f32>().ok())) else { continue };
        let sp = field("speed").and_then(|v| v.parse::<f32>().ok()).unwrap_or(f32::NAN);
        let k = match m {
            0 => 0,
            50 => 1,
            100 => 2,
            _ => continue,
        };
        let e = out.entry(name.to_ascii_lowercase()).or_insert(([f32::NAN; 3], [f32::NAN; 3]));
        e.0[k] = p;
        e.1[k] = sp;
    }
}
