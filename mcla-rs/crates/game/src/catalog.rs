//! The game's vehicle catalogue (`tune/vehicle/0xe94bde63` + the DUB / South Central
//! list `0xccb5daee`): class, performance group, price and showroom stats per car, and
//! the reputation thresholds that unlock each group (`tune/career/0x327a2cd4`,
//! system "Unlocking": C0 at 10 %, B0 at 34 %, A0 at 60 % of 39 800 rep).

use hashbrown::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct CarInfo {
    pub name: String,
    pub class: String,
    pub manufacturer: String,
    /// performance group the car races in (D0, C0, B0, A0)
    pub group: String,
    /// group that has to be unlocked before the car can be bought
    pub unlock_group: String,
    pub price: i64,
    pub base_accel: f32,
    pub mods_accel: f32,
    /// top speed in mph, stock and fully upgraded
    pub base_speed: f32,
    pub mods_speed: f32,
    pub base_handling: f32,
    pub mods_handling: f32,
}

#[derive(Default)]
pub struct Catalog {
    cars: HashMap<String, CarInfo>,
}

/// total reputation of the career (progression system `MaxAmount` of the Complete
/// Edition's table; the original release had 39 800)
pub const MAX_REP: i64 = 33_500;

/// Reputation needed before cars of `group` can be bought.
pub fn group_rep(group: &str) -> i64 {
    let pct = match group.to_ascii_uppercase().as_str() {
        "C0" => 0.10,
        "B0" => 0.34,
        "A0" => 0.60,
        _ => 0.0,
    };
    (MAX_REP as f64 * pct) as i64
}

fn parse_blocks(text: &str, out: &mut HashMap<String, CarInfo>) {
    let mut cur: Option<CarInfo> = None;
    let mut pending = false;
    for line in text.lines() {
        let l = line.trim();
        if l == "Vehicle" {
            pending = true;
            continue;
        }
        if l == "{" {
            if pending {
                cur = Some(CarInfo::default());
                pending = false;
            }
            continue;
        }
        pending = false;
        if l == "}" {
            if let Some(c) = cur.take() {
                if !c.name.is_empty() {
                    out.insert(c.name.to_ascii_lowercase(), c);
                }
            }
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        let mut it = l.split_whitespace();
        let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
        let f = || v.parse::<f32>().unwrap_or(0.0);
        match k {
            "Name" => c.name = v.to_string(),
            "Class" => c.class = v.to_string(),
            "Manufacturer" => c.manufacturer = v.to_string(),
            "Group" => c.group = v.to_string(),
            "UnlockingGroup" => c.unlock_group = v.to_string(),
            "Price" => c.price = v.parse().unwrap_or(0),
            "BaseAccel" => c.base_accel = f(),
            "ModsAccel" => c.mods_accel = f(),
            "BaseSpeed" => c.base_speed = f(),
            "ModsSpeed" => c.mods_speed = f(),
            "BaseHndlg" => c.base_handling = f(),
            "ModsHndlg" => c.mods_handling = f(),
            _ => {}
        }
    }
}

/// The cut Nissan GT-R (R35): its model, stock rims and garage pictures are on the disc,
/// but no vehicle list has it, its tuning file was never filled in and it has no set-up
/// file. The port offers it in the showroom (docs/DECISIONS.md, 2026-10-22).
pub const CUT_GTR: &str = "vp_nsn_gtr_08";

/// Its row: the Audi R8's group, unlock, showroom figures and districts (it drives on the
/// R8's tuning); the districts let the opponent pick draw it (`oppick`).
pub const CUT_CATALOG: &str = "Vehicle
{
Name vp_nsn_gtr_08
Class EXOTIC
DistrictFilter Hollywood
DistrictFilter Beaches
DistrictFilter Hills
DistrictFilter Downtown
Manufacturer Nissan
Group B0
UnlockingGroup A0
Price 118000
BaseAccel 0.26
ModsAccel 0.42
BaseSpeed 210.00
ModsSpeed 255.00
BaseHndlg 0.620
ModsHndlg 0.875
}
";

/// The set-up its opponent driver races it in (`CUT_OPPONENT`), written by `write_cut_setups`.
pub const CUT_SETUP: &str = "GH_12_2_vp_nsn_gtr_08";

/// What the opponent pick (`oppick::OpPick::load`'s `extra`) needs besides the vehicle row: the
/// R8's performance samples at mod 0 / 50 / 100 (those in force after the sample files load)
/// and a driver - GH_12, whose B car is the Skyline - with the GT-R in his car map.
pub const CUT_OPPONENT: &str = "Character
{
Name GH_12
Car
{
Name vp_nsn_gtr_08
Config
{
Name GH_12_2_vp_nsn_gtr_08
}
}
}
sample
{
name vp_nsn_gtr_08
mod 0
performance 0.580801
speed 0.487276
}
sample
{
name vp_nsn_gtr_08
mod 50
performance 0.707273
speed 0.619890
}
sample
{
name vp_nsn_gtr_08
mod 100
performance 0.822529
speed 0.750580
}
";

/// The GT-R's set-up files, which the disc lacks: its stock set-up `vp_nsn_gtr_08.xcc` (a bought
/// car's record, its wheel sizes) and its opponent's `CUT_SETUP`. Both are the R8's stock set-up
/// (its tuning's wheels) with the vehicle's hash (+0x1FA8) and names (+0x1FAD, +0x1FCD) changed,
/// no rim of the rim table (+0x844 = 255: the car keeps its own `whl_stk_nsn_gtr_08`) and the
/// Brembo base brake (+0x846 = 9; the R8's own brake is index 14); the opponent's plate (+0x847)
/// reads GODZILA. Written once into the data
/// folder's `resources/vehicle/configs`.
pub fn write_cut_setups(data_dir: &str) {
    let dir = Path::new(data_dir).join("resources/vehicle/configs");
    let Ok(base) = std::fs::read(dir.join("vp_aud_r8_08.xcc")) else { return };
    if base.len() != 8176 {
        return;
    }
    for name in [CUT_GTR, CUT_SETUP] {
        let path = dir.join(format!("{name}.xcc"));
        if path.is_file() {
            continue;
        }
        let mut b = base.clone();
        b[0x1FA8..0x1FAC].copy_from_slice(&mcla_formats::hash::joaat(CUT_GTR).to_be_bytes());
        for (at, text) in [(0x1FAD, CUT_GTR), (0x1FCD, name)] {
            b[at..at + 32].fill(0);
            b[at..at + text.len()].copy_from_slice(text.as_bytes());
        }
        b[0x844] = 255;
        b[0x846] = 9;
        // (the opponent's own plate, not the R8 set-ups' 5HZX106)
        if name == CUT_SETUP {
            b[0x847..0x84E].copy_from_slice(b"GODZILA");
        }
        match std::fs::write(&path, &b) {
            Ok(()) => log::info!("catalog: wrote the set-up {}", path.display()),
            Err(e) => log::warn!("catalog: cannot write {}: {}", path.display(), e),
        }
    }
}

/// Its names, in the string table's style ("1999 SKYLINE GT-R V-SPEC" / "99 SKYLINE"): the
/// table has none for it.
pub const CUT_STRINGS: [(&str, &str); 2] = [("vp_nsn_gtr_08", "2008 GT-R"), ("vp_nsn_gtr_08_shrt", "08 GT-R")];

/// The car whose tuning file a car drives on: the R8's for the cut GT-R (whose own is a
/// template: every gear 4.0, 500 hp, no dimensions).
pub fn tune_donor(car: &str) -> Option<&'static str> {
    // (a mod car: its manifest's `donors.tune` - modcars.rs)
    car.eq_ignore_ascii_case(CUT_GTR).then_some("vp_aud_r8_08").or_else(|| crate::modcars::tune_donor(car))
}

/// The car whose driving-camera files (`tune/camera`: boom 1..3, POV, bumper, cinematic, replay) a
/// car uses: the R8's for the cut GT-R, which has none of the eight (user's decision, 2026-10-22).
pub fn camera_car(car: &str) -> &str {
    if car.eq_ignore_ascii_case(CUT_GTR) {
        return "vp_aud_r8_08";
    }
    // (a mod car: its manifest's `donors.cameras` - modcars.rs)
    crate::modcars::car(car).and_then(|c| c.cameras.as_deref()).unwrap_or(car)
}

/// The price list a car's garage uses when the disc has none (`<car>_parts.gtun`): the cut GT-R
/// takes the Audi RS4's, the disc's dearest list with the GT-R's own layout (five of each body
/// piece, an intercooler, a wide body, six seats / wheels / exhausts), with its car-made pieces
/// (brand 0: bumpers, skirts, lamps, hood, body kit) raised to the R8's level by the body kits'
/// ratio (R8 $30,600 / RS4 $13,030, rounded to $5) and the branded aftermarket parts at the
/// list's own prices. The R8's own list has three of each piece, which leaves the GT-R's
/// fourth and fifth without a row. The third value is the maker's logo the stock rows show
/// (Nissan's 52, as in the 350Z's and the Skyline's lists, for the RS4's Audi 35).
pub fn price_donor(car: &str) -> Option<(&'static str, &'static str, i32)> {
    car.eq_ignore_ascii_case(CUT_GTR).then_some(("vp_aud_rs4_07", "vp_aud_r8_08", 52))
}

/// Scales the car-made rows of the donor's list (see `price_donor`) to the level of `level`'s.
pub fn scale_price_list(
    rows: &std::collections::HashMap<String, Vec<(i32, i64, usize)>>,
    level: &std::collections::HashMap<String, Vec<(i32, i64, usize)>>,
    maker: i32,
) -> std::collections::HashMap<String, Vec<(i32, i64, usize)>> {
    scale_price_list_to(rows, body_kit_price(level), Some(level), Some(maker))
}

/// A car's part price list as the garage holds it: per section its rows (brand logo id, price, part index).
pub type PartList = std::collections::HashMap<String, Vec<(i32, i64, usize)>>;

/// The body-kit price of a part list (its `body` row of part index 1; 0 without one).
pub fn body_kit_price(l: &std::collections::HashMap<String, Vec<(i32, i64, usize)>>) -> i64 {
    l.get("body").and_then(|b| b.iter().find(|r| r.2 == 1)).map_or(0, |r| r.1)
}

/// [`scale_price_list`] to a body-kit price `to` (a mod car's `donors.prices_kit`, MODS.md S3): the scissor doors
/// from `level`'s list when one is given, the stock rows' logo `maker` (None: the donor's own).
pub fn scale_price_list_to(
    rows: &std::collections::HashMap<String, Vec<(i32, i64, usize)>>,
    to: i64,
    level: Option<&std::collections::HashMap<String, Vec<(i32, i64, usize)>>>,
    maker: Option<i32>,
) -> std::collections::HashMap<String, Vec<(i32, i64, usize)>> {
    // (the donor's maker: the logo of its stock rows)
    let own = rows.get("bumper_front").and_then(|l| l.first()).map_or(-1, |r| r.0);
    let maker = maker.unwrap_or(own);
    let from = body_kit_price(rows);
    rows.iter()
        .map(|(sec, list)| {
            // (the scissor doors are a fitted kit whose price each list sets itself)
            let kit = level.and_then(|level| level.get(sec.as_str())).filter(|_| sec == "scissor_door");
            let list: Vec<(i32, i64, usize)> = kit.cloned().unwrap_or_else(|| {
                list.iter()
                    .map(|&(brand, price, index)| (brand, if brand == 0 && from > 0 && to > 0 { ((price * to) as f64 / from as f64 / 5.0).round() as i64 * 5 } else { price }, index))
                    .collect()
            });
            let list = list.into_iter().map(|(brand, price, index)| (if brand == own { maker } else { brand }, price, index)).collect();
            (sec.clone(), list)
        })
        .collect()
}

/// The car whose sound profile a car uses: the 350Z's V6 for the cut GT-R (the disc's
/// `VPD_NSN_GTR_08_PROFILE` points at the R8's engine; the user chose the 350Z's).
pub fn sound_car(car: &str) -> &str {
    if car.eq_ignore_ascii_case(CUT_GTR) {
        return "vp_nsn_350z_06";
    }
    // (a mod car: its manifest's `donors.sound` - modcars.rs)
    crate::modcars::car(car).and_then(|c| c.sound.as_deref()).unwrap_or(car)
}

/// The name in a garage picture's file (`sh_`, `msh_`, rewardvehicles): the GT-R's are
/// drawn under its downloadable name `vpd_nsn_gtr_08` (its `mr_` picture is not).
pub fn picture_name(car: &str) -> String {
    if car.eq_ignore_ascii_case(CUT_GTR) { "vpd_nsn_gtr_08".into() } else { car.to_string() }
}

impl Catalog {
    pub fn load(data_dir: &str) -> Catalog {
        let dir = Path::new(data_dir).join("tune/vehicle");
        let mut cars = HashMap::new();
        for f in ["0xe94bde63", "0xccb5daee", "0x3cb91343"] {
            if let Ok(bytes) = std::fs::read(dir.join(f)) {
                parse_blocks(&String::from_utf8_lossy(&bytes), &mut cars);
            }
        }
        parse_blocks(CUT_CATALOG, &mut cars);
        // (the mod cars' rows, as the extra list `vehicle0003.lst` would hold them: modcars.rs)
        parse_blocks(crate::modcars::extra_list(), &mut cars);
        log::info!("catalog: {} vehicles", cars.len());
        Catalog { cars }
    }

    /// Catalogue entry for a vehicle directory name.
    pub fn get(&self, dir_name: &str) -> Option<&CarInfo> {
        self.cars.get(&dir_name.to_ascii_lowercase())
    }

    pub fn len(&self) -> usize {
        self.cars.len()
    }
}

#[cfg(test)]
mod tests {
    /// The cut GT-R reads the R8's driving-camera files; every other car its own.
    #[test]
    fn cut_gtr_drives_on_the_r8_cameras() {
        assert_eq!(super::camera_car(super::CUT_GTR), "vp_aud_r8_08");
        assert_eq!(super::camera_car("VP_NSN_GTR_08"), "vp_aud_r8_08");
        assert_eq!(super::camera_car("vp_aud_r8_08"), "vp_aud_r8_08");
        assert_eq!(super::camera_car("vp_nsn_350z_06"), "vp_nsn_350z_06");
    }

    /// The cut GT-R's garage lists every one of its pieces at a price (the disc has no list).
    #[test]
    fn cut_gtr_parts_have_prices() {
        let data = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        if !std::path::Path::new(&data).join("tune/garage").is_dir() {
            return;
        }
        let mut g = crate::game::Game::new(vec![std::path::PathBuf::from(&data).join("resources/vehicle").join(super::CUT_GTR)], 0);
        g.load_perf_prices(&data);
        let rows = |s: &str| g.part_entries_sec(0, s).cloned().unwrap_or_default();
        let bf = rows("bumper_front");
        assert_eq!(bf.iter().map(|r| r.1).collect::<Vec<_>>(), vec![0, 2265, 3310, 4425, 5390]);
        assert_eq!(rows("body")[1].1, 30600);
        assert_eq!(rows("hood").len(), 5);
        // (a branded part keeps its own price: the 625 wing)
        assert_eq!(rows("spoiler")[1], (56, 625, 1));
        assert_eq!(rows("scissor_door")[1].1, 11800);
        // (Nissan's logo on the stock rows)
        assert_eq!(rows("steer_whl")[0].0, 52);
    }
}
