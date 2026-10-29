//! Named rivals: the hookmen of `tune/career/carcfg/*` (`Character { Name GH_06, Car { Name
//! vp_dsn_280z_75, Config { Name GH_06_0_vp_dsn_280z_75 } } ... }`, one car per
//! performance tier) and their car set-ups in `resources/vehicle/configs/<config>.xcc`
//! (a fixed 8176-byte big-endian block - the garage record's layout, `tools/mc4save/vehicle.py`:
//! the body's first paint colour, RGB, at 0x660; licence plate text at 0x847).
//! (Step 19e: 0x90 / 0xa0 / 0xb0, read here before, are the rims' colours A, B, C.)

use hashbrown::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct RivalCar {
    pub car: String,
    pub config: String,
}

#[derive(Clone, Debug)]
pub struct Character {
    /// "GH_06"
    pub id: String,
    pub cars: Vec<RivalCar>,
}

/// One opponent on a grid.
#[derive(Clone, Debug)]
pub struct Rival {
    pub id: String,
    pub car: String,
    pub color: [f32; 4],
    pub plate: String,
    /// car set-up name (its livery texture goes by it)
    pub config: String,
}

#[derive(Default)]
pub struct Rivals {
    pub characters: Vec<Character>,
    config_dir: PathBuf,
    paints: HashMap<String, Option<([f32; 3], String)>>,
}


/// Performance tier of a car group: D0 0, C0 1, B0 2, A0 3.
pub fn tier(group: &str) -> usize {
    match group.to_ascii_uppercase().chars().next() {
        Some('C') => 1,
        Some('B') => 2,
        Some('A') | Some('S') => 3,
        _ => 0,
    }
}

impl Rivals {
    pub fn load(data_dir: &str) -> Rivals {
        let root = Path::new(data_dir);
        let mut r = Rivals { config_dir: root.join("resources/vehicle/configs"), ..Default::default() };
        let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("tune/career/carcfg")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        files.sort();
        for f in files {
            let Ok(t) = std::fs::read_to_string(&f) else { continue };
            for ch in t.split("Character").skip(1) {
                let mut words = ch.split_whitespace();
                let mut id = String::new();
                let mut cars: Vec<RivalCar> = Vec::new();
                let mut depth = 0;
                while let Some(w) = words.next() {
                    match w {
                        "{" => depth += 1,
                        "}" => depth -= 1,
                        "Name" => {
                            let v = words.next().unwrap_or("").to_string();
                            match depth {
                                1 => id = v,
                                2 => cars.push(RivalCar { car: v.to_ascii_lowercase(), config: String::new() }),
                                3 => {
                                    if let Some(c) = cars.last_mut() {
                                        c.config = v;
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                // the same character appears in several files (main game, DLC): keep the first
                if !id.is_empty() && !cars.is_empty() && !r.characters.iter().any(|c| c.id == id) {
                    r.characters.push(Character { id, cars });
                }
            }
        }
        log::info!("rivals: {} characters", r.characters.len());
        r
    }

    /// The paint colour of a car set-up (for a vehicle that is not a rival's: a time trial's).
    pub fn paint_of(&mut self, config: &str) -> Option<[f32; 3]> {
        self.paint(config).map(|p| p.0)
    }

    /// Paint colour and plate of a car set-up.
    fn paint(&mut self, config: &str) -> Option<([f32; 3], String)> {
        if let Some(p) = self.paints.get(config) {
            return p.clone();
        }
        // (a mod car's opponent set-up lies in the mod or its cache: modcars.rs)
        let file = crate::modcars::setup_file(config).unwrap_or_else(|| self.config_dir.join(format!("{config}.xcc")));
        let p = std::fs::read(file).ok().filter(|d| d.len() >= 0x850).map(|d| {
            let f = |o: usize| f32::from_be_bytes(d[o..o + 4].try_into().unwrap());
            let plate: String = d[0x847..0x84e].iter().take_while(|&&b| b.is_ascii_graphic() || b == b' ').map(|&b| b as char).collect();
            ([f(0x660).clamp(0.0, 1.0), f(0x664).clamp(0.0, 1.0), f(0x668).clamp(0.0, 1.0)], plate.trim().to_string())
        });
        self.paints.insert(config.to_string(), p.clone());
        p
    }

    fn rival(&mut self, ci: usize, tier: usize) -> Rival {
        let ch = self.characters[ci].clone();
        let c = &ch.cars[tier.min(ch.cars.len() - 1)];
        let (rgb, plate) = self.paint(&c.config).unwrap_or(([0.6, 0.6, 0.62], String::new()));
        Rival { id: ch.id.clone(), car: c.car.clone(), color: [rgb[0], rgb[1], rgb[2], 1.0], plate, config: c.config.clone() }
    }

    /// Step 19d: a character in a given car and set-up (a slot hookman as his race's opponent).
    pub fn with_config(&mut self, id: &str, car: &str, config: &str) -> Rival {
        let (rgb, plate) = self.paint(config).unwrap_or(([0.6, 0.6, 0.62], String::new()));
        Rival { id: id.to_string(), car: car.to_string(), color: [rgb[0], rgb[1], rgb[2], 1.0], plate, config: config.to_string() }
    }

    /// The named character in its car of the given tier.
    pub fn of(&mut self, id: &str, tier: usize) -> Option<Rival> {
        let ci = self.characters.iter().position(|c| c.id.eq_ignore_ascii_case(id))?;
        Some(self.rival(ci, tier))
    }

    /// `n` different rivals for a grid; `usable` rejects cars that cannot race here.
    pub fn grid(&mut self, n: usize, tier: usize, seed: u32, usable: &dyn Fn(&str) -> bool) -> Vec<Rival> {
        let mut out: Vec<Rival> = Vec::new();
        if self.characters.is_empty() {
            return out;
        }
        let len = self.characters.len();
        let mut h = seed | 1;
        let mut tries = 0;
        while out.len() < n && tries < len * 4 {
            tries += 1;
            h ^= h << 13;
            h ^= h >> 17;
            h ^= h << 5;
            let ci = h as usize % len;
            let r = self.rival(ci, tier);
            if !usable(&r.car) || out.iter().any(|o| o.id == r.id) {
                continue;
            }
            out.push(r);
        }
        out
    }
}
