//! The mod manifest `mod.toml` (format: docs/reference/mods/manifest.md).
//!
//! Only the syntax and the field-level rules live here; the checks against the package's files and the disc data
//! (docs/reference/mods/vehicle_inventory.md section 5) are in `validate.rs`.

use serde::Deserialize;
use std::cmp::Ordering;

/// The manifest file at the root of a package (or inside its one top folder).
pub const MANIFEST_NAME: &str = "mod.toml";

/// Manifest schema this build reads. A higher number is refused ("needs a newer Rusty MCLA").
pub const SCHEMA: i64 = 1;

/// Top-level keys this build knows; any other top-level key or table is kept out with a warning (later content
/// types - parts, paint / vinyl packages, tune files, strings - get their own tables without changing these).
const KNOWN_TOP: [&str; 3] = ["schema", "mod", "car"];

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Manifest {
    #[serde(default = "one")]
    pub schema: i64,
    #[serde(rename = "mod")]
    pub info: ModHeader,
    #[serde(default, rename = "car")]
    pub cars: Vec<CarEntry>,
    /// top-level keys this build does not know (not deserialized; filled by [`parse`])
    #[serde(skip)]
    pub unknown: Vec<String>,
}

fn one() -> i64 {
    1
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModHeader {
    /// identity of the mod: `[a-z0-9._-]`, 1..64 characters; same id = same mod (an update replaces it)
    pub id: String,
    /// dotted numbers, e.g. `1.0.2`
    pub version: String,
    /// shown in the launcher's Mods tab
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
}

/// One car of a mod (`[[car]]`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CarEntry {
    /// the car's identity everywhere: resource folder, vehicle-list `Name`, save TypeName, set-up name, string keys
    pub name: String,
    /// string key `<name>` (lists and menus), e.g. "2008 R8 (SAMPLE)"
    pub long_name: String,
    /// string key `<name>_shrt` (HUD, GPS)
    pub short_name: String,
    pub row: Row,
    #[serde(default)]
    pub donors: Donors,
    #[serde(default)]
    pub opponent: Option<Opponent>,
}

/// The `Vehicle { }` block of the vehicle list (vehicle_inventory.md 2.7).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// TUNER MUSCLE EXOTIC SPORTBIKE LUXURY COP SUV LOWRIDER MISC1 (any case)
    pub class: String,
    /// one of the `Manufacturer { Name }` blocks of `vehicle.lst`
    pub manufacturer: String,
    /// D0 C0 B0 A0
    pub group: String,
    /// D0 C0 B0 A0
    pub unlocking_group: String,
    /// showroom price, > 0
    pub price: i64,
    /// Hollywood Beaches Hills Downtown SouthCentral (the opponent pick's district mask)
    #[serde(default)]
    pub districts: Vec<String>,
    #[serde(default)]
    pub roof: u32,
    #[serde(default)]
    pub player_only: bool,
    #[serde(default)]
    pub transmission: Option<u32>,
    /// default: the car's own name (as on every disc car but 11 download cars)
    #[serde(default)]
    pub police_desc: Option<String>,
    /// the showroom bars: stock and fully upgraded
    #[serde(default)]
    pub figures: Option<Figures>,
    /// or: copy them from this disc car's row
    #[serde(default)]
    pub figures_from: Option<String>,
    /// `ShowroomAccel / Speed / Hndlg` (only the DUB editions and two police specials have them on the disc)
    #[serde(default)]
    pub showroom: Option<Showroom>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Figures {
    pub base_accel: f32,
    pub mods_accel: f32,
    pub base_speed: f32,
    pub mods_speed: f32,
    pub base_handling: f32,
    pub mods_handling: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Showroom {
    pub accel: f32,
    pub speed: f32,
    pub handling: f32,
}

/// Disc cars whose files stand in for what the mod does not bring (D21's pattern; DECISIONS.md D28 answer D).
#[derive(Clone, Debug, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Donors {
    /// `.r50` tuning (inventory row 4)
    pub tune: Option<String>,
    /// stock set-up `<car>.xcc` the loader builds the car's own from (row 5)
    pub setup: Option<String>,
    /// stock rim folder name, e.g. `whl_stk_aud_r8_08` (row 8)
    pub rims: Option<String>,
    /// brake model, index in the brakes list (row 9)
    pub brake: Option<u8>,
    /// `<car>.garage` camera lines (row 10)
    pub garage: Option<String>,
    /// `<car>_parts.gtun` part price list (row 11)
    pub prices: Option<String>,
    /// sound profile (row 16)
    pub sound: Option<String>,
    /// driving cameras (row 17)
    pub cameras: Option<String>,
    /// driver animation pack key (row 18)
    pub anim: Option<String>,
    /// garage / showroom pictures `mr_ sh_ msh_` + `rewardvehicles` (row 19)
    pub pictures: Option<String>,
}

/// Opponent use (MODS.md design 7b). Parsed and checked only for completeness here; wiring into the opponent pick is
/// stage S4.
#[derive(Clone, Debug, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Opponent {
    #[serde(default)]
    pub enabled: bool,
    /// district rows; default: the row's `districts`
    #[serde(default)]
    pub districts: Vec<String>,
    /// performance samples at mod 0 / 50 / 100
    #[serde(default)]
    pub performance: Option<[f32; 3]>,
    /// the samples' `speed` values at mod 0 / 50 / 100
    #[serde(default)]
    pub speed: Option<[f32; 3]>,
    /// or: copy both from this disc car's samples
    #[serde(default)]
    pub samples_from: Option<String>,
    /// a `Character` id of `Opponent.lst` (GH_nn); none = the first free driver (original rule)
    #[serde(default)]
    pub driver: Option<String>,
    /// the opponent's set-up name (`GH_nn_k_<car>`)
    #[serde(default)]
    pub setup: Option<String>,
    /// the disc set-up it is built from
    #[serde(default)]
    pub setup_from: Option<String>,
    /// plate text of that set-up
    #[serde(default)]
    pub plate: Option<String>,
}

/// Parse a manifest. Syntax and type errors come back as one message; unknown top-level keys are listed in
/// `unknown` (not an error).
pub fn parse(text: &str) -> Result<Manifest, String> {
    let text = text.trim_start_matches('\u{feff}');
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| format!("mod.toml is not valid TOML: {}", e.message()))?;
    let unknown: Vec<String> = table.keys().filter(|k| !KNOWN_TOP.contains(&k.as_str())).cloned().collect();
    let mut known = table;
    known.retain(|k, _| KNOWN_TOP.contains(&k));
    let mut m: Manifest = known.try_into().map_err(|e: toml::de::Error| format!("mod.toml: {}", e.message()))?;
    m.unknown = unknown;
    Ok(m)
}

/// A mod id: `[a-z0-9._-]`, 1..64 characters, not starting with a dot.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && !id.starts_with('.') && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

/// A version: dotted decimal numbers, 1..4 parts (`1`, `1.0`, `1.2.3`).
pub fn parse_version(v: &str) -> Option<Vec<u64>> {
    let parts: Vec<u64> = v.trim().split('.').map(|p| if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) { None } else { p.parse().ok() }).collect::<Option<_>>()?;
    (1..=4).contains(&parts.len()).then_some(parts)
}

/// Compare two versions (missing parts are 0: `1.0` == `1.0.0`). Unparsable versions compare as text.
pub fn cmp_version(a: &str, b: &str) -> Ordering {
    match (parse_version(a), parse_version(b)) {
        (Some(x), Some(y)) => {
            let n = x.len().max(y.len());
            (0..n).map(|i| x.get(i).copied().unwrap_or(0).cmp(&y.get(i).copied().unwrap_or(0))).find(|o| *o != Ordering::Equal).unwrap_or(Ordering::Equal)
        }
        _ => a.cmp(b),
    }
}

/// The save's TypeName field (+0x1FAD, 32 bytes up to ProfileName +0x1FCD, NUL-terminated) holds at most 31
/// characters. Car names are lower-case `[a-z0-9_]`, starting `vp_` or `vpd_` (the port's garage lists those folders).
pub const MAX_CAR_NAME: usize = 31;

pub fn car_name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("empty name".into());
    }
    if name.len() > MAX_CAR_NAME {
        return Some(format!("name longer than {MAX_CAR_NAME} characters (the save's TypeName field)"));
    }
    if !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
        return Some("name must be lower-case letters, digits and _".into());
    }
    if !(name.starts_with("vp_") || name.starts_with("vpd_")) {
        return Some("name must start with vp_ or vpd_".into());
    }
    None
}
