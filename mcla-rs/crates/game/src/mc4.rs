//! The original save's data that the port's own save fields do not cover: the block `"mc4"`
//! of the port's save, written by `tools/import_mc4_save.py` (`mc4_block`) by the original's
//! names (layouts: `work/save/decode/*/*.md`).
//!
//! Everything here is kept through a load / save of the port: every struct carries the keys
//! it does not name in `extra`. A save without the block gets one as soon as the game has
//! something of it to keep (`Game::mc4_mut`).
#![allow(non_snake_case)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub type Extra = BTreeMap<String, Value>;

/// A float of the decoders: a number, or the words they write for what JSON has no number
/// for ("nan", "inf", "-inf" - junk floats of records the game never set up).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct F(pub f64);

impl Serialize for F {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.0.is_nan() {
            s.serialize_str("nan")
        } else if self.0.is_infinite() {
            s.serialize_str(if self.0 > 0.0 { "inf" } else { "-inf" })
        } else {
            s.serialize_f64(self.0)
        }
    }
}

impl<'de> Deserialize<'de> for F {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<F, D::Error> {
        match Value::deserialize(d)? {
            Value::Number(n) => Ok(F(n.as_f64().unwrap_or(0.0))),
            Value::String(s) => Ok(F(match s.as_str() {
                "inf" => f64::INFINITY,
                "-inf" => f64::NEG_INFINITY,
                _ => f64::NAN,
            })),
            Value::Null => Ok(F(0.0)),
            v => Err(serde::de::Error::custom(format!("not a number: {v}"))),
        }
    }
}

fn fv(v: &[f64]) -> Vec<F> {
    v.iter().map(|x| F(*x)).collect()
}

fn rgb_of(v: &[F]) -> Option<[f32; 3]> {
    (v.len() >= 3 && v[..3].iter().all(|c| c.0.is_finite())).then(|| [v[0].0 as f32, v[1].0 as f32, v[2].0 as f32])
}

/// Six decimals, as the decoders write their floats.
fn r6(x: f32) -> F {
    F((x as f64 * 1e6).round() / 1e6)
}

fn yes() -> bool {
    true
}

// ---------------------------------------------------------------------------------------
// the block
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Mc4 {
    pub container_version: u32,
    pub options: Options,
    pub options_unknown: Vec<UnknownBytes>,
    pub enable_gps: bool,
    pub garage_current_slot: i32,
    pub garage: Vec<Car>,
    pub vehicle_records: Vec<VehicleRecord>,
    pub ability_levels: AbilityLevels,
    pub career: Career,
    pub editor: Editor,
    pub vinyl_packages: Vec<VinylPackage>,
    pub race_favorites_removed: Vec<Value>,
    pub rubberband_step_in_use: i32,
    pub achievements: Vec<String>,
    pub tutorials_shown: Vec<String>,
    pub stats_summary: Extra,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Mc4 {
    fn default() -> Self {
        Mc4 {
            // (the container version the retail game writes: 14; the patched one 15)
            container_version: 15,
            options: Options::default(),
            options_unknown: Vec::new(),
            // (enableGps: default 1, F-38943)
            enable_gps: true,
            garage_current_slot: 0,
            garage: Vec::new(),
            vehicle_records: Vec::new(),
            ability_levels: AbilityLevels::default(),
            career: Career::default(),
            editor: Editor::default(),
            vinyl_packages: Vec::new(),
            race_favorites_removed: Vec::new(),
            rubberband_step_in_use: 0,
            achievements: Vec::new(),
            tutorials_shown: Vec::new(),
            stats_summary: Extra::new(),
            extra: Extra::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UnknownBytes {
    pub off: u32,
    pub len: u32,
    pub hex: String,
    pub note: String,
    #[serde(flatten)]
    pub extra: Extra,
}

// ---------------------------------------------------------------------------------------
// options (LocalOptions 0..1339)
// ---------------------------------------------------------------------------------------

/// One slot of a custom controller mapping (`custom_mapping_pad` / `_wheel`): the action's
/// slot number (table 0x8204FF90), the device kind ("none", "button", "axis") and its key.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Mapping {
    pub slot: u32,
    pub device: String,
    pub key: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button_guess: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Options {
    /// place in the controller list of `tune/controller` (0 Throttle_Stick, 1 Throttle_Trigger,
    /// 2 / 3 the same with manual gears, 4 Custom_Pad, 5 Steering_Wheel, 6 Steering_Wheel_Manual,
    /// 7 Custom_Steering_Wheel, 8 Throttle_Stick_Hydraulics)
    pub control_config_pad: i32,
    pub control_config_pad_name: Option<String>,
    pub control_config_wheel: i32,
    pub control_config_wheel_name: Option<String>,
    pub custom_mapping_pad: BTreeMap<String, Mapping>,
    pub custom_mapping_wheel: BTreeMap<String, Mapping>,
    pub vibration: bool,
    /// 0 none, 1 idle camera + camera shake, 2 idle camera only, 3 camera shake only
    pub cinematics: u32,
    pub idle_camera: bool,
    pub camera_shake: bool,
    /// 0 compass, 1 top, 2 none
    pub gps_arrow: u32,
    pub gps_arrow_name: Option<String>,
    /// "Default Camera": 0 Bumper, 1 POV, 2 Boom 1, 3 Boom 2, 4 Boom 3
    pub camera: u32,
    pub camera_name: Option<String>,
    /// -1.0 .. 1.0 in 21 steps
    pub steering_sensitivity: F,
    pub convertible_top_down: bool,
    pub subtitles: bool,
    pub volume_effects: F,
    pub volume_music: F,
    pub volume_voice: F,
    pub volume_player_vehicle: F,
    pub volume_cutscene: F,
    pub online_character: u32,
    pub language: u32,
    pub language_name: Option<String>,
    /// the garage's LOWERING PATTERN (LocalOptions + 1324): 0 together, 1 front to back,
    /// 2 back to front, 3 left to right, 4 right to left (garage.ChangeAirridePattern)
    pub airride_pattern: u8,
    pub music_shuffle: bool,
    /// 0 ECLECTIC .. 6 WEST_RAP, 7 ALL, 8 ON_THE_FLY (the custom playlist)
    pub music_genre: u32,
    pub music_genre_name: Option<String>,
    pub time_of_day_hours: F,
    pub custom_playlist: Vec<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

pub const CONTROL_CONFIGS: [&str; 9] = ["Throttle_Stick", "Throttle_Trigger", "Throttle_Stick_Manual", "Throttle_Trigger_Manual", "Custom_Pad", "Steering_Wheel", "Steering_Wheel_Manual", "Custom_Steering_Wheel", "Throttle_Stick_Hydraulics"];
pub const GPS_ARROWS: [&str; 3] = ["option_arrow_compass", "option_arrow_top", "option_arrow_none"];
pub const CAMERAS: [&str; 5] = ["Bumper", "POV", "Boom 1", "Boom 2", "Boom 3"];
pub const LANGUAGES: [&str; 5] = ["english", "spanish", "french", "german", "italian"];
pub const MUSIC_GENRES: [&str; 9] = ["MUSIC_GENRE_ECLECTIC", "MUSIC_GENRE_ELECTRONIC", "MUSIC_GENRE_HARDROCK", "MUSIC_GENRE_HIPHOP", "MUSIC_GENRE_ROCK", "MUSIC_GENRE_TECHNO", "MUSIC_GENRE_WEST_RAP", "MUSIC_GENRE_ALL", "MUSIC_GENRE_ON_THE_FLY"];

impl Default for Options {
    /// The original's defaults (F-63809).
    fn default() -> Self {
        Options {
            control_config_pad: 1,
            control_config_pad_name: Some(CONTROL_CONFIGS[1].into()),
            control_config_wheel: 5,
            control_config_wheel_name: Some(CONTROL_CONFIGS[5].into()),
            custom_mapping_pad: BTreeMap::new(),
            custom_mapping_wheel: BTreeMap::new(),
            vibration: true,
            cinematics: 1,
            idle_camera: true,
            camera_shake: true,
            gps_arrow: 0,
            gps_arrow_name: Some(GPS_ARROWS[0].into()),
            camera: 4,
            camera_name: Some(CAMERAS[4].into()),
            steering_sensitivity: F(0.0),
            convertible_top_down: false,
            subtitles: false,
            volume_effects: F(1.0),
            volume_music: F(1.0),
            volume_voice: F(1.0),
            volume_player_vehicle: F(1.0),
            volume_cutscene: F(1.0),
            online_character: 0,
            language: 0,
            language_name: Some(LANGUAGES[0].into()),
            airride_pattern: 0,
            music_shuffle: true,
            music_genre: 7,
            music_genre_name: Some(MUSIC_GENRES[7].into()),
            time_of_day_hours: F(0.0),
            custom_playlist: Vec::new(),
            extra: Extra::new(),
        }
    }
}

impl Options {
    /// A block of the first importer named the pattern byte `airbags_on` (a switch).
    pub fn upgrade(&mut self) {
        if let Some(v) = self.extra.remove("airbags_on") {
            if self.airride_pattern == 0 {
                self.airride_pattern = v.as_bool().map_or(v.as_u64().unwrap_or(0) as u8, |b| b as u8);
            }
        }
    }

    /// The "cinematics" word from its two halves (mcCameraOptionsForm, F-15111).
    pub fn set_cinematics(&mut self, idle: bool, shake: bool) {
        self.idle_camera = idle;
        self.camera_shake = shake;
        self.cinematics = match (idle, shake) {
            (true, true) => 1,
            (true, false) => 2,
            (false, true) => 3,
            (false, false) => 0,
        };
    }

    pub fn set_camera(&mut self, k: u32) {
        self.camera = k.min(4);
        self.camera_name = Some(CAMERAS[self.camera as usize].into());
    }

    pub fn set_gps_arrow(&mut self, k: u32) {
        self.gps_arrow = k.min(2);
        self.gps_arrow_name = Some(GPS_ARROWS[self.gps_arrow as usize].into());
    }

    pub fn set_music_genre(&mut self, k: u32) {
        self.music_genre = k.min(8);
        self.music_genre_name = Some(MUSIC_GENRES[self.music_genre as usize].into());
    }

    /// The slider's place, -10..10 (F-78269: the value times 10).
    pub fn sensitivity_step(&self) -> i32 {
        ((self.steering_sensitivity.0 * 10.0).round() as i32).clamp(-10, 10)
    }

    pub fn set_sensitivity_step(&mut self, step: i32) {
        // (F-24198: the slider's place times 0.1)
        self.steering_sensitivity = F((step.clamp(-10, 10) as f64 * 0.1 * 1e6).round() / 1e6);
    }
}

// ---------------------------------------------------------------------------------------
// the garage: one vehicle record (CarCfg, 8176 bytes) per owned vehicle
// ---------------------------------------------------------------------------------------

pub const PERF_PARTS: [&str; 13] = ["exhaust", "headers", "intake", "forced_induction", "engine", "electronics", "nitrous", "clutch", "gear_kit", "shocks", "sway_bars", "brakes", "tires"];
pub const PAINT_FADE: [&str; 5] = ["None", "ColorShift", "FrontBack", "LeftRight", "TopBottom"];
pub const PAINT_TYPE: [&str; 6] = ["DirtyMatte", "Matte", "Gloss", "Metallic", "Pearl", "Sparkle"];
pub const PAINT_AREA: [&str; 6] = ["Body", "Doors", "Hood", "Front Bumper", "Rear Bumper", "Skirts"];
pub const RIM_MATERIAL: [&str; 4] = ["Painted", "Chrome", "Carbon Fiber", "Brushed Metal"];
pub const ABILITY: [&str; 4] = ["ZONE", "AGRO", "ROAR", "EMP"];
pub const GAUGE_TEXT: [&str; 5] = ["speedometer", "odometer", "trip", "gear select", "clock"];
pub const INTERIOR_GROUP: [&str; 10] = ["group 1", "group 2", "group 3", "group 4", "group 5", "group 6 (hidden from user)", "group 7 (hidden from user)", "group 8 (hidden from user)", "group 9 (hidden from user)", "group 10 (hidden from user)"];
pub const LAMP: [&str; 19] = ["brake light", "taillight", "turn signal", "headlight", "high beam", "side marker", "reverse light", "interior light", "fog light", "red reflector", "amber reflector", "clear reflector", "third brakelight", "red coplight", "blue coplight", "turn signal FL", "turn signal FR", "turn signal BL", "turn signal BR"];
pub const KLIGHT: [&str; 17] = ["kLightBrakelight", "kLightTaillight", "kLightThirdBrakelight", "kLightHeadlight", "kLightHiBeam", "kLightFogLight", "kLightTurnSignal", "kLightPolice0", "kLightPolice1", "kLightPolice2", "kLightInterior", "kLightReverseLight", "kLightNone", "kLightTurnSignalFL", "kLightTurnSignalFR", "kLightTurnSignalBL", "kLightTurnSignalBR"];
/// The 62 bytes of a record's part block (0x860..0x89D), by the game's own names.
pub const PARTS: [&str; 62] = [
    "conv_top", "conv_topup", "hood", "bumper_r", "trunk_swap", "intercooler", "bumper_f", "fenders", "skirts", "spoiler", "spoiler_b", "headlight", "taillight", "taillight_b", "frontgrill", "wheeliebar", "blower", "bike_tail",
    "bike_tank", "bike_cowl", "door_l", "door_r", "conv_topdown", "exh_pipe", "exh_side", "armfl", "armfr", "armrl", "armrr", "shockfl", "shockfr", "shockrl", "shockrr", "guiderl", "guiderr", "axle", "subbumf", "subbumr", "widebody",
    "spoiler_wide", "hood_wide", "door_l_wide", "door_r_wide", "trunk_wide", "interior", "seats_f", "seats_p", "door_l_int", "door_r_int", "stereo", "stereo_l", "stereo_r", "steer_whl", "post_gauge", "occluder_", "RiderIdx", "CarbonFiberHood",
    "TaillightMaterialIdx", "ScissorDoors", "WideBodyKit", "SideExhaustInUse", "UseIntercooler",
];
/// `tires` and `brakes` of tune/vehicle/wheel_models.list (what TireTexIdx / BrakeMdlIdx index).
pub const TIRES: [&str; 16] = ["tir_car_bfg_kdw_blk", "tir_car_bfg_rta_blk", "tir_car_bfg_rta_blkrl", "tir_car_bfg_rta_ol", "tir_car_bfg_rta_wht", "tir_car_bfg_rta_whtrl", "tir_car_bfg_rta_yel", "tir_car_pir_p0_blk", "tir_car_pir_p0_corsa", "tir_car_pir_p0_nero", "tir_car_pir_p0_rosso", "tir_car_bfg_rta_whtw", "tir_streetbike_tst", "tir_bike_pir_corsa3", "tir_bike_pir_diablo_rosso", "tir_bike_pir_diablo_supercorsa"];
pub const BRAKES: [&str; 15] = ["brk_car_base", "brk_car_brembo_drilled", "brk_car_brembo_zinc", "brk_car_projmu_slotted", "brk_car_projmu_slotdrilled", "brk_stk_duc_999r_06", "brk_stk_duc_monster_s4r_07", "brk_stk_kaw_ninja_zx14_06", "brk_stk_am_vantage_07", "brk_car_brembo_base", "brk_car_brembo_base_blank", "brk_car_brembo_drilled_blank", "brk_car_brembo_slotted_blank", "brk_car_brembo_drilled_lrg_blank", "brk_stk_aud_r8_08"];
/// The four ability systems of the career's tables, in the order of LocalOptions' levels
/// (Zone, Agro, Roar, Pulse; "Pulse" is the EMP).
pub const ABILITY_SYSTEMS: [&str; 4] = ["Zone", "Agro", "Roar", "Pulse"];

/// The game's vehicle table (a record's `vehicle_index`, the sub id of the `v..` statistics).
pub const VEHICLES: [&str; 69] = [
    "vp_vw_golf_83", "vp_nsn_240sx_98", "vp_dsn_280z_75", "vp_vw_scirocco_88", "vp_chv_camaross_69", "vp_frd_boss302_69", "vp_pon_transam_79", "vp_chv_impala_96", "vp_buk_grandnational_87", "vp_dge_challenger_70", "vp_frd_focus_04",
    "vp_pon_solstice_06", "vp_mit_eclipse_99", "vp_chv_cobalt_05", "vp_mzd_rx8_06", "vp_duc_monster_s4r_07", "vp_frd_mustang_06", "vp_chv_camaro_06", "vp_chv_camaro_06_dub", "vp_dge_charger_06", "vp_dge_challenger_06",
    "vp_dge_challenger_06_dub", "vp_cry_300c_06", "vp_vw_golf_07", "vp_nsn_350z_06", "vp_mit_3000gt_99", "vp_mzd_rx7_95", "vp_nsn_skyline_99", "vp_aud_rs4_07", "vp_mit_evo_06", "vp_duc_999r_06", "vp_mbz_s600_07", "vp_mbz_s600_07_dub",
    "vp_lam_miura_71", "vp_aud_r8_08", "vp_am_vantage_07", "vp_mbz_sl65_07", "vp_chv_corvettez06_07", "vp_sln_s281_06", "vp_lam_gallardo_06", "vp_lam_murcielago_06", "vp_lam_murcielago_06_dub", "vp_frd_cobra_04", "vp_frd_gt_06",
    "vp_sln_s7_06", "vp_kaw_ninja_zx14_06", "vp_chv_police_96", "vp_citybus_07", "vpd_chv_belair_55", "vpd_buk_riviera_63", "vpd_chv_impala_64", "vpd_chv_malibu_65", "vpd_pon_gto_66", "vpd_chv_chevelle_70", "vpd_lr_hse_08",
    "vpd_lr_sport_08", "vpd_mit_eclipse_09", "vpd_aud_s5_08", "vpd_mit_evox_08", "vpd_cad_xlrv_08", "vpd_mbz_clk63amg_08", "vpd_mbz_cl65amg_08", "vpd_am_db9_08", "vpd_duc_1098_07", "vp_se_chv_plc_96", "vp_se_chv_chp_96", "vp_se_dge_plc_06",
    "vp_se_dge_ubp_06", "vp_se_dge_chp_06",
];

/// A vehicle's table index: its place in `VEHICLES`, a mod car's after them (69 + its place in name order,
/// `modcars`; MODS.md M-5), else -1.
pub fn vehicle_index(name: &str) -> i32 {
    let n = name.to_ascii_lowercase();
    VEHICLES.iter().position(|v| *v == n).map(|k| k as i32).or_else(|| crate::modcars::table_index(&n).map(|k| k as i32)).unwrap_or(-1)
}

/// The game's string hash (F-58929): lower case, one-at-a-time.
pub fn joaat(s: &str) -> u32 {
    let mut h = 0u32;
    for c in s.bytes() {
        h = h.wrapping_add(c.to_ascii_lowercase() as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h.wrapping_add(h << 15)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Performance {
    /// stage 0..2 of the thirteen performance parts (`PERF_PARTS`)
    pub stages: BTreeMap<String, u8>,
    /// HydraulicsState0: air bags (air ride) fitted
    pub airbags_fitted: u8,
    /// HydraulicsState1: hydraulics fitted
    pub hydraulics_fitted: u8,
    pub brand: BTreeMap<String, u8>,
    pub unused_56: String,
    pub upgrade_percent: i32,
    pub upgrade_percent_override: i32,
    pub unk_percent_61: Vec<i32>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RimMaterial {
    pub A: Value,
    pub B: Value,
    pub C: Value,
    pub D: Value,
    pub E: Value,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GaugeText {
    pub element: String,
    pub m_fColor: Vec<F>,
    pub m_fSkew: F,
    pub m_iFontIdx: i32,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Light {
    pub lamp: String,
    pub LightBulbColor: Vec<F>,
    pub LightCoverColor: Vec<F>,
    /// the kLight name, or its number when the table has none
    pub kind: Value,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BaseColor {
    pub group: String,
    pub rgb: Vec<F>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One PAINT AREA of a vehicle (Body, Doors, Hood, Front Bumper, Rear Bumper, Skirts).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PaintArea {
    pub area: String,
    /// primary colour, RGB as on screen
    pub ColorA: Vec<F>,
    /// secondary colour (used with a fade)
    pub ColorB: Vec<F>,
    /// a name of `PAINT_TYPE`, or the number when out of the table
    pub PaintType: Value,
    /// a name of `PAINT_FADE`, or the number
    pub PaintFade: Value,
    pub unpainted_flag: u8,
    #[serde(flatten)]
    pub extra: Extra,
}

fn enum_of(names: &[&str], v: &Value) -> Option<usize> {
    match v {
        Value::String(s) => names.iter().position(|n| n == s),
        Value::Number(n) => n.as_u64().map(|x| x as usize).filter(|x| *x < names.len()),
        _ => None,
    }
}

impl PaintArea {
    /// 0 DirtyMatte, 1 Matte, 2 Gloss, 3 Metallic, 4 Pearl, 5 Sparkle
    pub fn paint_type(&self) -> Option<u8> {
        enum_of(&PAINT_TYPE, &self.PaintType).map(|k| k as u8)
    }

    /// 0 None, 1 ColorShift, 2 FrontBack, 3 LeftRight, 4 TopBottom
    pub fn fade(&self) -> Option<u8> {
        enum_of(&PAINT_FADE, &self.PaintFade).map(|k| k as u8)
    }

    pub fn set_paint_type(&mut self, k: u8) {
        self.PaintType = Value::from(PAINT_TYPE[(k as usize).min(5)]);
    }

    pub fn set_fade(&mut self, k: u8) {
        self.PaintFade = Value::from(PAINT_FADE[(k as usize).min(4)]);
    }

    pub fn color_a(&self) -> Option<[f32; 3]> {
        rgb_of(&self.ColorA)
    }

    pub fn color_b(&self) -> Option<[f32; 3]> {
        rgb_of(&self.ColorB)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Visual {
    pub m_vInteriorCharacterNeonLightFalloff: F,
    pub m_vInteriorCharacterGaugeConeAng: F,
    pub m_vInteriorCharacterGaugeConeFalloff: F,
    /// exterior neon colour (colour slot 20), RGB as on screen
    pub ExtNeonColor: Vec<F>,
    pub IntNeonColor: Vec<F>,
    pub DashGaugeColor: Vec<F>,
    pub DashGaugeColorB: Vec<F>,
    pub RimColorA: Vec<F>,
    pub RimColorB: Vec<F>,
    pub RimColorC: Vec<F>,
    pub RimColorD: Vec<F>,
    pub RimColorE: Vec<F>,
    pub m_vBrushedMetalColorA: Vec<F>,
    pub m_vBrushedMetalColorB: Vec<F>,
    pub m_vBrushedMetalColorC: Vec<F>,
    pub m_vBrakeCaliperColor: Vec<F>,
    pub m_vHeadlightGlassColorNew: Vec<F>,
    pub m_vTaillightGlassColorNew: Vec<F>,
    pub m_vBumpedHeadlightColor: Vec<F>,
    pub m_vBumpedTaillightColor: Vec<F>,
    /// window tint: rgb the tint's colour, a its darkness (the tint menu's row)
    pub m_vWindowTintColorNew: Vec<F>,
    /// exterior neon on / off: 0.0 or 1.0 (F-16643)
    pub ExtNeonIntensity: F,
    pub IntNeonIntensity: F,
    pub RimMaterial: RimMaterial,
    pub ExtNeonFlashRate: u8,
    pub IntNeonFlashRate: u8,
    pub gauge_text: Vec<GaugeText>,
    pub lights: Vec<Light>,
    /// interior material colours: "group 1" .. "group 5" the user's, 6..10 hidden
    pub m_vBaseColor: Vec<BaseColor>,
    pub paint: Vec<PaintArea>,
    pub m_vInteriorCharacterNeonLightPos: Vec<F>,
    pub m_vInteriorCharacterGaugeLightPos: Vec<F>,
    pub m_vInteriorCharacterGaugeConeDir: Vec<F>,
    pub Camber: Vec<F>,
    pub TireOffset: Vec<u8>,
    pub RideHeight: Vec<u8>,
    pub TireWidth: Vec<u8>,
    pub RimSize: Vec<u8>,
    pub TireProfile: Vec<u8>,
    pub RimMdlIdx: u8,
    pub rim_model: Option<String>,
    pub TireTexIdx: u8,
    pub tire_model: Option<String>,
    /// place in `brakes` of wheel_models.list (`BRAKES`)
    pub BrakeMdlIdx: u8,
    pub brake_model: Option<String>,
    pub plate_letters: String,
    pub plate_letters_raw: Vec<u8>,
    /// plate background
    pub LicenseBG: u8,
    pub m_iCarPaintOverride: i32,
    /// NOT preset ids (the decode note's name): the vinyl generator's pending redraw, set
    /// and cleared again by the garage while it works - zone and layer to draw anew
    /// (F-93216: `F-77238(zone * 64 + layer)`, redraw, `F-68814` = -1, -1;
    /// or `F-90191(zone)` for every layer of a zone, then `F-37891` = -1). The
    /// renderer hands a changed pair to the generator and takes -1, -1 as "finished"
    /// (F-93469 -> F-79014 / F-19017). At rest - in every save and set-up
    /// file - both are -1. The keys keep their first names.
    pub vinyl_preset_group: i32,
    pub vinyl_preset_index: i32,
    /// one byte per part kind (`PARTS`): the part's number in the car's part file
    pub parts: BTreeMap<String, u8>,
    /// the redraw's "whole zone" flag (1 after `F-90191`, 0 after `F-77238`): what
    /// the garage's last redraw left behind; read only together with a pending redraw
    pub vinyl_is_user_slot: u8,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Derived {
    pub neon_exterior_on: bool,
    pub neon_interior_on: bool,
    pub window_tint_level: Option<u8>,
    pub origin: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// What a stored car keeps of its damage (LocalOptions + 0x3DCB4, 6 floats a slot): the
/// damage fraction and four values of the car's damage object.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Damage {
    pub damage: Option<F>,
    pub zones: Option<Vec<F>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Car {
    pub slot: u32,
    pub empty: bool,
    pub TypeName: String,
    pub ProfileName: String,
    pub vehicle_index: i32,
    pub vehicle_table_name: Option<String>,
    pub performance: Performance,
    pub visual: Visual,
    pub unk_8096: u32,
    pub in_garage: u32,
    pub source_config_hash: String,
    /// the set-up file the record was made from: the car's own name (bought), `starter_<car>`,
    /// `SC_pinkslip_<car>`, `SC_timetrial_<car>`, `SC_tournament_<car>` ...
    pub source_config: Option<String>,
    /// "ZONE" / "AGRO" / "ROAR" / "EMP", or -1 (none)
    pub ability: Value,
    pub derived: Derived,
    pub current: bool,
    /// the vehicle's folder name, lower case: the key of the port's per-car maps
    pub vehicle: String,
    pub damage: Damage,
    pub name_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vinyl_layers_flagged: Option<Vec<Value>>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// The tint menu's ten rows: darkness a = 0.372 + 0.528 * k (F-96638 at A-34580).
pub const TINT_K: [f64; 10] = [0.002, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];

pub fn tint_alpha(row: u8) -> f64 {
    ((0.372 + 0.528 * TINT_K[(row as usize).min(9)]) * 1e6).round() / 1e6
}

pub fn tint_row_of(alpha: f64) -> u8 {
    (((alpha - 0.372) / 0.528 * 10.0).round() as i64).clamp(0, 9) as u8
}

impl Car {
    pub fn hydraulics(&self) -> bool {
        self.performance.hydraulics_fitted != 0
    }

    pub fn airbags(&self) -> bool {
        self.performance.airbags_fitted != 0
    }

    /// The tint menu's row, 0..9.
    pub fn tint_row(&self) -> u8 {
        self.visual.m_vWindowTintColorNew.get(3).map_or(0, |a| if a.0.is_finite() { tint_row_of(a.0) } else { 0 })
    }

    pub fn tint_rgb(&self) -> Option<[f32; 3]> {
        rgb_of(&self.visual.m_vWindowTintColorNew)
    }

    /// The tint's darkness as the record holds it.
    pub fn tint_alpha(&self) -> f32 {
        self.visual.m_vWindowTintColorNew.get(3).map_or(0.372, |a| a.0 as f32)
    }

    pub fn set_tint(&mut self, row: u8, rgb: Option<[f32; 3]>) {
        let t = &mut self.visual.m_vWindowTintColorNew;
        if t.len() < 4 {
            // (the constructor's glass: F-35798)
            *t = fv(&[0.0, 0.0035, 0.005, 0.372]);
        }
        if tint_row_of(t[3].0) != row.min(9) {
            t[3] = F(tint_alpha(row));
        }
        if let Some(c) = rgb {
            for k in 0..3 {
                t[k] = r6(c[k]);
            }
        }
        self.derived.window_tint_level = Some(row.min(9));
    }

    pub fn neon_ext(&self) -> (bool, Option<[f32; 3]>) {
        (self.visual.ExtNeonIntensity.0 > 0.0, rgb_of(&self.visual.ExtNeonColor))
    }

    pub fn neon_int(&self) -> bool {
        self.visual.IntNeonIntensity.0 > 0.0
    }

    /// (F-16643 toggles the intensity between 0.0 and 1.0)
    pub fn set_neon_ext(&mut self, on: bool, rgb: Option<[f32; 3]>) {
        if (self.visual.ExtNeonIntensity.0 > 0.0) != on {
            self.visual.ExtNeonIntensity = F(if on { 1.0 } else { 0.0 });
        }
        if let Some(c) = rgb {
            self.visual.ExtNeonColor = vec![r6(c[0]), r6(c[1]), r6(c[2])];
        }
        self.derived.neon_exterior_on = on;
    }

    pub fn set_neon_int(&mut self, on: bool) {
        if (self.visual.IntNeonIntensity.0 > 0.0) != on {
            self.visual.IntNeonIntensity = F(if on { 1.0 } else { 0.0 });
        }
        self.derived.neon_interior_on = on;
    }

    /// 0 ZONE, 1 AGRO, 2 ROAR, 3 EMP; none = -1 in the record.
    pub fn ability_idx(&self) -> Option<usize> {
        enum_of(&ABILITY, &self.ability)
    }

    pub fn set_ability(&mut self, k: Option<usize>) {
        self.ability = match k {
            Some(k) if k < 4 => Value::from(ABILITY[k]),
            _ => Value::from(-1),
        };
    }

    /// A rim part's material (0 spoke A, 1 spoke B, 2 lip): 0 Painted, 1 Chrome,
    /// 2 Carbon Fiber, 3 Brushed Metal.
    pub fn rim_material(&self, k: usize) -> Option<u8> {
        let m = &self.visual.RimMaterial;
        enum_of(&RIM_MATERIAL, [&m.A, &m.B, &m.C, &m.D, &m.E][k.min(4)]).map(|x| x as u8)
    }

    pub fn set_rim_material(&mut self, k: usize, mat: u8) {
        let m = &mut self.visual.RimMaterial;
        let v = Value::from(RIM_MATERIAL[(mat as usize).min(3)]);
        match k {
            0 => m.A = v,
            1 => m.B = v,
            2 => m.C = v,
            3 => m.D = v,
            _ => m.E = v,
        }
    }

    pub fn part(&self, name: &str) -> u8 {
        self.visual.parts.get(name).copied().unwrap_or(0)
    }

    /// The origin of the car by its set-up file's name: "stock" (bought), "starter",
    /// "SC_pinkslip", "SC_timetrial", "SC_tournament", "pinkslip", "Delivery" ...
    pub fn origin(&self) -> String {
        match &self.source_config {
            Some(s) if !s.eq_ignore_ascii_case(&self.TypeName) && s.len() > self.TypeName.len() + 1 => s[..s.len() - self.TypeName.len() - 1].to_string(),
            _ => "stock".to_string(),
        }
    }

    /// A record as the game makes one from a set-up file (`resources/vehicle/configs/<name>.xcc`,
    /// 8176 bytes, the garage record's own layout; loader F-86225): what a vehicle that
    /// comes into the garage starts from. `config` is the file's name without the extension.
    pub fn from_xcc(b: &[u8], vehicle: &str, config: &str, slot: u32) -> Option<Car> {
        if b.len() != 8176 {
            return None;
        }
        let f = |o: usize| -> F {
            let x = f32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
            if x.is_finite() {
                r6(x)
            } else {
                F(x as f64)
            }
        };
        let u = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let v3 = |o: usize| vec![f(o), f(o + 4), f(o + 8)];
        let v4 = |o: usize| vec![f(o), f(o + 4), f(o + 8), f(o + 12)];
        let cstr = |o: usize, n: usize| -> String {
            let raw = &b[o..o + n];
            let end = raw.iter().position(|c| *c == 0).unwrap_or(n);
            raw[..end].iter().map(|c| *c as char).collect()
        };
        let named = |names: &[&str], v: u32| -> Value {
            names.get(v as usize).map_or(Value::from(v), |n| Value::from(*n))
        };
        let type_name = cstr(8109, 32);
        let vehicle = vehicle.to_ascii_lowercase();
        let vehicle_index = vehicle_index(&vehicle);
        let performance = Performance {
            stages: PERF_PARTS.iter().enumerate().map(|(k, n)| (n.to_string(), b[28 + k])).collect(),
            airbags_fitted: b[41],
            hydraulics_fitted: b[42],
            brand: PERF_PARTS.iter().enumerate().map(|(k, n)| (n.to_string(), b[43 + k])).collect(),
            unused_56: b[56..59].iter().map(|x| format!("{x:02x}")).collect(),
            upgrade_percent: b[59] as i8 as i32,
            upgrade_percent_override: b[60] as i8 as i32,
            unk_percent_61: b[61..64].iter().map(|x| *x as i32).collect(),
            extra: Extra::new(),
        };
        let mats: Vec<Value> = (0..5).map(|k| named(&RIM_MATERIAL, u(0x178 + 4 * k))).collect();
        let (rim, tire, brake) = (b[0x844], b[0x845], b[0x846]);
        let letters = &b[0x847..0x84e];
        let plate: String = letters
            .iter()
            .map(|c| match *c {
                0x5B => "[sign1]".to_string(),
                0x5C => "[sign2]".to_string(),
                0x5D => "[sign3]".to_string(),
                0x5E => "[sign4]".to_string(),
                c if (32..127).contains(&c) => (c as char).to_string(),
                c => format!("\\x{c:02x}"),
            })
            .collect();
        let visual = Visual {
            m_vInteriorCharacterNeonLightFalloff: f(68),
            m_vInteriorCharacterGaugeConeAng: f(72),
            m_vInteriorCharacterGaugeConeFalloff: f(76),
            ExtNeonColor: v3(0x50),
            IntNeonColor: v3(0x60),
            DashGaugeColor: v3(0x70),
            DashGaugeColorB: v3(0x80),
            RimColorA: v3(0x90),
            RimColorB: v3(0xa0),
            RimColorC: v3(0xb0),
            RimColorD: v3(0xc0),
            RimColorE: v3(0xd0),
            m_vBrushedMetalColorA: v3(0xe0),
            m_vBrushedMetalColorB: v3(0xf0),
            m_vBrushedMetalColorC: v3(0x100),
            m_vBrakeCaliperColor: v3(0x110),
            m_vHeadlightGlassColorNew: v4(0x120),
            m_vTaillightGlassColorNew: v4(0x130),
            m_vBumpedHeadlightColor: v4(0x140),
            m_vBumpedTaillightColor: v4(0x150),
            m_vWindowTintColorNew: v4(0x160),
            ExtNeonIntensity: f(0x170),
            IntNeonIntensity: f(0x174),
            RimMaterial: RimMaterial { A: mats[0].clone(), B: mats[1].clone(), C: mats[2].clone(), D: mats[3].clone(), E: mats[4].clone(), extra: Extra::new() },
            ExtNeonFlashRate: b[0x18c],
            IntNeonFlashRate: b[0x18d],
            gauge_text: (0..5)
                .map(|k| {
                    let o = 0x190 + 0x20 * k;
                    GaugeText { element: GAUGE_TEXT[k].into(), m_fColor: v4(o), m_fSkew: f(o + 16), m_iFontIdx: u(o + 20) as i32, extra: Extra::new() }
                })
                .collect(),
            lights: (0..19)
                .map(|k| {
                    let o = 0x230 + 0x30 * k;
                    Light { lamp: LAMP[k].into(), LightBulbColor: v3(o), LightCoverColor: v3(o + 16), kind: named(&KLIGHT, u(o + 32)), extra: Extra::new() }
                })
                .collect(),
            m_vBaseColor: (0..10).map(|k| BaseColor { group: INTERIOR_GROUP[k].into(), rgb: v3(0x5c0 + 16 * k), extra: Extra::new() }).collect(),
            paint: (0..6)
                .map(|k| {
                    let o = 0x660 + 0x30 * k;
                    PaintArea { area: PAINT_AREA[k].into(), ColorA: v3(o), ColorB: v3(o + 16), PaintType: named(&PAINT_TYPE, u(o + 32)), PaintFade: named(&PAINT_FADE, u(o + 36)), unpainted_flag: b[o + 40], extra: Extra::new() }
                })
                .collect(),
            m_vInteriorCharacterNeonLightPos: v3(0x800),
            m_vInteriorCharacterGaugeLightPos: v3(0x810),
            m_vInteriorCharacterGaugeConeDir: v3(0x820),
            Camber: vec![f(0x830), f(0x834)],
            TireOffset: vec![b[0x838], b[0x839]],
            RideHeight: vec![b[0x83a], b[0x83b]],
            TireWidth: vec![b[0x83c], b[0x83d]],
            RimSize: vec![b[0x83e], b[0x83f]],
            TireProfile: vec![b[0x840], b[0x841]],
            RimMdlIdx: rim,
            rim_model: None,
            TireTexIdx: tire,
            tire_model: TIRES.get(tire as usize).map(|s| s.to_string()),
            BrakeMdlIdx: brake,
            brake_model: BRAKES.get(brake as usize).map(|s| s.to_string()),
            plate_letters: plate,
            plate_letters_raw: letters.to_vec(),
            LicenseBG: b[0x84f],
            m_iCarPaintOverride: u(0x854) as i32,
            vinyl_preset_group: u(0x858) as i32,
            vinyl_preset_index: u(0x85c) as i32,
            parts: PARTS.iter().enumerate().map(|(k, n)| (n.to_string(), b[0x860 + k])).collect(),
            vinyl_is_user_slot: b[8068],
            extra: Extra::new(),
        };
        let ab = b[8108] as i8;
        let mut car = Car {
            slot,
            empty: false,
            // (the file's own spelling of the folder name; the port's key is `vehicle`)
            TypeName: if type_name.is_empty() || type_name == "blank" { vehicle.clone() } else { type_name },
            ProfileName: cstr(8141, 34),
            vehicle_index,
            vehicle_table_name: (vehicle_index >= 0).then(|| vehicle.clone()),
            performance,
            visual,
            unk_8096: u(8096),
            // (a garage car: its edits go back to the slot - F-31112)
            in_garage: 1,
            // (F-86225: the hash of the set-up file's name)
            source_config_hash: format!("0x{:08x}", joaat(config)),
            source_config: Some(config.to_string()),
            ability: Value::Null,
            derived: Derived::default(),
            current: false,
            vehicle: vehicle.clone(),
            // (a car that is added has its damage zeroed: F-91320)
            damage: Damage { damage: Some(F(0.0)), zones: Some(fv(&[0.0, 0.0, 0.0, 0.0])), extra: Extra::new() },
            name_hash: Some(format!("0x{:08x}", joaat(&vehicle))),
            vinyl_layers_flagged: None,
            extra: Extra::new(),
        };
        car.set_ability(if (0..4).contains(&ab) { Some(ab as usize) } else { None });
        car.derived = Derived {
            neon_exterior_on: car.neon_ext().0,
            neon_interior_on: car.neon_int(),
            window_tint_level: Some(car.tint_row()),
            origin: Some(car.origin()),
            extra: Extra::new(),
        };
        Some(car)
    }
}

// ---------------------------------------------------------------------------------------
// the per-model purchase records (LocalOptions + 0x3C370, 84 bytes a model)
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VehicleRecord {
    pub index: i32,
    pub in_use: bool,
    pub hash: String,
    pub vehicle: Option<String>,
    pub refund_value: i32,
    /// the prices of the parts bought for this model, summed in 16 bits, plus 1
    pub spent_plus_1: u32,
    /// owned part numbers by category (`PART_CATEGORIES`: the sections of the part files)
    pub owned_parts: BTreeMap<String, Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vtable_word: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// The categories of a purchase record's bit sets (number, name): the number is what the
/// garage's menu section gives (F-70916), the name the section of the vehicle's part
/// file that the same menu section lists (its part slot: F-92682) - 1 hood, 2 bumper_r,
/// 3 bumper_f, 5 skirts, 6 spoiler, 7 headlight, 8 taillight, 9 frontgrill, 11 blower,
/// 12 intercooler, 13..15 bike_tail / tank / cowl, 16 door_l (scissor doors), 18 widebody
/// ("body"), 24 seats_f, 25 stereo, 28 steer_whl, 29 post_gauge, 30 rims, 31 tires, 32 brakes,
/// 34 exh_pipe, 35..46 performance, 47 exh_side. 5 and 6 are the paint shop and the vinyl
/// editor, bought once per model for $1000 (`garage.FinishMiscPurchase(0 / 1)`, F-96638
/// at A-64047 / A-84400). No part file has a frontgrill or stereo section.
pub const PART_CATEGORIES: [(u8, &str); 27] = [
    (1, "intercooler"), (2, "bike_tail"), (3, "bike_tank"), (4, "bike_cowl"), (5, "paint"), (6, "vinyl"), (7, "tires"), (8, "rims"),
    (9, "brakes"), (10, "exhaust"), (11, "bumper_front"), (12, "bumper_rear"), (13, "spoiler"), (14, "hood"), (16, "blower"),
    (17, "skirts"), (18, "headlight"), (19, "taillight"), (21, "body"), (30, "frontgrill"), (35, "performance"), (43, "seats_f"),
    (44, "stereo"), (45, "steer_whl"), (46, "post_gauge"), (52, "side_exhaust"), (53, "scissor_door"),
];

/// The record category of one of the port's part tags' sections ("rim" is the port's word
/// for the rims' category; the others are the part files' own section names).
pub fn part_category(section: &str) -> Option<&'static str> {
    let section = if section == "rim" { "rims" } else { section };
    PART_CATEGORIES.iter().find(|(_, n)| *n == section).map(|(_, n)| *n)
}

impl VehicleRecord {
    /// A block of the first importer had numbers for most categories (`cat11`): by name.
    pub fn upgrade(&mut self) {
        for (n, name) in PART_CATEGORIES {
            if let Some(mut v) = self.owned_parts.remove(&format!("cat{n}")) {
                let have = self.owned_parts.entry(name.to_string()).or_default();
                have.append(&mut v);
                have.sort_unstable();
                have.dedup();
            }
        }
        // (the first decoder's guess at the performance bits, kept as a note)
        self.extra.remove("owned_performance_guess");
    }

    /// The port's tags ("<section>:<index>") for what the record owns. Part 0 of a body
    /// part's list is the stock part, which needs no tag; tire 0 and rim 0 are rows of
    /// their lists.
    pub fn owned_tags(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (cat, v) in &self.owned_parts {
            let sec = if cat == "rims" { "rim" } else { cat.as_str() };
            let zero = matches!(cat.as_str(), "rims" | "tires" | "performance" | "paint" | "vinyl");
            out.extend(v.iter().filter(|i| **i != 0 || zero).map(|i| format!("{sec}:{i}")));
        }
        out
    }

    /// The highest stage (0..2) of performance part `k` (0..12, the record's order) the
    /// model owns: F-32441 - bit k * 3 + 2, else bit k * 3 + 1.
    pub fn perf_owned(&self, k: usize) -> u8 {
        let k = k as u32 * 3;
        if self.owns("performance", k + 2) { 2 } else if self.owns("performance", k + 1) { 1 } else { 0 }
    }

    /// A model's record as the constructor leaves it (F-69059): nothing owned, the sum 1.
    pub fn new(vehicle: &str) -> VehicleRecord {
        let v = vehicle.to_ascii_lowercase();
        VehicleRecord { index: vehicle_index(&v), in_use: true, hash: format!("0x{:08x}", joaat(&v)), vehicle: Some(v), refund_value: 0, spent_plus_1: 1, ..Default::default() }
    }

    /// What was spent on parts for this model (0xCDCC + 1 = never set up, 0 likewise).
    pub fn spent(&self) -> u32 {
        match self.spent_plus_1 {
            0 | 0xCDCD => 0,
            n => n - 1,
        }
    }

    /// A part was bought (F-66800): its bit is set and its price joins the sum.
    pub fn buy(&mut self, category: &str, index: u32, price: u32) {
        let have = self.owned_parts.entry(category.to_string()).or_default();
        if have.contains(&index) {
            // (a bit that is set: nothing is added)
            return;
        }
        have.push(index);
        have.sort_unstable();
        self.add_spent(price);
    }

    /// The sum takes a price, as F-66800 does it: the 16-bit word plus the price, held
    /// at 0xFFFF; 0xCDCC (the debug fill, "never set up") stays what it is.
    pub fn add_spent(&mut self, price: u32) {
        let t = self.spent_plus_1 as i64 + price as i64 - 1;
        self.spent_plus_1 = if t == 0xCDCC {
            0xCDCC
        } else if t < 0xFFFF {
            ((t & 0xFFFF) as u32 + 1) & 0xFFFF
        } else {
            0xFFFF
        };
    }

    pub fn owns(&self, category: &str, index: u32) -> bool {
        self.owned_parts.get(category).map_or(false, |v| v.contains(&index))
    }

    /// The bits for the port's tags ("<section>:<index>"); a tag of no category is passed over.
    pub fn take_tags(&mut self, tags: &[String]) {
        for t in tags {
            let Some((sec, idx)) = t.split_once(':') else { continue };
            let (Some(cat), Ok(k)) = (part_category(sec), idx.parse::<u32>()) else { continue };
            let have = self.owned_parts.entry(cat.to_string()).or_default();
            if !have.contains(&k) {
                have.push(k);
                have.sort_unstable();
            }
        }
    }
}

/// What a vehicle fetches when sold or traded in (A-93343..A-99774, the same at the
/// showroom's trade A-22953): the purchase record's 16-bit sum word plus the vehicle's
/// price less one, times 0.8, cut to a whole number; nothing while the free-parts flag is up.
pub fn trade_in_value(spent_plus_1: u32, price: i64, free: bool) -> i64 {
    if free {
        return 0;
    }
    ((spent_plus_1 as i64 + price - 1) as f32 * 0.8) as i64
}

/// The garage's category numbers of the purchase records (F-70916's section -> the
/// category handed to F-66800 / F-10180) by name. `None`: not named.
pub fn category_name(cat: u32) -> Option<&'static str> {
    Some(match cat {
        7 => "tires",
        8 => "rims",
        9 => "brakes",
        13 => "spoiler",
        35 => "performance",
        _ => return None,
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AbilityLevels {
    pub zone: u8,
    pub agro: u8,
    pub roar: u8,
    pub pulse: u8,
    #[serde(flatten)]
    pub extra: Extra,
}

// ---------------------------------------------------------------------------------------
// the Career entry
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CareerFlags {
    pub raw: u32,
    pub career_startup_done: bool,
    pub police_unlocked: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Completions {
    pub Easy: u32,
    pub Medium: u32,
    pub Hard: u32,
    pub Impossible: u32,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Completions {
    pub fn get(&self, difficulty: usize) -> u32 {
        [self.Easy, self.Medium, self.Hard, self.Impossible][difficulty.min(3)]
    }

    pub fn bump(&mut self, difficulty: usize) {
        match difficulty.min(3) {
            0 => self.Easy += 1,
            1 => self.Medium += 1,
            2 => self.Hard += 1,
            _ => self.Impossible += 1,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Event {
    pub name: String,
    pub completions: Completions,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Threshold {
    pub id: String,
    pub reached: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct System {
    pub amount: i64,
    pub thresholds: Vec<Threshold>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl System {
    pub fn reached(&self, id: &str) -> Option<bool> {
        self.thresholds.iter().find(|t| t.id == id).map(|t| t.reached)
    }

    pub fn set_reached(&mut self, id: &str) {
        match self.thresholds.iter_mut().find(|t| t.id == id) {
            Some(t) => t.reached = true,
            None => self.thresholds.push(Threshold { id: id.to_string(), reached: true, extra: Extra::new() }),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Experience {
    /// `Progression` (the reputation) and `Unlocking`
    pub main: BTreeMap<String, System>,
    /// the nine vehicle classes' reputation
    pub classes: BTreeMap<String, System>,
    /// Zone, Agro, Roar, Pulse, Grip: the ability's use count
    pub abilities: BTreeMap<String, System>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Condition {
    /// Experience, String, Mission, Race, Tally, TallyRace, TallyMission, Stat
    #[serde(rename = "type")]
    pub kind: Value,
    pub met: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_amount: Option<i64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Requirement {
    pub hidden: bool,
    pub satisfied: bool,
    pub conditions: Vec<Condition>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Mission {
    pub name: String,
    pub index: i32,
    pub active_slot: Option<i32>,
    pub completed_slot: Option<i32>,
    pub state: String,
    pub state_id: u32,
    pub addon: bool,
    pub prerequisites: Vec<Requirement>,
    pub objectives: Vec<Requirement>,
    #[serde(flatten)]
    pub extra: Extra,
}

pub const MISSION_STATES: [&str; 6] = ["INACTIVE", "CHECKING_PREREQUISITES", "READY_TO_START", "CHECKING_OBJECTIVES", "READY_TO_COMPLETE", "COMPLETE"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CruisingSlots {
    pub district: String,
    pub district_locked: bool,
    pub slots_hidden: Vec<bool>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AltHookman {
    pub uid: u32,
    pub visible: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TimeTrials {
    pub district: String,
    pub unlocked: bool,
    pub target_ms: Vec<u32>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryRace {
    pub race_index: i64,
    pub race: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// The ring of the last twelve races (F-63023).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RaceHistory {
    pub oldest_first: Vec<HistoryRace>,
    pub write_index: u32,
    pub head: u32,
    pub count: u32,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PinkSlip {
    pub slot: u32,
    #[serde(rename = "type")]
    pub kind: i32,
    pub empty: bool,
    pub name: String,
    pub uid: i32,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PinkSlips {
    pub uid_counter: u32,
    pub slots: Vec<PinkSlip>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Script {
    pub name: String,
    pub stack_size: u32,
    pub saved_bytes: u32,
    pub saved_words: Vec<i64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Addon {
    pub present: u32,
    pub missions: Vec<String>,
    pub scripts: Vec<Script>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Career {
    pub flags: CareerFlags,
    pub events: Vec<Event>,
    pub experience: Experience,
    pub missions: Vec<Mission>,
    pub cruising_slots: Vec<CruisingSlots>,
    pub alt_hookmen: Vec<AltHookman>,
    pub tournaments_won: BTreeMap<String, bool>,
    /// D0..D4, C0..C4, B0..B4, A0..A4
    pub performance_levels_unlocked: BTreeMap<String, bool>,
    /// vehicle class -> the body part types the shop offers
    pub body_parts_unlocked: BTreeMap<String, Vec<String>>,
    /// vehicle class -> the part types that still wear the "new" marker
    pub body_parts_new: BTreeMap<String, Vec<String>>,
    pub abilities_unlocked: bool,
    pub races_unlocked: Vec<Value>,
    pub time_trials: Vec<TimeTrials>,
    pub race_history: RaceHistory,
    pub pink_slips: PinkSlips,
    pub scripts: Vec<Script>,
    pub addon: Addon,
    pub active_order: Vec<String>,
    pub completed_order: Vec<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

pub const PERF_LEVELS: [&str; 20] = ["D0", "D1", "D2", "D3", "D4", "C0", "C1", "C2", "C3", "C4", "B0", "B1", "B2", "B3", "B4", "A0", "A1", "A2", "A3", "A4"];
pub const DISTRICTS: [&str; 5] = ["Hollywood", "Beaches", "Hills", "Downtown", "SouthCentral"];

impl Career {
    pub fn event(&self, name: &str) -> Option<&Event> {
        self.events.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    pub fn event_mut(&mut self, name: &str) -> &mut Event {
        if let Some(k) = self.events.iter().position(|e| e.name.eq_ignore_ascii_case(name)) {
            return &mut self.events[k];
        }
        self.events.push(Event { name: name.to_string(), ..Default::default() });
        self.events.last_mut().unwrap()
    }

    pub fn set_police_unlocked(&mut self, on: bool) {
        self.flags.police_unlocked = on;
        self.flags.raw = self.flags.raw & !4 | if on { 4 } else { 0 };
    }

    pub fn set_startup_done(&mut self) {
        self.flags.career_startup_done = true;
        self.flags.raw |= 1;
    }

    /// A race joins the ring of the last twelve (F-63023). A race that is in it
    /// already is taken out first (F-98664: the later ones move down a place, the write
    /// index steps back), so a race is in the ring once; with twelve in it the oldest goes
    /// (the head steps on); then the race is written to the cell after the one written last.
    pub fn add_race_to_history(&mut self, race_index: i64, race: Option<String>) {
        let h = &mut self.race_history;
        if let Some(k) = h.oldest_first.iter().position(|r| r.race_index == race_index) {
            h.oldest_first.remove(k);
            h.write_index = (h.write_index + 11) % 12;
        }
        if h.oldest_first.len() >= 12 {
            h.oldest_first.remove(0);
            h.head = (h.head + 1) % 12;
        }
        h.oldest_first.push(HistoryRace { race_index, race, extra: Extra::new() });
        // (the oldest entry is at (head + 1) % 12 - the samples: two races, cells 1 and 2,
        // head 0; twelve, written last 8, head 8)
        h.write_index = (h.write_index + 1) % 12;
        h.count = h.oldest_first.len() as _;
    }

    /// Where a race stands in the ring (F-97924): 0 the oldest; None, not in it.
    pub fn history_position(&self, race_index: i64) -> Option<usize> {
        self.race_history.oldest_first.iter().position(|r| r.race_index == race_index)
    }

    /// The race of `races` (indices of the race table, in the order of the array) a hookman
    /// takes next (mcRaceArray_FindNextRaceIndex, F-18968): the first that is not in the
    /// history; with all of them in it, the one run longest ago.
    pub fn next_race_of(&self, races: &[i64]) -> Option<i64> {
        let mut best: Option<(usize, i64)> = None;
        for r in races {
            match self.history_position(*r) {
                None => return Some(*r),
                Some(p) => {
                    if best.map_or(true, |(b, _)| b > p) {
                        best = Some((p, *r));
                    }
                }
            }
        }
        best.map(|(_, r)| r)
    }

    /// A race is put on the list of unlocked races (F-81609): at the first free cell of
    /// the 250, unless it is on the list. True when it was added.
    pub fn unlock_race(&mut self, name: &str) -> bool {
        if name.is_empty() || self.races_unlocked.len() >= 250 || self.race_is_unlocked(name) {
            return false;
        }
        self.races_unlocked.push(Value::String(name.to_string()));
        true
    }

    /// The race is on the list (F-64698 -> F-56529; the list holds the hashes of
    /// the names in lower case).
    pub fn race_is_unlocked(&self, name: &str) -> bool {
        self.races_unlocked.iter().any(|r| r.as_str().map_or(false, |s| s.eq_ignore_ascii_case(name)))
    }
}

// ---------------------------------------------------------------------------------------
// the race editor, vinyl packages
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Checkpoint {
    pub pos: Vec<F>,
    /// metres (25 in saved races)
    pub activation_height: u32,
    pub activation_radius: u32,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EditorRace {
    /// the slot, 0..19 (`LosAngeles<slot>`)
    pub slot: u32,
    pub name: String,
    pub in_use: bool,
    pub count: u32,
    pub city: u32,
    pub grid_variant: i32,
    pub flag_1041: u32,
    pub count_word: u32,
    pub count_byte: u32,
    pub checkpoints: Vec<Checkpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count_from_word: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding_not_cd: Option<bool>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl EditorRace {
    pub fn points(&self) -> Vec<[f32; 3]> {
        self.checkpoints.iter().take(self.count as usize).filter(|c| c.pos.len() >= 3).map(|c| [c.pos[0].0 as f32, c.pos[1].0 as f32, c.pos[2].0 as f32]).collect()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Editor {
    pub races: Vec<EditorRace>,
    pub received: Vec<EditorRace>,
    pub received_cursor: u32,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VinylPackage {
    pub slot: u32,
    pub used: bool,
    /// the package's layers as the port's `vinyl::Livery` (kept as written: the importer's
    /// floats are the record's halves)
    pub livery: Value,
    #[serde(flatten)]
    pub extra: Extra,
}

// ---------------------------------------------------------------------------------------
// the block as a whole
// ---------------------------------------------------------------------------------------

impl Mc4 {
    /// A block for a game that has none: the original's defaults. `fresh_game`: a new
    /// career - else the block joins a career of the port's that is under way, where the
    /// police are about already (the port has them from the start).
    pub fn seeded(fresh_game: bool) -> Mc4 {
        let mut m = Mc4::default();
        m.extra.insert("made_by_port".into(), Value::Bool(true));
        if !fresh_game {
            m.career.set_police_unlocked(true);
        }
        m
    }

    /// The block was made by the port (not brought over from an original save): the state
    /// the original's start-up scripts set (unlocked races, cruising slots ...) is not in it.
    pub fn made_by_port(&self) -> bool {
        self.extra.get("made_by_port").and_then(|v| v.as_bool()).unwrap_or(false)
    }

    /// The block of a save's JSON. A block that does not fit the types is left to the caller
    /// to keep as it is (`Err`), so that nothing of it is lost.
    pub fn from_value(v: &Value) -> Result<Mc4, serde_json::Error> {
        Mc4::deserialize(v)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn car(&self, vehicle: &str) -> Option<&Car> {
        self.garage.iter().find(|c| c.vehicle.eq_ignore_ascii_case(vehicle))
    }

    pub fn car_mut(&mut self, vehicle: &str) -> Option<&mut Car> {
        self.garage.iter_mut().find(|c| c.vehicle.eq_ignore_ascii_case(vehicle))
    }

    pub fn record(&self, vehicle: &str) -> Option<&VehicleRecord> {
        self.vehicle_records.iter().find(|r| r.vehicle.as_deref().map_or(false, |v| v.eq_ignore_ascii_case(vehicle)))
    }

    /// The model's purchase record, made when there is none (the original keeps one per
    /// entry of the vehicle table, sorted by its index: F-90134).
    pub fn record_mut(&mut self, vehicle: &str) -> &mut VehicleRecord {
        if let Some(k) = self.vehicle_records.iter().position(|r| r.vehicle.as_deref().map_or(false, |v| v.eq_ignore_ascii_case(vehicle))) {
            return &mut self.vehicle_records[k];
        }
        let r = VehicleRecord::new(vehicle);
        let at = if r.index < 0 { self.vehicle_records.len() } else { self.vehicle_records.iter().position(|o| o.index > r.index || o.index < 0).unwrap_or(self.vehicle_records.len()) };
        self.vehicle_records.insert(at, r);
        &mut self.vehicle_records[at]
    }

    /// The garage in the port's order: the records follow `owned` (a record of a car that
    /// is no longer owned goes, as the original moves the later slots down - F-87162),
    /// the slot numbers and the slot in use with them.
    pub fn sync_garage(&mut self, owned: &[String], current: &str) {
        let mut out = Vec::with_capacity(self.garage.len());
        for name in owned {
            if let Some(k) = self.garage.iter().position(|c| c.vehicle.eq_ignore_ascii_case(name)) {
                out.push(self.garage.remove(k));
            }
        }
        // (records of cars the port does not list as owned stay behind the others: the
        // port's `owned` may lag behind while a car is being bought)
        let rest: Vec<Car> = self.garage.drain(..).collect();
        let keep_rest = owned.is_empty();
        if keep_rest {
            out.extend(rest);
        }
        for (k, c) in out.iter_mut().enumerate() {
            if c.slot != k as u32 {
                c.slot = k as u32;
            }
            let cur = c.vehicle.eq_ignore_ascii_case(current);
            if c.current != cur {
                c.current = cur;
            }
            if cur && self.garage_current_slot != k as i32 {
                self.garage_current_slot = k as i32;
            }
        }
        self.garage = out;
    }
}

// ---------------------------------------------------------------------------------------
// the game's side: what of the block is live
// ---------------------------------------------------------------------------------------

use crate::game::Game;
use crate::vehicles::PALETTE;

/// The state of the vehicle in use that the block keeps per vehicle and the port's older
/// fields have no room for. The port's menus pick these colours as swatches of `PALETTE`
/// (`tint_color`, `neon_color`, `paint_b`); the original keeps a free colour, RGB as on
/// screen. A record's colour stands here beside the swatch that was the menu's when it was
/// taken out, and holds while that swatch is not changed.
#[derive(Clone, Debug, Default)]
pub struct CarLive {
    /// air bags fitted (HydraulicsState0; `hydraulics` of the game is HydraulicsState1)
    pub airbags: bool,
    pub tint_rgb: Option<(usize, [f32; 3])>,
    pub neon_rgb: Option<(usize, [f32; 3])>,
    pub paint_b_rgb: Option<(usize, [f32; 3])>,
    /// the special ability chosen for this vehicle (0 ZONE, 1 AGRO, 2 ROAR, 3 EMP)
    pub ability: Option<usize>,
    /// the vehicle whose state is out (lower case)
    pub car: String,
    /// its damage and its four zones' as the game has them now, and what its slot kept, to
    /// be put on the vehicle when it stands there
    pub damage: Option<(f32, [f32; 4])>,
    pub damage_in: Option<(f32, [f32; 4])>,
    /// the garage's purchase box is asking about the paint shop (0) or the vinyl editor (1)
    pub misc_pending: Option<usize>,
    /// X was pressed in the hood list: CARBON FIBER ON/OFF
    pub swap_cf: bool,
    /// the top GPS arrow: how far it has faded in (0..1) and where its target last was, in
    /// the car's level frame (right, up, back)
    pub top_arrow: (f32, [f32; 3]),
    /// the garage's REPAIR box is up: what goes on after YES (0 the row under the cursor,
    /// 1 out of the garage) and the price shown
    pub repair_pending: Option<(u8, i64)>,
    /// YES was given: the row (0) or the way out (1) is taken on the next frame
    pub repair_go: Option<u8>,
    /// the six paint areas of the vehicle in use (made by `mc4_paint_sync`)
    pub paint: Option<PaintLive>,
    /// INTERIOR COLOR D and E (colour groups 4 and 5, `m_vBaseColor[3]`, `[4]`) of the
    /// vehicle in use: the swatch on the row and a mixed colour (hue, saturation,
    /// brightness as on screen), as the port keeps A..C in `int_colors` / `slot_hsv`
    pub int_de_idx: [StripIdx; 2],
    pub int_de_hsv: [Option<[f32; 3]>; 2],
    /// (step 22) a vinyl editor box is up: what its YES does
    pub vinyl_ask: Option<VinylAsk>,
    /// (step 22) the STORE VINYL PACKAGES page is open: the row under the cursor (0 .. 2)
    pub vinyl_save: Option<usize>,
    /// (step 21b) a mission's scene plays at the garage's door: the garage opens after it
    pub garage_after_scene: bool,
    /// (step 21b) the hangouts / delivery / payback places whose set-up has ended with a visit
    /// and whose restarter waits for the player at its point (`HANGOUT_RESTARTERS`)
    pub hangout_wait: Vec<String>,
}

/// One paint area's entry of the vehicle record, without its first colour (which the
/// port keeps as the body's colour and `CarConfig::part_paint`): paint type (0 DirtyMatte,
/// 1 Matte, 2 Gloss, 3 Metallic, 4 Pearl, 5 Sparkle), fade (0 None, 1 ColorShift,
/// 2 FrontBack, 3 LeftRight, 4 TopBottom), second colour as on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaPaint {
    pub ty: u8,
    pub fade: u8,
    pub b: [f32; 3],
}

/// The six areas (Body, Doors, Hood, Front Bumper, Rear Bumper, Skirts) of the vehicle in
/// use, and what the paint shop's rows stood at when they were last looked at.
#[derive(Clone, Debug)]
pub struct PaintLive {
    pub areas: [AreaPaint; 6],
    seen_ty: u8,
    seen_fade: u8,
    seen_b: [f32; 3],
    seen_a: [f32; 4],
    seen_area: usize,
    seen_parts: [Option<[u8; 3]>; 5],
}

/// What the renderer gets for the player's paint: the body's type, fade and second colour
/// (linear), and per area after the body its first colour (linear; w 1 = the area has a
/// paint of its own) and its second (w = type + 8 x fade).
#[derive(Clone, Copy, Debug)]
pub struct PaintOut {
    pub ty: u8,
    pub fade: u8,
    pub b: [f32; 3],
    pub area_a: [[f32; 4]; 5],
    pub area_b: [[f32; 4]; 5],
}

/// The second colour the two matte paint types are drawn with, whatever the entry holds
/// (F-13513 at A-97806: types 0 and 1 get (0.6, 0.6, 0.55) for ColorB, keep the
/// type's paint falloff and have no fade direction).
pub const MATTE_B: [f32; 3] = [0.6, 0.6, 0.55];

/// What the original's paint set-up makes of an entry (F-13513): a matte type shifts
/// to `MATTE_B` by the type's falloff (the port's fade 1) and has no other fade.
fn paint_as_drawn(p: AreaPaint) -> AreaPaint {
    if p.ty <= 1 { AreaPaint { ty: p.ty, fade: 1, b: MATTE_B } } else { p }
}

/// What the game's running side hands over for the options the block keeps (the driving
/// camera and the clock live in the window's loop, not in `Game`).
#[derive(Clone, Debug, Default)]
pub struct OptLive {
    /// a camera for the loop to take up (the port's view number: 0 far .. 2 near, 3 driver,
    /// 4 bumper), and the view it has now
    pub cam_in: Option<u8>,
    pub cam: Option<u8>,
    /// the time of day to resume from, in hours, and the clock as it stands
    pub hours_in: Option<f32>,
    pub hours: Option<f32>,
    /// the music lists (the original's music manager), shared with the audio; made by
    /// `Game::mc4_music` when the disc's audio config is there (`music_tried`: it was looked for)
    pub music: Option<MusicShared>,
    pub music_tried: bool,
    /// the disc's controller maps (`control_maps`), read once
    pub control_maps: Option<Vec<ControlMap>>,
    /// the controller pages of OPTIONS (step 13b), and the pad as it stands this frame
    pub ctrl: CtrlUi,
    pub pad: Option<PadState>,
    /// step 19c: the ARCADE options' Custom rows, LAST MAN OUT, the time trial's clock, the
    /// borrowed vehicle's set-up, the payback leg
    pub arcade: ArcadeLive,
    /// step 23: the slow motion, the tutorials, the statistics' counters
    pub s23: S23Live,
    /// step 23b: the driving statistics' counters, the test drive seen
    pub s23b: crate::mc4b::S23bLive,
}

/// The original's "Default Camera" numbers (0 Bumper, 1 POV, 2 Boom 1, 3 Boom 2, 4 Boom 3;
/// boom 1 is the farthest) as the port's views (0 far, 1 medium, 2 near, 3 driver, 4 bumper).
pub const CAMERA_TO_PORT: [u8; 5] = [4, 3, 0, 1, 2];
pub const PORT_TO_CAMERA: [u32; 5] = [2, 3, 4, 1, 0];

/// A row of CAMERA OPTIONS was stepped: 48 "Default Camera" (the list's order: Bumper, POV,
/// Boom 1 .. 3), 49 "Idle Camera". (On the game's two fields, for the menu code that holds
/// other parts of the game while it runs.)
/// STEERING SENS. one step on (see `Game::mc4_sens_set`; on the fields themselves, for the
/// menu's code, which holds other parts of the game while it runs).
pub fn sens_set(mc4: &mut Option<Mc4>, sens: &mut u8, forward: bool, wrap: bool, fresh_game: bool) {
    let o = &mut mc4.get_or_insert_with(|| Mc4::seeded(fresh_game)).options;
    let cur = match o.sensitivity_step() {
        step if ((step + 10) as f32 / 2.0).round().clamp(0.0, 10.0) as u8 == *sens => step,
        _ => (*sens).min(10) as i32 * 2 - 10,
    };
    let next = if wrap {
        if cur >= 10 { -10 } else { cur + 1 }
    } else if forward {
        (cur + 1).min(10)
    } else {
        (cur - 1).max(-10)
    };
    o.set_sensitivity_step(next);
    *sens = ((next + 10) as f32 / 2.0).round().clamp(0.0, 10.0) as u8;
}

pub fn option_step(mc4: &mut Option<Mc4>, opt_x: &mut OptLive, page: u8, forward: bool, fresh_game: bool) {
    let o = &mut mc4.get_or_insert_with(|| Mc4::seeded(fresh_game)).options;
    match page {
        48 => {
            let cur = opt_x.cam.and_then(|c| PORT_TO_CAMERA.get(c as usize).copied()).unwrap_or(o.camera.min(4));
            let c = (cur + if forward { 1 } else { 4 }) % 5;
            o.set_camera(c);
            opt_x.cam_in = Some(CAMERA_TO_PORT[c as usize]);
            opt_x.cam = opt_x.cam_in;
        }
        49 => o.set_cinematics(!o.idle_camera, o.camera_shake),
        // (GAME OPTIONS, CONVERTIBLE TOP: "option_convertible", up / down - LocalOptions + 1228)
        50 => o.convertible_top_down = !o.convertible_top_down,
        // (GAME OPTIONS, ARROW: "option_arrow" - compass, top, none: LocalOptions + 1204)
        51 => o.set_gps_arrow((o.gps_arrow.min(2) + if forward { 1 } else { 2 }) % 3),
        _ => {}
    }
}

/// A colour of the screen (a record's RGB) as the renderer's linear colour, and back.
pub fn to_linear(c: [f32; 3]) -> [f32; 3] {
    [c[0].max(0.0).powf(2.2), c[1].max(0.0).powf(2.2), c[2].max(0.0).powf(2.2)]
}

pub fn to_screen(c: [f32; 3]) -> [f32; 3] {
    [c[0].max(0.0).powf(1.0 / 2.2), c[1].max(0.0).powf(1.0 / 2.2), c[2].max(0.0).powf(1.0 / 2.2)]
}

fn swatch_screen(k: usize) -> [f32; 3] {
    let c = PALETTE[k % PALETTE.len()];
    to_screen([c[0], c[1], c[2]])
}

/// The swatch nearest a screen colour (what the port's menu row shows for it).
fn nearest_swatch(c: [f32; 3]) -> usize {
    let mut best = (0usize, f32::MAX);
    for k in 0..PALETTE.len() {
        let s = swatch_screen(k);
        let d = (0..3).map(|i| (s[i] - c[i]) * (s[i] - c[i])).sum::<f32>();
        if d < best.1 {
            best = (k, d);
        }
    }
    best.0
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 2e-6)
}

impl Game {
    fn mc4_car_name(&self, i: usize) -> String {
        self.car_dirs.get(i).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
    }

    /// The block of a save that was read.
    pub fn mc4_load(&mut self, v: Option<Value>) {
        self.mc4 = None;
        self.career.unlocking = None;
        self.mc4_raw = None;
        self.car_x = CarLive::default();
        let Some(v) = v else { return };
        match Mc4::from_value(&v) {
            Ok(mut m) => {
                m.options.upgrade();
                for r in m.vehicle_records.iter_mut() {
                    r.upgrade();
                }
                // (a mod car's table index follows the mods installed now: rewritten from its name, as the
                // original rewrites every slot's from its TypeName on load - MODS.md M-2, F-90134)
                for c in m.garage.iter_mut() {
                    if let Some(k) = crate::modcars::table_index(&c.vehicle) {
                        c.vehicle_index = k as i32;
                    }
                }
                for r in m.vehicle_records.iter_mut() {
                    if let Some(k) = r.vehicle.as_deref().and_then(crate::modcars::table_index) {
                        r.index = k as i32;
                    }
                }
                // (what the port's own tags say was bought, into the records of a block the
                // port made: its first build wrote four categories only)
                if m.made_by_port() {
                    let tags: Vec<(String, Vec<String>)> = self.parts_owned.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                    for (car, tags) in tags {
                        m.record_mut(&car).take_tags(&tags);
                    }
                }
                // (the parts the port keeps in `xparts` since: from the records of a block
                // that was imported before)
                for c in m.garage.iter() {
                    let have = self.xparts.entry(c.vehicle.to_ascii_lowercase()).or_default();
                    for (key, byte) in XPART_BYTES {
                        if !have.contains_key(key) && c.part(byte) != 0 {
                            have.insert(key.to_string(), c.part(byte) as usize);
                        }
                    }
                }
                // (and what the records own is the player's in the port's shops)
                for r in m.vehicle_records.iter() {
                    let Some(car) = r.vehicle.as_ref().filter(|_| r.in_use) else { continue };
                    let tags = r.owned_tags();
                    if !tags.is_empty() {
                        let have = self.parts_owned.entry(car.to_ascii_lowercase()).or_default();
                        for t in tags {
                            if !have.contains(&t) {
                                have.push(t);
                            }
                        }
                    }
                }
                self.mc4 = Some(m);
                self.mc4_options_in();
                self.mc4_career_in();
            }
            Err(e) => {
                // (kept as it is and written back untouched)
                log::warn!("save: the mc4 block does not fit ({e}); kept as read, not used");
                self.mc4_raw = Some(v);
            }
        }
    }

    /// The block for the save about to be written: the one in hand with the state of the
    /// vehicle in use and the garage's order put into it.
    pub fn mc4_save(&self) -> Option<Value> {
        if let Some(raw) = &self.mc4_raw {
            return Some(raw.clone());
        }
        let mut m = self.mc4.clone().unwrap_or_else(|| Mc4::seeded(self.fresh_game));
        self.mc4_store(&mut m);
        Some(m.to_value())
    }

    /// The block, made if the save had none.
    pub fn mc4_mut(&mut self) -> &mut Mc4 {
        let fresh = self.fresh_game;
        self.mc4.get_or_insert_with(|| Mc4::seeded(fresh))
    }

    /// A vehicle's record in `m`, made from its set-up file when there is none
    /// (`config`: the file's name; none = the car's own, as for a bought car).
    fn mc4_car_in<'a>(&self, m: &'a mut Mc4, car: &str, config: Option<&str>) -> Option<&'a mut Car> {
        if car.is_empty() {
            return None;
        }
        if m.car(car).is_none() {
            let cfg = config.map(|s| s.to_string()).unwrap_or_else(|| car.to_string());
            let slot = m.garage.len() as u32;
            // (by the set-up's name hash where the install named the file otherwise: the S7's and the
            // Murcielago DUB's set-ups - render/car_lamps.rs `setup_file`)
            let root = std::path::Path::new(&self.data_dir);
            let read = |n: &str| crate::render::car_lamps::setup_file(root, n).and_then(|f| std::fs::read(f).ok());
            let made = read(&cfg).and_then(|b| Car::from_xcc(&b, car, &cfg, slot)).or_else(|| read(car).and_then(|b| Car::from_xcc(&b, car, car, slot)));
            // (no set-up file at hand: the constructor's record - F-35798)
            let made = made.unwrap_or_else(|| {
                let mut c = Car { slot, TypeName: car.to_string(), vehicle: car.to_ascii_lowercase(), in_garage: 1, source_config: Some(cfg.clone()), source_config_hash: format!("0x{:08x}", joaat(&cfg)), ..Default::default() };
                c.vehicle_index = vehicle_index(&c.vehicle);
                c.visual.m_vWindowTintColorNew = fv(&[0.0, 0.0035, 0.005, 0.372]);
                c.visual.ExtNeonColor = fv(&[0.52, 0.52, 1.0]);
                c.visual.LicenseBG = 4;
                c.visual.vinyl_preset_group = -1;
                c.visual.vinyl_preset_index = -1;
                c.set_ability(None);
                c.derived.origin = Some(c.origin());
                c.derived.window_tint_level = Some(0);
                c
            });
            m.garage.push(made);
        }
        m.car_mut(car)
    }

    /// A vehicle came into the garage (bought: no `config`; a starter, a prize: the set-up
    /// file it is made from, e.g. `starter_vp_vw_golf_83`). The original builds the slot's
    /// record from that file (F-86225) and zeroes the slot's damage (F-91320).
    pub fn mc4_car_added(&mut self, car: &str, config: Option<&str>) {
        let mut m = self.mc4.take().unwrap_or_else(|| Mc4::seeded(self.fresh_game));
        let car = car.to_ascii_lowercase();
        if m.car(&car).is_none() {
            self.mc4_car_in(&mut m, &car, config);
        }
        m.record_mut(&car);
        // (what its set-up file fits of the parts the port keeps in `xparts`: a prize car's
        // carbon fibre hood, its intercooler)
        if let Some(c) = m.car(&car) {
            let have = self.xparts.entry(car.clone()).or_default();
            for (key, byte) in XPART_BYTES {
                if !have.contains_key(key) && c.part(byte) != 0 {
                    have.insert(key.to_string(), c.part(byte) as usize);
                }
            }
        }
        self.mc4 = Some(m);
    }

    /// `garage.SwapCarbonFiber()` (X in the hood list; F-96638 at A-65149): the
    /// hood of the vehicle in the garage in carbon fibre, or back - free, any hood.
    pub fn mc4_swap_carbon_fiber(&mut self) {
        let car = self.mc4_car_name(self.current_car);
        let have = self.xparts.entry(car).or_default();
        let on = have.get("carbon_hood").copied().unwrap_or(0) != 0;
        have.insert("carbon_hood".to_string(), (!on) as usize);
        self.dirty = true;
    }

    /// The state of the vehicle in use, into its record of `m`.
    fn mc4_put_car(&self, m: &mut Mc4) {
        let car = if self.car_x.car.is_empty() { self.mc4_car_name(self.current_car) } else { self.car_x.car.clone() };
        // (a vehicle that is not the player's - a test drive, a borrowed one - has no slot)
        if !self.owned.iter().any(|o| o.eq_ignore_ascii_case(&car)) {
            return;
        }
        let x = &self.car_x;
        let (hyd, tint_level, neon_on, int_neon) = (self.hydraulics, self.tint_level, self.neon_on, self.int_neon);
        let tint = match x.tint_rgb {
            Some((k, c)) if k == self.tint_color => c,
            _ => swatch_screen(self.tint_color),
        };
        let neon = match x.neon_rgb {
            Some((k, c)) if k == self.neon_color => c,
            _ => swatch_screen(self.neon_color),
        };
        let paint_b = match x.paint_b_rgb {
            Some((k, c)) if k == self.paint_b => c,
            _ => swatch_screen(self.paint_b),
        };
        let (airbags, mut ability, fade, damage) = (x.airbags, x.ability, self.paint_fade, x.damage);
        // (a vehicle the player had before the save had the block: the port's one ability
        // was every vehicle's, so its new record gets the one in hand - not the set-up
        // file's "none", which is for a vehicle that comes into the garage from now on)
        if ability.is_none() && m.car(&car).is_none() && self.mc4_abilities_unlocked() {
            ability = Some(self.ability_sel.min(3));
        }
        let Some(c) = self.mc4_car_in(m, &car, None) else { return };
        if let Some((d, z)) = damage {
            let have = (c.damage.damage.map_or(0.0, |v| v.0 as f32), c.damage.zones.as_ref().map_or([0.0; 4], |v| [0, 1, 2, 3].map(|k| v.get(k).map_or(0.0, |x| x.0 as f32))));
            if (have.0 - d).abs() > 1e-6 || (0..4).any(|k| (have.1[k] - z[k]).abs() > 1e-6) {
                c.damage.damage = Some(r6(d));
                c.damage.zones = Some(z.iter().map(|v| r6(*v)).collect());
            }
        }
        if c.hydraulics() != hyd {
            c.performance.hydraulics_fitted = hyd as u8;
        }
        if c.airbags() != airbags {
            c.performance.airbags_fitted = airbags as u8;
        }
        // (the tint: the row, and - once a row is chosen - the colour; stock glass keeps the
        // record's own colour)
        let row = tint_level.min(9);
        let rgb = (row > 0 && c.tint_rgb().map_or(true, |o| !close(o, tint))).then_some(tint);
        if c.tint_row() != row || rgb.is_some() {
            c.set_tint(row, rgb);
        }
        let (on, old) = c.neon_ext();
        let rgb = (neon_on && old.map_or(true, |o| !close(o, neon))).then_some(neon);
        if on != neon_on || rgb.is_some() {
            c.set_neon_ext(neon_on, rgb);
        }
        if c.neon_int() != int_neon {
            c.set_neon_int(int_neon);
        }
        if ability.is_some() && c.ability_idx() != ability {
            c.set_ability(ability);
        }
        // (the second colour: the body's, and with it every area that had the body's -
        // unless the six areas are in hand, which the record takes whole: `mc4_store`)
        if fade != 0 && self.car_x.paint.is_none() {
            let old = c.visual.paint.first().and_then(|p| p.color_b());
            if old.map_or(true, |o| !close(o, paint_b)) {
                for p in c.visual.paint.iter_mut() {
                    if p.area == "Body" || p.color_b().map_or(true, |b| old.map_or(true, |o| close(o, b))) {
                        p.ColorB = vec![r6(paint_b[0]), r6(paint_b[1]), r6(paint_b[2])];
                    }
                }
            }
        }
    }

    /// The vehicle in use changed (or a save was read): its record's state becomes the
    /// game's. A vehicle without a record keeps what the game has, as before.
    pub fn mc4_take_car(&mut self, car: &str) {
        let car = car.to_ascii_lowercase();
        self.car_x.car = car.clone();
        self.car_x.damage = None;
        self.car_x.damage_in = None;
        self.car_x.paint = None;
        let Some(c) = self.mc4.as_ref().and_then(|m| m.car(&car)).cloned() else { return };
        self.car_x.damage_in = Some((c.damage.damage.map_or(0.0, |v| v.0 as f32), c.damage.zones.as_ref().map_or([0.0; 4], |v| [0, 1, 2, 3].map(|k| v.get(k).map_or(0.0, |x| x.0 as f32)))));
        self.hydraulics = c.hydraulics();
        self.car_x.airbags = c.airbags();
        self.tint_level = c.tint_row();
        if let Some(rgb) = c.tint_rgb() {
            self.tint_color = nearest_swatch(rgb);
            self.car_x.tint_rgb = Some((self.tint_color, rgb));
        }
        let (on, rgb) = c.neon_ext();
        self.neon_on = on;
        if let Some(rgb) = rgb {
            self.neon_color = nearest_swatch(rgb);
            self.car_x.neon_rgb = Some((self.neon_color, rgb));
        }
        self.int_neon = c.neon_int();
        // (a body in DirtyMatte, type 0: of the set-ups only, the shop does not list it)
        if c.visual.paint.first().and_then(|p| p.paint_type()) == Some(0) {
            self.paint_type = 0;
        }
        // (INTERIOR COLOR D and E: the record's groups 4 and 5; a black one is the stock
        // cabin, as the importer has it for A..C)
        self.car_x.int_de_idx = Default::default();
        self.car_x.int_de_hsv = [None, None];
        for k in 0..2 {
            if let Some(rgb) = c.visual.m_vBaseColor.get(3 + k).and_then(|g| rgb_of(&g.rgb)) {
                let h = rgb_hsv(rgb);
                if h[2] >= 0.08 {
                    self.car_x.int_de_hsv[k] = Some(h);
                }
            }
        }
        // (a rim part in carbon fibre, material 2: the port's finish 3 - the record is its home)
        for k in 0..3 {
            if c.rim_material(k) == Some(2) {
                self.rim_mats[k] = 3;
            }
        }
        self.car_x.ability = c.ability_idx();
        // (the vehicle's ability is the one in hand: mcRacer::GetActiveAbility reads the record)
        if let Some(k) = self.car_x.ability {
            self.ability_sel = k;
            self.ability_pick = Some(k);
        }
        if let Some(rgb) = c.visual.paint.first().and_then(|p| p.color_b()) {
            self.paint_b = nearest_swatch(rgb);
            self.car_x.paint_b_rgb = Some((self.paint_b, rgb));
        }
    }

    /// The vehicle in use is about to change: its state goes into its record.
    pub fn mc4_leave_car(&mut self) {
        if self.car_x.car.is_empty() {
            return;
        }
        let mut m = self.mc4.take().unwrap_or_else(|| Mc4::seeded(self.fresh_game));
        self.mc4_put_car(&mut m);
        self.mc4 = Some(m);
    }

    /// Everything the game holds that the block is the home of, into `m`.
    fn mc4_store(&self, m: &mut Mc4) {
        self.mc4_put_car(m);
        let cur = self.mc4_car_name(self.current_car);
        for car in &self.owned {
            self.mc4_car_in(m, &car.to_ascii_lowercase(), None);
        }
        // (step 21b: while a pink slip race runs the staked vehicle is out of the garage)
        let owned = self.mc4_owned_out();
        let cur = self.mc4_current_out().unwrap_or(cur);
        m.sync_garage(&owned, &cur);
        self.mc4_sync_cars(m);
        self.mc4_options_out(m);
        m.enable_gps = self.mc4_gps_enabled();
        self.mc4_career_out(m);
        self.mc4_editor_out(m);
    }

    /// The career state of a save that was read: which thresholds of the progression tables
    /// have handed out their rewards, and the ability systems' amounts.
    fn mc4_career_in(&mut self) {
        let Some(m) = self.mc4.as_ref() else { return };
        let x = &m.career.experience;
        self.career.reached_ids = x.main.values().chain(x.classes.values()).chain(x.abilities.values()).flat_map(|s| s.thresholds.iter()).filter(|t| t.reached).map(|t| t.id.clone()).collect();
        // (a block the port made holds the port's own counts; an original's the systems')
        self.career.ability_amounts = ABILITY_SYSTEMS.iter().filter_map(|s| x.abilities.get(*s).map(|v| (s.to_string(), v.amount))).collect();
        // (the amount of the Unlocking system: a save of the original has it; a block of the
        // port from before it was counted has not, and follows the reputation)
        let counted = !m.made_by_port() || x.extra.get("unlocking_counted").and_then(|v| v.as_bool()).unwrap_or(false);
        self.career.unlocking = x.main.get("Unlocking").filter(|_| counted).map(|s| s.amount);
    }

    /// The career state the block is the home of, into `m`: the "reached" flags the game
    /// has set, the reputation (the Progression system's amount), the ability systems'
    /// amounts and levels, the performance levels the shop sells, the two unlock flags.
    fn mc4_career_out(&self, m: &mut Mc4) {
        let port_made = m.made_by_port();
        // (the missions' records and the career's scripts: step 20)
        self.mc4_missions_out(m);
        self.mc4_scripts_apply(m);
        // (the hangouts' own pink slip cars loaded: the id counter - step 21)
        if self.career.loaded() {
            self.mc4_pink_count(m);
        }
        // (the list of unlocked races: step 17)
        self.mc4_races_apply(m);
        // (the time trials' first targets and unlock flags: step 18)
        self.mc4_tt_apply_block(m);
        let c = &mut m.career;
        // (a new career of the port lists the tables' events, as a new game of the original
        // does: every counter 0)
        if port_made && self.fresh_game && c.events.is_empty() {
            for e in &self.career.exp_tables.events {
                c.event_mut(&e.name);
            }
        }
        // (the thresholds of the two main systems: every one the share of the system has
        // come to is reached, whatever its rewards - F-61830; the Unlocking amount)
        for def in self.career.exp_tables.systems.iter().filter(|d| d.is_main()) {
            let amount = if def.name == "Unlocking" { self.mc4_unlocking() } else { self.rep };
            let f = def.fraction(amount);
            let s = c.experience.main.entry(def.name.clone()).or_default();
            for th in &def.thresholds {
                if f >= th.percent && s.reached(&th.id) != Some(true) {
                    s.set_reached(&th.id);
                }
            }
        }
        if let Some(u) = self.career.unlocking {
            let s = c.experience.main.entry("Unlocking".to_string()).or_default();
            if s.amount != u {
                s.amount = u;
            }
            if port_made && !c.experience.extra.contains_key("unlocking_counted") {
                c.experience.extra.insert("unlocking_counted".into(), Value::Bool(true));
            }
        }
        for id in &self.career.reached_ids {
            let Some((sys, _)) = id.rsplit_once('_') else { continue };
            let x = &mut c.experience;
            let s = if sys == "Progression" || sys == "Unlocking" {
                x.main.entry(sys.to_string()).or_default()
            } else if ABILITY_SYSTEMS.contains(&sys) {
                x.abilities.entry(sys.to_string()).or_default()
            } else {
                x.classes.entry(sys.to_string()).or_default()
            };
            if s.reached(id) != Some(true) {
                s.set_reached(id);
            }
        }
        // (the reputation is the Progression system's amount: F-22911)
        let p = c.experience.main.entry("Progression".to_string()).or_default();
        if p.amount != self.rep {
            p.amount = self.rep;
        }
        // (the abilities: uses counted by the system, its thresholds - level 1 at half the
        // table's MaxAmount, level 2 at all of it - and the level LocalOptions keeps)
        for (k, sys) in ABILITY_SYSTEMS.iter().enumerate() {
            let stat = ["gZON", "gAGR", "gROR", "gPUL"][k];
            let amount = self.career.ability_amounts.get(*sys).copied().unwrap_or_else(|| self.career.stats.get(stat).copied().unwrap_or(0));
            let level = self.career.ability_level(sys, stat);
            let s = c.experience.abilities.entry(sys.to_string()).or_default();
            if s.amount != amount {
                s.amount = amount;
            }
            let mut steps: Vec<i64> = self.career.ability_levels.get(*sys).map(|v| v.iter().map(|(need, _)| *need).collect()).unwrap_or_default();
            steps.sort_unstable();
            for (n, need) in steps.iter().enumerate() {
                let id = format!("{sys}_{n}");
                if amount >= *need && s.reached(&id) != Some(true) {
                    s.set_reached(&id);
                }
            }
            let have = match k {
                0 => &mut m.ability_levels.zone,
                1 => &mut m.ability_levels.agro,
                2 => &mut m.ability_levels.roar,
                _ => &mut m.ability_levels.pulse,
            };
            // (a level the save has is not taken back)
            if *have < level {
                *have = level;
            }
        }
        // (performance levels: D1 .. A2 as the reputation reaches what the table asks)
        for (id, need) in &self.career.perf_unlocks {
            if PERF_LEVELS.contains(&id.as_str()) && self.mc4_unlocking() >= *need && c.performance_levels_unlocked.get(id) != Some(&true) {
                c.performance_levels_unlocked.insert(id.clone(), true);
            }
        }
        if self.mc4_police_rule() && !c.flags.police_unlocked {
            c.set_police_unlocked(true);
        }
        if self.career.reached("INSTALL_ABILITY") && !c.abilities_unlocked {
            c.abilities_unlocked = true;
        }
        if self.career.loaded() && !c.flags.career_startup_done {
            c.set_startup_done();
        }
    }

    /// The race editor's races (the game's `user_races`, in the order of the slots) into the
    /// block's: a race that is the slot's keeps the slot's record as it is; a changed one
    /// gets its checkpoints (activation height and radius 25 m, as the editor saves them:
    /// F-60473); a new one the first free slot of the twenty, named as the constructor
    /// names it ("LosAngeles<slot>", F-65243).
    fn mc4_editor_out(&self, m: &mut Mc4) {
        let races = &mut m.editor.races;
        for (k, pts) in self.user_races.iter().enumerate() {
            let cps = |old: Option<&EditorRace>| -> Vec<Checkpoint> {
                pts.iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let o = old.and_then(|r| r.checkpoints.get(i));
                        Checkpoint { pos: p.iter().map(|v| F(*v as f64)).collect(), activation_height: o.map_or(25, |c| c.activation_height), activation_radius: o.map_or(25, |c| c.activation_radius), extra: o.map(|c| c.extra.clone()).unwrap_or_default() }
                    })
                    .collect()
            };
            if let Some(r) = races.get_mut(k) {
                if r.points() != *pts {
                    r.checkpoints = cps(Some(&*r));
                    r.count = pts.len() as u32;
                    r.count_byte = pts.len() as u32;
                    r.in_use = true;
                }
            } else {
                let slot = (0..20u32).find(|s| !races.iter().any(|r| r.slot == *s)).unwrap_or(races.len() as u32);
                // (the city's number, byte 1043: 3 in the 100 % save's own race - the only
                // value seen for this city; the word at 0 is written 0 beside a city number)
                races.push(EditorRace { slot, name: format!("LosAngeles{slot}"), in_use: true, count: pts.len() as u32, city: 3, grid_variant: 0, flag_1041: 0, count_word: 0, count_byte: pts.len() as u32, checkpoints: cps(None), ..Default::default() });
            }
        }
        races.truncate(self.user_races.len());
    }

    /// The number the port gives the `k`-th race of the editor: its slot ("LosAngeles<slot>",
    /// 0 .. 19) plus one; without a slot record, its place in the list.
    pub fn mc4_editor_number(&self, k: usize) -> usize {
        self.mc4.as_ref().and_then(|m| m.editor.races.get(k)).map_or(k + 1, |r| r.slot as usize + 1)
    }

    /// "Save As Vinyl Package" (F-59911): every layer of the vehicle in use into one of
    /// the three packages. False when the vehicle has no layers of its own.
    pub fn mc4_vinyl_store(&mut self, slot: usize) -> bool {
        // (the vehicle the editor is on: the key of `liveries`; else the one in use)
        let car = if self.vinyl_for.is_empty() { self.mc4_car_name(self.current_car) } else { self.vinyl_for.clone() };
        let Some(l) = self.liveries.get(&car).cloned() else { return false };
        let Ok(livery) = serde_json::to_value(&l) else { return false };
        let m = self.mc4_mut();
        let slot = slot.min(2);
        while m.vinyl_packages.len() <= slot {
            let n = m.vinyl_packages.len() as u32;
            m.vinyl_packages.push(VinylPackage { slot: n, used: false, livery: serde_json::json!({ "zones": [[], [], [], [], []] }), extra: Extra::new() });
        }
        let p = &mut m.vinyl_packages[slot];
        p.used = true;
        p.livery = livery;
        self.dirty = true;
        true
    }

    /// A package back onto the vehicle in use (F-57683): its layers replace the
    /// vehicle's. False for a package never saved.
    pub fn mc4_vinyl_load(&mut self, slot: usize) -> bool {
        let Some(p) = self.mc4.as_ref().and_then(|m| m.vinyl_packages.get(slot)).filter(|p| p.used) else { return false };
        let Ok(l) = serde_json::from_value::<crate::vinyl::Livery>(p.livery.clone()) else { return false };
        let car = if self.vinyl_for.is_empty() { self.mc4_car_name(self.current_car) } else { self.vinyl_for.clone() };
        self.liveries.insert(car.clone(), l);
        self.vinyls.insert(car, "custom".to_string());
        self.livery_rev += 1;
        self.dirty = true;
        true
    }

    /// The missions whose scripts call mcCareer_UnlockPolice when they start (GARAGE_BEACHES,
    /// PAYBACK, PAYBACK_GH_18, PAYBACK_INTRO: `missions/<id>.sco`; the tutorial's own script
    /// does it too).
    fn mc4_police_rule(&self) -> bool {
        ["GARAGE_BEACHES", "PAYBACK", "PAYBACK_GH_18", "PAYBACK_INTRO"].iter().any(|id| self.career.reached(id))
    }

    /// Police are about in the cruise (mcCareer_IsPoliceUnlocked: bit 2 of the career's flag
    /// word). A game without the block has them from the start, as before.
    pub fn mc4_police_unlocked(&self) -> bool {
        match &self.mc4 {
            Some(m) => m.career.flags.police_unlocked || self.mc4_police_rule() || !self.career.loaded(),
            None => true,
        }
    }

    /// The special abilities are fitted (mcCareer_UnlockAbilities, called when the mission
    /// INSTALL_ABILITY is ready to start: `events/INSTALL_ABILITY_READY_TO_START_ON_ENTER.sco`).
    pub fn mc4_abilities_unlocked(&self) -> bool {
        !self.career.loaded() || self.career.reached("INSTALL_ABILITY") || self.mc4.as_ref().map_or(false, |m| m.career.abilities_unlocked)
    }

    /// A performance level (`D1` .. `A2`) is unlocked in the save (reward PerformanceLevel,
    /// F-52004; read by the shop, F-85976).
    pub fn mc4_perf_level_unlocked(&self, id: &str) -> bool {
        self.mc4.as_ref().and_then(|m| m.career.performance_levels_unlocked.get(id).copied()).unwrap_or(false)
    }

    /// A race is on the lists of races to choose from (F-64698): an original save's
    /// list of unlocked races. A game without one lists every race, as before.
    pub fn mc4_race_unlocked(&self, id: &str) -> bool {
        match self.mc4.as_ref() {
            // (a career of the port keeps the list from its first race on - step 17; one that
            // was under way before that has every race, as before)
            Some(m) if m.made_by_port() && !races_counted(m) => !self.fresh_game || self.mc4_start_race(id),
            Some(m) if !m.made_by_port() && m.career.races_unlocked.is_empty() => true,
            Some(m) => m.career.race_is_unlocked(&format!("game/Race/{}", race_rel(id))),
            // (no block yet: a new career has the races its start-up unlocks; a save
            // without the block lists every race)
            None => !self.fresh_game || self.mc4_start_race(id),
        }
    }

    /// A race was finished: it is unlocked from then on (F-20860 -> F-81609 on
    /// GENERIC_RACE_COMPLETE, not for a race of the editor; the list holds 250).
    pub fn mc4_race_finished(&mut self, id: &str) {
        if id.is_empty() || id.starts_with("user/") {
            return;
        }
        // (a block of an original save without a list: left alone, every race is listed)
        if self.mc4.as_ref().map_or(false, |m| !m.made_by_port() && m.career.races_unlocked.is_empty()) {
            return;
        }
        let mut m = self.mc4.take().unwrap_or_else(|| Mc4::seeded(self.fresh_game));
        let mut changed = self.mc4_races_apply(&mut m);
        changed |= m.career.unlock_race(&self.mc4_race_name(id));
        self.mc4 = Some(m);
        if changed {
            self.dirty = true;
        }
        // (the time trials' block state after what the race moved: step 18)
        self.mc4_tt_sync();
    }

    /// The options of a save that was read become the game's: the volumes (sliders of eleven
    /// places, 0.0 .. 1.0), subtitles, camera shake, the steering sensitivity, the music
    /// genre, the camera to start in and the time of day to resume from.
    fn mc4_options_in(&mut self) {
        let Some(o) = self.mc4.as_ref().map(|m| m.options.clone()) else { return };
        let vol = |x: F| ((x.0 * 10.0).round() as i64).clamp(0, 10) as u8;
        self.opts.master = vol(o.volume_effects);
        self.opts.music = vol(o.volume_music);
        self.opts.voice = vol(o.volume_voice);
        self.opts.vehicle = vol(o.volume_player_vehicle);
        self.opts.cutscene = vol(o.volume_cutscene);
        self.opts.subtitles = o.subtitles;
        self.opts.shake = o.camera_shake;
        self.opts.sens = ((o.sensitivity_step() + 10) as f32 / 2.0).round().clamp(0.0, 10.0) as u8;
        // (the game's language: the string table's block. Test hook: MCLA_LANGUAGE=<0..4>)
        let lang = std::env::var("MCLA_LANGUAGE").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(o.language as usize);
        self.career.strings.set_lang(lang);
        // (the port's list has ALL first: 7 -> 0, ECLECTIC .. WEST_RAP 0..6 -> 1..7; 8, the
        // custom playlist, plays as ALL)
        self.music_genre = if o.music_genre <= 6 { o.music_genre as u8 + 1 } else { 0 };
        self.opt_x.cam_in = CAMERA_TO_PORT.get(o.camera as usize).copied();
        let h = o.time_of_day_hours.0;
        self.opt_x.hours_in = (h.is_finite() && (0.0..24.0).contains(&h)).then_some(h as f32);
    }

    /// The game's options into `m`. A value the block's own stands for is left as it is.
    fn mc4_options_out(&self, m: &mut Mc4) {
        let o = &mut m.options;
        let vol = |x: F| ((x.0 * 10.0).round() as i64).clamp(0, 10) as u8;
        for (have, want) in [(&mut o.volume_effects, self.opts.master), (&mut o.volume_music, self.opts.music), (&mut o.volume_voice, self.opts.voice), (&mut o.volume_player_vehicle, self.opts.vehicle), (&mut o.volume_cutscene, self.opts.cutscene)] {
            if vol(*have) != want {
                *have = F(want as f64 / 10.0);
            }
        }
        if o.subtitles != self.opts.subtitles {
            o.subtitles = self.opts.subtitles;
        }
        if o.camera_shake != self.opts.shake {
            o.set_cinematics(o.idle_camera, self.opts.shake);
        }
        // (the port's slider has eleven places, the original's twenty-one)
        if ((o.sensitivity_step() + 10) as f32 / 2.0).round().clamp(0.0, 10.0) as u8 != self.opts.sens {
            o.set_sensitivity_step(self.opts.sens as i32 * 2 - 10);
        }
        let genre = if o.music_genre <= 6 { o.music_genre as u8 + 1 } else { 0 };
        if genre != self.music_genre {
            o.set_music_genre(if self.music_genre == 0 { 7 } else { self.music_genre as u32 - 1 });
        }
        // (the music lists: shuffle, the list that plays - the custom playlist too - and the
        // playlist's first 32 songs, as F-39888 / F-44753 write them on leaving
        // the playlist menu)
        if let Some(l) = self.opt_x.music.as_ref().and_then(|m| m.lock().ok()) {
            if o.music_shuffle != l.shuffle {
                o.music_shuffle = l.shuffle;
            }
            if o.music_genre != l.genre as u32 {
                o.set_music_genre(l.genre as u32);
            }
            let ids: Vec<Value> = l.custom_ids().iter().map(|id| Value::from(format!("0x{:08x}", id))).collect();
            if o.custom_playlist != ids {
                o.custom_playlist = ids;
            }
        }
        // (the camera in use is the option: F-93502 -> F-25166 writes it whenever
        // the player changes the view; the port's cinematic view, 5, is no option)
        if let Some(c) = self.opt_x.cam.and_then(|c| PORT_TO_CAMERA.get(c as usize).copied()) {
            if o.camera != c {
                o.set_camera(c);
            }
        }
        // (the clock at the time of saving: F-51180)
        if let Some(h) = self.opt_x.hours {
            if (o.time_of_day_hours.0 - h as f64).abs() > 1e-6 {
                o.time_of_day_hours = r6(h.rem_euclid(24.0));
            }
        }
    }

    /// The steering sensitivity on the port's scale of 0 .. 10 - with the original's odd
    /// steps between (its slider runs -10 .. 10: F-24198).
    pub fn sens_exact(&self) -> f32 {
        match self.mc4.as_ref().map(|m| m.options.sensitivity_step()) {
            Some(step) if ((step + 10) as f32 / 2.0).round().clamp(0.0, 10.0) as u8 == self.opts.sens => (step + 10) as f32 / 2.0,
            _ => self.opts.sens as f32,
        }
    }

    /// CAMERA OPTIONS' "Idle Camera": the cinematic camera that takes over when the player
    /// lets the car stand (the "cinematics" word, F-15111; on by default).
    pub fn mc4_idle_camera(&self) -> bool {
        self.mc4.as_ref().map_or(true, |m| m.options.idle_camera)
    }

    /// The original's number of the camera in use (0 Bumper, 1 POV, 2 .. 4 Boom 1 .. 3).
    pub fn mc4_camera(&self) -> u32 {
        self.opt_x.cam.and_then(|c| PORT_TO_CAMERA.get(c as usize).copied()).or(self.mc4.as_ref().map(|m| m.options.camera.min(4))).unwrap_or(4)
    }

    pub fn mc4_camera_name(&self) -> &'static str {
        CAMERAS[self.mc4_camera() as usize]
    }


    /// The window's loop hands its driving view over: a camera that was read or chosen is
    /// taken up, and the view in use is the option from then on.
    pub fn mc4_camera_sync(&mut self, cam_mode: &mut u8) {
        if let Some(c) = self.opt_x.cam_in.take() {
            *cam_mode = c;
        }
        if self.opt_x.cam != Some(*cam_mode) {
            if self.opt_x.cam.is_some() && *cam_mode < 5 {
                self.dirty = true;
            }
            self.opt_x.cam = Some(*cam_mode);
        }
    }

    /// And its clock: a save's time of day is where the game resumes (F-90712), unless
    /// the run was given one.
    pub fn mc4_clock_sync(&mut self, hours: &mut f32, given: bool) {
        if let Some(h) = self.opt_x.hours_in.take() {
            if !given {
                *hours = h;
            }
        }
        self.opt_x.hours = Some(*hours);
    }

    /// The window tint for the renderer: the record's `m_vWindowTintColorNew` - the colour
    /// as the glass programs take it (TintColors), and the darkness of the menu's row.
    pub fn tint_rgba(&self) -> [f32; 4] {
        let row = self.tint_level.min(9);
        let own = self.mc4.as_ref().and_then(|m| m.car(&self.car_x.car)).filter(|o| o.tint_row() == row).map(|o| o.tint_alpha());
        let a = own.unwrap_or(tint_alpha(row) as f32);
        match self.car_x.tint_rgb {
            Some((k, c)) if k == self.tint_color => [c[0], c[1], c[2], a],
            // (stock glass and no record: the constructor's glass)
            _ if row == 0 => [0.0, 0.0035, 0.005, 0.372],
            _ => {
                let c = swatch_screen(self.tint_color);
                [c[0], c[1], c[2], a]
            }
        }
    }

    /// The exterior neon's colour, linear.
    pub fn neon_linear(&self) -> [f32; 3] {
        match self.car_x.neon_rgb {
            Some((k, c)) if k == self.neon_color => to_linear(c),
            _ => {
                let c = PALETTE[self.neon_color % PALETTE.len()];
                [c[0], c[1], c[2]]
            }
        }
    }

    /// The body's second colour, linear.
    pub fn paint_b_linear(&self) -> [f32; 3] {
        match self.car_x.paint_b_rgb {
            Some((k, c)) if k == self.paint_b => to_linear(c),
            _ => {
                let c = PALETTE[self.paint_b % PALETTE.len()];
                [c[0], c[1], c[2]]
            }
        }
    }
}

fn hsv_rgb(h: [f32; 3]) -> [f32; 3] {
    let x = h[0].rem_euclid(1.0) * 6.0;
    let (i, f) = (x.floor() as i32 % 6, x - x.floor());
    let (v, p, q, t) = (h[2], h[2] * (1.0 - h[1]), h[2] * (1.0 - h[1] * f), h[2] * (1.0 - h[1] * (1.0 - f)));
    match i {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

fn rgb_f(c: [f32; 3]) -> Vec<F> {
    vec![r6(c[0]), r6(c[1]), r6(c[2])]
}

/// The port's parts and the record's part bytes that hold them (`cfg` slots, then `xparts`),
/// with the byte the game's set-part writes alongside (F-20885's mirrors).
const PART_MAP: [(&str, &str, Option<&str>); 14] = [
    ("hood", "hood", None),
    ("bumper_r", "bumper_r", None),
    ("bumper_f", "bumper_f", Some("intercooler")),
    ("skirts", "skirts", None),
    ("spoiler", "spoiler", Some("spoiler_b")),
    ("exhaust", "exh_pipe", None),
    ("seats", "seats_f", Some("seats_p")),
    ("body", "WideBodyKit", None),
    ("headlight", "headlight", None),
    ("taillight", "taillight", Some("taillight_b")),
    ("steer_whl", "steer_whl", None),
    ("post_gauge", "post_gauge", None),
    ("scissor_door", "ScissorDoors", None),
    ("blower", "blower", None),
];

/// The colour group (0..9: `m_vBaseColor`) of each of the 25 zones of a vehicle's cabin trim,
/// from one of its model resources: the root's pointer at +72 leads to 25 words. The
/// original's F-19629 builds the InteriorTrim program's table from it every frame:
/// zone z gets `m_vBaseColor[groups[z]]` with the surface values of the zone's kind (zones
/// 0..2 kind 0, 3..5 kind 2, 6..8 kind 1, 9..11 kind 3, 12..14 kind 4, 15..17 kind 5,
/// 18 kind 6, 19..21 kind 7, 22..24 kind 8).
pub fn zone_groups(file: &std::path::Path) -> Option<[u8; 25]> {
    let res = crate::assets::load_resource_file(file).ok()?;
    let d = res.virt();
    if d.len() < 76 {
        return None;
    }
    let at = res.ptr(u32::from_be_bytes([d[72], d[73], d[74], d[75]]))?;
    if at + 100 > res.virtual_size {
        return None;
    }
    let mut out = [0u8; 25];
    for (k, o) in out.iter_mut().enumerate() {
        let v = u32::from_be_bytes([d[at + 4 * k], d[at + 4 * k + 1], d[at + 4 * k + 2], d[at + 4 * k + 3]]);
        if v > 9 {
            return None;
        }
        *o = v as u8;
    }
    Some(out)
}

/// The zone table of the vehicle whose resources are in `dir` (its .r83 files - two, of
/// the same table: 0 .. 7 on the disc, 5 and up the groups the garage does not offer).
pub fn zone_groups_dir(dir: &std::path::Path) -> Option<[u8; 25]> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "r83")).collect();
    files.sort();
    files.iter().find_map(|p| zone_groups(p))
}

/// A colour row's place on the garage's strip of swatches; 9 is the stock cabin's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StripIdx(pub usize);

impl Default for StripIdx {
    fn default() -> Self {
        StripIdx(9)
    }
}

/// RGB to hue / saturation / value as F-47840 does it (the arithmetic order, the reciprocal of the
/// range, the fused multiply-subtract, hue turned by 1/6 last); bit-equal on 4,716 cases (map_hsv_matches_the_original).
fn rgb_hsv(c: [f32; 3]) -> [f32; 3] {
    let (r, g, b) = (c[0], c[1], c[2]);
    let mut v = r;
    if r < g {
        v = g;
    }
    if v < b {
        v = b;
    }
    let mut m = r;
    if r > g {
        m = g;
    }
    if m > b {
        m = b;
    }
    let d = v - m;
    if d == 0.0 {
        return [0.0, 0.0, v];
    }
    let s = d / v;
    let inv = 1.0 / d;
    let mut h = if r == v {
        let t = (v - g) * inv;
        (v - b).mul_add(inv, -t)
    } else if g == v {
        let t = (v - b) * inv;
        (v - r).mul_add(inv, -t) + 2.0
    } else {
        let t = (v - r) * inv;
        (v - g).mul_add(inv, -t) + 4.0
    };
    if h < 0.0 {
        h += 6.0;
    }
    [h * 0.16666667, s, v]
}

/// The part bytes of a record that the port keeps in `xparts` under its own keys:
/// (key, the record's byte). "intercooler" is the switch UseIntercooler - the garage's
/// "part" of the intercooler list, F-20669 - and "carbon_hood" CarbonFiberHood.
pub const XPART_BYTES: [(&str, &str); 5] = [("intercooler", "UseIntercooler"), ("bike_tail", "bike_tail"), ("bike_tank", "bike_tank"), ("bike_cowl", "bike_cowl"), ("carbon_hood", "CarbonFiberHood")];

/// The GPS button works (the original's `enableGps`: "mcMainState_EnableGpsButton", read
/// by the button's handler and the pause bar, F-25450). `base`: what the save says
/// (none: a save without the block); then the career's three events that set it, in the
/// career's order: RACE_GH_06's start switches it off, RACE_GH_02's and RACE_SH_01's on
/// (their READY_TO_START_ON_ENTER scripts). `career`: the career's tables are there.
pub fn gps_button(base: Option<bool>, career: bool, gh06: bool, gh02: bool, sh01: bool) -> bool {
    if !career {
        return base.unwrap_or(true);
    }
    if gh02 || sh01 {
        true
    } else if gh06 {
        false
    } else {
        base.unwrap_or(true)
    }
}

/// The tuning of the HUD's 3D GPS arrow (`tune/ui/0xf9e58c68`, the lines GPSArrow...; class
/// mcGPSArrow, registered by F-17004). Defaults = the disc's values.
#[derive(Clone, Debug, PartialEq)]
pub struct TopArrowTune {
    pub in_target: bool,
    pub scale: f32,
    pub position: [f32; 3],
    pub pivot: [f32; 3],
    pub color: [f32; 3],
    pub opacity: f32,
    pub ambient: [u8; 3],
    pub slow_in_out: bool,
    pub fovy: f32,
    pub lookat: [f32; 3],
    pub blit: [f32; 2],
    pub blit_scale: f32,
    /// (diffuse colour, the direction the light goes)
    pub lights: [([u8; 3], [f32; 3]); 2],
}

impl Default for TopArrowTune {
    fn default() -> Self {
        TopArrowTune {
            in_target: true,
            scale: 0.18000001,
            position: [0.0, 0.38500005, -1.8049988],
            pivot: [0.0, 0.0, 0.83333331],
            color: [0.0, 0.886275, 0.94902],
            opacity: 0.909804,
            ambient: [20, 20, 20],
            slow_in_out: true,
            fovy: 21.0,
            lookat: [0.0, 0.20000009, -1.0],
            blit: [0.0, -0.089999989],
            blit_scale: 1.0,
            lights: [([39, 176, 176], [-0.57735026, -0.57735026, -0.57735026]), ([173, 154, 173], [0.57735026, 0.57735026, -0.57735026])],
        }
    }
}

impl TopArrowTune {
    pub fn load(data_dir: &str) -> TopArrowTune {
        let mut t = TopArrowTune::default();
        let Ok(text) = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/ui/0xf9e58c68")) else { return t };
        // (a colour is a whole number, 0xAARRGGBB as a signed 32-bit one)
        let rgb = |s: &str| {
            let u = s.parse::<i64>().unwrap_or(0) as u32;
            [(u >> 16) as u8, (u >> 8) as u8, u as u8]
        };
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let Some(key) = it.next() else { continue };
            let words: Vec<&str> = it.collect();
            let v: Vec<f32> = words.iter().filter_map(|x| x.parse().ok()).collect();
            let v3 = |k: usize| [v.get(k).copied().unwrap_or(0.0), v.get(k + 1).copied().unwrap_or(0.0), v.get(k + 2).copied().unwrap_or(0.0)];
            if v.is_empty() {
                continue;
            }
            match key {
                "GPSArrowInRenderTarget" => t.in_target = v[0] != 0.0,
                "GPSArrowScale" => t.scale = v[0],
                "GPSArrowPosition" => t.position = v3(0),
                "GPSArrowPivot" => t.pivot = v3(0),
                // (red, green, blue and the opacity: F-47907 multiplies the fourth by the fade)
                "GPSArrowMaterialColor" if v.len() >= 4 => {
                    t.color = v3(0);
                    t.opacity = v[3];
                }
                "GPSArrowAmbientLight" => t.ambient = rgb(words[0]),
                "GPSArrowSlowInOut" => t.slow_in_out = v[0] != 0.0,
                "GPSArrowFovy" => t.fovy = v[0],
                "GPSArrowLookat" => t.lookat = v3(0),
                "GPSArrowBlitPosition" if v.len() >= 2 => t.blit = [v[0], v[1]],
                "GPSArrowBlitScale" => t.blit_scale = v[0],
                "GPSArrowLight0_Diffuse" => t.lights[0].0 = rgb(words[0]),
                "GPSArrowLight0_Direction" => t.lights[0].1 = v3(0),
                "GPSArrowLight1_Diffuse" => t.lights[1].0 = rgb(words[0]),
                "GPSArrowLight1_Direction" => t.lights[1].1 = v3(0),
                _ => {}
            }
        }
        t
    }
}

/// The square of the screen the top arrow's picture goes to, [x, y, width, height] in
/// pixels (F-29366): k = BlitScale x lines / 720, at least 1; from
/// y = BlitPosition.y x lines + 0.075 x lines, cut to a whole number, to y + 256 k, cut
/// likewise (255 lines high at 720); trunc(128 k x the screen's shape / the display's) to
/// either side of the middle (the control's +1676, set by F-92461: 16:9 for a wide
/// display, 4:3 otherwise).
/// (approx.) The original asks the console whether the display is wide; here a window wider
/// than 3:2 counts as wide.
pub fn top_arrow_rect(t: &TopArrowTune, width: f32, height: f32) -> [f32; 4] {
    let (cols, lines) = (width.trunc(), height.trunc());
    let k = (t.blit_scale * lines * (1.0 / 720.0)).max(1.0);
    let shape = cols / lines.max(1.0);
    let display = if shape > 1.5 { 16.0 / 9.0 } else { 4.0 / 3.0 };
    let half = (shape / display * k * 128.0).trunc();
    let y = t.blit[1] * lines + lines * 0.075;
    [cols * 0.5 - half, y.trunc(), 2.0 * half, (y + k * 256.0).trunc() - y.trunc()]
}

/// The arrow's model (`resources/ui/meshes/direction_arrow`, shader "Arrow"): its corners,
/// their normals, the share of the ambient light each takes, the triangles, and the
/// material's specular colour and power (`materialSpecular`, `shininess`).
pub struct TopArrowModel {
    pub pos: Vec<glam::Vec3>,
    pub nrm: Vec<glam::Vec3>,
    pub amb: Vec<f32>,
    pub tris: Vec<[u32; 3]>,
    pub specular: glam::Vec3,
    pub shininess: f32,
}

/// The model, read once. Its vertices carry two colours; the shader takes one (COLOR0) and
/// uses its red, green and blue as light of its own and its fourth number as the share of
/// the ambient light. The first of the two is (128, 128, 128) with 0, the second (0, 0, 0)
/// with 21..255 in its first byte.
/// (footage) Which of the two the shader gets was not read from the code; the second is
/// taken, the only one that gives the footage's colours (no red, faces away from both
/// lights nearly black) - the first would lay a half grey over all of it.
pub fn top_arrow_model(data_dir: &str) -> Option<&'static TopArrowModel> {
    static MODEL: std::sync::OnceLock<Option<TopArrowModel>> = std::sync::OnceLock::new();
    MODEL
        .get_or_init(|| {
            use mcla_formats::rage::drawable::ShaderParam;
            let m = crate::vehicle_model::load_simple(&std::path::Path::new(data_dir).join("resources/ui/meshes/direction_arrow.r102"), None).ok()?;
            let mut out = TopArrowModel { pos: Vec::new(), nrm: Vec::new(), amb: Vec::new(), tris: Vec::new(), specular: glam::Vec3::ONE, shininess: 30.0 };
            for (mesh, si) in &m.meshes {
                let base = out.pos.len() as u32;
                for (k, p) in mesh.positions.iter().enumerate() {
                    out.pos.push(*p);
                    out.nrm.push(mesh.normals.get(k).copied().filter(|n| n.is_finite() && n.length_squared() > 0.25).unwrap_or(glam::Vec3::Y));
                    out.amb.push(mesh.colors1.get(k).map_or(1.0, |c| c[0] as f32 / 255.0));
                }
                for tri in mesh.indices.chunks_exact(3) {
                    if tri.iter().all(|&k| (k as usize) < mesh.positions.len()) {
                        out.tris.push([base + tri[0], base + tri[1], base + tri[2]]);
                    }
                }
                // (the parameters by the hash of their names: materialSpecular, shininess)
                if let Some(sh) = m.shaders.get(*si as usize) {
                    for (h, p) in &sh.params {
                        if let ShaderParam::Vector(v) = p {
                            match (*h, v.first()) {
                                (0xfede494e, Some(v)) => out.specular = v.truncate(),
                                (0x111c9c95, Some(v)) => out.shininess = v.x,
                                _ => {}
                            }
                        }
                    }
                }
            }
            (!out.tris.is_empty()).then_some(out)
        })
        .as_ref()
}

/// The arrow's own frame (F-30300), rows x, y, z: z away from the target (`local`:
/// the target from the car, in the car's level frame - right, up, back; the car's matrix
/// with its z row laid flat, F-32331 - with its height, so the arrow tips up or down
/// at a target above or below), x = z x up, y = x x z. The arrow's point is at its -z end,
/// so it points at the target. The frame is a mirror (x = z x up, not up x z); the model is
/// the same on both sides, so only its lighting could tell, and the normals go through the
/// same frame.
pub fn top_arrow_axes(local: glam::Vec3) -> [glam::Vec3; 3] {
    let z = (-local).normalize_or(glam::Vec3::Z);
    let x = z.cross(glam::Vec3::Y).normalize_or(glam::Vec3::NEG_X);
    let y = x.cross(z).normalize_or(glam::Vec3::Y);
    [x, y, z]
}

/// One corner of the arrow as the vertex program hands it on: where it is in the target
/// (0..1 from the left and from the top), how far from the eye, its colour and opacity.
#[derive(Clone, Copy, Debug)]
pub struct TopArrowVertex {
    pub x: f32,
    pub y: f32,
    pub depth: f32,
    pub rgb: glam::Vec3,
    pub alpha: f32,
}

/// The arrow's corners in its target (F-74360 -> F-47907, the shader's
/// VS_TransformAndTextureColor).
/// Place: (corner + GPSArrowPivot) x GPSArrowScale, through the arrow's frame, +
/// GPSArrowPosition - the pivot is all but the arrow's point (-0.83 against -0.79), so it
/// turns about its point. Seen from the origin towards GPSArrowLookat (F-97951, up =
/// +y) through a lens of GPSArrowFovy, a square picture; near 0.01, far 100.
/// Colour: share x GPSArrowAmbientLight + GPSArrowMaterialColor x (the two lights' colours x
/// what of each falls on the normal; a light comes from against its GPSArrowLightN_Direction,
/// GPSArrowDirectional = 1: the directions count, not the positions) + the material's
/// specular colour x each light's colour x (normal . half way between light and eye) ^
/// shininess, where the light falls at all. The texture is `white`.
/// Opacity: 1 + (the material colour's fourth number x `alpha` - 1) x how far the face looks
/// at the eye (0.91 face on, 1 edge on).
pub fn top_arrow_vertices(t: &TopArrowTune, m: &TopArrowModel, local: [f32; 3], alpha: f32) -> Vec<TopArrowVertex> {
    use glam::Vec3;
    let [ax, ay, az] = top_arrow_axes(Vec3::from(local));
    let turn = |v: Vec3| ax * v.x + ay * v.y + az * v.z;
    let (pivot, pos) = (Vec3::from(t.pivot), Vec3::from(t.position));
    // (the camera: its z row is -Lookat, x = up x z, y = z x x; it sits at the origin)
    let cz = (-Vec3::from(t.lookat)).normalize_or(Vec3::Z);
    let cx = Vec3::Y.cross(cz).normalize_or(Vec3::X);
    let cy = cz.cross(cx);
    let tan = (t.fovy.to_radians() * 0.5).tan();
    let c8 = |c: [u8; 3]| Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32) / 255.0;
    let (hue, amb) = (Vec3::from(t.color), c8(t.ambient));
    let lights = t.lights.map(|(col, dir)| (c8(col), (-Vec3::from(dir)).normalize_or_zero()));
    let opacity = t.opacity * alpha;
    (0..m.pos.len())
        .map(|k| {
            let w = turn((m.pos[k] + pivot) * t.scale) + pos;
            let n = turn(m.nrm[k]).normalize_or_zero();
            let eye = (-w).normalize_or_zero();
            let mut rgb = amb * m.amb[k];
            for (col, l) in lights {
                let fall = n.dot(l).max(0.0);
                rgb += hue * col * fall;
                if fall > 0.0 {
                    rgb += m.specular * col * n.dot((eye + l).normalize_or_zero()).max(0.0).powf(m.shininess);
                }
            }
            let depth = -w.dot(cz);
            let d = (depth * tan).max(1e-6);
            TopArrowVertex { x: 0.5 + 0.5 * w.dot(cx) / d, y: 0.5 - 0.5 * w.dot(cy) / d, depth, rgb, alpha: 1.0 + (opacity - 1.0) * n.dot(eye).clamp(0.0, 1.0) }
        })
        .collect()
}

/// The arrow's target as a picture, `size` x `size`, four bytes a point (red, green, blue
/// not yet times the opacity, opacity) - what the blit's shader (ArrowBlit: colour / alpha,
/// alpha x the fade) hands to the screen. The nearest face wins at each point (the target
/// has a depth buffer); colours and opacity run evenly between a triangle's corners.
/// (approx.) The target's own size and edge smoothing were not read: the picture is made
/// at the size it is shown, each point from 3 x 3 samples. Faces behind others are taken as
/// hidden outright (the opacity is 0.91 or more), whatever the order of drawing and the
/// culling in force were.
pub fn top_arrow_picture(t: &TopArrowTune, m: &TopArrowModel, size: usize, local: [f32; 3]) -> Vec<u8> {
    const SS: usize = 3;
    let mut out = vec![0u8; size * size * 4];
    if size == 0 {
        return out;
    }
    let v = top_arrow_vertices(t, m, local, 1.0);
    let s = (size * SS) as f32;
    // (only the part of the target the arrow can reach is worked on)
    let seen: Vec<&TopArrowVertex> = v.iter().filter(|p| p.depth > 0.01).collect();
    if seen.is_empty() {
        return out;
    }
    let lo = |f: fn(&TopArrowVertex) -> f32| seen.iter().map(|p| f(p)).fold(f32::MAX, f32::min);
    let hi = |f: fn(&TopArrowVertex) -> f32| seen.iter().map(|p| f(p)).fold(f32::MIN, f32::max);
    let px0 = ((lo(|p| p.x) * size as f32).floor().max(0.0) as usize).min(size);
    let py0 = ((lo(|p| p.y) * size as f32).floor().max(0.0) as usize).min(size);
    let px1 = ((hi(|p| p.x) * size as f32).ceil().max(0.0) as usize + 1).min(size);
    let py1 = ((hi(|p| p.y) * size as f32).ceil().max(0.0) as usize + 1).min(size);
    if px1 <= px0 || py1 <= py0 {
        return out;
    }
    let (bw, bh) = ((px1 - px0) * SS, (py1 - py0) * SS);
    let (ox, oy) = ((px0 * SS) as f32, (py0 * SS) as f32);
    // (per sample: 1 / depth of the nearest face so far, its colour and opacity)
    let mut near = vec![0.0f32; bw * bh];
    let mut col = vec![[0.0f32; 4]; bw * bh];
    for tri in &m.tris {
        let [a, b, c] = tri.map(|k| v[k as usize]);
        if a.depth <= 0.01 || b.depth <= 0.01 || c.depth <= 0.01 || a.depth > 100.0 || b.depth > 100.0 || c.depth > 100.0 {
            continue;
        }
        let (x0, y0, x1, y1, x2, y2) = (a.x * s - ox, a.y * s - oy, b.x * s - ox, b.y * s - oy, c.x * s - ox, c.y * s - oy);
        let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
        if area.abs() < 1e-9 {
            continue;
        }
        let sx0 = x0.min(x1).min(x2).floor().max(0.0) as usize;
        let sy0 = y0.min(y1).min(y2).floor().max(0.0) as usize;
        let sx1 = (x0.max(x1).max(x2).ceil().max(0.0) as usize + 1).min(bw);
        let sy1 = (y0.max(y1).max(y2).ceil().max(0.0) as usize + 1).min(bh);
        let (ia, ib, ic) = (1.0 / a.depth, 1.0 / b.depth, 1.0 / c.depth);
        for y in sy0..sy1 {
            let gy = y as f32 + 0.5;
            for x in sx0..sx1 {
                let gx = x as f32 + 0.5;
                let w0 = ((x1 - gx) * (y2 - gy) - (x2 - gx) * (y1 - gy)) / area;
                let w1 = ((x2 - gx) * (y0 - gy) - (x0 - gx) * (y2 - gy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let inv = w0 * ia + w1 * ib + w2 * ic;
                let at = y * bw + x;
                if inv <= near[at] {
                    continue;
                }
                near[at] = inv;
                // (even in space, not on the screen: each corner's weight / its depth)
                let (p0, p1, p2) = (w0 * ia / inv, w1 * ib / inv, w2 * ic / inv);
                let rgb = (a.rgb * p0 + b.rgb * p1 + c.rgb * p2).clamp(glam::Vec3::ZERO, glam::Vec3::ONE);
                col[at] = [rgb.x, rgb.y, rgb.z, (a.alpha * p0 + b.alpha * p1 + c.alpha * p2).clamp(0.0, 1.0)];
            }
        }
    }
    for py in py0..py1 {
        for px in px0..px1 {
            let (mut sum, mut hit) = ([0.0f32; 4], 0.0f32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let at = ((py - py0) * SS + sy) * bw + (px - px0) * SS + sx;
                    if near[at] > 0.0 {
                        hit += 1.0;
                        for k in 0..4 {
                            sum[k] += col[at][k];
                        }
                    }
                }
            }
            if hit > 0.0 {
                let o = (py * size + px) * 4;
                for k in 0..3 {
                    out[o + k] = (sum[k] / hit * 255.0 + 0.5) as u8;
                }
                out[o + 3] = (sum[3] / (SS * SS) as f32 * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

/// The top arrow's fade (F-47090): towards 1 while it has a target, towards 0
/// without, 3 a second; it keeps the direction it had while it goes.
pub fn top_arrow_fade(x: &mut CarLive, local: Option<[f32; 3]>, dt: f32) {
    match local {
        Some(l) => {
            x.top_arrow.0 = (x.top_arrow.0 + 3.0 * dt).min(1.0);
            x.top_arrow.1 = l;
        }
        None => x.top_arrow.0 = (x.top_arrow.0 - 3.0 * dt).max(0.0),
    }
}

/// The top GPS arrow, drawn (ARROW: TOP). `local`: the target from the car, in the car's
/// level frame (right, up, back); none = no target, or another ARROW mode. As the original:
/// the arrow goes into a square picture of its own, the picture onto the screen with the
/// fade as its opacity.
pub fn top_arrow(x: &mut CarLive, data_dir: &str, ctx: &egui::Context, local: Option<[f32; 3]>, dt: f32) {
    static TUNE: std::sync::OnceLock<TopArrowTune> = std::sync::OnceLock::new();
    // (the picture on the card, and the size and direction it was last made for)
    static SHOWN: std::sync::Mutex<Option<(egui::TextureHandle, usize, [f32; 3])>> = std::sync::Mutex::new(None);
    top_arrow_fade(x, local, dt);
    if x.top_arrow.0 <= 0.0 {
        return;
    }
    let t = TUNE.get_or_init(|| TopArrowTune::load(data_dir));
    let Some(model) = top_arrow_model(data_dir) else { return };
    let screen = ctx.screen_rect();
    let ppp = ctx.pixels_per_point();
    let r = top_arrow_rect(t, screen.width() * ppp, screen.height() * ppp);
    let size = (r[2].max(r[3]) as usize).clamp(16, 2048);
    let Ok(mut shown) = SHOWN.lock() else { return };
    if !matches!(&*shown, Some((_, s, l)) if *s == size && *l == x.top_arrow.1) {
        let image = egui::ColorImage::from_rgba_unmultiplied([size, size], &top_arrow_picture(t, model, size, x.top_arrow.1));
        match shown.as_mut() {
            Some((tex, s, l)) => {
                tex.set(image, egui::TextureOptions::LINEAR);
                (*s, *l) = (size, x.top_arrow.1);
            }
            None => *shown = Some((ctx.load_texture("mc4_top_arrow", image, egui::TextureOptions::LINEAR), size, x.top_arrow.1)),
        }
    }
    let Some((tex, _, _)) = shown.as_ref() else { return };
    // (GPSArrowSlowInOut: the fade eased at both ends, (1 - cos(pi t)) / 2 - F-29366 at
    // A-82125, F-39265)
    let a = if t.slow_in_out { (1.0 - (x.top_arrow.0 * std::f32::consts::PI).cos()) * 0.5 } else { x.top_arrow.0 };
    let rect = egui::Rect::from_min_size(egui::pos2(r[0] / ppp, r[1] / ppp), egui::vec2(r[2] / ppp, r[3] / ppp));
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("mc4_top_arrow"))).image(tex.id(), rect, uv, egui::Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
}

/// One song of the disc's audio config (`audio/X360/config/game.dat`, objects of type 0x23).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MusicSong {
    /// what the save keeps of a song of the custom playlist (the object's word at +11)
    pub id: u32,
    /// 0 ECLECTIC .. 6 WEST_RAP (the object's byte at +10)
    pub genre: u8,
    /// the string table's key of "ARTIST - TITLE" (the object's text at +20)
    pub key: String,
    /// the object's own name ("HARD_ROCK_NIN_DISCIPLINE")
    pub object: String,
    /// the title as the string table has it (the object's name without one)
    pub title: String,
    /// the port's converted track of it
    pub file: Option<usize>,
}

/// One genre's play order (the original's 24-byte table, F-19442 ..): the songs'
/// places in the genre's list, `count` of them in use, and where the player is in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MusicOrder {
    pub arr: Vec<usize>,
    pub count: usize,
    pub pos: usize,
}

/// The original's music manager (mcMusicManager), as far as the playlists go.
/// Lists 0..6 are the genres, 7 is ALL, 8 the custom playlist ("ON THE FLY").
#[derive(Clone, Debug, Default)]
pub struct MusicLists {
    pub songs: Vec<MusicSong>,
    /// the lists: indices into `songs`. The custom one keeps what a removal leaves behind
    /// its end, as the original's array does; `counts` is what is in use.
    pub lists: [Vec<usize>; 9],
    pub counts: [usize; 9],
    pub order: [MusicOrder; 9],
    /// the list that plays, and the one before it (+52, +56)
    pub genre: usize,
    pub prev_genre: usize,
    /// the place in the list of the song that is on (+48), and the song (+68)
    pub cur: usize,
    pub now: Option<usize>,
    pub shuffle: bool,
    /// PLAY was chosen in the menu: the window's loop starts `now`
    pub play: bool,
    /// the original's generator (F-62480): a multiply-with-carry pair
    rng: (u32, u32),
}

/// The object names' genre prefixes, and the four songs whose converted track carries
/// another name than the object.
const MUSIC_PREFIX: [&str; 7] = ["HARD_ROCK_", "ROCK_", "ELECTRONIC_", "TECHNO_", "HIPHOP_", "WESTCOASTRAP_", "ECLECTIC_"];
const MUSIC_ALIAS: [(&str, &str); 4] = [
    ("THECHEMICALBROTHERS_MIDNIGHTMADNESS", "CHEMICALBROTHERS_MIDNIGHTMADNESS"),
    ("WOLFGANGGARTNER_SQUARES", "JOEYYOUNGMAN_SQUARES"),
    ("JOHNACQUAVIVAANDOLIVIERGIACOMOTTOE_GOODDOPAMINERMX", "JOHNACQUAVIVAANDOLIVIERGIACOMOTTOE_GOOD"),
    ("ANTIFORM_BOOMBOXSCREWFACEREMIX", "ANTIFORM_BOOMBOX"),
];

impl MusicLists {
    /// The manager as the original builds it (F-96264): the songs of MUSIC_0_MANAGER,
    /// then those of MUSIC_0_MANAGER_SC (their names start "SC_"; F-44823 takes them
    /// while bit 4 of the content word at 0x827E9174 is set, and F-68286 sets it at the
    /// start), each into its genre's list and into ALL (F-44823); the nine orders
    /// (F-18025; the custom one has room for every song); shuffle on.
    pub fn from_game_dat(d: &[u8]) -> Option<MusicLists> {
        let be = |o: usize| d.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        // (the file: a version, the objects' block and its size, a block of four bytes, then
        // the names: each a length, the text, and the object's place and size in the block)
        let mut p = 8 + be(4)? as usize;
        p += 4 + be(p)? as usize;
        let count = be(p)? as usize;
        p += 8;
        let mut objects: std::collections::HashMap<u32, (String, usize, usize)> = std::collections::HashMap::new();
        for _ in 0..count {
            let n = *d.get(p)? as usize;
            let name = String::from_utf8_lossy(d.get(p + 1..p + 1 + n)?).to_string();
            p += 1 + n;
            objects.insert(joaat(&name), (name, 8 + be(p)? as usize, be(p + 4)? as usize));
            p += 8;
        }
        let mut m = MusicLists { rng: (0x2545_F491, 0x1234_5678), ..Default::default() };
        if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            m.rng = (t.subsec_nanos() | 1, t.as_secs() as u32);
        }
        for manager in ["MUSIC_0_MANAGER", "MUSIC_0_MANAGER_SC"] {
            let Some((_, at, size)) = objects.get(&joaat(manager)) else { continue };
            let o = d.get(*at..*at + *size)?;
            // (seven lists from +42: a count, that many names' hashes - F-73467)
            let mut q = 42;
            for _ in 0..7 {
                let n = *o.get(q)? as usize;
                for k in 0..n {
                    let h = o.get(q + 1 + 4 * k..q + 5 + 4 * k).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))?;
                    let Some((name, at, size)) = objects.get(&h) else { continue };
                    let Some(s) = d.get(*at..*at + *size) else { continue };
                    if s.len() < 20 || s[10] > 6 {
                        continue;
                    }
                    let key = String::from_utf8_lossy(s.get(20..20 + s[19] as usize).unwrap_or(&[])).to_string();
                    let index = m.songs.len();
                    m.songs.push(MusicSong { id: u32::from_be_bytes([s[11], s[12], s[13], s[14]]), genre: s[10], key, object: name.clone(), title: name.replace('_', " "), file: None });
                    m.lists[s[10] as usize].push(index);
                    m.lists[7].push(index);
                }
                q += 1 + 4 * n;
            }
        }
        if m.songs.is_empty() {
            return None;
        }
        for g in 0..9 {
            m.counts[g] = m.lists[g].len();
            m.order[g].arr = (0..m.counts[if g == 8 { 7 } else { g }]).collect();
            m.resize(g, m.counts[g]);
        }
        m.set_shuffle(true);
        Some(m)
    }

    /// F-62480(lo, hi): the state x 0x5CDCFAA7 + the carry; its low 31 bits into the span.
    fn rand(&mut self, lo: usize, hi: usize) -> usize {
        let t = self.rng.0 as u64 * 0x5CDC_FAA7 + self.rng.1 as u64;
        self.rng = (t as u32, (t >> 32) as u32);
        lo + (self.rng.0 & 0x7FFF_FFFF) as usize % (hi - lo + 1)
    }

    /// F-22709: eight passes, each swapping every place in use with a random one; the
    /// player goes to the head of the order.
    fn shuffle_order(&mut self, g: usize) {
        let n = self.order[g].count.min(self.order[g].arr.len());
        for _ in 0..8 {
            for i in 0..n {
                let j = self.rand(0, n - 1);
                self.order[g].arr.swap(i, j);
            }
        }
        self.order[g].pos = 0;
    }

    /// F-90563: `n` places in use, the order straight again - and then shuffled: the
    /// orders are made with the flag for it (F-19442 from F-18025), whatever the
    /// shuffle option says.
    fn resize(&mut self, g: usize, n: usize) {
        self.order[g].count = n;
        for (i, v) in self.order[g].arr.iter_mut().enumerate() {
            *v = i;
        }
        self.shuffle_order(g);
    }

    /// F-48642. On: every order shuffled, and the place of the song that is on becomes
    /// the head of the playing list's order (the song itself goes on). Off: every order
    /// straight, the player where it was.
    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        for g in 0..9 {
            if on {
                self.shuffle_order(g);
            } else {
                for (i, v) in self.order[g].arr.iter_mut().enumerate() {
                    *v = i;
                }
            }
        }
        if on {
            if let Some(&first) = self.order[self.genre].arr.first() {
                self.cur = first;
            }
        }
    }

    /// F-66833: the list that plays (the original only complains about an empty one).
    pub fn set_genre(&mut self, g: usize) {
        let g = g.min(8);
        if g != self.genre {
            self.prev_genre = self.genre;
            self.genre = g;
        }
    }

    /// F-51486 / F-87381: the next (previous) place of the playing list's order,
    /// round and round. None while the list is empty.
    pub fn step(&mut self, forward: bool) -> Option<usize> {
        let g = self.genre;
        let o = &mut self.order[g];
        if o.count == 0 || o.arr.is_empty() {
            return None;
        }
        o.pos = if forward { (o.pos + 1) % o.count } else if o.pos == 0 { o.count - 1 } else { o.pos - 1 };
        self.cur = *o.arr.get(o.pos)?;
        self.now = self.lists[g].get(self.cur).copied();
        self.now
    }

    /// F-89347: the song at `index` of the playing list goes on; the order's player
    /// moves to where that index stands (F-36254).
    pub fn play_index(&mut self, index: usize) {
        let g = self.genre;
        if let Some(p) = self.order[g].arr.iter().position(|v| *v == index) {
            self.order[g].pos = p;
        }
        self.cur = index;
        self.now = self.lists[g].get(index).copied();
        self.play = true;
    }

    /// F-71612: the song is in the custom playlist.
    pub fn in_custom(&self, song: usize) -> bool {
        self.lists[8][..self.counts[8]].contains(&song)
    }

    /// F-92408: a song joins the custom playlist unless it is there; its order is made
    /// again for the new length (and so shuffled).
    pub fn add_custom(&mut self, song: usize) -> bool {
        if song >= self.songs.len() || self.in_custom(song) {
            return false;
        }
        let n = self.counts[8];
        if n < self.lists[8].len() {
            self.lists[8][n] = song;
        } else {
            self.lists[8].push(song);
        }
        self.counts[8] = n + 1;
        self.resize(8, n + 1);
        true
    }

    /// F-18610: the song with this id leaves the custom playlist, the later ones move
    /// up; its order is left as it was. With the last one gone the playing list becomes the
    /// one before.
    pub fn remove_custom(&mut self, id: u32) {
        let n = self.counts[8];
        let Some(i) = (0..n).find(|i| self.songs[self.lists[8][*i]].id == id) else { return };
        for j in i..n - 1 {
            self.lists[8][j] = self.lists[8][j + 1];
        }
        self.counts[8] = n - 1;
        if self.counts[8] == 0 {
            self.genre = self.prev_genre;
        }
    }

    /// F-34712: the song with this id, looked for in the lists 0..7.
    pub fn find(&self, id: u32) -> Option<usize> {
        (0..8).find_map(|g| self.lists[g].iter().copied().find(|s| self.songs[*s].id == id))
    }

    /// What a save holds, as F-55961 takes it up: the custom playlist's songs, the
    /// genre, shuffle.
    pub fn load(&mut self, custom: &[u32], genre: usize, shuffle: bool) {
        for id in custom {
            if let Some(s) = self.find(*id) {
                self.add_custom(s);
            }
        }
        self.set_genre(genre);
        self.set_shuffle(shuffle);
    }

    /// What the save keeps of the custom playlist (F-44753): its first 32 songs' ids.
    pub fn custom_ids(&self) -> Vec<u32> {
        self.lists[8][..self.counts[8]].iter().take(32).map(|s| self.songs[*s].id).collect()
    }

    /// The port's converted tracks, by their assets' names ("NIN_DISCIPLINE"), found for the
    /// songs: an object's name without its genre is the asset's, letters and digits alone.
    pub fn bind_files(&mut self, assets: &[String]) {
        let squash = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
        let by: std::collections::HashMap<String, usize> = assets.iter().enumerate().filter(|(_, a)| !a.is_empty()).map(|(i, a)| (squash(a), i)).collect();
        for s in &mut self.songs {
            let tail = MUSIC_PREFIX.iter().find_map(|p| s.object.strip_prefix(p)).unwrap_or(&s.object);
            let tail = MUSIC_ALIAS.iter().find(|a| a.0 == tail).map_or(tail, |a| a.1);
            s.file = by.get(&squash(tail)).copied();
        }
    }

    /// The song before that has a track: its file (MUSIC_DOWN).
    pub fn prev_file(&mut self) -> Option<usize> {
        for _ in 0..self.order[self.genre].count.max(1) {
            let s = self.step(false)?;
            if let Some(f) = self.songs[s].file {
                return Some(f);
            }
        }
        None
    }

    /// The next song that has a track: its file.
    pub fn next_file(&mut self) -> Option<usize> {
        for _ in 0..self.order[self.genre].count.max(1) {
            let s = self.step(true)?;
            if let Some(f) = self.songs[s].file {
                return Some(f);
            }
        }
        None
    }

    /// The file of the song that is on (PLAY).
    pub fn now_file(&self) -> Option<usize> {
        self.now.and_then(|s| self.songs[s].file)
    }

    /// The title of the song a file is.
    pub fn title_of_file(&self, file: usize) -> Option<String> {
        self.songs.iter().find(|s| s.file == Some(file)).map(|s| s.title.clone())
    }
}

/// The music lists as the game's running side shares them with the audio.
pub type MusicShared = std::sync::Arc<std::sync::Mutex<MusicLists>>;

/// The port's row number of a list (0 ALL, 1..7 the genres, 8 the custom playlist) as the
/// original's (0..6 the genres, 7 ALL, 8 custom), and back as the port's saved genre (the
/// custom playlist has no place there: ALL).
pub fn music_list(port: usize) -> usize {
    match port {
        0 => 7,
        1..=7 => port - 1,
        _ => 8,
    }
}

fn music_port_genre(g: usize) -> u8 {
    if g <= 6 { g as u8 + 1 } else { 0 }
}

/// The titles of a list's songs, in the list's order (the song form shows the manager's
/// own array: F-54463). None without the lists.
pub fn music_rows(x: &OptLive, port: usize) -> Option<Vec<String>> {
    let l = x.music.as_ref()?.lock().ok()?;
    let g = music_list(port);
    Some(l.lists[g][..l.counts[g]].iter().map(|s| l.songs[*s].title.clone()).collect())
}

/// Shuffle, as the manager has it.
pub fn music_shuffle(x: &OptLive) -> Option<bool> {
    Some(x.music.as_ref()?.lock().ok()?.shuffle)
}

/// X on MUSIC PLAYLISTS (F-37911, button 62): shuffle the other way.
pub fn music_toggle_shuffle(x: &OptLive) {
    if let Some(mut l) = x.music.as_ref().and_then(|m| m.lock().ok()) {
        let on = !l.shuffle;
        l.set_shuffle(on);
    }
}

/// The row MUSIC PLAYLISTS opens on: the list that plays (F-41165).
pub fn music_cursor(x: &OptLive) -> Option<usize> {
    Some(x.music.as_ref()?.lock().ok()?.genre)
}

/// A on a song (PLAY, F-37737): its list becomes the one that plays, the song goes
/// on. Gives the port's saved genre.
pub fn music_play(x: &OptLive, port: usize, row: usize) -> Option<u8> {
    let mut l = x.music.as_ref()?.lock().ok()?;
    let g = music_list(port);
    if l.counts[g] == 0 {
        return None;
    }
    l.set_genre(g);
    let row = row.min(l.counts[g] - 1);
    l.play_index(row);
    Some(music_port_genre(l.genre))
}

/// Y on a song (F-22536, button 63): into the custom playlist, or out of it when it
/// is there. Gives the port's saved genre (the playing list changes when the playlist
/// runs empty).
pub fn music_add_remove(x: &OptLive, port: usize, row: usize) -> Option<u8> {
    let mut l = x.music.as_ref()?.lock().ok()?;
    let g = music_list(port);
    if l.counts[g] == 0 {
        return None;
    }
    let song = l.lists[g][row.min(l.counts[g] - 1)];
    if !l.add_custom(song) {
        let id = l.songs[song].id;
        l.remove_custom(id);
    }
    Some(music_port_genre(l.genre))
}

/// The song under the cursor is in the custom playlist (the Y prompt: F-31312).
pub fn music_in_custom(x: &OptLive, port: usize, row: usize) -> bool {
    let Some(l) = x.music.as_ref().and_then(|m| m.lock().ok()) else { return false };
    let g = music_list(port);
    l.counts[g] > 0 && l.in_custom(l.lists[g][row.min(l.counts[g] - 1)])
}

/// The port's own list of genres (no menu movie) picks one: the list that plays.
pub fn music_set_genre(x: &OptLive, port: usize) {
    if let Some(mut l) = x.music.as_ref().and_then(|m| m.lock().ok()) {
        let g = music_list(port);
        if l.counts[g] > 0 {
            l.set_genre(g);
        }
    }
}

/// One controller map of the disc's list (`tune/controller/0xdcd4b916`): its place in the
/// list is the options' `control_config_*`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlMap {
    /// "Throttle_Trigger" - also the string table's key of the style's name
    pub name: String,
    /// "xenon_controls_trigger" (the file is `<that>.map`, by its name's hash)
    pub file: String,
    /// Transmission Manual
    pub manual: bool,
    /// action ("GAS") -> (device: 0 none, 8 button, 9 axis; key)
    pub slots: BTreeMap<String, (u32, u32)>,
}

/// The map files' names in the extracted folder: the hash of "<name>.map"; the trigger
/// map's file does not follow from its name and is the one left over.
const CONTROL_FILES: [(&str, &str); 1] = [("xenon_controls_trigger", "0x0c7fefe3")];

/// A map file's entries: `<ACTION>_DVC n` / `<ACTION>_KEY n` (one action is spelled `_DEV`).
pub fn control_map_slots(text: &str) -> BTreeMap<String, (u32, u32)> {
    let mut out: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(val)) = (it.next(), it.next().and_then(|v| v.parse::<u32>().ok())) else { continue };
        if let Some(a) = key.strip_suffix("_DVC").or_else(|| key.strip_suffix("_DEV")) {
            out.entry(a.to_string()).or_default().0 = val;
        } else if let Some(a) = key.strip_suffix("_KEY") {
            out.entry(a.to_string()).or_default().1 = val;
        }
    }
    out
}

/// The disc's controller maps, in the list's order (empty without the data).
pub fn control_maps(data_dir: &str) -> Vec<ControlMap> {
    let dir = std::path::Path::new(data_dir).join("tune/controller");
    let Ok(list) = std::fs::read_to_string(dir.join("0xdcd4b916")) else { return Vec::new() };
    let mut out: Vec<ControlMap> = Vec::new();
    for line in list.lines() {
        let mut it = line.split_whitespace();
        match (it.next(), it.next()) {
            (Some("Map"), None) => out.push(ControlMap::default()),
            (Some("Filename"), Some(v)) => {
                if let Some(m) = out.last_mut() {
                    m.file = v.to_string();
                }
            }
            (Some("Name"), Some(v)) => {
                if let Some(m) = out.last_mut() {
                    m.name = v.to_string();
                }
            }
            (Some("Transmission"), Some(v)) => {
                if let Some(m) = out.last_mut() {
                    m.manual = v.eq_ignore_ascii_case("Manual");
                }
            }
            _ => {}
        }
    }
    for m in &mut out {
        let named = CONTROL_FILES.iter().find(|f| f.0 == m.file).map(|f| f.1.to_string());
        let file = named.unwrap_or_else(|| format!("0x{:08x}", joaat(&format!("{}.map", m.file))));
        if let Ok(text) = std::fs::read_to_string(dir.join(file)) {
            m.slots = control_map_slots(&text);
        }
    }
    out
}

/// The pad as the original's poll has it (F-31331): sixteen buttons (bit = the maps'
/// button key: 0 LT, 1 RT, 2 LB, 3 RB, 4 Y, 5 B, 6 A, 7 X, 8 BACK, 9 L3, 10 R3, 11 START,
/// 12..15 the d-pad up, right, down, left), the two triggers' own bytes, and the sticks'
/// bytes LX, LY, RX, RY (128 the centre; 0 is left, and up).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    pub buttons: u16,
    pub triggers: [u8; 2],
    pub sticks: [u8; 4],
}

impl PadState {
    /// From sticks -1..1 (x to the right, y up), triggers 0..1 and the buttons' bits. The
    /// triggers' own bits are set from 64 of 255 on, as the poll does.
    pub fn new(sticks: [f32; 4], triggers: [f32; 2], buttons: u16) -> PadState {
        let byte = |v: f32| (127.5 + v.clamp(-1.0, 1.0) * 127.5).round().clamp(0.0, 255.0) as u8;
        let t = triggers.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let mut buttons = buttons & !3;
        for k in 0..2 {
            if t[k] >= 64 {
                buttons |= 1 << k;
            }
        }
        PadState { buttons, triggers: t, sticks: [byte(sticks[0]), byte(-sticks[1]), byte(sticks[2]), byte(-sticks[3])] }
    }

    /// A slot's byte (the mapper's update, F-66987). Device 8, a button: 255 while it is
    /// down - a trigger gives its own byte while that is not zero. Device 9, an axis: 0..3 the
    /// sticks' bytes; 4..7 (12..15) the left (right) stick as four switches, up / down / left
    /// / right - past 0.8 of the way with the other axis under 0.57; 20 / 21 the d-pad as two
    /// axes; 22..25 (26..29) the left (right) stick's four half-axes, up / down / left / right.
    pub fn byte(&self, device: u32, key: u32) -> u8 {
        let half = |b: u8, low: bool| if low { 255i32 - 2 * b as i32 } else { 2 * b as i32 - 255 }.clamp(0, 255) as u8;
        let signed = stick_float;
        let switch = |main: u8, other: u8, low: bool| {
            let (m, o) = (signed(main), signed(other));
            if (if low { -m } else { m }) > 0.8 && o.abs() < 0.57 { 255 } else { 0 }
        };
        let s = &self.sticks;
        match (device, key) {
            (8, 0 | 1) if self.triggers[key as usize] != 0 => self.triggers[key as usize],
            (8, 0..=15) => if self.buttons >> key & 1 != 0 { 255 } else { 0 },
            (9, 0..=3) => s[key as usize],
            (9, 4) => switch(s[1], s[0], true),
            (9, 5) => switch(s[1], s[0], false),
            (9, 6) => switch(s[0], s[1], true),
            (9, 7) => switch(s[0], s[1], false),
            (9, 12) => switch(s[3], s[2], true),
            (9, 13) => switch(s[3], s[2], false),
            (9, 14) => switch(s[2], s[3], true),
            (9, 15) => switch(s[2], s[3], false),
            (9, 20) => if self.buttons >> 15 & 1 != 0 { 0 } else if self.buttons >> 13 & 1 != 0 { 255 } else { 128 },
            (9, 21) => if self.buttons >> 12 & 1 != 0 { 0 } else if self.buttons >> 14 & 1 != 0 { 255 } else { 128 },
            (9, 22) => half(s[1], true),
            (9, 23) => half(s[1], false),
            (9, 24) => half(s[0], true),
            (9, 25) => half(s[0], false),
            (9, 26) => half(s[3], true),
            (9, 27) => half(s[3], false),
            (9, 28) => half(s[2], true),
            (9, 29) => half(s[2], false),
            _ => 0,
        }
    }
}

/// The active map's slots: action -> (device, key).
pub type ControlSlots = BTreeMap<String, (u32, u32)>;

/// An action's value (F-33017): a button's byte / 255 past a dead zone of 0.05; the
/// stick and d-pad axes (keys 0..3, 20, 21) signed about the centre, the other axis keys
/// byte / 255, past a dead zone of 0.1. Past the dead zone the rest is stretched to the
/// whole (F-93983). 0 for an action the map does not bind.
pub fn pad_analog(p: &PadState, slots: &ControlSlots, action: &str) -> f32 {
    let Some(&(device, key)) = slots.get(action) else { return 0.0 };
    let b = p.byte(device, key) as f32;
    let (v, dead) = match (device, key) {
        (8, _) => (b / 255.0, 0.05),
        (9, 0..=3 | 20 | 21) => ((b - 127.5) / 127.5, 0.1),
        (9, _) => (b / 255.0, 0.1),
        _ => return 0.0,
    };
    if v.abs() <= dead { 0.0 } else { v.signum() * (v.abs() - dead) / (1.0 - dead) }
}

/// An action is down (F-70872): its byte above 127.
pub fn pad_down(p: &PadState, slots: &ControlSlots, action: &str) -> bool {
    slots.get(action).map_or(false, |&(device, key)| device != 0 && p.byte(device, key) > 127)
}

/// Manual gears (F-31769): the map binds both SHIFT_UP and SHIFT_DOWN. (The list's own
/// "Transmission" line is read by nothing.)
pub fn pad_manual(slots: &ControlSlots) -> bool {
    ["SHIFT_UP", "SHIFT_DOWN"].iter().all(|a| slots.get(*a).map_or(false, |s| s.0 != 0))
}

/// The map the pad is read through (F-92127): with the hydraulics switched on the
/// list's ninth, whatever the style; else the style's - LocalOptions + 0, Throttle_Trigger
/// for a save without the block - and for the custom style the save's own mapping (every
/// slot from the save; a mapping never filled starts as the list's file for it, the stick's,
/// as F-78614 fills a new profile's).
pub fn control_slots(maps: &[ControlMap], options: Option<&Options>, hydraulics: bool) -> ControlSlots {
    let style = options.map_or(1, |o| o.control_config_pad).clamp(0, 8) as usize;
    if hydraulics {
        return maps.get(8).map(|m| m.slots.clone()).unwrap_or_default();
    }
    if style == 4 {
        if let Some(o) = options.filter(|o| !o.custom_mapping_pad.is_empty()) {
            return o
                .custom_mapping_pad
                .iter()
                .map(|(action, m)| {
                    let device = match m.device.as_str() {
                        "button" => 8,
                        "axis" => 9,
                        d => d.parse().unwrap_or(0),
                    };
                    (action.clone(), (device, m.key))
                })
                .collect();
        }
    }
    maps.get(style).map(|m| m.slots.clone()).unwrap_or_default()
}

/// A on a row of CONTROL STYLE (F-36113): the style is the pad's from now on. The
/// wheel's rows and the row that is set already do nothing.
pub fn control_set(mc4: &mut Option<Mc4>, style: i32, fresh: bool) -> bool {
    if !(0..=4).contains(&style) || mc4.as_ref().map_or(1, |m| m.options.control_config_pad) == style {
        return false;
    }
    let o = &mut mc4.get_or_insert_with(|| Mc4::seeded(fresh)).options;
    o.control_config_pad = style;
    o.control_config_pad_name = CONTROL_CONFIGS.get(style as usize).map(|s| s.to_string());
    true
}

// ---------------------------------------------------------------------------------------
// step 13b: the custom mapping editor and viewer (mcControllerCustomizeForm), the options'
// "save changes" for the controller, the look-back rule of the pad
// ---------------------------------------------------------------------------------------

/// The actions' places in a mapping (table 0x8204FF90): 57 of the 74 slots have a name.
pub const CONTROL_ACTIONS: [(usize, &str); 57] = [
    (0, "STEER_AXIS"), (1, "STEER_AXIS_UP_DN"), (2, "GAS"), (3, "BRAKE"), (4, "SHIFT_UP"), (5, "SHIFT_DOWN"), (6, "HANDBRAKE"), (7, "WEIGHT_XFER"),
    (8, "NITRO"), (9, "HEADLIGHTS"), (10, "HORN"), (11, "IN_ZONE"), (12, "CAM_UP"), (13, "CAM_DOWN"), (14, "MAP"), (15, "CAM_ZOOM_IN"),
    (16, "CAM_ZOOM_OUT"), (17, "PAUSE"), (18, "TALK"), (19, "MUSIC_UP"), (20, "MUSIC_DOWN"), (21, "MUSIC_PAUSE"), (22, "CAM_PAN_LEFT"),
    (23, "CAM_PAN_RIGHT"), (24, "CAM_PAN_UP"), (25, "CAM_PAN_DOWN"), (26, "LOOKBACK"), (27, "CAMERA_LEFT"), (28, "CAMERA_RIGHT"), (29, "CAMERA_UP"),
    (30, "POPUP_LEFT"), (31, "POPUP_RIGHT"), (32, "POPUP_UP"), (33, "POPUP_DOWN"), (34, "POPUP_ANALOG_LEFT_RIGHT"), (35, "POPUP_ANALOG_UP_DOWN"),
    (36, "POPUP_SELECT"), (37, "POPUP_BACK"), (38, "WHEEL_FRONT_LEFT"), (39, "WHEEL_FRONT_RIGHT"), (40, "WHEEL_BACK_LEFT"), (41, "WHEEL_BACK_RIGHT"),
    (42, "HUD_PLAYERLIST"), (43, "CONVERTIBLE_UP_DN"), (44, "HYDRAULICS"), (45, "UI_START"), (46, "HUD_CYCLE"), (64, "PHOTO_CAM_MOVE_UP"),
    (65, "PHOTO_CAM_MOVE_DN"), (66, "PHOTO_CAM_MOVE_LT"), (67, "PHOTO_CAM_MOVE_RT"), (68, "PHOTO_CAM_ZOOM_IN"), (69, "PHOTO_CAM_ZOOM_OUT"),
    (70, "PHOTO_CAM_PAN_UP"), (71, "PHOTO_CAM_PAN_DN"), (72, "PHOTO_CAM_PAN_LT"), (73, "PHOTO_CAM_PAN_RT"),
];

/// The importer's names of the sixteen buttons (`button_guess` of a saved slot).
const BUTTON_NAMES: [&str; 16] = ["L2", "R2", "L1", "R1", "RUP", "RRIGHT", "RDOWN", "RLEFT", "SELECT", "L3", "R3", "START", "LUP", "LRIGHT", "LDOWN", "LLEFT"];

/// A mapping as the original holds it: 74 x (device, key), by slot.
pub type SlotMap = [(u32, u32); 74];

/// The editor's rows: slots 0..44 in the table's order without the hidden ones.
pub const CONTROL_ROWS: [usize; 23] = [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 19, 20, 21, 26, 27, 28, 29, 42, 43, 44];

/// The pause menu's controller pages (the port's page numbers): the page with GAMEPAD /
/// KEYBOARD / RACING WHEEL (the port's own, asked for by the user), the original's CONTROL
/// STYLE list, the mapping editor / viewer, and the two pages that are not there yet.
pub const PAGE_CONTROLLER: u8 = 5;
pub const PAGE_STYLE: u8 = 70;
pub const PAGE_EDIT: u8 = 71;
pub const PAGE_KEYBOARD: u8 = 72;
pub const PAGE_WHEEL: u8 = 73;
/// The port's own pages of OPTIONS (DECISIONS.md D19): DISPLAY (a placeholder) and PERFORMANCE.
pub const PAGE_DISPLAY: u8 = 74;
pub const PAGE_PERFORMANCE: u8 = 75;

fn action_name(slot: usize) -> Option<&'static str> {
    CONTROL_ACTIONS.iter().find(|a| a.0 == slot).map(|a| a.1)
}

/// A map's named slots as the 74.
pub fn slots_array(s: &ControlSlots) -> SlotMap {
    let mut out = [(0, 0); 74];
    for (slot, name) in CONTROL_ACTIONS {
        if let Some(v) = s.get(name) {
            out[slot] = *v;
        }
    }
    out
}

/// A saved mapping (`custom_mapping_pad`) as the 74.
pub fn mapping_array(m: &BTreeMap<String, Mapping>) -> SlotMap {
    let mut out = [(0, 0); 74];
    for (name, e) in m {
        let slot = CONTROL_ACTIONS.iter().find(|a| a.1 == name).map_or(e.slot as usize, |a| a.0);
        let device = match e.device.as_str() {
            "button" => 8,
            "axis" => 9,
            d => d.parse().unwrap_or(0),
        };
        if slot < 74 {
            out[slot] = (device, e.key);
        }
    }
    out
}

/// The 74 into a saved mapping, as the importer writes one (`tools/mc4save/options.py`,
/// `_mapping`): every named action with its slot, device word and key, a button's name
/// beside it; a slot without a name only while it holds something.
pub fn mapping_store(m: &mut BTreeMap<String, Mapping>, a: &SlotMap) {
    for slot in 0..74 {
        let (device, key) = a[slot];
        let name = match action_name(slot) {
            Some(n) => n.to_string(),
            None => {
                let n = format!("slot{}", slot);
                if (device, key) == (0, 0) {
                    m.remove(&n);
                    continue;
                }
                n
            }
        };
        let e = m.entry(name).or_default();
        e.slot = slot as u32;
        e.device = match device {
            0 => "none".to_string(),
            8 => "button".to_string(),
            9 => "axis".to_string(),
            d => d.to_string(),
        };
        e.key = key;
        e.button_guess = (device == 8 && key < 16).then(|| BUTTON_NAMES[key as usize].to_string());
    }
}

/// A slot the editor does not show (F-36505): the lean axis, CAM_DOWN, the zooms, PAUSE,
/// TALK, the pans, the POPUP_* and the hydraulics' four wheels.
pub fn control_hidden(slot: usize) -> bool {
    matches!(slot, 1 | 13 | 15 | 16 | 17 | 18 | 22..=25 | 30..=41)
}

/// F-26260: what sits on one of the pad's six axes is taken off every slot 0..44 -
/// 0 / 1 the left stick's x / y (the axis, its two half-axes and its two switches), 2 / 3
/// the right stick's, 20 / 21 the d-pad's left-right / up-down (the axis and its two
/// buttons, by key or by mask). Any other number takes nothing off.
pub fn control_clear(m: &mut SlotMap, axis: u32) {
    for s in 0..45 {
        let (d, k) = m[s];
        let hit = match axis {
            0 => d == 9 && matches!(k, 6 | 7 | 0 | 24 | 25),
            1 => d == 9 && matches!(k, 4 | 5 | 1 | 22 | 23),
            2 => d == 9 && matches!(k, 14 | 15 | 2 | 28 | 29),
            3 => d == 9 && matches!(k, 12 | 13 | 3 | 26 | 27),
            20 => (d == 8 && matches!(k, 15 | 13)) || (d == 6 && matches!(k, 32768 | 8192)) || (d == 9 && k == 20),
            21 => (d == 8 && matches!(k, 12 | 14)) || (d == 6 && matches!(k, 4096 | 16384)) || (d == 9 && k == 21),
            _ => false,
        };
        if hit {
            m[s] = (0, 0);
        }
    }
}

/// The editor's assignment (F-23125): (device, key) for the action in `slot` of the
/// working copy. False = refused (the "INVALID CHOICE" box), the mapping untouched.
/// - keys 8 and 11 never (BACK and START - by the key's number alone, whatever the device);
/// - STEER_AXIS: an axis only, and not the one GAS or BRAKE is on; the left stick (key 0 or
///   1) takes STEER_AXIS_UP_DN with it and clears what else sat on the stick; on a pad the
///   d-pad as well (key 20 or 21), the same way; nothing else;
/// - GAS / BRAKE: an axis - not the left stick's, the d-pad's or one STEER_AXIS /
///   STEER_AXIS_UP_DN has by key; the right stick (2 or 3) gives the pair, GAS up and BRAKE
///   down, and clears the stick's other users. A trigger gives the pair too - the pressed
///   one to the row that is edited, the other to the other pedal - and clears every other
///   slot on a trigger. Anything else (pad only): assigned alone; whoever had it loses it
///   (hidden slots too), and for an axis the slots on d-pad buttons are cleared;
/// - any other action: not the left stick while STEER_AXIS is an axis; not the right stick
///   while GAS or BRAKE is an axis; not a d-pad button while STEER_AXIS has key 20; not LB /
///   RB while GAS or BRAKE has key 2 or 3; else LOOKBACK not on a d-pad button (a wheel: not
///   a trigger while the pedals are the triggers). Accepted: every other SHOWN slot with
///   the same (device, key) is emptied - no swap - and hidden slots are left alone.
pub fn control_assign(device: u32, key: u32, slot: usize, m: &mut SlotMap, wheel: bool) -> bool {
    if key == 11 || key == 8 {
        return false;
    }
    // (F-74264 is "false" for every slot; what is left of its test)
    if key == 1 && device == 0 {
        return true;
    }
    if slot == 0 {
        if device != 9 || (m[2].0 == 9 && m[2].1 == key) || (m[3].0 == 9 && m[3].1 == key) {
            return false;
        }
        let (x, y) = match key {
            0 | 1 => (0, 1),
            20 | 21 if !wheel => (20, 21),
            _ => return false,
        };
        control_clear(m, x);
        control_clear(m, y);
        m[0] = (9, x);
        m[1] = (9, y);
        return true;
    }
    if slot == 2 || slot == 3 {
        if device == 9 {
            if matches!(key, 20 | 21 | 0 | 1) || m[0].1 == key || m[1].1 == key {
                return false;
            }
            if key == 2 || key == 3 {
                // (28, 29, 26 and 27 are asked for too; F-26260 knows no such number)
                for axis in [2, 28, 29, 3, 26, 27] {
                    control_clear(m, axis);
                }
                m[2] = (9, 26);
                m[3] = (9, 27);
                return true;
            }
        } else if device == 8 && (key == 0 || key == 1) {
            let gas = if slot == 2 { key } else { 1 - key };
            m[2] = (8, gas);
            m[3] = (8, 1 - gas);
            for s in 0..45 {
                if s != 2 && s != 3 && (m[s].0 == 8 || m[s].0 == 6) && (m[s].1 == 0 || m[s].1 == 1) {
                    m[s] = (0, 0);
                }
            }
            return true;
        }
        if wheel {
            return false;
        }
        for s in 0..45 {
            if s != slot && m[s] == (device, key) {
                m[s] = (0, 0);
            } else if device == 9 && m[s].0 == 8 && (12..=15).contains(&m[s].1) {
                m[s] = (0, 0);
            }
        }
        m[slot] = (device, key);
        return true;
    }
    if device == 9 && m[0].0 == 9 && matches!(key, 0 | 1 | 4..=7 | 9 | 10 | 22..=25) {
        return false;
    }
    if device == 9 && (m[2].0 == 9 || m[3].0 == 9) && matches!(key, 2 | 3 | 12..=19 | 26..=29) {
        return false;
    }
    let dpad = (12..=15).contains(&key);
    if device == 8 && dpad && m[0].1 == 20 {
        return false;
    }
    let (brake, gas) = (m[3].1, m[2].1);
    if matches!(brake, 2 | 3) || matches!(gas, 2 | 3) {
        if (device == 8 || device == 6) && (key == 2 || key == 3) {
            return false;
        }
    } else if wheel && (matches!(brake, 0 | 1) || matches!(gas, 0 | 1)) {
        if (device == 8 || device == 6) && (key == 0 || key == 1) {
            return false;
        }
    } else if device == 8 && dpad && slot == 26 {
        return false;
    }
    for s in 0..45 {
        if s != slot && !control_hidden(s) && m[s] == (device, key) {
            m[s] = (0, 0);
        }
    }
    if slot < 74 {
        m[slot] = (device, key);
    }
    true
}

/// A stick byte as a float (F-73799 the right stick's X, F-43402 its Y): (byte - 127.5) * 1/127 as an f32 (3c010204; not
/// the literal 0.00787402, one ulp off, and not 127.5), held to -1..1. The mapper's switch tests (past 0.8 / under 0.57) and the poll read it so.
pub fn stick_float(b: u8) -> f32 {
    ((b as f32 - 127.5) * (1.0f32 / 127.0)).clamp(-1.0, 1.0)
}

/// The slots of the analog actions STEER_AXIS, GAS and BRAKE (the predicate F-56931).
pub fn is_analog_slot(slot: usize) -> bool {
    matches!(slot, 0 | 2 | 3)
}

/// What the editor takes as the new button while it waits (F-56470), from the pad now
/// and the buttons that were down the frame before. In this order -
/// an action that is no axis (every one but STEER_AXIS, GAS, BRAKE): the lowest newly
/// pressed button (LT, RT, LB, RB, Y, B, A, X, BACK, L3, R3, START, d-pad up, right, down,
/// left; a trigger counts from a quarter of its way) as (8, button); else a stick past half
/// its way as its half-axis - left x (9, 24 / 25), left y (9, 22 / 23), right x (9, 28 /
/// 29), right y (9, 26 / 27);
/// STEER_AXIS, GAS, BRAKE: for the two pedals a newly pressed LB (8, 2) or RB (8, 3) first;
/// then a stick more than 0.3 off its centre as the whole axis, (9, 0..3) in the order left
/// x, left y, right x, right y; then a newly pressed d-pad left or right as (9, 20), up or
/// down as (9, 21); then the lowest newly pressed button.
pub fn control_poll(slot: usize, now: &PadState, prev_buttons: u16) -> Option<(u32, u32)> {
    let new = now.buttons & !prev_buttons;
    let stick = now.sticks.map(stick_float);
    if !is_analog_slot(slot) {
        if new != 0 {
            return Some((8, new.trailing_zeros()));
        }
        for (i, (low, high)) in [(24, 25), (22, 23), (28, 29), (26, 27)].into_iter().enumerate() {
            if stick[i] < -0.5 {
                return Some((9, low));
            }
            if stick[i] > 0.5 {
                return Some((9, high));
            }
        }
        return None;
    }
    if slot != 0 {
        if new & 4 != 0 {
            return Some((8, 2));
        }
        if new & 8 != 0 {
            return Some((8, 3));
        }
    }
    // (the place the sticks are measured from, form + 2784.., is only ever written as 0)
    for i in 0..4 {
        if stick[i].abs() > 0.3 {
            return Some((9, i as u32));
        }
    }
    if new == 0 {
        None
    } else if new & 0xA000 != 0 {
        Some((9, 20))
    } else if new & 0x5000 != 0 {
        Some((9, 21))
    } else {
        Some((8, new.trailing_zeros()))
    }
}

/// An unbound row reads "MUST BE MAPPED" (F-88529): every one while exactly one of the
/// two shifts is bound; else the steering and the pedals, HANDBRAKE, NITRO, HEADLIGHTS,
/// IN_ZONE and MAP (and, of the slots no row shows, 15..18, 22..25, 30..41).
pub fn control_recommended(m: &SlotMap, slot: usize) -> bool {
    if (m[4] != (0, 0)) != (m[5] != (0, 0)) {
        return true;
    }
    slot < 4 || slot > 44 || matches!(slot, 6 | 8 | 9 | 11 | 14..=18 | 22..=25 | 30..=41)
}

/// What the BUTTON column shows for a slot (F-31353).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlLabel {
    /// nothing bound (or a device the column does not know)
    None,
    /// a button glyph: the frame of the movie's button icons
    Glyph(u32),
    /// a text, by its key in the string table (label table 0x8204FDB8)
    Text(&'static str),
}

pub fn control_label(device: u32, key: u32) -> ControlLabel {
    use ControlLabel::*;
    match (device, key) {
        (8, 0) => Glyph(7),
        (8, 1) => Glyph(8),
        (8, 2) => Glyph(5),
        (8, 3) => Glyph(6),
        (8, 4) => Glyph(4),
        (8, 5) => Glyph(2),
        (8, 6) => Glyph(1),
        (8, 7) => Glyph(3),
        (8, 8) => Glyph(10),
        (8, 9) => Glyph(29),
        (8, 10) => Glyph(28),
        (8, 11) => Glyph(9),
        (8, 12) => Text("DPad Up"),
        (8, 13) => Text("DPad Right"),
        (8, 14) => Text("DPad Down"),
        (8, 15) => Text("DPad Left"),
        (9, 0) => Text("Left Analog X"),
        (9, 1) => Text("Left Analog Y"),
        (9, 2) => Text("Right Analog X"),
        (9, 3) => Text("Right Analog Y"),
        (9, 4) => Text("Left Digital Up"),
        (9, 5) => Text("Left Digital Down"),
        (9, 6) => Text("Left Digital Left"),
        (9, 7) => Text("Left Digital Right"),
        (9, 12) => Text("Right Digital Up"),
        (9, 13) => Text("Right Digital Down"),
        (9, 14) => Text("Right Digital Left"),
        (9, 15) => Text("Right Digital Right"),
        (9, 20) => Glyph(21),
        (9, 21) => Glyph(20),
        (9, 22) => Glyph(32),
        (9, 23) => Glyph(33),
        (9, 24) => Glyph(31),
        (9, 25) => Glyph(30),
        (9, 26) => Glyph(36),
        (9, 27) => Glyph(37),
        (9, 28) => Glyph(35),
        (9, 29) => Glyph(34),
        _ => None,
    }
}

/// The pad's actions that are read as "down" by the window's loop, in the order of its bits.
pub const PAD_ACTS: [&str; 20] = ["HANDBRAKE", "WEIGHT_XFER", "NITRO", "WHEEL_FRONT_LEFT", "WHEEL_FRONT_RIGHT", "WHEEL_BACK_LEFT", "WHEEL_BACK_RIGHT", "MAP", "HYDRAULICS", "HEADLIGHTS", "CAM_UP", "CAM_DOWN", "MUSIC_UP", "MUSIC_DOWN", "IN_ZONE", "SHIFT_UP", "SHIFT_DOWN", "CONVERTIBLE_UP_DN", "HORN", "LOOKBACK"];

/// Looking back (F-75699): LOOKBACK down - or CAMERA_LEFT and CAMERA_RIGHT both down,
/// which forces it (the stick style: both triggers).
pub fn pad_lookback(p: &PadState, slots: &ControlSlots) -> bool {
    pad_down(p, slots, "LOOKBACK") || (pad_down(p, slots, "CAMERA_LEFT") && pad_down(p, slots, "CAMERA_RIGHT"))
}

/// The map as the pad is read this frame (F-75699): while looking back every action
/// but the four CAM_PAN_* that has the (device, key) of one of them reads nothing - the
/// d-pad then only pans (the song, map and hydraulics buttons on it do nothing).
pub fn pad_live(p: &PadState, slots: &ControlSlots) -> ControlSlots {
    let mut out = slots.clone();
    if pad_lookback(p, slots) {
        const PANS: [&str; 4] = ["CAM_PAN_LEFT", "CAM_PAN_RIGHT", "CAM_PAN_UP", "CAM_PAN_DOWN"];
        let pans: Vec<(u32, u32)> = PANS.iter().filter_map(|a| slots.get(*a).copied()).filter(|v| *v != (0, 0)).collect();
        for (name, v) in out.iter_mut() {
            if !PANS.contains(&name.as_str()) && pans.contains(v) {
                *v = (0, 0);
            }
        }
    }
    out
}

/// The bits of `PAD_ACTS` that are down, through `pad_live`.
pub fn pad_acts_down(p: &PadState, slots: &ControlSlots) -> u32 {
    let live = pad_live(p, slots);
    let mut down = 0u32;
    for (i, a) in PAD_ACTS.iter().enumerate() {
        if pad_down(p, &live, a) {
            down |= 1 << i;
        }
    }
    if pad_lookback(p, slots) {
        down |= 1 << 19;
    }
    down
}

/// The idle camera is ended by the pad (F-24075, its Idle view): CAM_UP, HYDRAULICS,
/// CAM_DOWN, LOOKBACK, MUSIC_DOWN, MUSIC_UP, MUSIC_PAUSE or MAP down, or CAMERA_LEFT /
/// CAMERA_RIGHT / CAMERA_UP above 0.2 (the constant at 0x82007A4C).
pub fn pad_wakes_idle(p: &PadState, slots: &ControlSlots) -> bool {
    ["CAM_UP", "HYDRAULICS", "CAM_DOWN", "LOOKBACK", "MUSIC_DOWN", "MUSIC_UP", "MUSIC_PAUSE", "MAP"].iter().any(|a| pad_down(p, slots, a))
        || ["CAMERA_LEFT", "CAMERA_RIGHT", "CAMERA_UP"].iter().any(|a| pad_analog(p, slots, a) > 0.2)
}

/// A box of the controller pages that is up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CtrlBox {
    /// "INVALID CHOICE" (F-32568): OK
    Invalid,
    /// "MISSING MAPPINGS" on the way out of the editor (F-53815): OK leaves, BACK stays
    Unmapped,
    /// "SET TO DEFAULT?" (event 62): YES / NO
    Defaults,
    /// "SAVE OPTIONS" on the way out of OPTIONS (F-92772): YES / NO
    Save,
    /// the keyboard page's two (the port's own page, `keybinds`): the same boxes
    KbdUnmapped,
    KbdDefaults,
}

/// The controller pages' state while OPTIONS is open (the original's forms 31..33).
#[derive(Clone, Debug)]
pub struct CtrlUi {
    /// OPTIONS is open: the mapping was loaded (F-94952 -> F-87567)
    pub open: bool,
    /// the style the save had when OPTIONS opened, and the one chosen since (SET)
    pub style_was: i32,
    pub style: i32,
    /// the editor's working copy (form + 5160) and the save's mapping it is compared with
    pub work: SlotMap,
    pub saved: SlotMap,
    /// the row of the style list the editor was opened for (+ 5156), and "view only" (+ 5152)
    pub row: usize,
    pub read_only: bool,
    /// waiting for a button (+ 2832); the first frame of it is passed over (+ 2833)
    pub polling: bool,
    pub poll_skip: bool,
    /// the pad's buttons the frame before
    pub prev_buttons: u16,
    pub box_kind: Option<CtrlBox>,
    /// frames counted for the test hook
    pub hook_frame: u32,
    pub hook_pad: Option<(PadState, u32)>,
    /// the keyboard's bindings and their page (the port's own, `keybinds`)
    pub kbd: crate::keybinds::KbdUi,
}

impl Default for CtrlUi {
    fn default() -> Self {
        CtrlUi { open: false, style_was: 1, style: 1, work: [(0, 0); 74], saved: [(0, 0); 74], row: 0, read_only: false, polling: false, poll_skip: false, prev_buttons: 0, box_kind: None, hook_frame: 0, hook_pad: None, kbd: Default::default() }
    }
}

/// OPTIONS opens: the style and the mapping as the save has them. A mapping never filled
/// is the custom style's own file (the list's fifth entry), as a new profile's is
/// (F-78614).
pub fn ctrl_open(c: &mut CtrlUi, maps: &[ControlMap], options: Option<&Options>) {
    c.style_was = options.map_or(1, |o| o.control_config_pad);
    c.style = c.style_was;
    c.saved = match options.filter(|o| !o.custom_mapping_pad.is_empty()) {
        Some(o) => mapping_array(&o.custom_mapping_pad),
        None => maps.get(4).map(|m| slots_array(&m.slots)).unwrap_or([(0, 0); 74]),
    };
    c.work = c.saved;
    c.polling = false;
    c.box_kind = None;
    c.open = true;
    // (the keyboard's working copy, the port's own)
    c.kbd.work = c.kbd.binds;
    c.kbd.polling = false;
}

/// Something of the controller's was changed since OPTIONS opened (its part of F-66374).
pub fn ctrl_changed(c: &CtrlUi) -> bool {
    c.open && (c.style != c.style_was || c.work != c.saved || c.kbd.work != c.kbd.binds)
}

/// "SAVE OPTIONS", YES (F-87944, result 3): an edited mapping is written to the save
/// and the style becomes the row the editor was opened for (F-57485 - only when the
/// working copy differs); then the style stands. True when the save's block was changed.
pub fn ctrl_commit(c: &mut CtrlUi, mc4: &mut Option<Mc4>, fresh: bool) -> bool {
    let mut wrote = false;
    // (the keyboard's bindings, the port's own: they are no part of the block - `SaveData::keyboard`)
    if c.open && c.kbd.work != c.kbd.binds {
        c.kbd.binds = c.kbd.work;
        wrote = true;
    }
    if c.open && c.work != c.saved {
        let o = &mut mc4.get_or_insert_with(|| Mc4::seeded(fresh)).options;
        mapping_store(&mut o.custom_mapping_pad, &c.work);
        c.style = c.row as i32;
        wrote = true;
    }
    if c.open && mc4.as_ref().map_or(1, |m| m.options.control_config_pad) != c.style {
        wrote |= control_set(mc4, c.style, fresh);
    }
    c.open = false;
    wrote
}

/// The mapping the editor shows: the working copy, or the list's map of the row it views.
pub fn ctrl_shown(c: &CtrlUi, maps: &[ControlMap]) -> SlotMap {
    if c.read_only {
        maps.get(c.row).map(|m| slots_array(&m.slots)).unwrap_or([(0, 0); 74])
    } else {
        c.work
    }
}

/// A row that must be mapped is not (form + 2816, set as the rows are filled).
pub fn ctrl_unmapped(m: &SlotMap) -> bool {
    CONTROL_ROWS.iter().any(|&s| m[s].0 == 0 && control_recommended(m, s))
}

/// A frame of the editor (F-23338): while it waits, the pad is looked at - not on the
/// first frame - and what comes is assigned. `Some(false)`: it was refused.
pub fn ctrl_poll_frame(c: &mut CtrlUi, pad: Option<&PadState>, slot: usize) -> Option<bool> {
    let mut out = None;
    if c.polling {
        if c.poll_skip {
            c.poll_skip = false;
        } else if let Some((device, key)) = pad.and_then(|p| control_poll(slot, p, c.prev_buttons)) {
            out = Some(control_assign(device, key, slot, &mut c.work, false));
            c.polling = false;
        }
    }
    c.prev_buttons = pad.map_or(0, |p| p.buttons);
    out
}

/// A page of the controller menus, ready for the pause menu: its rows as label, value (with
/// the menu movie's marks) and "can be chosen"; the rows' `property1`; the prompts; the
/// column headers of the editor.
#[derive(Clone, Debug, Default)]
pub struct CtrlPage {
    pub title: String,
    pub rows: Vec<(String, String, bool)>,
    pub props: Vec<i32>,
    pub prompts: Vec<(String, i32)>,
    pub headers: Vec<String>,
}

/// What the test hook presses this frame (MCLA_CTRL_TEST).
#[derive(Clone, Copy, Debug, Default)]
pub struct CtrlHook {
    pub up: bool,
    pub down: bool,
    pub enter: bool,
    pub back: bool,
    pub x: bool,
    pub y: bool,
}

/// Test hook, for runs without a pad: MCLA_CTRL_TEST="frame:what,frame:what,..." - frames
/// counted while an OPTIONS page is up; `what` is up, down, a, b, x, y, or pad=D.K - the
/// pad's button K (D 8) or stick axis K pushed to its high end (D 9; "9.K-" its low end)
/// for four frames. With the hook set a pad at rest stands in for a missing one.
pub fn ctrl_hook(x: &mut OptLive) -> CtrlHook {
    let mut out = CtrlHook::default();
    let Ok(script) = std::env::var("MCLA_CTRL_TEST") else { return out };
    let frame = x.ctrl.hook_frame;
    x.ctrl.hook_frame += 1;
    for step in script.split(',') {
        let Some((at, what)) = step.trim().split_once(':') else { continue };
        if at.trim().parse::<u32>().ok() != Some(frame) {
            continue;
        }
        match what.trim() {
            "up" => out.up = true,
            "down" => out.down = true,
            "a" => out.enter = true,
            "b" => out.back = true,
            "x" => out.x = true,
            "y" => out.y = true,
            w => {
                if let Some((d, k)) = w.strip_prefix("pad=").and_then(|v| v.split_once('.')) {
                    let low = k.ends_with('-');
                    let k: u32 = k.trim_end_matches('-').parse().unwrap_or(0);
                    let mut p = PadState::new([0.0; 4], [0.0; 2], 0);
                    match d {
                        "8" if k < 2 => p = PadState::new([0.0; 4], [if k == 0 { 1.0 } else { 0.0 }, if k == 1 { 1.0 } else { 0.0 }], 0),
                        "8" if k < 16 => p.buttons |= 1 << k,
                        "9" if k < 4 => p.sticks[k as usize] = if low { 0 } else { 255 },
                        _ => {}
                    }
                    x.ctrl.hook_pad = Some((p, 4));
                }
            }
        }
    }
    x.pad = match x.ctrl.hook_pad.as_mut() {
        Some((p, left)) if *left > 0 => {
            *left -= 1;
            Some(*p)
        }
        _ => Some(x.pad.unwrap_or_else(|| PadState::new([0.0; 4], [0.0; 2], 0))),
    };
    out
}

impl Game {
    fn ctrl_st(&self, key: &str, fallback: &str) -> String {
        self.career.strings.get(key).filter(|v| !v.contains('<')).map(|v| v.to_string()).unwrap_or_else(|| fallback.to_string()).to_uppercase()
    }

    fn ctrl_maps(&mut self) {
        if self.opt_x.control_maps.is_none() {
            self.opt_x.control_maps = Some(control_maps(&self.data_dir));
        }
    }

    /// One of the controller pages' boxes goes up (the original's box kinds 1, 2 and 3 / 4
    /// of F-75153: OK alone; OK with BACK; YES / NO with the cursor on NO).
    fn ctrl_box(&mut self, kind: CtrlBox) {
        let yes_no = vec![("ctrl_a".to_string(), "messagebox_yes".to_string()), ("ctrl_b".to_string(), "messagebox_no".to_string())];
        let ok = vec![("ctrl_a".to_string(), "messagebox_ok".to_string())];
        let (title, description, options, at) = match kind {
            CtrlBox::Invalid => ("customcontrol_invalid_title", "customcontrol_invalid_desc", ok, 0),
            CtrlBox::Unmapped | CtrlBox::KbdUnmapped => ("customcontrol_unmapped_title", "customcontrol_unmapped_desc", ok, 0),
            CtrlBox::Defaults | CtrlBox::KbdDefaults => ("customcontrol_default_title", "customcontrol_default_desc", yes_no, 1),
            CtrlBox::Save => ("Dlg_Title_SaveOptions", "Dlg_Desc_SaveOptions", yes_no, 1),
        };
        // (`box_sel` says afterwards which row was taken; it stays as it is on BACK)
        self.box_sel = usize::MAX;
        self.message_box = Some((crate::flash_ui::MessageBoxDef { title: title.into(), description: description.into(), options }, at, crate::game::MenuAction::None));
        self.opt_x.ctrl.box_kind = Some(kind);
    }

    /// The controller pages' part of a frame of the pause menu, before the menu's own:
    /// what was pressed (up, down, accept, back; X and Y) is taken where a controller page
    /// answers it, and what the menu's own code is still to see comes back.
    /// - OPTIONS (page 4): BACK with something of the controller's changed asks "SAVE
    ///   OPTIONS" (F-92772): YES commits (`ctrl_commit`), NO goes back to what was;
    /// - CONTROLLER (5, the port's own page): GAMEPAD, KEYBOARD, RACING WHEEL;
    /// - CONTROL STYLE (70, mcControllerStyleForm, F-22867): A sets the row's style
    ///   (the pad's rows; kept until the options are saved), Y opens the editor on the
    ///   custom row and the viewer on the others;
    /// - the editor (71, mcControllerCustomizeForm, F-38381): A waits for a button, Y
    ///   clears the row, X asks for the defaults, BACK leaves - past the "MISSING MAPPINGS"
    ///   box when a row that must be mapped is not. While it waits nothing else is heard.
    pub fn mc4_ctrl_input(&mut self, pressed: [bool; 4], key_x: bool, key_y: bool) -> [bool; 4] {
        let page = self.pause_page;
        if !matches!(page, 4..=8 | PAGE_STYLE..=PAGE_WHEEL | PAGE_DISPLAY | PAGE_PERFORMANCE) {
            // (OPTIONS was left some other way: nothing was confirmed - but the port's plain
            // menu, without the movie, has no box to ask with and leaves from any page: as if YES)
            if self.flash_menu.is_none() && ctrl_changed(&self.opt_x.ctrl) && ctrl_commit(&mut self.opt_x.ctrl, &mut self.mc4, self.fresh_game) {
                self.dirty = true;
            }
            self.opt_x.ctrl.open = false;
            self.opt_x.ctrl.polling = false;
            self.opt_x.ctrl.kbd.polling = false;
            self.opt_x.ctrl.kbd.queue.clear();
            return pressed;
        }
        // (the keys the window's loop handed to the keyboard page since the last frame)
        let mut kbd_keys = std::mem::take(&mut self.opt_x.ctrl.kbd.queue);
        self.ctrl_maps();
        if !self.opt_x.ctrl.open {
            let maps = self.opt_x.control_maps.clone().unwrap_or_default();
            ctrl_open(&mut self.opt_x.ctrl, &maps, self.mc4.as_ref().map(|m| &m.options));
        }
        // (a box of these pages that has gone: what was answered)
        if let Some(kind) = self.opt_x.ctrl.box_kind {
            if self.message_box.is_some() {
                return pressed;
            }
            self.opt_x.ctrl.box_kind = None;
            match (kind, self.box_sel) {
                (CtrlBox::Defaults, 0) => {
                    // (F-17745: the working copy is the list's map of the row)
                    let row = self.opt_x.ctrl.row;
                    if let Some(m) = self.opt_x.control_maps.as_ref().and_then(|m| m.get(row)) {
                        self.opt_x.ctrl.work = slots_array(&m.slots);
                    }
                }
                (CtrlBox::Unmapped, 0) => {
                    self.menu_sel = self.opt_x.ctrl.row;
                    self.pause_page = PAGE_STYLE;
                }
                (CtrlBox::KbdDefaults, 0) => self.opt_x.ctrl.kbd.work = Default::default(),
                (CtrlBox::KbdUnmapped, 0) => {
                    self.menu_sel = 1;
                    self.pause_page = PAGE_CONTROLLER;
                }
                (CtrlBox::Save, 0) => {
                    // (and the game is saved, as F-87944 has "mc4.sav" written)
                    if ctrl_commit(&mut self.opt_x.ctrl, &mut self.mc4, self.fresh_game) {
                        self.dirty = true;
                        let d = self.data_dir.clone();
                        self.write_save(&d);
                    }
                    self.menu_sel = 0;
                    self.pause_page = 0;
                }
                (CtrlBox::Save, 1) => {
                    // (F-87944, result 2: the options as they were)
                    self.opt_x.ctrl.open = false;
                    self.menu_sel = 0;
                    self.pause_page = 0;
                }
                _ => {}
            }
            return [false; 4];
        }
        let hook = ctrl_hook(&mut self.opt_x);
        let [up, down, enter, back] = [pressed[0] || hook.up, pressed[1] || hook.down, pressed[2] || hook.enter, pressed[3] || hook.back];
        let sel = self.menu_sel;
        match page {
            4 => {
                if back && ctrl_changed(&self.opt_x.ctrl) {
                    if self.flash_popup.is_some() {
                        self.ctrl_box(CtrlBox::Save);
                        return [up, down, enter, false];
                    }
                    // (no box to ask with: as if YES)
                    if ctrl_commit(&mut self.opt_x.ctrl, &mut self.mc4, self.fresh_game) {
                        self.dirty = true;
                        let d = self.data_dir.clone();
                        self.write_save(&d);
                    }
                } else if back {
                    self.opt_x.ctrl.open = false;
                }
                [up, down, enter, back]
            }
            PAGE_CONTROLLER => {
                let _ = (std::mem::take(&mut self.nav_x), std::mem::take(&mut self.nav_y));
                if enter {
                    let to = [PAGE_STYLE, PAGE_KEYBOARD, PAGE_WHEEL][sel.min(2)];
                    self.menu_sel = if to == PAGE_STYLE { (self.opt_x.ctrl.style.clamp(0, 7)) as usize } else { 0 };
                    self.pause_page = to;
                }
                [up, down, false, back]
            }
            PAGE_STYLE => {
                let y = std::mem::take(&mut self.nav_y) || key_y || hook.y;
                self.nav_x = false;
                let c = &mut self.opt_x.ctrl;
                if enter && sel < 5 && sel as i32 != c.style {
                    c.style = sel as i32;
                }
                if y && sel < 5 {
                    c.row = sel;
                    c.read_only = sel != 4;
                    c.polling = false;
                    self.menu_sel = 0;
                    self.pause_page = PAGE_EDIT;
                } else if back {
                    self.menu_sel = 0;
                    self.pause_page = PAGE_CONTROLLER;
                }
                [up, down, false, false]
            }
            PAGE_EDIT => {
                let x = std::mem::take(&mut self.nav_x) || key_x || hook.x;
                let y = std::mem::take(&mut self.nav_y) || key_y || hook.y;
                let slot = CONTROL_ROWS[sel.min(CONTROL_ROWS.len() - 1)];
                let waiting = self.opt_x.ctrl.polling;
                let pad = self.opt_x.pad;
                if ctrl_poll_frame(&mut self.opt_x.ctrl, pad.as_ref(), slot) == Some(false) {
                    self.ctrl_box(CtrlBox::Invalid);
                }
                if waiting {
                    return [false; 4];
                }
                let (read_only, row) = (self.opt_x.ctrl.read_only, self.opt_x.ctrl.row);
                if read_only {
                    if back {
                        self.menu_sel = row;
                        self.pause_page = PAGE_STYLE;
                    }
                } else if enter {
                    // (the port only waits with a pad there to answer)
                    if pad.is_some() {
                        self.opt_x.ctrl.polling = true;
                        self.opt_x.ctrl.poll_skip = true;
                    }
                } else if y {
                    self.opt_x.ctrl.work[slot] = (0, 0);
                } else if x {
                    self.ctrl_box(CtrlBox::Defaults);
                } else if back {
                    if ctrl_unmapped(&self.opt_x.ctrl.work) && self.flash_popup.is_some() {
                        self.ctrl_box(CtrlBox::Unmapped);
                    } else {
                        self.menu_sel = row;
                        self.pause_page = PAGE_STYLE;
                    }
                }
                [up, down, false, false]
            }
            PAGE_KEYBOARD => {
                // (the port's own page, made after the pad's editor: Enter / A waits for a key -
                // Escape, or B on a pad, gives up; Delete / Backspace / Y clears the row; F5 / X
                // asks for the defaults; BACK leaves, past the "MISSING MAPPINGS" box)
                use winit::keyboard::KeyCode as K;
                let x = std::mem::take(&mut self.nav_x) || hook.x;
                let y = std::mem::take(&mut self.nav_y) || hook.y;
                kbd_keys.extend(crate::keybinds::hook_keys(self.opt_x.ctrl.hook_frame.wrapping_sub(1)));
                let rows = crate::keybinds::rows();
                let (act, place) = rows[sel.min(rows.len() - 1)];
                let popup = self.flash_popup.is_some();
                let k = &mut self.opt_x.ctrl.kbd;
                if k.polling {
                    let mut refused = false;
                    if back {
                        k.polling = false;
                    } else if let Some(&code) = kbd_keys.first() {
                        k.polling = false;
                        refused = code != K::Escape && !k.work.assign(act, place, code);
                    }
                    if refused && popup {
                        self.ctrl_box(CtrlBox::Invalid);
                    }
                    return [false; 4];
                }
                if enter {
                    k.polling = true;
                } else if y || kbd_keys.iter().any(|c| matches!(c, K::Delete | K::Backspace)) {
                    k.work.clear(act, place);
                } else if x || kbd_keys.contains(&K::F5) {
                    if popup {
                        self.ctrl_box(CtrlBox::KbdDefaults);
                    } else {
                        k.work = Default::default();
                    }
                } else if back {
                    if k.work.unmapped() && popup {
                        self.ctrl_box(CtrlBox::KbdUnmapped);
                    } else {
                        self.menu_sel = 1;
                        self.pause_page = PAGE_CONTROLLER;
                    }
                }
                [up, down, false, false]
            }
            PAGE_WHEEL => {
                let _ = (std::mem::take(&mut self.nav_x), std::mem::take(&mut self.nav_y));
                if back {
                    self.menu_sel = 2;
                    self.pause_page = PAGE_CONTROLLER;
                }
                [up, down, false, false]
            }
            _ => [up, down, enter, back],
        }
    }

    /// The page the pause menu shows for a controller page (see `CtrlPage`).
    pub fn mc4_ctrl_page(&self) -> CtrlPage {
        use crate::flash_ui::{CHECK_OFF, CHECK_ON, ICON_A, ICON_B, ICON_X, ICON_Y};
        let st = |k: &str, fb: &str| self.ctrl_st(k, fb);
        let c = &self.opt_x.ctrl;
        let sel = self.menu_sel;
        let back = (st("prompt_back", "BACK"), ICON_B);
        let mut out = CtrlPage::default();
        const STYLES: [(&str, &str); 8] = [("Throttle_Stick", "STICK - AUTOMATIC"), ("Throttle_Trigger", "TRIGGER - AUTOMATIC"), ("Throttle_Stick_Manual", "STICK - MANUAL"), ("Throttle_Trigger_Manual", "TRIGGER - MANUAL"), ("Custom_Pad", "CUSTOM CONTROLLER"), ("Steering_Wheel", "RACING WHEEL - AUTOMATIC"), ("Steering_Wheel_Manual", "RACING WHEEL - MANUAL"), ("Custom_Steering_Wheel", "CUSTOM RACING WHEEL")];
        match self.pause_page {
            PAGE_STYLE => {
                // (mcControllerStyleForm: the list's first eight by their names as text keys;
                // the row of the style has the tick; the wheel's rows greyed on a pad)
                out.title = st("controller styles", "CONTROL STYLE");
                let style = if c.open { c.style } else { self.mc4.as_ref().map_or(1, |m| m.options.control_config_pad) };
                for (i, (key, fb)) in STYLES.iter().enumerate() {
                    out.rows.push((st(key, fb), if i >= 5 { String::new() } else if style == i as i32 { CHECK_ON.to_string() } else { CHECK_OFF.to_string() }, i < 5));
                }
                // (F-59161: by the row under the cursor)
                out.prompts = match sel {
                    4 => vec![(st("prompt_set", "SET"), ICON_A), back, (st("prompt_edit", "EDIT"), ICON_Y)],
                    0..=3 => vec![(st("prompt_set", "SET"), ICON_A), back, (st("prompt_view", "VIEW"), ICON_Y)],
                    _ => vec![back],
                };
            }
            PAGE_EDIT => {
                out.title = if c.read_only { st("customcontrol_readonly_title", "PRESET CONTROLS") } else { st("customcontrol_title", "CUSTOM CONTROLS") };
                out.headers = vec![st("customcontrol_column_action", "ACTION"), st("customcontrol_column_button", "BUTTON")];
                let shown = ctrl_shown(c, self.opt_x.control_maps.as_deref().unwrap_or(&[]));
                for (i, &slot) in CONTROL_ROWS.iter().enumerate() {
                    let name = action_name(slot).unwrap_or("");
                    let (device, key) = shown[slot];
                    let must = device == 0 && control_recommended(&shown, slot);
                    // (F-47959: while it waits the row is blank and the others are greyed)
                    let value = if c.polling {
                        if i == sel { String::new() } else { Self::ctrl_value(&st, device, key, must) }
                    } else {
                        Self::ctrl_value(&st, device, key, must)
                    };
                    let on = !c.polling || i == sel;
                    out.rows.push((st(name, &name.replace('_', " ")), value, on));
                    // (the row's own writer, F-46822: `property1` 20 on a row that must be mapped)
                    out.props.push(if must && !(c.polling && i == sel) { 20 } else { on as i32 });
                }
                out.prompts = if c.polling {
                    vec![(st("controller_custom_prompt_poll", "PRESS A BUTTON"), 0)]
                } else if c.read_only {
                    vec![back]
                } else {
                    vec![(st("controller_custom_prompt_change", "CHANGE"), ICON_A), back, (st("controller_custom_prompt_delete", "CLEAR"), ICON_Y), (st("prompt_defaults", "DEFAULTS"), ICON_X)]
                };
            }
            PAGE_KEYBOARD => {
                // (the port's own page, laid out as the pad's editor: ACTION / KEY, the keys
                // by name; a row for an action's second key where the port has two by default.
                // The prompts name keys, not pad buttons - the pad's A B Y X do the same)
                let k = &c.kbd;
                let shown = if c.open { k.work } else { k.binds };
                out.title = "KEYBOARD CONTROLS".to_string();
                out.headers = vec![st("customcontrol_column_action", "ACTION"), "KEY".to_string()];
                for (i, (act, place)) in crate::keybinds::rows().into_iter().enumerate() {
                    let def = &crate::keybinds::ACTS[act];
                    let mut label = if def.shared { st(def.name, def.label) } else { def.label.to_string() };
                    if place == 1 {
                        label.push_str(" (2ND KEY)");
                    }
                    let must = place == 0 && shown.must(act);
                    let on = !k.polling || i == sel;
                    let value = if k.polling && i == sel {
                        String::new()
                    } else if must {
                        st("mapping_recommended", "MUST BE MAPPED")
                    } else {
                        shown.keys[act][place].and_then(crate::keybinds::key_name).unwrap_or("").to_string()
                    };
                    out.rows.push((label, value, on));
                    out.props.push(if must && !(k.polling && i == sel) { 20 } else { on as i32 });
                }
                out.prompts = if k.polling {
                    vec![("PRESS A KEY - ESC: CANCEL".to_string(), 0)]
                } else {
                    vec![(format!("ENTER: {}", st("controller_custom_prompt_change", "CHANGE")), 0), (format!("ESC: {}", st("prompt_back", "BACK")), 0), (format!("DEL: {}", st("controller_custom_prompt_delete", "CLEAR")), 0), (format!("F5: {}", st("prompt_defaults", "DEFAULTS")), 0)]
                };
            }
            PAGE_WHEEL => {
                // (the port's own page; the wheel is not driven yet - its three styles, greyed)
                out.title = "RACING WHEEL".to_string();
                for (key, fb) in &STYLES[5..] {
                    out.rows.push((st(key, fb), String::new(), false));
                }
                out.prompts = vec![back];
            }
            _ => {
                // (the port's own page in front of the original's)
                out.title = st("controller", "CONTROLLER");
                for name in ["GAMEPAD", "KEYBOARD", "RACING WHEEL"] {
                    out.rows.push((name.to_string(), String::new(), true));
                }
                out.prompts = vec![(st("prompt_accept", "ACCEPT"), ICON_A), back];
            }
        }
        if out.props.is_empty() {
            out.props = out.rows.iter().map(|r| r.2 as i32).collect();
        }
        out
    }

    /// A slot in the BUTTON column: a glyph (the movie's row kind 7), a text, or "MUST BE
    /// MAPPED" (F-25949).
    fn ctrl_value(st: &dyn Fn(&str, &str) -> String, device: u32, key: u32, must: bool) -> String {
        if must {
            return st("mapping_recommended", "MUST BE MAPPED");
        }
        match control_label(device, key) {
            ControlLabel::Glyph(n) => format!("{}{}", crate::flash_ui::GLYPH, n),
            ControlLabel::Text(k) => st(k, k),
            ControlLabel::None => String::new(),
        }
    }
}

/// What the paint shop and the vinyl editor cost, once per model (`li r6,1000` at A-64047 /
/// A-84400; the affordability test F-69834 gets the same 1000).
pub const MISC_PRICE: i64 = 1000;

/// The paint areas after the body, by the port's `part_paint` keys.
const AREA_KEYS: [&str; 5] = ["doors", "hood", "bumper_f", "bumper_r", "skirts"];

impl Game {
    /// The brake model a vehicle's record names: `brakes` of tune/vehicle/wheel_models.list
    /// by `BrakeMdlIdx` (F-75123) - the stock models too, which are no rows of the
    /// BRAKES menu (the Ford GT's 11 `brk_car_brembo_drilled_blank`, the R8's own 14 ...).
    pub fn mc4_brake_name(&self, car: &str) -> Option<String> {
        let c = self.mc4.as_ref()?.car(car)?;
        BRAKES.get(c.visual.BrakeMdlIdx as usize).map(|s| s.to_string())
    }

    /// Parts were paid for on `car`: the port's tags ("<section>:<index>") get their bits in
    /// the model's purchase record (every category: `PART_CATEGORIES`), and the model's sum
    /// takes what was paid (F-66800).
    pub fn mc4_parts_bought(&mut self, car: &str, tags: &[String], cost: i64) {
        let seed = self.mc4.is_none();
        let key = car.to_ascii_lowercase();
        // (a block made here, for a save that had none: what that save's tags say was
        // bought goes into the records first)
        let old: Vec<(String, Vec<String>)> = if seed { self.parts_owned.iter().map(|(k, v)| (k.clone(), v.clone())).collect() } else { Vec::new() };
        let m = self.mc4_mut();
        for (c, t) in old {
            m.record_mut(&c).take_tags(&t);
        }
        let r = m.record_mut(&key);
        r.take_tags(tags);
        if cost > 0 {
            r.add_spent(cost.min(0xFFFF) as u32);
        }
    }

    /// The highest stage of performance part `k` the model of `car` owns (F-32441).
    /// The shop makes the stages up to it free (F-41623 before A-28976: price 0 for
    /// stage 1 when the level is above 0, for stage 2 when above 1). The port's tags
    /// "performance:<k * 3 + stage>" hold it, with or without the block.
    pub fn perf_owned(&self, car: &str, k: usize) -> u8 {
        let Some(tags) = self.parts_owned.get(&car.to_ascii_lowercase()) else { return 0 };
        let has = |s: usize| tags.iter().any(|t| t.strip_prefix("performance:").and_then(|n| n.parse::<usize>().ok()) == Some(k * 3 + s));
        if has(2) { 2 } else if has(1) { 1 } else { 0 }
    }

    /// The performance shop's purchase (F-69428): every part's fitted stage becomes
    /// the model's - bit part * 3 + stage, and with a stage 2 the stage 1 as well - and the
    /// sum takes the price.
    pub fn mc4_perf_bought(&mut self, car: &str, stages: &[u8], cost: i64) {
        let mut tags = Vec::new();
        for (k, s) in stages.iter().take(13).enumerate() {
            tags.push(format!("performance:{}", k * 3 + (*s).min(2) as usize));
            if *s >= 2 {
                tags.push(format!("performance:{}", k * 3 + 1));
            }
        }
        self.mc4_parts_bought(car, &tags, cost);
        let have = self.parts_owned.entry(car.to_ascii_lowercase()).or_default();
        for t in tags {
            if !have.contains(&t) {
                have.push(t);
            }
        }
    }

    /// The cabin's five colour groups for the renderer (INTERIOR COLOR A..E; linear, w 1 =
    /// chosen, 0 = the stock cabin): `Vehicles::player_interior`. Which zones of the trim
    /// take which group is the vehicle's own table (`zone_groups`).
    pub fn interior_groups(&self) -> [[f32; 4]; 5] {
        let abc = self.interior_finish();
        let mut out = [abc[0], abc[1], abc[2], [0.0; 4], [0.0; 4]];
        for k in 0..2 {
            out[3 + k] = match self.car_x.int_de_hsv[k] {
                Some(h) => {
                    let c = to_linear(hsv_rgb(h));
                    [c[0], c[1], c[2], 1.0]
                }
                None if self.car_x.int_de_idx[k].0 != 9 => {
                    let p = PALETTE[self.car_x.int_de_idx[k].0 % PALETTE.len()];
                    [p[0], p[1], p[2], 1.0]
                }
                None => [0.0; 4],
            };
        }
        out
    }

    /// The second colour in the shop's hands, as on screen.
    fn paint_b_screen(&self) -> [f32; 3] {
        match self.car_x.paint_b_rgb {
            Some((k, c)) if k == self.paint_b => c,
            _ => swatch_screen(self.paint_b),
        }
    }

    /// The six paint areas of the vehicle in use follow the paint shop, by the original's
    /// rules (called every frame):
    /// - PAINT OPTIONS: ALL PARTS (the garage's flag, `garage.ChangePaintOverride(1)`): a
    ///   paint type, a fade, a colour goes into all six entries (F-31035, F-83553,
    ///   F-87462 with the flag set) - so the parts lose a colour of their own;
    /// - PARTS: into the entry of the PAINT AREA in hand only, and a colour set there takes
    ///   that area's fade off (F-40495 at A-90030: `F-83553(.., area, 0)`);
    /// - changing the PAINT AREA shows that area's type on the row (MatchPaintTypeAndFade).
    /// The areas start from the vehicle's record; a vehicle without one starts with the
    /// body's paint on every area, a part painted by itself without a fade (as it was drawn).
    pub fn mc4_paint_sync(&mut self) {
        let car = if self.car_x.car.is_empty() { self.mc4_car_name(self.current_car) } else { self.car_x.car.clone() };
        if car.is_empty() {
            return;
        }
        let body_b = self.paint_b_screen();
        let body_a = self.paint_color();
        let parts: [Option<[u8; 3]>; 5] = [0, 1, 2, 3, 4].map(|k| self.cfg.part_paint.get(AREA_KEYS[k]).copied());
        let area = self.paint_area.min(5);
        let Some(mut live) = self.car_x.paint.take() else {
            let from_record = self.mc4.as_ref().and_then(|m| m.car(&car)).filter(|c| c.visual.paint.len() == 6).map(|c| {
                [0, 1, 2, 3, 4, 5].map(|k| {
                    let p = &c.visual.paint[k];
                    AreaPaint { ty: p.paint_type().unwrap_or(2), fade: p.fade().unwrap_or(0), b: p.color_b().unwrap_or(body_b) }
                })
            });
            let areas = from_record.unwrap_or_else(|| {
                let body = AreaPaint { ty: self.paint_type.min(5), fade: self.paint_fade.min(4), b: body_b };
                [0, 1, 2, 3, 4, 5].map(|k| if k > 0 && parts[k - 1].is_some() { AreaPaint { fade: 0, ..body } } else { body })
            });
            self.car_x.paint = Some(PaintLive { areas, seen_ty: self.paint_type, seen_fade: self.paint_fade, seen_b: body_b, seen_a: body_a, seen_area: area, seen_parts: parts });
            return;
        };
        // (only the garage paints)
        if matches!(self.mode, crate::game::Mode::Garage) {
            let whole = !self.paint_parts;
            if area != live.seen_area {
                self.paint_type = live.areas[area].ty;
                if area == 0 {
                    self.paint_fade = live.areas[0].fade;
                }
            } else {
                let (ty, fade) = (self.paint_type.min(5), self.paint_fade.min(4));
                for k in 0..6 {
                    if !(whole || k == area) {
                        continue;
                    }
                    if self.paint_type != live.seen_ty {
                        live.areas[k].ty = ty;
                    }
                    if self.paint_fade != live.seen_fade {
                        live.areas[k].fade = fade;
                    }
                    if !close(body_b, live.seen_b) {
                        live.areas[k].b = body_b;
                    }
                }
                let body_moved = (0..3).any(|i| (body_a[i] - live.seen_a[i]).abs() > 1e-4);
                if body_moved && whole {
                    // (every area takes the colour: no part keeps one of its own)
                    for key in AREA_KEYS {
                        if self.cfg.part_paint.remove(key).is_some() {
                            self.dirty = true;
                        }
                    }
                } else if body_moved && area == 0 {
                    // (MAIN BODY alone: the other areas keep the colour they had)
                    let old = [0, 1, 2].map(|i| (live.seen_a[i].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8);
                    for key in AREA_KEYS {
                        if !self.cfg.part_paint.contains_key(key) {
                            self.cfg.part_paint.insert(key.to_string(), old);
                        }
                    }
                    live.areas[0].fade = 0;
                    self.paint_fade = 0;
                    self.dirty = true;
                } else if !whole && area > 0 && parts[area - 1] != live.seen_parts[area - 1] {
                    live.areas[area].fade = 0;
                }
            }
        }
        live.seen_ty = self.paint_type;
        live.seen_fade = self.paint_fade;
        live.seen_b = self.paint_b_screen();
        live.seen_a = body_a;
        live.seen_area = area;
        live.seen_parts = [0, 1, 2, 3, 4].map(|k| self.cfg.part_paint.get(AREA_KEYS[k]).copied());
        self.car_x.paint = Some(live);
    }

    /// The player's paint for the renderer: each area as the original's body draw sets it
    /// up before the parts of that area (F-92759 at A-29193 -> F-13513).
    pub fn paint_out(&self) -> PaintOut {
        let body_a = self.paint_color();
        let Some(live) = self.car_x.paint.as_ref() else {
            let b = paint_as_drawn(AreaPaint { ty: self.paint_type.min(5), fade: self.paint_fade.min(4), b: self.paint_b_screen() });
            return PaintOut { ty: b.ty, fade: b.fade, b: to_linear(b.b), area_a: [[0.0; 4]; 5], area_b: [[0.0; 4]; 5] };
        };
        let body = paint_as_drawn(live.areas[0]);
        let mut out = PaintOut { ty: body.ty, fade: body.fade, b: to_linear(body.b), area_a: [[0.0; 4]; 5], area_b: [[0.0; 4]; 5] };
        for k in 1..6 {
            let own_a = self.cfg.part_paint.get(AREA_KEYS[k - 1]).map(|c| to_linear([c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]));
            let own = own_a.is_some() || live.areas[k] != live.areas[0];
            let a = own_a.unwrap_or([body_a[0], body_a[1], body_a[2]]);
            let p = paint_as_drawn(live.areas[k]);
            let b = to_linear(p.b);
            out.area_a[k - 1] = [a[0], a[1], a[2], own as u8 as f32];
            out.area_b[k - 1] = [b[0], b[1], b[2], p.ty as f32 + 8.0 * p.fade as f32];
        }
        out
    }

    /// The garage's top menu, before a row or the way out is taken: (ACCEPT, BACK) as the
    /// menu may act on them. `top`: the GARAGE menu itself; `at_exit`: the cursor is on EXIT
    /// GARAGE; `row`: the cursor's row (0 MANAGE VEHICLES, 1 CUSTOMIZE VEHICLE, 2 VEHICLE
    /// SHOWROOM); `car`: the player's car - its four damage zones and its damage (0..1).
    /// - a row: `garage.RepairBeforeCustomizing` (F-32431) - a zone above 0.05 brings
    ///   the REPAIR box (FullFixConfirm / ..Manage / ..Showroom of MessageBoxes.sc.xml);
    /// - EXIT GARAGE or BACK: `garage.RepairBeforeExiting` (F-70109) - damage above
    ///   0.9 brings FullFixConfirmExit; below that the car leaves as it is.
    /// YES repairs (`game.repairCar()`) and goes on; NO closes the box.
    /// (approx.) The box's price line: the original sets it by `game.SetQuickFixPrice()` into
    /// the box's movie; here it stands under the text. What `repairCar` does when the money
    /// is short was not found: the port pays what is there, as its wreck repair does.
    pub fn mc4_repair_gate(&mut self, top: bool, enter: bool, back: bool, at_exit: bool, row: usize, car: Option<([f32; 4], f32)>) -> (bool, bool) {
        if let Some(kind) = self.car_x.repair_go.take() {
            return if kind == 0 { (true, false) } else { (false, true) };
        }
        let Some((mut zones, mut damage)) = car else { return (enter, back) };
        let (mut enter, mut back, mut row) = (enter, back, row);
        // (test hook: MCLA_REPAIR_TEST=0 | 1 | 2 presses that row of the GARAGE menu with a dented
        // car, MCLA_REPAIR_TEST=exit BACK with a wreck, once the menu has stood for 500 frames)
        if let (true, Ok(v)) = (top, std::env::var("MCLA_REPAIR_TEST")) {
            static STOOD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if STOOD.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 500 {
                match v.parse::<usize>() {
                    Ok(r) => {
                        zones = [0.0, 0.3, 0.0, 0.0];
                        enter = true;
                        row = r;
                    }
                    Err(_) => {
                        damage = 0.95;
                        back = true;
                    }
                }
            }
        }
        if !top || self.borrowed || self.selected_car != self.current_car || !(enter || back) {
            return (enter, back);
        }
        let leaving = back || (enter && at_exit);
        let need = if leaving { damage > 0.9 } else { zones.iter().any(|z| *z > 0.05) };
        if !need {
            // (RepairBeforeCustomizing with no zone above 0.05: no box, the car repaired as it goes in - free,
            // scrapes and all - `F-32431` -> `F-12181`)
            if !leaving {
                self.repair_car = true;
            }
            return (enter, back);
        }
        let mean = zones.iter().map(|z| z.clamp(0.0, 1.0)).sum::<f32>() / 4.0;
        let price = self.fix_price(damage.max(mean), zones.iter().any(|z| *z > 0.05)).min(self.money.max(0));
        let id = if leaving { "FullFixConfirmExit" } else { ["FullFixConfirmManage", "FullFixConfirm", "FullFixConfirmShowroom"][row.min(2)] };
        let def = crate::flash_ui::MessageBoxDef::load(&self.data_dir, id).filter(|d| !d.options.is_empty()).map(|mut d| {
            let text = self.career.strings.get(&d.description).map(|s| s.to_string()).unwrap_or_else(|| d.description.clone());
            d.description = format!("{}\n${}", text, price);
            d
        });
        match def {
            Some(def) if self.flash_popup.is_some() => {
                self.message_box = Some((def, 0, crate::game::MenuAction::None));
                self.car_x.repair_pending = Some((leaving as u8, price));
                (false, false)
            }
            // (no box to ask in: repaired as chosen)
            _ => {
                self.money -= price;
                self.repair_car = true;
                self.dirty = true;
                (enter, back)
            }
        }
    }

    /// YES in the REPAIR box: the car is repaired at the price shown (`repair_car`: the
    /// window's loop resets its damage, the scrapes too), and the row or the way out is taken.
    pub fn mc4_repair_finish(&mut self) {
        if let Some((kind, price)) = self.car_x.repair_pending.take() {
            self.money -= price.min(self.money.max(0));
            self.repair_car = true;
            self.dirty = true;
            self.car_x.repair_go = Some(kind);
        }
    }

    /// PAINT SHOP (`n` 0) or VINYL EDITOR (1) was chosen in the garage:
    /// `garage.CheckMiscPartPurchased(n)` (F-96638 at A-97737). The shop opens when
    /// the model's purchase record has it (category 5 / 6, bit 0) or the parts are free;
    /// else the player is asked to buy it for $1000 (`Confirm_Buy_Paint` /
    /// `Confirm_Buy_Vinyl` of tune/ui/garage/Main.sc.xml), or told that $1000 are needed
    /// (`G_OverBudgetErrorMisc`). True: go in.
    /// (approx.) The original asks on a page of the garage's own movie (template 43); the
    /// port asks in its message box, with the same title, text and choices.
    pub fn mc4_misc_check(&mut self, n: usize) -> bool {
        const TAGS: [&str; 2] = ["paint:0", "vinyl:0"];
        let n = n.min(1);
        let car = self.mc4_car_name(self.current_car);
        if self.parts_free() || self.parts_owned.get(&car).map_or(false, |v| v.iter().any(|t| t == TAGS[n])) {
            return true;
        }
        if self.money < MISC_PRICE {
            if !self.open_message_box("G_OverBudgetErrorMisc", crate::game::MenuAction::None) {
                self.show_message("YOU NEED $1000");
            }
            return false;
        }
        let id = ["Confirm_Buy_Paint", "Confirm_Buy_Vinyl"][n];
        let def = crate::flash_ui::MessageBoxDef::load(&self.data_dir, id).filter(|d| !d.options.is_empty()).map(|mut d| {
            // (the page's title is the state's own name, its text the page's description)
            d.title = ["G_PurchasePaint", "G_PurchaseVinyls"][n].to_string();
            d.description = ["Confirm_Buy_Paint_For_1000", "Confirm_Buy_Vinyl_For_1000"][n].to_string();
            d
        });
        match def {
            Some(def) if self.flash_popup.is_some() => {
                self.message_box = Some((def, 0, crate::game::MenuAction::None));
                self.car_x.misc_pending = Some(n);
                false
            }
            // (no box to ask in - the game's movies are not there: bought as chosen)
            _ => {
                self.mc4_misc_buy(n);
                self.show_message(["PAINT SHOP BOUGHT FOR $1000", "VINYL EDITOR BOUGHT FOR $1000"][n]);
                true
            }
        }
    }

    /// `garage.FinishMiscPurchase(n)` (F-96638 at A-53975): bit 0 of category 5 / 6
    /// for $1000 (F-66800, so the sum of the model's record takes it too), and the
    /// money goes (F-38145).
    pub fn mc4_misc_buy(&mut self, n: usize) {
        let tag = ["paint:0", "vinyl:0"][n.min(1)].to_string();
        let car = self.mc4_car_name(self.current_car);
        self.money -= MISC_PRICE;
        self.mc4_parts_bought(&car, &[tag.clone()], MISC_PRICE);
        let have = self.parts_owned.entry(car).or_default();
        if !have.contains(&tag) {
            have.push(tag);
        }
        self.dirty = true;
    }

    /// Into the paint shop (0) / the vinyl editor (1) from the CUSTOMIZATION menu.
    pub fn mc4_misc_enter(&mut self, n: usize) {
        if n == 1 && self.flash_garage.is_none() {
            // (without the garage's movie the editor is a row, not a page)
            self.vinyl_open_req = true;
        } else {
            self.garage_stack.push((self.garage_menu, self.garage_cur));
            self.garage_menu = if n == 0 { 3 } else { 15 };
            self.garage_cur = 0;
        }
    }

    /// No purchase box is up: nothing is being asked.
    pub fn mc4_misc_idle(&mut self) {
        self.car_x.misc_pending = None;
        // (step 22: no vinyl editor box is up either)
        self.car_x.vinyl_ask = None;
        // (test hook, step 22: MCLA_VINYL_STORE=<slot> opens STORE VINYL PACKAGES on that row
        // 60 frames after the vinyl editor's menu is up; "<slot>,ask" puts its box up 60
        // frames later (MCLA_BOX_ACCEPT answers it); "pkg" opens VINYL PACKAGES on its last row)
        if let Ok(v) = std::env::var("MCLA_VINYL_STORE") {
            static T: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if matches!(self.mode, crate::game::Mode::Garage) && self.garage_menu == 15 && self.vinyl_edit.is_none() {
                let t = T.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let slot = v.split(',').next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0).min(2);
                if t == 60 && v == "pkg" {
                    self.garage_cur = 0;
                    self.vinyl_pkg = Some(self.mc4_vinyl_rows().len().saturating_sub(1));
                } else if t == 60 {
                    self.garage_cur = 3;
                    if self.mc4_vinyl_has_layers() {
                        self.car_x.vinyl_save = Some(slot);
                    }
                    log::info!("vinyl store hook: rows {:?}, used {:?}, layers {}", self.mc4_vinyl_rows().len(), self.mc4_vinyl_used(), self.mc4_vinyl_has_layers());
                } else if t == 120 && v.contains(",ask") && self.car_x.vinyl_save.is_some() {
                    self.mc4_vinyl_ask(VinylAsk::Store(slot));
                } else if t == 200 && v.contains(",save") {
                    // ("<slot>,ask,save": the save copy written once the box has been answered)
                    let keep = self.no_save;
                    self.no_save = false;
                    let dir = self.data_dir.clone();
                    self.write_save(&dir);
                    self.no_save = keep;
                    log::info!("vinyl store hook: save written, used {:?}", self.mc4_vinyl_used());
                }
            }
        }
        self.car_x.repair_pending = None;
        if std::mem::take(&mut self.car_x.swap_cf) {
            self.mc4_swap_carbon_fiber();
        }
        // (test hook: MCLA_MISC_TEST=0 | 1 chooses PAINT SHOP / VINYL EDITOR once the garage's
        // top menu has stood for 500 frames; with MCLA_BOX_ACCEPT the box is answered YES)
        if let Some(n) = std::env::var("MCLA_MISC_TEST").ok().and_then(|v| v.parse::<usize>().ok()) {
            static STOOD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if matches!(self.mode, crate::game::Mode::Garage) && self.garage_menu == 0 && STOOD.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 500 {
                self.garage_stack.push((0, 1));
                self.garage_menu = 1;
                self.garage_cur = if n == 0 { 3 } else { 2 };
                if self.mc4_misc_check(n) {
                    self.mc4_misc_enter(n);
                }
            }
        }
    }

    /// YES in the purchase box: the shop is bought; which one (0 paint, 1 vinyl).
    pub fn mc4_misc_finish(&mut self) -> Option<usize> {
        let n = self.car_x.misc_pending.take()?;
        self.mc4_misc_buy(n);
        Some(n)
    }

    /// The music lists, made once from the disc's audio config and what the save says
    /// (none without the config: the port's own way of picking songs stays).
    pub fn mc4_music(&mut self) -> Option<MusicShared> {
        // (the titles are the string table's: not before it is read)
        if self.opt_x.music.is_none() && !self.opt_x.music_tried && !self.career.strings.keys.is_empty() {
            self.opt_x.music_tried = true;
            let bytes = std::fs::read(std::path::Path::new(&self.data_dir).join("audio/X360/config/game.dat")).ok();
            if let Some(mut l) = bytes.and_then(|b| MusicLists::from_game_dat(&b)) {
                for s in &mut l.songs {
                    if let Some(t) = self.career.strings.get(&s.key) {
                        s.title = t.to_string();
                    }
                }
                match self.mc4.as_ref().map(|m| &m.options) {
                    Some(o) => {
                        let ids: Vec<u32> = o.custom_playlist.iter().filter_map(|v| v.as_str().and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()).or_else(|| v.as_u64().map(|n| n as u32))).collect();
                        l.load(&ids, o.music_genre as usize, o.music_shuffle);
                    }
                    // (a save without the block: the port's genre; it always picked at random)
                    None => l.load(&[], music_list(self.music_genre as usize), true),
                }
                self.opt_x.music = Some(std::sync::Arc::new(std::sync::Mutex::new(l)));
            }
        }
        self.opt_x.music.clone()
    }

    /// PLAY was chosen in the playlist menu since the last look.
    pub fn mc4_music_take_play(&mut self) -> bool {
        self.opt_x.music.as_ref().and_then(|m| m.lock().ok()).map_or(false, |mut l| std::mem::take(&mut l.play))
    }

    /// The map the pad is read through now (`hydraulics`: they are switched on). Empty
    /// without the disc's controller files: the pad then drives nothing.
    pub fn mc4_controls(&mut self, hydraulics: bool) -> ControlSlots {
        if self.opt_x.control_maps.is_none() {
            self.opt_x.control_maps = Some(control_maps(&self.data_dir));
        }
        control_slots(self.opt_x.control_maps.as_deref().unwrap_or(&[]), self.mc4.as_ref().map(|m| &m.options), hydraulics)
    }

    /// GAME OPTIONS' ARROW: 0 compass (the arrow on the minimap's ring), 1 top (the 3D arrow
    /// at the top of the screen), 2 none. A save without the block: compass, as before.
    pub fn mc4_gps_arrow(&self) -> u32 {
        self.mc4.as_ref().map_or(0, |m| m.options.gps_arrow.min(2))
    }

    /// The GPS button works: `gps_button` on the save's switch and the career's events.
    pub fn mc4_gps_enabled(&self) -> bool {
        // (step 21b: RACE_GH_06's READY_TO_START_ON_ENTER switches the button off as it starts
        // (0x309); RACE_GH_02's switches it on at its end (0x89d): after its call, ChangeState(3)
        // and the mission log's lesson (the port's lesson ends with the target set: "tut:target")
        let lesson = self.career.stats.contains_key("tut:mission_log") && !self.career.stats.contains_key("tut:target");
        let gh02 = self.career.state_at_least("RACE_GH_02", 5) || (self.career.state_at_least("RACE_GH_02", 3) && !lesson);
        gps_button(self.mc4.as_ref().map(|m| m.enable_gps), self.career.loaded(), self.career.reached("RACE_GH_06"), gh02, self.career.reached("RACE_SH_01"))
    }

    /// STEERING SENS. as the original's slider has it: -10 .. 10 (the saved value x 10,
    /// F-78269; 0 in a new game). A save without the block: the port's eleven places,
    /// 0..10, are the even steps.
    pub fn mc4_sens_step(&self) -> i32 {
        match self.mc4.as_ref().map(|m| m.options.sensitivity_step()) {
            // (the block's step, while the port's own field still stands for it)
            Some(step) if ((step + 10) as f32 / 2.0).round().clamp(0.0, 10.0) as u8 == self.opts.sens => step,
            _ => self.opts.sens.min(10) as i32 * 2 - 10,
        }
    }

    /// The row moves one step of the twenty-one (`wrap`: the pause movie's ACCEPT, which
    /// goes round). The port's own field follows as the nearest of its eleven places.
    pub fn mc4_sens_set(&mut self, forward: bool, wrap: bool) {
        sens_set(&mut self.mc4, &mut self.opts.sens, forward, wrap, self.fresh_game);
        self.dirty = true;
    }

    /// The vehicle in use has no special ability: its record says -1 (set-up files and
    /// every vehicle until the garage's SPECIAL ABILITIES page gives it one, F-73337;
    /// `mcRacer::GetActiveAbility`, F-74686, hands the record's byte to the scripts
    /// as it is). A vehicle without a record keeps the ability in hand, as before.
    /// In GOAL ATTACK no vehicle has one: the racer's current ability (F-78162, its
    /// ability object's +52) answers -1 while the mode's +796 (or +797) is set.
    pub fn mc4_ability_none(&self) -> bool {
        if self.goal_attack {
            return true;
        }
        self.car_x.ability.is_none() && !self.car_x.car.is_empty() && self.mc4.as_ref().map_or(false, |m| m.car(&self.car_x.car).is_some())
    }

    /// The sum word of a model's purchase record (1 = nothing spent: the constructor's,
    /// F-69059).
    pub fn mc4_spent_word(&self, car: &str) -> u32 {
        self.mc4.as_ref().and_then(|m| m.record(car)).map_or(1, |r| r.spent_plus_1)
    }

    /// The player's vehicle as it stands (its model's name, its damage and the damage of
    /// its four zones - left, right, front, rear: the original keeps these per garage slot
    /// while a vehicle is stored, LocalOptions + 0x3DCB4, F-72475). A vehicle that was
    /// just taken out gets what its slot kept.
    pub fn mc4_damage(&mut self, model: &str, damage: &mut f32, zones: &mut [f32; 4]) {
        if self.car_x.car.is_empty() || !model.eq_ignore_ascii_case(&self.car_x.car) {
            return;
        }
        if let Some((d, z)) = self.car_x.damage_in.take() {
            *damage = d;
            *zones = z;
        }
        self.car_x.damage = Some((*damage, *zones));
    }

    /// The plate background of the vehicle in use: `backgrounds` of
    /// tune/vehicle/license_plates.list by the record's `LicenseBG` (4 = plate_ca_00).
    pub fn mc4_plate_bg(&self) -> Option<u8> {
        self.mc4.as_ref()?.car(&self.car_x.car).map(|c| c.visual.LicenseBG)
    }

    /// That background's name (the list is read once).
    pub fn mc4_plate_bg_name(&self) -> Option<String> {
        static LIST: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        let k = self.mc4_plate_bg()? as usize;
        let mut list = LIST.lock().ok()?;
        if list.is_empty() {
            // (not kept while it cannot be read: the game's data folder is set after the
            // first frames)
            let t = std::fs::read_to_string(std::path::Path::new(&self.data_dir).join("tune/vehicle/license_plates.list")).unwrap_or_default();
            // ("backgrounds", "{", the names, "}")
            *list = t.lines().map(|l| l.trim()).skip_while(|l| *l != "{").skip(1).take_while(|l| *l != "}").filter(|l| !l.is_empty()).map(|l| l.to_string()).collect();
        }
        list.get(k).cloned()
    }

    /// What the port's own per-vehicle fields hold, into the vehicles' records, so that the
    /// block is the state of the game: paint, parts, performance parts and brands, wheel
    /// figures, tires, brakes, plate, rim materials. A value the record already stands for
    /// is left as it is (the port's fields are coarser: nothing is rounded twice).
    fn mc4_sync_cars(&self, m: &mut Mc4) {
        let cur = if self.car_x.car.is_empty() { self.mc4_car_name(self.current_car) } else { self.car_x.car.clone() };
        let stock_brake = |car: &str| -> Option<u8> {
            let b = std::fs::read(crate::render::car_lamps::setup_file(std::path::Path::new(&self.data_dir), car)?).ok()?;
            b.get(0x846).copied()
        };
        for c in m.garage.iter_mut() {
            let name = c.vehicle.to_ascii_lowercase();
            let is_cur = name == cur;
            // --- paint: the body's colour, type and fade; the areas painted by themselves ---
            let paint = if is_cur {
                let (idx, hsv) = match (self.paint_area != 0, self.paint_body) {
                    (true, Some(b)) => b,
                    _ => (self.color_idx, self.pick_slot.map_or(self.paint_hsv, |p| p.1)),
                };
                Some((hsv, idx, self.paint_type, self.paint_fade))
            } else {
                self.paints.get(&name).copied()
            };
            let cfg = if is_cur { Some(&self.cfg) } else { self.cfgs.get(&name) };
            if let (Some((hsv, idx, kind, fade)), true) = (paint, c.visual.paint.len() == 6) {
                let lin = match hsv {
                    Some(h) => hsv_rgb(h),
                    None => {
                        let p = PALETTE[idx % PALETTE.len()];
                        [p[0], p[1], p[2]]
                    }
                };
                let body_a = c.visual.paint[0].color_a();
                let (old_t, old_f) = (c.visual.paint[0].paint_type(), c.visual.paint[0].fade());
                let same = body_a.map_or(false, |a| {
                    let l = to_linear(a);
                    (0..3).all(|i| (l[i] - lin[i]).abs() < 1.5e-3)
                });
                let new_a = if same { body_a.unwrap() } else { to_screen(lin) };
                for (k, p) in c.visual.paint.iter_mut().enumerate() {
                    let own = if k == 0 { None } else { cfg.and_then(|g| g.part_paint.get(AREA_KEYS[k - 1])).copied() };
                    let have = p.color_a();
                    match own {
                        // (a part painted by itself: its colour as the screen shows it)
                        Some(rgb) => {
                            if have.map_or(true, |h| (0..3).any(|i| ((h[i] * 255.0).round() - rgb[i] as f32).abs() > 1.0)) {
                                p.ColorA = rgb_f([rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0]);
                            }
                        }
                        // (the body's: an area that stood within the importer's 0.02 of the
                        // body's colour is the body's)
                        None => {
                            let was_body = match (have, body_a) {
                                (Some(h), Some(b)) => (0..3).all(|i| (h[i] - b[i]).abs() <= 0.02),
                                _ => true,
                            };
                            // (an area with a colour of its own that the port's parts no longer
                            // have - PAINT OPTIONS: ALL PARTS went over it - is the body's again)
                            let reset = if k == 0 || was_body { !same } else { cfg.is_some() };
                            if reset && have.map_or(true, |h| !close(h, new_a)) {
                                p.ColorA = rgb_f(new_a);
                            }
                        }
                    }
                    // (type and fade: the body's, and the areas that had the body's)
                    if old_t != Some(kind.min(5)) && (k == 0 || p.paint_type() == old_t) {
                        p.set_paint_type(kind.min(5));
                    }
                    if old_f != Some(fade.min(4)) && (k == 0 || p.fade() == old_f) {
                        p.set_fade(fade.min(4));
                    }
                }
                // (the vehicle in use: its six areas as the shop left them)
                if let (true, Some(live)) = (is_cur, self.car_x.paint.as_ref()) {
                    for (k, p) in c.visual.paint.iter_mut().enumerate() {
                        let a = live.areas[k];
                        if p.paint_type() != Some(a.ty) {
                            p.set_paint_type(a.ty);
                        }
                        if p.fade() != Some(a.fade) {
                            p.set_fade(a.fade);
                        }
                        if p.color_b().map_or(true, |b| !close(b, a.b)) {
                            p.ColorB = vec![r6(a.b[0]), r6(a.b[1]), r6(a.b[2])];
                        }
                    }
                }
            }
            // --- parts ---
            let xp = self.xparts.get(&name);
            for (port, byte, mirror) in PART_MAP {
                let v = match (cfg, port) {
                    (Some(g), "hood") => Some(g.hood),
                    (Some(g), "bumper_r") => Some(g.bumper_r),
                    (Some(g), "bumper_f") => Some(g.bumper_f),
                    (Some(g), "skirts") => Some(g.skirts),
                    (Some(g), "spoiler") => Some(g.spoiler),
                    (Some(g), "exhaust") => Some(g.exhaust),
                    (Some(g), "seats") => Some(g.seats),
                    (Some(g), "body") => Some(g.body),
                    (_, "hood" | "bumper_r" | "bumper_f" | "skirts" | "spoiler" | "exhaust" | "seats" | "body") => None,
                    _ => xp.and_then(|x| x.get(port)).copied(),
                };
                let Some(v) = v else { continue };
                let v = v.min(255) as u8;
                if c.part(byte) != v {
                    c.visual.parts.insert(byte.to_string(), v);
                    if let Some(mir) = mirror {
                        c.visual.parts.insert(mir.to_string(), v);
                    }
                }
            }
            // (the intercooler's switch, the bikes' three parts, the carbon fibre hood)
            for (key, byte) in XPART_BYTES {
                if let Some(v) = xp.and_then(|x| x.get(key)).copied() {
                    if c.part(byte) != v.min(255) as u8 {
                        c.visual.parts.insert(byte.to_string(), v.min(255) as u8);
                    }
                }
            }
            // (the side exhaust: its part, and the switch the set-part writes with it)
            if let Some(v) = xp.and_then(|x| x.get("side_exhaust")).copied() {
                if c.part("exh_side") != v.min(255) as u8 {
                    c.visual.parts.insert("exh_side".into(), v.min(255) as u8);
                    c.visual.parts.insert("SideExhaustInUse".into(), (v != 0) as u8);
                }
            }
            if let Some(v) = xp.and_then(|x| x.get("tires")).copied() {
                if c.visual.TireTexIdx as usize != v {
                    c.visual.TireTexIdx = v.min(255) as u8;
                    c.visual.tire_model = TIRES.get(v).map(|s| s.to_string());
                }
            }
            // (the brakes: a row of the BRAKES menu, 1..4; without one the car's own - a
            // record that still names a menu row goes back to the set-up's)
            let want = match xp.and_then(|x| x.get("brakes")).copied() {
                Some(b) if (1..=4).contains(&b) => Some(b as u8),
                _ if (1..=4).contains(&c.visual.BrakeMdlIdx) && xp.is_some() => stock_brake(&name).filter(|b| !(1..=4).contains(b)),
                _ => None,
            };
            if let Some(b) = want {
                if c.visual.BrakeMdlIdx != b {
                    c.visual.BrakeMdlIdx = b;
                    c.visual.brake_model = BRAKES.get(b as usize).map(|s| s.to_string());
                }
            }
            // --- performance parts and their makers ---
            if let Some(st) = self.perf_parts.get(&name).filter(|s| s.len() == 13) {
                for (k, part) in PERF_PARTS.iter().enumerate() {
                    if c.performance.stages.get(*part) != Some(&st[k]) {
                        c.performance.stages.insert(part.to_string(), st[k]);
                    }
                }
            }
            if let Some(br) = self.perf_brand.get(&name) {
                for (k, part) in PERF_PARTS.iter().enumerate() {
                    if c.performance.brand.get(*part) != Some(&br[k]) {
                        c.performance.brand.insert(part.to_string(), br[k]);
                    }
                }
            }
            // --- wheel figures: ride height, tire width, rim size, tire profile (front, rear) ---
            if let Some(d) = self.dims.get(&name) {
                let v = &mut c.visual;
                for (k, f) in [&mut v.RideHeight, &mut v.TireWidth, &mut v.RimSize, &mut v.TireProfile].into_iter().enumerate() {
                    if f.as_slice() != &d[2 * k..2 * k + 2] {
                        *f = d[2 * k..2 * k + 2].to_vec();
                    }
                }
            }
            // --- the plate: seven characters, the four signs 0x5B..0x5E ---
            if let Some(p) = self.plates.get(&name) {
                let mut raw: Vec<u8> = p.bytes().take(7).collect();
                raw.resize(7, b' ');
                if c.visual.plate_letters_raw != raw {
                    c.visual.plate_letters = raw.iter().map(|ch| match *ch { 0x5B => "[sign1]".to_string(), 0x5C => "[sign2]".to_string(), 0x5D => "[sign3]".to_string(), 0x5E => "[sign4]".to_string(), ch => (ch as char).to_string() }).collect();
                    c.visual.plate_letters_raw = raw;
                }
            }
            // --- the colour rows: interior neon, gauge A / B, interior A..C, rim spoke A / B /
            // lip (the port's nine slots) and interior D / E. A row with a mixed colour that is
            // not the record's goes into the record; a row left at stock leaves it alone ---
            let slots: Option<Vec<Option<[f32; 3]>>> = if is_cur { Some(self.slot_hsv.to_vec()) } else { self.finishes.get(&name).map(|f| f.0.clone()) };
            if let Some(slots) = slots {
                let far = |have: &[F], rgb: [f32; 3]| have.len() < 3 || (0..3).any(|i| (have[i].0 as f32 - rgb[i]).abs() > 2e-3);
                let put = |rgb: [f32; 3]| vec![r6(rgb[0]), r6(rgb[1]), r6(rgb[2])];
                let v = &mut c.visual;
                for (k, h) in slots.iter().enumerate().take(9) {
                    let Some(rgb) = h.map(hsv_rgb) else { continue };
                    let field: &mut Vec<F> = match k {
                        0 => &mut v.IntNeonColor,
                        1 => &mut v.DashGaugeColor,
                        2 => &mut v.DashGaugeColorB,
                        3..=5 => match v.m_vBaseColor.get_mut(k - 3) {
                            Some(g) => &mut g.rgb,
                            None => continue,
                        },
                        6 => &mut v.RimColorA,
                        7 => &mut v.RimColorB,
                        _ => &mut v.RimColorC,
                    };
                    if far(field, rgb) {
                        *field = put(rgb);
                    }
                }
                if is_cur {
                    for k in 0..2 {
                        let rgb = match self.car_x.int_de_hsv[k] {
                            Some(h) => hsv_rgb(h),
                            None if self.car_x.int_de_idx[k].0 != 9 => swatch_screen(self.car_x.int_de_idx[k].0),
                            None => continue,
                        };
                        if let Some(g) = v.m_vBaseColor.get_mut(3 + k) {
                            if far(&g.rgb, rgb) {
                                g.rgb = put(rgb);
                            }
                        }
                    }
                }
            }
            // --- the rim's three materials: the port's 0 painted, 1 chrome, 2 brushed, 3 carbon
            // fibre are the original's 0, 1, 3, 2 (the shop offers 0, 1, 3 only:
            // garage.SetRimMaterial of Wheels.sc.xml; a 2 comes with a record and stays until
            // the shop changes the row) ---
            let mats = if is_cur { Some(self.rim_mats) } else { self.finishes.get(&name).map(|f| f.1) };
            if let Some(mats) = mats {
                for k in 0..3 {
                    let have = c.rim_material(k);
                    let seen = match have { Some(0) => 0, Some(3) => 2, Some(2) => 3, _ => 1 };
                    // (a vehicle not in use whose finish was imported before the port knew
                    // carbon fibre - it stands as chrome there: the record's is kept)
                    if !is_cur && have == Some(2) && mats[k] == 1 {
                        continue;
                    }
                    if seen != mats[k].min(3) {
                        c.set_rim_material(k, [0, 1, 3, 2][mats[k].min(3) as usize]);
                    }
                }
            }
        }
    }
}

// ---- the experience systems: class reputation and the Unlocking amount ----
//
// The original's experience manager (global 0x8287484C) gives experience at the end of an
// event (handler F-12673):
//   GENERIC_RACE_COMPLETE, FREEWAY_RACE_COMPLETE (F-48818) and WAGER_RACE_COMPLETE
//   (F-51495): the race's experience to `Progression` (not in a network game) and the
//   same to `Unlocking` (always); one point to the system of the class of the vehicle
//   driven, whatever the place; one point to the system of the ability fitted.
//   PAYBACK_LEG_COMPLETE (F-66865), last leg: the experience to both.
//   BEAT_ME_THERE_RACE_COMPLETE (F-90736), COP_EVADE (F-67002): `Progression` only.
//   DELIVERY_RACE_COMPLETE (F-96289), TIME_TRIAL_RACE_COMPLETE (F-58178): money only.
// An addition (F-55689) is followed by the system's thresholds (F-61830): each one
// not yet reached whose `Percent` the system's share (F-14176) has come to is marked
// reached and gives its rewards (F-61974).

/// The vehicle classes, in the order of the original's table (0x827E9ABC); each has an
/// experience system of its name.
pub const CLASS_SYSTEMS: [&str; 9] = ["Tuner", "Muscle", "Exotic", "SportBike", "Luxury", "Cop", "SUV", "LowRider", "Misc1"];
/// the statistic of races won in a class: what the port counted before the class systems
const CLASS_WIN_STATS: [(&str, &str); 7] = [("Tuner", "kTUW"), ("Muscle", "kMUW"), ("Exotic", "kEXW"), ("Luxury", "kLUW"), ("SportBike", "kMOW"), ("SUV", "kSUW"), ("LowRider", "kLOW")];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThresholdDef {
    pub id: String,
    pub percent: f32,
    /// rewards of type PartUnlock: the names of the body part types
    pub parts: Vec<String>,
    /// rewards of type PerformanceLevel: D1, C0 ...
    pub perf: Vec<String>,
    /// (step 23) rewards of type Achievement: aZON ...
    pub achievements: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemDef {
    pub name: String,
    pub max: i64,
    pub thresholds: Vec<ThresholdDef>,
}

/// The experience systems of the career tables (`tune/career/0x3b8ff776` and the South
/// Central one, `0x30974b1d`): `System { Type MaxAmount IncrementValue Threshold { Id
/// Percent Reward { Type ... } } }` (read by F-93962, F-30482).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExpTables {
    pub systems: Vec<SystemDef>,
    /// the events of the event reward tables
    pub events: Vec<EventDef>,
}

impl ExpTables {
    /// The text of one table, added to what is here: a system of a name that is here
    /// already gets the thresholds whose ids it does not have (South Central: `Progression`).
    pub fn parse(&mut self, text: &str) {
        let text = text.replace('{', " { ").replace('}', " } ");
        let tok: Vec<&str> = text.split_whitespace().collect();
        let mut path: Vec<&str> = Vec::new();
        let mut sys: Option<SystemDef> = None;
        let mut th: Option<ThresholdDef> = None;
        let mut reward: (String, String) = Default::default();
        let mut i = 0;
        while i < tok.len() {
            let t = tok[i];
            if tok.get(i + 1) == Some(&"{") {
                path.push(t);
                match t {
                    "System" => sys = Some(SystemDef::default()),
                    "Threshold" => th = Some(ThresholdDef::default()),
                    "Reward" => reward = Default::default(),
                    _ => {}
                }
                i += 2;
                continue;
            }
            if t == "}" {
                match path.pop() {
                    Some("Reward") => {
                        if let Some(th) = th.as_mut() {
                            match reward.0.as_str() {
                                "PartUnlock" => th.parts.push(reward.1.clone()),
                                "PerformanceLevel" => th.perf.push(reward.1.to_ascii_uppercase()),
                                "Achievement" => th.achievements.push(reward.1.clone()),
                                _ => {}
                            }
                        }
                    }
                    Some("Threshold") => {
                        if let (Some(s), Some(mut th)) = (sys.as_mut(), th.take()) {
                            if th.id.is_empty() {
                                // (the first edition table has no ids)
                                th.id = format!("{}_{}", s.name, s.thresholds.len());
                            }
                            s.thresholds.push(th);
                        }
                    }
                    Some("System") => {
                        if let Some(s) = sys.take() {
                            match self.systems.iter_mut().find(|x| x.name == s.name) {
                                Some(have) => {
                                    for th in s.thresholds {
                                        if !have.thresholds.iter().any(|x| x.id == th.id) {
                                            have.thresholds.push(th);
                                        }
                                    }
                                }
                                None => self.systems.push(s),
                            }
                        }
                    }
                    _ => {}
                }
                i += 1;
                continue;
            }
            let v = tok.get(i + 1).copied().unwrap_or("");
            match (path.last().copied(), t) {
                (Some("System"), "Type") => {
                    if let Some(s) = sys.as_mut() {
                        s.name = v.to_string();
                    }
                }
                (Some("System"), "MaxAmount") => {
                    if let Some(s) = sys.as_mut() {
                        s.max = v.parse().unwrap_or(0);
                    }
                }
                (Some("Threshold"), "Id") => {
                    if let Some(th) = th.as_mut() {
                        th.id = v.to_string();
                    }
                }
                (Some("Threshold"), "Percent") => {
                    if let Some(th) = th.as_mut() {
                        th.percent = v.parse().unwrap_or(2.0);
                    }
                }
                (Some("Reward"), "Type") => reward.0 = v.to_string(),
                (Some("Reward"), "Name") | (Some("Reward"), "PerformanceId") => reward.1 = v.to_string(),
                _ => {}
            }
            i += 2;
        }
    }

    pub fn system(&self, name: &str) -> Option<&SystemDef> {
        self.systems.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }
}

impl SystemDef {
    pub fn is_main(&self) -> bool {
        self.name == "Progression" || self.name == "Unlocking"
    }

    /// The share of the system, 0..1 (F-14176): the two main systems go through the
    /// thirteen levels of the manager, the others are amount / MaxAmount; the amount counts
    /// up to MaxAmount.
    pub fn fraction(&self, amount: i64) -> f32 {
        if self.is_main() {
            crate::rewards::progress(amount.min(self.max).max(0)).1
        } else if self.max <= 0 {
            0.0
        } else {
            amount.min(self.max) as f32 / self.max as f32
        }
    }

    /// The step the system stands on (F-96803): the last one with the share at 1,
    /// else the first threshold the share is below.
    pub fn index(&self, amount: i64) -> usize {
        let n = self.thresholds.len();
        if n == 0 {
            return 0;
        }
        let f = self.fraction(amount);
        if f >= 1.0 {
            return n - 1;
        }
        // (the original runs on past the list when no threshold is above the share)
        self.thresholds.iter().position(|t| f < t.percent).unwrap_or(n - 1)
    }

    /// The amounts the step runs between (F-57870, F-53104): Percent x MaxAmount,
    /// cut to a whole number; 0 below the first threshold, 100 for a system without any.
    pub fn span(&self, amount: i64) -> (i64, i64) {
        if self.thresholds.is_empty() {
            return (0, 100);
        }
        let i = self.index(amount);
        let at = |k: usize| (self.thresholds[k].percent * self.max as f32) as i64;
        (if i > 0 { at(i - 1) } else { 0 }, at(i))
    }
}

/// The smallest amount at which a main system (Progression, Unlocking) has share `pct`:
/// where a threshold of that `Percent` is reached (F-61830 compares the share).
pub fn main_amount_at(pct: f32) -> i64 {
    let max = crate::catalog::MAX_REP;
    if !(pct <= 1.0) {
        return (pct * max as f32) as i64;
    }
    let (mut lo, mut hi) = (0i64, max);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if crate::rewards::progress(mid).1 >= pct {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// The smallest amount at which a system counted by amount / MaxAmount has share `pct`.
pub fn class_amount_at(pct: f32, max: i64) -> i64 {
    (0..=max.max(0)).find(|a| max > 0 && *a as f32 / max as f32 >= pct).unwrap_or(max.max(0) + 1)
}

impl Game {
    /// The experience system of a vehicle class as the catalogue names it.
    pub fn mc4_class_system(class: &str) -> Option<&'static str> {
        CLASS_SYSTEMS.iter().find(|s| s.eq_ignore_ascii_case(class)).copied()
    }

    /// The amounts of the class systems are the block's own (a save of the original, or a
    /// block of the port's that has begun to count them); else the earlier rule of the port
    /// holds: races won in the class.
    fn mc4_classes_tracked(&self) -> bool {
        self.mc4.as_ref().map_or(false, |m| !m.made_by_port() || m.career.experience.extra.get("classes_counted").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    /// The class reputation: the points of the system of the class.
    pub fn mc4_class_amount(&self, system: &str) -> i64 {
        if self.mc4_classes_tracked() {
            return self.mc4.as_ref().and_then(|m| m.career.experience.classes.get(system)).map_or(0, |s| s.amount);
        }
        CLASS_WIN_STATS.iter().find(|(s, _)| *s == system).and_then(|(_, st)| self.career.stats.get(*st)).copied().unwrap_or(0)
    }

    /// The amount of the Unlocking system (a save that does not count it: the reputation).
    pub fn mc4_unlocking(&self) -> i64 {
        self.career.unlocking.unwrap_or(self.rep)
    }

    /// What `perf_rep` answers for a level the Unlocking system gives at amount `need`:
    /// the shops compare it with the reputation, so it is put in terms of the reputation
    /// (0 when the Unlocking amount is there; above the reputation by what is missing).
    pub fn mc4_perf_need(&self, need: i64) -> i64 {
        match self.career.unlocking {
            Some(u) if need > 0 => {
                if u >= need {
                    0
                } else {
                    self.rep + (need - u)
                }
            }
            _ => need,
        }
    }

    /// What `price_of` answers for a vehicle of unlock group `group` (C0, B0, A0: levels the
    /// Unlocking system gives, like the performance levels; F-61830): 0 when the save
    /// has the level or the Unlocking amount is at its threshold, else in terms of the
    /// reputation, as `mc4_perf_need`.
    pub fn mc4_group_need(&self, group: &str) -> i64 {
        let id = group.to_ascii_uppercase();
        if crate::catalog::group_rep(&id) == 0 || self.mc4_perf_level_unlocked(&id) {
            return 0;
        }
        let need = self.career.perf_unlocks.get(&id).copied().unwrap_or_else(|| main_amount_at(match id.as_str() {
            "C0" => 0.10,
            "B0" => 0.34,
            _ => 0.60,
        }));
        self.mc4_perf_need(need)
    }

    /// A block of the port's begins to count the class systems: the amounts start from the
    /// races won (what the port went by), every system gets its thresholds, and what those
    /// amounts have reached is handed out to the class of the system itself.
    fn mc4_exp_seed(&self, m: &mut Mc4) {
        if !m.made_by_port() || m.career.experience.extra.get("classes_counted").and_then(|v| v.as_bool()).unwrap_or(false) {
            return;
        }
        m.career.experience.extra.insert("classes_counted".into(), Value::Bool(true));
        for sys in CLASS_SYSTEMS {
            let amount = CLASS_WIN_STATS.iter().find(|(s, _)| *s == sys).and_then(|(_, st)| self.career.stats.get(*st)).copied().unwrap_or(0);
            let s = m.career.experience.classes.entry(sys.to_string()).or_default();
            s.amount = amount;
            if let Some(def) = self.career.exp_tables.system(sys) {
                for th in &def.thresholds {
                    if s.reached(&th.id).is_none() {
                        s.thresholds.push(Threshold { id: th.id.clone(), reached: false, extra: Extra::new() });
                    }
                }
            }
            self.mc4_class_thresholds(m, sys, sys);
        }
    }

    /// The thresholds of a class system after an addition (F-61830): each one not yet
    /// reached that the share has come to is marked and gives its rewards - PartUnlock
    /// (F-70978): the body part type is unlocked, and marked new, for the class of the
    /// vehicle the player is in (`class`).
    fn mc4_class_thresholds(&self, m: &mut Mc4, system: &str, class: &str) -> Vec<String> {
        let mut given = Vec::new();
        let Some(def) = self.career.exp_tables.system(system) else { return given };
        let c = &mut m.career;
        let s = c.experience.classes.entry(system.to_string()).or_default();
        let f = def.fraction(s.amount);
        for th in &def.thresholds {
            if s.reached(&th.id) == Some(true) || !(f >= th.percent) {
                continue;
            }
            s.set_reached(&th.id);
            for part in &th.parts {
                let have = c.body_parts_unlocked.entry(class.to_string()).or_default();
                if !have.iter().any(|p| p.eq_ignore_ascii_case(part)) {
                    have.push(part.clone());
                    given.push(part.clone());
                }
                let new = c.body_parts_new.entry(class.to_string()).or_default();
                if !new.iter().any(|p| p.eq_ignore_ascii_case(part)) {
                    new.push(part.clone());
                }
            }
        }
        given
    }

    /// The experience of a finished race, beside the reputation the game has added
    /// (GENERIC_RACE_COMPLETE / FREEWAY_RACE_COMPLETE: F-48818; WAGER_RACE_COMPLETE:
    /// F-51495): `exp` to the Unlocking system too, one point to the system of the
    /// class of the vehicle driven (whatever the place) and one to the system of the
    /// ability fitted. `rep_before`: the reputation before that of the race was added.
    /// Returns the body part types this unlocked.
    pub fn mc4_race_experience(&mut self, rep_before: i64, exp: i64) -> Vec<String> {
        self.mc4_rewards_begin();
        self.mc4_unlocking_add(rep_before, exp);
        let mut given = Vec::new();
        let class = self.catalog.get(&self.mc4_car_name(self.current_car)).map(|c| c.class.clone()).unwrap_or_default();
        if let Some(sys) = Game::mc4_class_system(&class) {
            let mut m = self.mc4.take().unwrap_or_else(|| Mc4::seeded(self.fresh_game));
            self.mc4_exp_seed(&mut m);
            m.career.experience.classes.entry(sys.to_string()).or_default().amount += 1;
            given = self.mc4_class_thresholds(&mut m, sys, sys);
            self.mc4 = Some(m);
        }
        // (the ability fitted to the vehicle: byte 8108 of its record, 255 = none)
        if self.mc4_abilities_unlocked() && !self.mc4_ability_none() {
            let k = self.car_x.ability.unwrap_or(self.ability_sel).min(3);
            let (sys, stat) = (ABILITY_SYSTEMS[k], ["gZON", "gAGR", "gROR", "gPUL"][k]);
            let before = self.career.ability_level(sys, stat);
            let uses = self.career.stats.get(stat).copied().unwrap_or(0);
            *self.career.ability_amounts.entry(sys.to_string()).or_insert(uses) += 1;
            let after = self.career.ability_level(sys, stat);
            // (step 23: a threshold of the ability's system reached gives its Achievement
            // reward - aZON / aAGR / aROR / aPUL with level 2; the reward sets the statistic
            // once, F-13951)
            if after > before {
                let amount = self.career.ability_amounts.get(sys).copied().unwrap_or(0);
                let due: Vec<String> = self.career.exp_tables.system(sys).map(|d| { let f = d.fraction(amount); d.thresholds.iter().filter(|t| f >= t.percent).flat_map(|t| t.achievements.iter().cloned()).collect() }).unwrap_or_default();
                for a in due {
                    self.mc4_award(&a);
                }
            }
            if after > before {
                // ("ZONE LEVEL 2" is the text for reaching level 1, and so on)
                let key = format!("{}_LEVEL_{}", sys.to_ascii_uppercase(), after);
                let text = self.career.strings.get(&key).map(|s| s.to_string()).unwrap_or_else(|| format!("{} LEVEL {}", ABILITY[k], after + 1));
                // (a reward of type AbilityLevel: a line of the rewards screen's REWARDED
                // group - F-36204 A-62815 -, not a message of its own)
                self.career.rewarded_now.push(text.to_uppercase());
            }
        }
        given
    }

    /// Experience for the Unlocking system (the manager gives what is above 1).
    pub fn mc4_unlocking_add(&mut self, rep_before: i64, exp: i64) {
        if exp > 1 {
            let before = self.career.unlocking.unwrap_or(rep_before);
            self.career.unlocking = Some(before + exp);
            // (the thresholds this reaches give their PerformanceLevel rewards: lines of the
            // rewards screen's UNLOCKED group)
            if let Some(def) = self.career.exp_tables.system("Unlocking") {
                let (f0, f1) = (def.fraction(before), def.fraction(before + exp));
                let had = |id: &str| self.mc4.as_ref().and_then(|m| m.career.experience.main.get("Unlocking")).and_then(|s| s.reached(id)) == Some(true);
                let new: Vec<String> = def.thresholds.iter().filter(|th| !(f0 >= th.percent) && f1 >= th.percent && !had(&th.id)).flat_map(|th| th.perf.iter().cloned()).collect();
                self.career.perf_now.extend(new);
            }
        }
    }

    /// What keeps a body part type of the class tables from the shop for a vehicle of
    /// `class`: (points needed, points so far); None = unlocked. With the class systems
    /// counted the flag of the type decides (F-57000), as the garage of the original
    /// asks; else the earlier rule of the port, races won in the class.
    pub fn mc4_part_lock(&self, class: &str, part: &str) -> Option<(i64, i64)> {
        let (need, wins) = self.career.part_wins_needed(class, part);
        if need <= 0 {
            return None;
        }
        let Some(sys) = Game::mc4_class_system(class).filter(|_| self.mc4_classes_tracked()) else {
            return if wins < need { Some((need, wins)) } else { None };
        };
        let unlocked = self.mc4.as_ref().and_then(|m| m.career.body_parts_unlocked.get(sys)).map_or(false, |v| v.iter().any(|p| p.eq_ignore_ascii_case(part)));
        if unlocked { None } else { Some((need, self.mc4_class_amount(sys))) }
    }

    /// The rank and meter of the class as the map screen shows them (F-71884): rank =
    /// the step of the system of the class + 1; "(a/b)" = the points on that step and its
    /// length.
    pub fn mc4_class_rank(&self, class: &str) -> (usize, i64, i64) {
        let Some(sys) = Game::mc4_class_system(class) else { return (1, 0, 100) };
        let amount = self.mc4_class_amount(sys);
        match self.career.exp_tables.system(sys) {
            Some(def) => {
                let (lo, hi) = def.span(amount);
                (def.index(amount) + 1, amount.min(def.max) - lo, hi - lo)
            }
            None => (1, amount, 100),
        }
    }
}

// ---- step 19: the slot system's district locks ----
//
// The cruising hookmen's slot system (manager +8596) saves, per district, three "slot hidden"
// flags and one "district locked" byte (manager +8741 + d). A new game has every district locked,
// Hollywood too, and every slot hidden: the slot system's constructor (F-18978, from the
// manager's F-77509) sets +145..+151 to 1, and the slot template (F-57269) hidden +80 to
// 1; nothing native clears them. A mission's script opens a district when the mission starts:
// mcSlotSystem_HideDistrict(d, 0) - the slots shown (F-16040 -> F-58249) - and
// mcSlotSystem_UnlockDistrict(d, 1) - the lock off and the slots evaluated (F-12898).
// Hollywood is opened by RACE_GH_02's and RACE_SH_01's core scripts when their race is won
// (`DISTRICTS_WON`).

/// (mission, district, evaluate): the story core scripts that open a district when their
/// mission is won (mcMission_GetState == 5): RACE_GH_02_CORE (`Story/0x81d83d96` 0x42c7, 0x43e6:
/// HideDistrict(0, 0), UnlockDistrict(0, 0), then ClearAllDistricts and EvaluateAllDistricts
/// 0x4401..0x445b) and RACE_SH_01_CORE (`Story/0xc6518ca3` 0x24f5, 0x2597: the same two calls, no
/// evaluation - the slots are filled at the next ClearAll / EvaluateAll: the garage, a career
/// load `game/career/0x62e9486d` 0x5e4..0x63e).
pub const DISTRICTS_WON: [(&str, usize, bool); 2] = [("RACE_GH_02", 0, true), ("RACE_SH_01", 0, false)];

/// (mission, district): the mission scripts that open a district (`game/career/missions/*.sco`,
/// `sc/career/missions/*.sco`). Districts: 1 Beaches, 2 Hills, 3 Downtown, 4 South Central.
pub const MISSION_DISTRICTS: [(&str, usize); 12] = [
    ("FIND_SH_01", 3),
    ("REPEAT_BESTOF_DOWNTOWN", 3),
    ("FIND_SH_03B", 1),
    ("GARAGE_BEACHES", 1),
    ("REPEAT_BESTOF_BEACHES", 1),
    ("RACE_SH_04_MOTORCYCLE", 2),
    ("REPEAT_BESTOF_HILLS", 2),
    ("BESTOF_GH_31", 4),
    ("DELIVERY_SC", 4),
    ("FIND_GH_27", 4),
    ("GARAGE_SC", 4),
    ("REPEAT_BESTOF_SC", 4),
];

impl Game {
    /// The block's district locks after the career moved; true when they changed. A block
    /// without the list gets it: a new career the new game's (every district locked, every
    /// slot hidden), a career under way every district open (the port had racers cruising
    /// everywhere).
    pub(crate) fn mc4_slots_apply(&self, m: &mut Mc4) -> bool {
        let mut changed = false;
        if m.career.cruising_slots.is_empty() {
            if !m.made_by_port() {
                return false;
            }
            for name in DISTRICTS {
                let locked = self.fresh_game;
                m.career.cruising_slots.push(CruisingSlots { district: name.to_string(), district_locked: locked, slots_hidden: vec![locked; 3], extra: Extra::new() });
            }
            changed = true;
        }
        let won = DISTRICTS_WON.iter().filter(|r| self.career.state_at_least(r.0, 5)).map(|r| (r.0, r.1));
        for (_, d) in MISSION_DISTRICTS.into_iter().filter(|r| self.career.mission_started(r.0)).chain(won) {
            if let Some(s) = m.career.cruising_slots.get_mut(d) {
                if s.district_locked || s.slots_hidden.iter().any(|h| *h) {
                    s.district_locked = false;
                    s.slots_hidden.iter_mut().for_each(|h| *h = false);
                    changed = true;
                }
            }
        }
        changed
    }

    /// The district's cruising slots are in use (mcSlotSystem_IsDistrictLocked, F-54458, is
    /// false for it). A name that is no district of the five, and a game without the block
    /// that is not a new career, are open.
    pub fn mc4_district_open(&self, district: &str) -> bool {
        let Some(d) = DISTRICTS.iter().position(|n| n.eq_ignore_ascii_case(district.trim())) else { return true };
        match self.mc4.as_ref().and_then(|m| m.career.cruising_slots.get(d)) {
            Some(s) => !s.district_locked,
            None => !self.fresh_game,
        }
    }

    /// The district locks brought up to date in the block in hand (`mc4_slots_apply`).
    pub fn mc4_slots_sync(&mut self) {
        if let Some(mut m) = self.mc4.take() {
            if self.mc4_slots_apply(&mut m) {
                self.dirty = true;
            }
            self.mc4 = Some(m);
        }
    }

    /// A district that has just opened has its slots evaluated at once: the missions'
    /// UnlockDistrict(d, 1) and RACE_GH_02's EvaluateAllDistricts; Hollywood opened by RACE_SH_01
    /// alone waits for the next evaluation (`DISTRICTS_WON`).
    pub fn mc4_open_evaluates(&self, d: usize) -> bool {
        d != 0 || DISTRICTS_WON.iter().any(|r| r.2 && r.1 == d && self.career.state_at_least(r.0, 5))
    }
}

// ---- step 17b: the launchers' unlocks, the ARCADE menu's rows, the rewards screen's car page ----

/// The launcher scripts that unlock their races when they start (step 17b): (script, races).
pub const LAUNCHERS: [(&str, &[&str]); 22] = [
    // series hangout
    ("be_local_hangout1_setup", &["game/Race/Beaches/Ordered/be_ordered_jj_03", "game/Race/Beaches/Ordered/be_ordered_mb_06", "game/Race/Beaches/Circuit/be_circuit_mg_01b", "game/Race/Beaches/Circuit/be_circuit_mg_02", "game/Race/Beaches/Ordered/be_ordered_ds_01"]),
    // time trial hookman
    ("be_time_trial_hk1", &["game/Race/Beaches/Ordered/be_ordered_mb_04"]),
    // tournament hookman
    ("be_tournament_hk1", &["game/Race/Beaches/Ordered/be_ordered_mb_06", "game/Race/Hollywood/Ordered/hw_ordered_mb_02", "game/Race/Beaches/Ordered/be_ordered_ds_02"]),
    // wager hangout
    ("be_wager_hangout1_setup", &["game/Race/Beaches/Ordered/be_ordered_mb_01", "game/Race/Beaches/Ordered/be_ordered_mg_02", "game/Race/Beaches/Circuit/be_circuit_mg_02", "game/Race/Beaches/Ordered/be_ordered_ds_02"]),
    // series hangout
    ("dt_local_hangout1_setup", &["game/Race/Hollywood/Ordered/hw_ordered_ds_04", "game/Race/Downtown/Ordered/dt_ordered_mb_01", "game/Race/Hollywood/Circuit/hw_circuit_gs_01", "game/Race/Downtown/Circuit/dt_circuit_mg_01", "game/Race/Downtown/Ordered/dt_ordered_ts_01"]),
    // time trial hookman
    ("dt_time_trial_hk1", &["game/Race/Downtown/Ordered/dt_ordered_mb_04"]),
    // tournament hookman
    ("dt_tournament_hk1", &["game/Race/Downtown/Ordered/dt_ordered_ds_01", "game/Race/Downtown/Circuit/dt_circuit_jp_01", "game/Race/Downtown/Ordered/dt_ordered_mb_02"]),
    // wager hangout
    ("dt_wager_hangout1_setup", &["game/Race/Downtown/Circuit/dt_circuit_as_02", "game/Race/Downtown/Ordered/dt_ordered_mb_03", "game/Race/Downtown/Ordered/dt_ordered_jp_02", "game/Race/Downtown/Ordered/dt_ordered_jp_01"]),
    // series hangout
    ("hl_local_hangout1_setup", &["game/Race/Hills/Ordered/hl_ordered_ts_04", "game/Race/Hollywood/Ordered/hw_ordered_jj_01", "game/Race/Hills/Circuit/hl_circuit_jj_02", "game/Race/Hills/Circuit/hl_circuit_jj_01", "game/Race/Hills/Ordered/hl_ordered_ts_05"]),
    // time trial hookman
    ("hl_time_trial_hk1", &["game/Race/Hills/Ordered/hl_ordered_ts_03"]),
    // tournament hookman
    ("hl_tournament_hk1", &["game/Race/Hills/Ordered/hl_ordered_mb_03", "game/Race/Hollywood/Ordered/hw_ordered_mb_07b", "game/Race/Hills/Ordered/hl_ordered_jj_03"]),
    // wager hangout
    ("hl_wager_hangout1_setup", &["game/Race/Hills/Ordered/hl_ordered_jj_01", "game/Race/Hills/Ordered/hl_ordered_ts_02", "game/Race/Hills/Circuit/hl_circuit_jj_02", "game/Race/Hills/Ordered/hl_ordered_ts_01"]),
    // series hangout
    ("hw_local_hangout1_setup", &["game/Race/Hollywood/Ordered/hw_ordered_mg_01", "game/Race/Hollywood/Ordered/hw_ordered_ds_02", "game/Race/Downtown/Circuit/dt_circuit_as_01", "game/Race/Hollywood/Circuit/hw_circuit_jp_03", "game/Race/Beaches/Circuit/be_circuit_jj_02"]),
    // series hangout
    ("hw_local_hangout2_setup", &["game/Race/Hollywood/Circuit/hw_circuit_mb_04a", "game/Race/Hills/Ordered/hl_ordered_ts_05", "game/Race/Beaches/Ordered/be_ordered_mg_02", "game/Race/Hills/Circuit/hl_circuit_ds_01", "game/Race/Hollywood/Circuit/hw_circuit_jj_01c"]),
    // series hangout
    ("hw_local_hangout3_setup", &["game/Race/Hollywood/Ordered/hw_ordered_jp_10", "game/Race/Hollywood/Ordered/hw_ordered_dh_02c", "game/Race/Hollywood/Circuit/hw_circuit_jp_02", "game/Race/Downtown/Circuit/dt_circuit_mb_01", "game/Race/Hollywood/Ordered/hw_ordered_ds_03"]),
    // time trial hookman
    ("hw_time_trial_hk1", &["game/Race/Hollywood/Circuit/hw_circuit_mb_01"]),
    // tournament hookman
    ("hw_tournament_hk1", &["game/Race/Hollywood/Ordered/hw_ordered_gs_01", "game/Race/Hollywood/Circuit/hw_circuit_ds_02a", "game/Race/Hollywood/Ordered/hw_ordered_mb_05"]),
    // wager hangout
    ("hw_wager_hangout1_setup", &["game/Race/Hollywood/Ordered/hw_ordered_mb_01", "game/Race/Hollywood/Ordered/hw_ordered_jp_10", "game/Race/Hollywood/Ordered/hw_ordered_mb_03a", "game/Race/Hollywood/Ordered/hw_ordered_jp_04"]),
    // series hangout
    ("sc_local_hangout1_setup", &["game/Race/SouthCentral/Circuit/sc_circuit_ds_01", "game/Race/SouthCentral/Ordered/sc_ordered_dh_03", "game/Race/SouthCentral/Ordered/sc_ordered_jp_02", "game/Race/SouthCentral/Ordered/sc_ordered_jp_03", "game/Race/SouthCentral/Ordered/sc_ordered_mb_04"]),
    // time trial hookman
    ("sc_time_trial_hk1", &["game/Race/SouthCentral/Ordered/sc_ordered_dh_11"]),
    // tournament hookman
    ("sc_tournament_hk1", &["game/Race/SouthCentral/Circuit/sc_circuit_jj_01b", "game/Race/SouthCentral/Ordered/sc_ordered_mb_01", "game/Race/SouthCentral/Ordered/sc_ordered_cb_02"]),
    // wager hangout
    ("sc_wager_hangout1_setup", &["game/Race/SouthCentral/Ordered/sc_ordered_dh_02", "game/Race/SouthCentral/Ordered/sc_ordered_jp_01", "game/Race/SouthCentral/Circuit/sc_circuit_cb_02", "game/Race/SouthCentral/Ordered/sc_ordered_mb_03"]),
];

/// The mission scripts that start one of them: (mission, launcher).
pub const MISSION_LAUNCHERS: [(&str, &str); 51] = [
    ("FIND_SH_04", "dt_wager_hangout1_setup"),
    ("FIND_SH_05", "dt_local_hangout1_setup"),
    ("HANGOUT_PINKSLIP_BEACHES", "be_wager_hangout1_setup"),
    ("HANGOUT_PINKSLIP_DOWNTOWN", "dt_wager_hangout1_setup"),
    ("HANGOUT_PINKSLIP_HILLS", "hl_wager_hangout1_setup"),
    ("HANGOUT_PINKSLIP_HOLLYWOOD", "hw_wager_hangout1_setup"),
    ("HANGOUT_PINKSLIP_SC", "sc_wager_hangout1_setup"),
    ("HANGOUT_SERIES_BEACHES", "be_local_hangout1_setup"),
    ("HANGOUT_SERIES_DOWNTOWN", "dt_local_hangout1_setup"),
    ("HANGOUT_SERIES_HILLS", "hl_local_hangout1_setup"),
    ("HANGOUT_SERIES_HOLLYWOOD", "hw_local_hangout1_setup"),
    ("HANGOUT_SERIES_HOLLYWOODB", "hw_local_hangout2_setup"),
    ("HANGOUT_SERIES_HOLLYWOODC", "hw_local_hangout3_setup"),
    ("HANGOUT_SERIES_SC", "sc_local_hangout1_setup"),
    ("HANGOUT_WAGER_BEACHES", "be_wager_hangout1_setup"),
    ("HANGOUT_WAGER_DOWNTOWN", "dt_wager_hangout1_setup"),
    ("HANGOUT_WAGER_HILLS", "hl_wager_hangout1_setup"),
    ("HANGOUT_WAGER_HOLLYWOOD", "hw_wager_hangout1_setup"),
    ("HANGOUT_WAGER_SC", "sc_wager_hangout1_setup"),
    ("REPEAT_HANGOUT_WAGER_BEACHES", "be_wager_hangout1_setup"),
    ("REPEAT_HANGOUT_WAGER_DOWNTOWN", "dt_wager_hangout1_setup"),
    ("REPEAT_HANGOUT_WAGER_HILLS", "hl_wager_hangout1_setup"),
    ("REPEAT_HANGOUT_WAGER_HOLLYWOOD", "hw_wager_hangout1_setup"),
    ("REPEAT_HANGOUT_WAGER_SC", "sc_wager_hangout1_setup"),
    ("REPEAT_TOURNAMENT_BEACHES", "be_tournament_hk1"),
    ("REPEAT_TOURNAMENT_DOWNTOWN", "dt_tournament_hk1"),
    ("REPEAT_TOURNAMENT_HILLS", "hl_tournament_hk1"),
    ("REPEAT_TOURNAMENT_HOLLYWOOD", "hw_tournament_hk1"),
    ("REPEAT_TOURNAMENT_SC", "sc_tournament_hk1"),
    ("TIMETRIAL_BEACHES", "be_time_trial_hk1"),
    ("TIMETRIAL_BEACHESB", "be_time_trial_hk1"),
    ("TIMETRIAL_BEACHESC", "be_time_trial_hk1"),
    ("TIMETRIAL_DOWNTOWN", "dt_time_trial_hk1"),
    ("TIMETRIAL_DOWNTOWNB", "dt_time_trial_hk1"),
    ("TIMETRIAL_DOWNTOWNC", "dt_time_trial_hk1"),
    ("TIMETRIAL_HILLS", "hl_time_trial_hk1"),
    ("TIMETRIAL_HILLSB", "hl_time_trial_hk1"),
    ("TIMETRIAL_HILLSC", "hl_time_trial_hk1"),
    ("TIMETRIAL_HOLLYWOOD", "hw_time_trial_hk1"),
    ("TIMETRIAL_HOLLYWOODB", "hw_time_trial_hk1"),
    ("TIMETRIAL_HOLLYWOODC", "hw_time_trial_hk1"),
    ("TIMETRIAL_SCB", "sc_time_trial_hk1"),
    ("TIMETRIAL_SCC", "sc_time_trial_hk1"),
    ("TOURNAMENT_BEACHES", "be_tournament_hk1"),
    ("TOURNAMENT_BEACHESB", "be_tournament_hk1"),
    ("TOURNAMENT_DOWNTOWN", "dt_tournament_hk1"),
    ("TOURNAMENT_DOWNTOWNB", "dt_tournament_hk1"),
    ("TOURNAMENT_HILLS", "hl_tournament_hk1"),
    ("TOURNAMENT_HILLSB", "hl_tournament_hk1"),
    ("TOURNAMENT_HOLLYWOOD", "hw_tournament_hk1"),
    ("TOURNAMENT_HOLLYWOODB", "hw_tournament_hk1"),
];

/// The red light hookmen's ids by district (Hollywood, Beaches, Hills, Downtown, South
/// Central: `hw_redlight_hk1.sco` .. `sc_redlight_hk1.sco`, mcHookman_SetId).
pub const REDLIGHT_HOOKMEN: [u32; 5] = [350, 351, 352, 353, 354];

/// The mark before the title of a results panel that is the time trial's (form 42,
/// `TIMETRIAL_RESULTS_TITLE`; constructor F-72304, update F-65302): a list of
/// the rewards' template with the header TIME_HEADER - row kind 25: the COMPLETE / FAIL
/// stamp by `bComplete` in its fourth text, the number of rows under it in its second -
/// and TARGET_TIME_FORMAT, YOUR_TIME_FORMAT as rows of kind 19; one prompt, CONTINUE
/// (table 0x82906FE4). The script shows it after the banner of form 56 (the end of race
/// sequence's indices 12 and 5: generic_timetrial_hookman.sco 0x255d..0x2647, 0x2952..0x2990).
pub const TT_PANEL: char = '\u{1}';
/// the purse's limit either way (AddMoney F-71075 clamps above, SpendMoney F-38145 below)
pub const PURSE_CAP: i64 = 2_140_000_000;

/// One row of the ARCADE menu's options page.
#[derive(Clone, Debug, PartialEq)]
pub struct ArcadeRow {
    pub label: String,
    /// where its choice is kept: 0..5 `Game::arcade_opts`, 6.. `Game::arcade_x`
    pub slot: usize,
    pub choices: Vec<String>,
    /// the choice DEFAULT puts back (F-84256)
    pub default: usize,
}

impl Game {
    /// mcCareer_GetAltHookmanVisibility (F-25257): the alternative hookman - one who
    /// holds no cruising slot: the freeway racers 300..307 and the red light racers 350..354 -
    /// has been marked visible. A save without the block: no.
    pub fn mc4_alt_visible(&self, uid: u32) -> bool {
        self.mc4.as_ref().map_or(false, |m| m.career.alt_hookmen.iter().any(|a| a.uid == uid && a.visible))
    }

    /// The player has pulled up beside a district's red light racer. The hookman's script
    /// (`<district>_redlight_hk1.sco`, e.g. `game/CruisingHookmen/0x18ac539b` 0x3e58..0x3f3f):
    /// when the player can flash him and he is not yet marked visible (or is not hidden),
    /// he is marked - mcCareer_SetAltHookmanVisibility(id, 1) - and every race of the
    /// script's list that is not unlocked is unlocked: the district's landmark (Blitz)
    /// races (Hollywood 4, Beaches 3, Hills 2, Downtown 3; South Central's script names none).
    pub fn mc4_redlight_seen(&mut self, district: &str) {
        let d = number_of(&RACE_DISTRICTS, district);
        if !(0..5).contains(&d) {
            return;
        }
        let uid = REDLIGHT_HOOKMEN[d as usize];
        if self.mc4.as_ref().map_or(false, |m| m.career.alt_hookmen.iter().any(|a| a.uid == uid && a.visible)) {
            return;
        }
        let races = if d < 4 { self.mc4_district_races(d, -1, 1 << 5) } else { Vec::new() };
        let mut m = self.mc4.take().unwrap_or_else(|| Mc4::seeded(self.fresh_game));
        match m.career.alt_hookmen.iter_mut().find(|a| a.uid == uid) {
            Some(a) => a.visible = true,
            None => m.career.alt_hookmen.push(AltHookman { uid, visible: true, extra: Extra::new() }),
        }
        // (a block without a list lists every race: nothing to add to)
        if m.made_by_port() || !m.career.races_unlocked.is_empty() {
            self.mc4_races_apply(&mut m);
            for r in races {
                m.career.unlock_race(&r);
            }
        }
        self.mc4 = Some(m);
        self.dirty = true;
    }

    /// The rows of the ARCADE menu's options page for a race (F-71627, the off line
    /// branch A-67715..A-92713; the widgets and their lists: F-14101; the
    /// defaults: F-84256):
    /// NUMBER OF OPPONENTS 1 .. the race's `OpponentsAllowed` (default 3, or that if less);
    /// LAST MAN OUT off / on - a circuit only; DIFFICULTY Easy / Medium / Hard / Impossible
    /// (default Medium); NUMBER OF LAPS 1 .. 5 (default the race's `NumLaps`) - a circuit
    /// only; POWER UPS off / on / Custom; TRAFFIC none / low / default (default the last);
    /// WEATHER dynamic / Clear / Cloudy / Foggy / Stormy; TIME OF DAY dynamic / Midnight /
    /// Dawn / Noon / Dusk.
    pub fn mc4_arcade_rows(&self, race: Option<usize>) -> Vec<ArcadeRow> {
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_string()).filter(|v| !v.contains('<')).unwrap_or_else(|| d.to_string()).to_uppercase();
        let list = |keys: &[(&str, &str)]| keys.iter().map(|(k, d)| st(k, d)).collect::<Vec<_>>();
        let def = race.and_then(|i| self.race_defs.get(i));
        let circuit = def.map_or(false, |d| d.sub_type.eq_ignore_ascii_case("Circuit"));
        let allowed = def.map_or(7, |d| d.opponents).clamp(1, 15);
        let laps = def.map_or(2, |d| d.laps as usize).clamp(1, 5);
        let mut rows = vec![ArcadeRow { label: st("RO_NumberofOpponents", "NUMBER OF OPPONENTS"), slot: 0, choices: (1..=allowed).map(|n| n.to_string()).collect(), default: allowed.min(3) - 1 }];
        if circuit {
            rows.push(ArcadeRow { label: st("RO_LastManOut", "LAST MAN OUT"), slot: 7, choices: list(&[("off", "OFF"), ("on", "ON")]), default: 0 });
        }
        rows.push(ArcadeRow { label: st("RO_Difficulty", "DIFFICULTY"), slot: 1, choices: list(&[("Easy", "EASIEST"), ("Medium", "MEDIUM"), ("Hard", "HARD"), ("Impossible", "HARDEST")]), default: 1 });
        if circuit {
            rows.push(ArcadeRow { label: st("RO_NumberofLaps", "NUMBER OF LAPS"), slot: 6, choices: (1..=5).map(|n| n.to_string()).collect(), default: laps - 1 });
        }
        rows.push(ArcadeRow { label: st("RO_PowerUps", "POWER UPS"), slot: 2, choices: list(&[("off", "OFF"), ("on", "ON"), ("Custom", "CUSTOM")]), default: 0 });
        // (Custom: the eleven power-ups follow, each off / on, on by default - F-93221,
        // F-84256 A-93001..A-92303)
        if self.arcade_opts[2] == 2 {
            for (k, (key, text)) in POWER_UP_ROWS.iter().enumerate() {
                let label = self.career.strings.get(*key).map(|v| v.to_uppercase()).unwrap_or_else(|| format!("\u{b1}{text}"));
                rows.push(ArcadeRow { label, slot: ARCADE_CUSTOM_SLOT + k, choices: list(&[("off", "OFF"), ("on", "ON")]), default: 1 });
            }
        }
        rows.push(ArcadeRow { label: st("RO_Traffic", "TRAFFIC"), slot: 3, choices: list(&[("traf_none", "NONE"), ("traf_low", "LOW"), ("traf_default", "NORMAL")]), default: 2 });
        rows.push(ArcadeRow { label: st("RO_Weather", "WEATHER"), slot: 4, choices: list(&[("RO_Dynamic", "DYNAMIC"), ("Clear", "CLEAR"), ("Cloudy", "PARTLY CLOUDY"), ("Foggy", "CLOUDY"), ("Stormy", "RAINY")]), default: 0 });
        rows.push(ArcadeRow { label: st("RO_TimeofDay", "TIME OF DAY"), slot: 5, choices: list(&[("RO_Dynamic", "DYNAMIC"), ("Midnight", "MIDNIGHT"), ("Dawn", "DAWN"), ("Noon", "NOON"), ("Dusk", "DUSK")]), default: 0 });
        rows
    }

    /// The choice of an ARCADE row.
    pub fn mc4_arcade_value(&self, slot: usize) -> usize {
        if let Some(k) = Self::mc4_custom_row(slot) {
            return !self.opt_x.arcade.custom_off[k] as usize;
        }
        if slot < 6 { self.arcade_opts[slot] } else { self.arcade_x[(slot - 6).min(1)] }
    }

    /// The laps of a circuit started from the ARCADE menu: the menu's NUMBER OF LAPS, which
    /// its launcher puts in the race data (F-28426 A-66352: +3116).
    pub fn mc4_arcade_laps(&self, def: &crate::races::RaceDef) -> Option<usize> {
        // (LAST MAN OUT: as many laps as opponents - RaceWrapper, `game/race/0xc4e26a20`
        // 0x2bd1..0x2bfa: +3116 = the racers less one)
        if self.arcade_restore.is_some() && def.sub_type.eq_ignore_ascii_case("Circuit") && self.arcade_x[1] != 0 {
            return Some(def.opponents.max(1));
        }
        (self.arcade_restore.is_some() && def.sub_type.eq_ignore_ascii_case("Circuit")).then(|| self.arcade_x[0].min(4) + 1)
    }

    /// The text of a Car reward on the rewards screen: the string of the reward's
    /// description, shown through `EOR_car_reward_format` (F-36204 A-65125..A-72924;
    /// the string table has the cars' set-up names as keys: `timetrial_vp_mit_eclipse_99`
    /// = "1999 ECLIPSE GSX"). A key the table has not: `!!key!!`.
    pub fn mc4_car_reward_label(&self, name: &str) -> String {
        let s = &self.career.strings;
        match [name.to_string(), name.to_ascii_lowercase()].iter().find_map(|k| s.get(k)).filter(|v| !v.contains('<')) {
            Some(v) => v.to_uppercase(),
            None => format!("!!{name}!!"),
        }
    }
}

// ---- step 18: time trials ----
//
// A district has one time trial: a hookman of type 3 (ids 250..254) cruising with one race and
// four vehicles, each with a target time (`<district>_time_trial_hk1.sco` sets the tables and
// starts generic_timetrial_hookman.sco). The save holds, per district, the flag "unlocked"
// (career +0x16C2C) and the four current target times in ms (manager +260); the best times are
// statistics, `k:TB` with the sub id district * 4 + vehicle.

/// One district's time trial, as its hookman's script sets it up.
pub struct TimeTrialDef {
    /// the identifier of the MISSION_EVENT the hookman posts for a beaten time
    pub key: &'static str,
    pub race: &'static str,
    pub hookman: u32,
    /// the vehicles' set-ups, row by row
    pub cars: [&'static str; 4],
    /// mcCareer_SetTimeTrialTargetTime(district, row, 0, ms): the first target
    pub first: [u32; 4],
    /// (district, row, 1, ms): the bound the target is tightened towards
    pub low: [u32; 4],
    /// the three missions of the district
    pub missions: [&'static str; 3],
}

/// By district: Hollywood, Beaches, Hills, Downtown, South Central (`game/CruisingHookmen/`
/// `0x92269d82`, `0xb5f960a7`, `0x5d500a1a`, `0xfde0f29a`, `sc/CruisingHookmen/0x31932ec9`).
pub const TIME_TRIALS: [TimeTrialDef; 5] = [
    TimeTrialDef { key: "TIMETRIAL_HW", race: "game/Race/Hollywood/Circuit/hw_circuit_mb_01", hookman: 250, cars: ["timetrial_vp_mit_eclipse_99", "timetrial_vp_vw_golf_83", "timetrial_vp_frd_mustang_06", "timetrial_vp_lam_gallardo_06"], first: [145000, 150000, 137000, 115000], low: [125000, 135000, 120000, 103000], missions: ["TIMETRIAL_HOLLYWOOD", "TIMETRIAL_HOLLYWOODB", "TIMETRIAL_HOLLYWOODC"] },
    TimeTrialDef { key: "TIMETRIAL_BE", race: "game/Race/Beaches/Ordered/be_ordered_mb_04", hookman: 251, cars: ["timetrial_vp_frd_boss302_69", "timetrial_vp_frd_focus_04", "timetrial_vp_chv_camaro_06", "timetrial_vp_frd_gt_06"], first: [240000, 230000, 210000, 173000], low: [197000, 210000, 185000, 150000], missions: ["TIMETRIAL_BEACHES", "TIMETRIAL_BEACHESB", "TIMETRIAL_BEACHESC"] },
    TimeTrialDef { key: "TIMETRIAL_HL", race: "game/Race/Hills/Ordered/hl_ordered_ts_03", hookman: 252, cars: ["timetrial_vp_lam_miura_71", "timetrial_vp_vw_scirocco_88", "timetrial_vp_pon_transam_79", "timetrial_vp_vw_golf_07"], first: [210000, 230000, 220000, 205000], low: [175000, 207000, 200000, 185000], missions: ["TIMETRIAL_HILLS", "TIMETRIAL_HILLSB", "TIMETRIAL_HILLSC"] },
    TimeTrialDef { key: "TIMETRIAL_DT", race: "game/Race/Downtown/Ordered/dt_ordered_mb_04", hookman: 253, cars: ["timetrial_vp_mit_3000gt_99", "timetrial_vp_dge_challenger_70", "timetrial_vp_mbz_sl65_07", "timetrial_vp_kaw_ninja_zx14_06"], first: [180000, 170000, 150000, 135000], low: [150000, 150000, 130000, 115000], missions: ["TIMETRIAL_DOWNTOWN", "TIMETRIAL_DOWNTOWNB", "TIMETRIAL_DOWNTOWNC"] },
    TimeTrialDef { key: "TIMETRIAL_SC", race: "game/Race/SouthCentral/Ordered/sc_ordered_dh_11", hookman: 254, cars: ["SC_timetrial_vpd_chv_impala_64", "SC_timetrial_vpd_pon_gto_66", "SC_timetrial_vpd_lr_hse_08", "SC_timetrial_vpd_mbz_clk63amg_08"], first: [180000, 160000, 165000, 135000], low: [150000, 120000, 120000, 120000], missions: ["TIMETRIAL_SC", "TIMETRIAL_SCB", "TIMETRIAL_SCC"] },
];

/// mcCareer_EvaluateCurrentTargetTime (F-96015 -> F-93929): with the player's best
/// time above the bound, the target becomes best - (best - bound) / 10; with a best time at
/// or below it, best - 15; with no best time it stays.
pub fn tt_evaluate(best: i64, low: u32) -> Option<u32> {
    if best > low as i64 {
        Some((best - (best - low as i64) / 10) as u32)
    } else if best != 0 {
        Some((best - 15).max(0) as u32)
    } else {
        None
    }
}

/// A time beats a target as the result's routine has it (F-58463 A-63797..A-52016,
/// the same test in F-83470): both cut to hundredths of a second, time not above target.
pub fn tt_beaten(time_ms: i64, target_ms: i64) -> bool {
    target_ms / 10 * 10 - time_ms / 10 * 10 >= 0
}

/// A race clock as the game writes it (UIManager_FormatRaceClock, F-26034 "%s%02d:%02d.%02d"): mm:ss.cc.
pub fn tt_clock(ms: i64) -> String {
    crate::flash_ui::race_time(ms.clamp(0, i32::MAX as i64) as i32)
}

/// One row of the hookman's panel: the vehicle, its target time, the player's best.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeTrialRow {
    pub car: String,
    pub target: String,
    pub best: String,
    pub enabled: bool,
}

fn tt_block(m: &mut Mc4, d: usize) -> &mut TimeTrials {
    while m.career.time_trials.len() < 5 {
        let k = m.career.time_trials.len();
        m.career.time_trials.push(TimeTrials { district: DISTRICTS[k].to_string(), unlocked: false, target_ms: vec![0; 4], extra: Extra::new() });
    }
    let t = &mut m.career.time_trials[d.min(4)];
    if t.target_ms.len() < 4 {
        t.target_ms.resize(4, 0);
    }
    t
}

impl Game {
    /// mcCareer_IsTimeTrialUnlocked (F-37759).
    pub fn mc4_tt_unlocked(&self, d: usize) -> bool {
        self.mc4.as_ref().and_then(|m| m.career.time_trials.get(d)).map_or(false, |t| t.unlocked)
    }

    /// mcCareer_GetTimeTrialTargetTime (F-90742): the saved target; while that is 0, the
    /// script's first target.
    pub fn mc4_tt_target(&self, d: usize, k: usize) -> u32 {
        let (d, k) = (d.min(4), k.min(3));
        match self.mc4.as_ref().and_then(|m| m.career.time_trials.get(d)).and_then(|t| t.target_ms.get(k)).copied() {
            Some(t) if t != 0 => t,
            _ => TIME_TRIALS[d].first[k],
        }
    }

    /// The player's best time of a row, ms: the statistic `k:TB`, sub id district * 4 + row.
    pub fn mc4_tt_best(&self, d: usize, k: usize) -> Option<i64> {
        self.career.ostats.get(&format!("k:TB/{}", d * 4 + k)).map(|v| *v as i64).filter(|v| *v > 0)
    }

    /// The district's hookman is about: one of its missions is under way, or the time trial
    /// is unlocked (after a beaten time he is deleted until then: generic_timetrial_hookman.sco
    /// 0x33bc..0x3417).
    pub fn mc4_tt_open(&self, d: usize) -> bool {
        self.mc4_tt_unlocked(d) || TIME_TRIALS[d.min(4)].missions.iter().any(|m| self.career.mission_active(m))
    }

    /// The time trial a race is, if it is started as one: the district. (From the ARCADE
    /// menu the race is an ordinary race.)
    pub fn mc4_tt_of(&self, race_id: &str) -> Option<usize> {
        if self.arcade_restore.is_some() {
            return None;
        }
        let name = format!("game/Race/{}", race_rel(race_id));
        (0..5).find(|d| TIME_TRIALS[*d].race.eq_ignore_ascii_case(&name) && self.mc4_tt_open(*d))
    }

    /// The row the trial is driven with: the first until the district's time trial is
    /// unlocked (the panel disables the others, 0x616..0x726), then the one chosen
    /// (`career.tt_pick`; MCLA_TT_PICK: a test hook).
    pub fn mc4_tt_row(&self, d: usize) -> usize {
        if !self.mc4_tt_unlocked(d) {
            return 0;
        }
        std::env::var("MCLA_TT_PICK").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(self.career.tt_pick).min(3)
    }

    /// The hookman's panel (generic_timetrial_hookman.sco 0x41f..0x726, form 65): a row for
    /// each vehicle - its name (the set-up's string), the target time, the best time or
    /// `--:--.--`; before the district's time trial is unlocked only the first row is
    /// enabled, and the others carry no times.
    pub fn mc4_tt_rows(&self, d: usize) -> Vec<TimeTrialRow> {
        let def = &TIME_TRIALS[d.min(4)];
        let open = self.mc4_tt_unlocked(d);
        (0..4)
            .map(|k| {
                let on = open || k == 0;
                TimeTrialRow {
                    car: self.mc4_car_reward_label(def.cars[k]),
                    target: if on { tt_clock(self.mc4_tt_target(d, k) as i64) } else { String::new() },
                    best: if on { self.mc4_tt_best(d, k).map_or("--:--.--".to_string(), tt_clock) } else { String::new() },
                    enabled: on,
                }
            })
            .collect()
    }

    /// The vehicle (model folder), its set-up and the target time in seconds of the time
    /// trial a race is about to be started as.
    pub fn mc4_tt_run(&self, race_id: &str) -> Option<(String, String, f32)> {
        let d = self.mc4_tt_of(race_id)?;
        let k = self.mc4_tt_row(d);
        let cfg = TIME_TRIALS[d].cars[k];
        let l = cfg.to_ascii_lowercase();
        let car = l.find("vpd_").or_else(|| l.find("vp_")).map(|i| l[i..].to_string())?;
        Some((car, cfg.to_string(), self.mc4_tt_target(d, k) as f32 / 1000.0))
    }

    /// A race definition as its time trial runs it: one lap, the row's target time (the
    /// script: racers 1, opponents 0, laps +3116 = 1, difficulty +3100 = 1, 0xc69..0xcd2;
    /// Race_SetTargetTime with the target, 0x1d8e..0x1dab).
    pub fn mc4_tt_apply(&self, def: &mut crate::races::RaceDef) {
        if let Some((_, _, target)) = self.mc4_tt_run(&def.id) {
            def.laps = 1;
            def.goal_time = target;
        }
    }

    /// The difficulty a race is run at whatever its field: the ARCADE menu's, and Medium for
    /// a time trial.
    pub fn mc4_fixed_difficulty(&self) -> Option<usize> {
        let tt = self.current_race.and_then(|i| self.race_defs.get(i)).and_then(|d| self.mc4_tt_of(&d.id));
        self.mc4_arcade_difficulty().or(tt.map(|_| 1))
    }

    /// The block's time trial state after the career moved (`m`): true when it changed.
    /// - The hookman's script starts with a mission of the district; its
    ///   mcCareer_SetTimeTrialTargetTime(d, k, 0, ms) puts the first target where none is
    ///   saved (F-82902).
    /// - The third mission's reward runs `<district>_unlock_time_trial.sco`:
    ///   mcCareer_UnlockTimeTrial(d).
    fn mc4_tt_apply_block(&self, m: &mut Mc4) -> bool {
        // (the slot system's district locks go with it: step 19)
        let mut changed = self.mc4_slots_apply(m);
        for (d, def) in TIME_TRIALS.iter().enumerate() {
            let started = def.missions.iter().any(|x| self.career.mission_started(x));
            let done = self.career.mission_complete(def.missions[2]);
            if !started && !done {
                continue;
            }
            let t = tt_block(m, d);
            for k in 0..4 {
                if t.target_ms[k] == 0 {
                    t.target_ms[k] = def.first[k];
                    changed = true;
                }
            }
            if done && !t.unlocked {
                t.unlocked = true;
                changed = true;
            }
        }
        changed
    }

    /// The second and third mission of a district open with "your time has been beaten":
    /// their READY_TO_START script calls mcCareer_EvaluateCurrentTargetTime(d, 0, d * 4)
    /// (`game/career/events/TIMETRIAL_<D>B|C_READY_TO_START_ON_ENTER.sco` 0x1e0), which
    /// tightens the first row's target by the player's best time (`tt_evaluate`). It is done
    /// for the missions the career has just put at READY_TO_START (the reward of the
    /// `_PRE` mission, state READY_TO_START).
    pub fn mc4_tt_sync(&mut self) {
        // (the missions a reward has just put at READY_TO_START: `career.ready_now`, filled
        // where the career gives a Mission reward with that state - step 18b)
        let ready = std::mem::take(&mut self.career.ready_now);
        let fresh: Vec<String> = ready.into_iter().filter(|m| TIME_TRIALS.iter().any(|t| t.missions[1..].contains(&m.as_str()))).collect();
        // (the achievement counters the hookman's script sets after a run, 0x31e2..0x33bb:
        // `aBAT` when the four first missions of the main city are complete, `aB1T` when a
        // third mission is - mode 4, value 1)
        if self.career.tt_last.is_some() {
            if ["TIMETRIAL_BEACHES", "TIMETRIAL_DOWNTOWN", "TIMETRIAL_HILLS", "TIMETRIAL_HOLLYWOOD"].iter().all(|m| self.career.mission_complete(m)) {
                self.career.ostats.insert("aBAT/0".to_string(), 1.0);
            }
            if ["TIMETRIAL_BEACHESC", "TIMETRIAL_DOWNTOWNC", "TIMETRIAL_HILLSC", "TIMETRIAL_HOLLYWOODC"].iter().any(|m| self.career.mission_complete(m)) {
                self.career.ostats.insert("aB1T/0".to_string(), 1.0);
            }
        }
        let mut m = match self.mc4.take() {
            Some(m) => m,
            None if !fresh.is_empty() => Mc4::seeded(self.fresh_game),
            None => return,
        };
        let mut changed = self.mc4_tt_apply_block(&mut m);
        for id in fresh {
            let Some(d) = TIME_TRIALS.iter().position(|t| t.missions.contains(&id.as_str())) else { continue };
            if let Some(v) = tt_evaluate(self.mc4_tt_best(d, 0).unwrap_or(0), TIME_TRIALS[d].low[0]) {
                tt_block(&mut m, d).target_ms[0] = v;
                changed = true;
            }
        }
        self.mc4 = Some(m);
        if changed {
            self.dirty = true;
        }
    }

    /// A time trial is over (generic_timetrial_hookman.sco 0x2085..0x213b): `time` the
    /// player's time in seconds, None when the run was not finished. For a finished run the
    /// best time is kept - StatsManager_UpdateStatInt("k:TB", district * 4 + row, 1, time):
    /// the lower one stays -; a time not above the target (`tt_beaten`) is "complete": the
    /// hookman posts MISSION_EVENT with the district's key (the missions' objective) and
    /// TIME_TRIAL_RACE_COMPLETE (the money, F-58178). Returns whether the time was
    /// beaten; None when the race was not a time trial. The result's panel is put together
    /// (F-65302: TIME, the stamp, TARGET TIME, YOUR TIME).
    pub fn mc4_tt_end(&mut self, race_id: &str, time: Option<f32>) -> Option<bool> {
        self.career.tt_last = None;
        let d = self.mc4_tt_of(race_id)?;
        let k = self.mc4_tt_row(d);
        let target = self.mc4_tt_target(d, k) as i64;
        let ms = time.map(|t| (t as f64 * 1000.0).round() as i64);
        let beaten = ms.map_or(false, |t| tt_beaten(t, target));
        if let Some(t) = ms {
            let key = format!("k:TB/{}", d * 4 + k);
            let old = self.career.ostats.get(&key).copied().unwrap_or(0.0) as i64;
            if old <= 0 || t < old {
                self.career.ostats.insert(key, t as f64);
            }
            self.dirty = true;
        }
        let st = |k: &str, f: &str| self.career.strings.get(k).map(|v| v.to_string()).filter(|v| !v.contains('<')).unwrap_or_else(|| f.to_string());
        let line = |k: &str, f: &str, v: String| {
            let fmt = self.career.strings.get(k).map(|v| v.to_string()).unwrap_or_else(|| f.to_string());
            match fmt.find('<') {
                Some(i) => format!("{}{}", &fmt[..i], v),
                None => format!("{fmt} {v}"),
            }
        };
        // (the mark: the page is the time trial's own form, not a mission's - `TT_PANEL`)
        let panel = (format!("{}{}", TT_PANEL, st("TIME_HEADER", "TIME")), vec![line("TARGET_TIME_FORMAT", "TARGET TIME: ", tt_clock(target)), line("YOUR_TIME_FORMAT", "YOUR TIME: ", ms.map_or("--:--.--".to_string(), tt_clock))], beaten);
        self.mission_results.insert(0, panel);
        self.career.tt_last = Some(beaten);
        // (step 19d: form 56's banner for 3.5 s first - a run that was complete or failed;
        // `mc4_banner_page`)
        if ms.is_some() {
            self.opt_x.arcade.d.banner = Some((3.5, beaten));
            self.opt_x.arcade.d.banner_kind = 2;
        }
        Some(beaten)
    }
}

// ---- step 17: the unlocked races, the race history, the pages of the rewards screen ----
//
// The list of unlocked races (career +0x16844, 250 hashes of race names in lower case,
// filled from the front): written by F-81609 alone - the script command
// mcCareer_UnlockRace (F-61993), mcCareer_UnlockDistrictRaces (F-84897 ->
// F-86648: every race of the race table whose district, type and sub type pass) and
// every finished race that is not the editor's (F-20860); read by F-64698 - the
// ARCADE menu's lists (F-71344, F-52452), the hookmen's race arrays
// (mcRaceArray_Fill, F-50395) and mcCareer_IsRaceUnlocked.

/// The districts, race types and race sub types as the race records number them (the name
/// tables at 0x827E91DC, 0x827E92F0 and 0x827E930C; a record: type +0, sub type +4,
/// district +8).
pub const RACE_DISTRICTS: [&str; 7] = ["Hollywood", "Beaches", "Hills", "Downtown", "SouthCentral", "Unknown", "All"];
pub const RACE_TYPES: [&str; 7] = ["None", "Ordered", "Unordered", "CaptureTheFlag", "Delivery", "Payback", "Other"];
pub const RACE_SUB_TYPES: [&str; 14] = ["Unknown", "Ordered", "Circuit", "Unordered", "TimeTrial", "Blitz", "Delivery", "Payback", "CTF_FFA", "CTF_SplitBase", "CTF_BaseWar", "Stockpile_FFA", "Stockpile_SplitBase", "KeepAway"];

/// The twelve races the career's start-up script unlocks by name (CareerStartup.sco,
/// `game/career/0x400221ca`, 0x906..0xb7b), in its order.
pub const START_RACES: [&str; 12] = [
    "game/Race/Hollywood/Ordered/hw_ordered_jp_11",
    "game/Race/Hollywood/Ordered/hw_ordered_mg_01",
    "game/Race/Hollywood/Ordered/hw_ordered_ds_02",
    "game/Race/Downtown/Circuit/dt_circuit_as_01",
    "game/Race/Hollywood/Circuit/hw_circuit_jp_03",
    "game/Race/Beaches/Circuit/be_circuit_jj_02",
    "game/Race/Hollywood/Circuit/hw_circuit_mb_04a",
    "game/Race/Hills/Ordered/hl_ordered_ts_05",
    "game/Race/Beaches/Ordered/be_ordered_mg_02",
    "game/Race/Hills/Circuit/hl_circuit_ds_01",
    "game/Race/Hollywood/Circuit/hw_circuit_jj_01c",
    "game/Race/Downtown/Circuit/dt_circuit_mb_01",
];

/// The fourteen payback races be_payback_setup.sco unlocks (`game/Hangouts/0xbaa0ed14`,
/// 0x665..0x947), in its order.
pub const PAYBACK_RACES: [&str; 14] = [
    "game/Race/Beaches/Payback/be_payback_ds_01",
    "game/Race/Beaches/Payback/be_payback_ds_02",
    "game/Race/Hollywood/Payback/hw_payback_ds_01",
    "game/Race/Hollywood/Payback/hw_payback_ds_02",
    "game/Race/Hollywood/Payback/hw_payback_ds_03",
    "game/Race/Hollywood/Payback/hw_payback_ds_04",
    "game/Race/Hollywood/Payback/hw_payback_ds_05",
    "game/Race/Hollywood/Payback/hw_payback_ds_06",
    "game/Race/Hollywood/Payback/hw_payback_ts_01",
    "game/Race/Hollywood/Payback/hw_payback_ts_02",
    "game/Race/Downtown/Payback/dt_payback_ts_01",
    "game/Race/Downtown/Payback/dt_payback_ds_01",
    "game/Race/Downtown/Payback/dt_payback_ds_02",
    "game/Race/Hills/Payback/hl_payback_ds_01",
];

/// What the mission scripts unlock when their mission starts (`game/career/missions/*.sco`,
/// which run through once): (mission, races by name, district call).
const MISSION_UNLOCKS: [(&str, &[&str], Option<(i32, i32, i32)>); 9] = [
    // delivery.sco starts hw_delivery_setup.sco: mcCareer_UnlockDistrictRaces(0, 16, 64)
    ("DELIVERY", &[], Some((0, 16, 64))),
    // DELIVERY_SC.sco starts sc_delivery_setup.sco: (4, 16, 64)
    ("DELIVERY_SC", &[], Some((4, 16, 64))),
    // these four start be_payback_setup.sco
    ("PAYBACK_INTRO", &PAYBACK_RACES, None),
    ("PAYBACK", &PAYBACK_RACES, None),
    ("PAYBACK_GH_18", &PAYBACK_RACES, None),
    ("GARAGE_BEACHES", &PAYBACK_RACES, None),
    // hangout_series_hollywoodb.sco: (0, 2, 6)
    ("HANGOUT_SERIES_HOLLYWOODB", &[], Some((0, 2, 6))),
    ("HANGOUT_PINKSLIP_HILLS", &["game/Race/Hills/Ordered/hl_ordered_ts_01"], None),
    ("HANGOUT_PINKSLIP_HOLLYWOOD", &["game/Race/Hollywood/Ordered/hw_ordered_jp_04"], None),
];

/// The block of the port has had its list of unlocked races begun (step 17).
fn races_counted(m: &Mc4) -> bool {
    m.career.extra.get("races_counted").and_then(|v| v.as_bool()).unwrap_or(false)
}

/// A race's name without the `game/Race/` it may begin with (the race definitions carry
/// it, the port's own callers may not).
fn race_rel(id: &str) -> &str {
    match id.get(..10) {
        Some(p) if p.eq_ignore_ascii_case("game/Race/") => &id[10..],
        _ => id,
    }
}

fn number_of(names: &[&str], v: &str) -> i32 {
    names.iter().position(|n| n.eq_ignore_ascii_case(v.trim())).map_or(-1, |i| i as i32)
}

/// A race record (district, type, sub type) passes mcCareer_UnlockDistrictRaces' and
/// mcRaceArray_Fill's test (F-86648 A-60228..A-41892, F-50395): its district
/// is the one asked for, or 6 - all - is asked; the bit of its type is in `types`, or
/// `types` is -1; the same for the sub type.
pub fn race_passes(record: (i32, i32, i32), district: i32, types: i32, subs: i32) -> bool {
    let bit = |n: i32, mask: i32| mask == -1 || ((0..32).contains(&n) && mask & (1 << n) != 0);
    (record.0 == district || district == 6) && bit(record.1, types) && bit(record.2, subs)
}

/// One row of a page of the rewards screen after the first: (the row's kind - 17 the
/// REWARDED header, 13 the UNLOCKED header, 19 an item -, its text, the header's count).
pub type RewardRow = (i32, String, String);

/// The pages of the rewards form after the one of the reputation and the money
/// (F-36204, A-89717..A-74152; a row's page is its word +1268, the form's page
/// count +3680; CONTINUE shows the next page while there is one, F-48737):
/// - the REWARDED items (rewards of type AbilityLevel, PartLevel, Part), eight at the
///   most, under their header with their number, on a page of their own;
/// - then, when REWARDED had items and they, the UNLOCKED rewards and the two headers are
///   more than six rows, one more page is counted - it has no rows - and the count of
///   items starts again;
/// - the UNLOCKED items (rewards of type PerformanceLevel and PartUnlock, in the order
///   they were given) while the count of items is below eight - it goes on from REWARDED's
///   unless it started again -, under their header with the number of all of them.
pub fn reward_pages(rewarded: &[String], unlocked: &[String], unlocked_label: &str, rewarded_label: &str) -> Vec<Vec<RewardRow>> {
    let mut pages: Vec<Vec<RewardRow>> = Vec::new();
    let mut shown = 0usize;
    if !rewarded.is_empty() {
        let items: Vec<&String> = rewarded.iter().take(8).collect();
        shown = items.len();
        let mut rows = vec![(17, rewarded_label.to_string(), shown.to_string())];
        rows.extend(items.into_iter().map(|t| (19, t.clone(), String::new())));
        pages.push(rows);
    }
    if shown != 0 && unlocked.len() + shown + 2 > 6 {
        pages.push(Vec::new());
        shown = 0;
    }
    let items: Vec<&String> = unlocked.iter().take(8usize.saturating_sub(shown)).collect();
    if !items.is_empty() {
        let mut rows = vec![(13, unlocked_label.to_string(), unlocked.len().to_string())];
        rows.extend(items.into_iter().map(|t| (19, t.clone(), String::new())));
        pages.push(rows);
    }
    pages
}

impl Game {
    /// The race's name as the race table spells it (`game/Race/<district>/<kind>/<name>`).
    pub fn mc4_race_name(&self, id: &str) -> String {
        let want = format!("game/Race/{}", race_rel(id));
        self.race_table.iter().find(|t| t.eq_ignore_ascii_case(&want)).cloned().unwrap_or(want)
    }

    /// The race's place in the race table (`tune/race/0x0f1f4d8d`): what the history holds.
    pub fn mc4_race_index(&self, id: &str) -> Option<usize> {
        let want = format!("game/Race/{}", race_rel(id));
        self.race_table.iter().position(|t| t.eq_ignore_ascii_case(&want))
    }

    /// (district, type, sub type) of a race of the table, as its record numbers them.
    fn mc4_race_record(&self, table_name: &str) -> (i32, i32, i32) {
        let rel = race_rel(table_name);
        let mut path = rel.split('/');
        let (dir, kind) = (path.next().unwrap_or(""), path.next().unwrap_or(""));
        let of_kind = match number_of(&RACE_SUB_TYPES, kind) {
            1 | 2 | 5 => 1,
            3 => 2,
            6 => 4,
            7 => 5,
            8..=13 => 3,
            _ => -1,
        };
        match self.race_defs.iter().find(|d| race_rel(&d.id).eq_ignore_ascii_case(rel)) {
            Some(d) => {
                let pick = |a: i32, b: i32| if a >= 0 { a } else { b };
                (pick(number_of(&RACE_DISTRICTS, &d.district), number_of(&RACE_DISTRICTS, dir)), pick(number_of(&RACE_TYPES, &d.race_type), of_kind), pick(number_of(&RACE_SUB_TYPES, &d.sub_type), number_of(&RACE_SUB_TYPES, kind)))
            }
            // (the race's file was not read: the folders of its name say the same)
            None => (number_of(&RACE_DISTRICTS, dir), of_kind, number_of(&RACE_SUB_TYPES, kind)),
        }
    }

    /// The races mcCareer_UnlockDistrictRaces(district, types, subs) names: the races of the
    /// table that pass, in the table's order (F-86648; the test-only races too).
    pub fn mc4_district_races(&self, district: i32, types: i32, subs: i32) -> Vec<String> {
        self.race_table.iter().filter(|t| race_passes(self.mc4_race_record(t), district, types, subs)).cloned().collect()
    }

    /// The list a new career begins with: the start-up script's twelve races, then its
    /// mcCareer_UnlockDistrictRaces(6, 8, -1) - every flag game -, then South Central's
    /// start-up (`sc/career/0x70e862b0`): (4, -1, -1), every race of that district. (The
    /// script's (6, 63, 255) is in the branch of the `unlockcareer` command line.) This is
    /// the list of the new game sample, entry for entry: 127 races.
    pub fn mc4_start_races(&self) -> Vec<String> {
        let mut out: Vec<String> = START_RACES.iter().map(|s| s.to_string()).collect();
        for r in self.mc4_district_races(6, 8, -1).into_iter().chain(self.mc4_district_races(4, -1, -1)) {
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&r)) {
                out.push(r);
            }
        }
        out
    }

    /// The race (definition id) is one of `mc4_start_races`.
    fn mc4_start_race(&self, id: &str) -> bool {
        let name = format!("game/Race/{}", race_rel(id));
        if START_RACES.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
            return true;
        }
        let rec = self.mc4_race_record(&name);
        race_passes(rec, 6, 8, -1) || race_passes(rec, 4, -1, -1)
    }

    /// The list of unlocked races of the block is brought up to date; true when it changed.
    /// - A block the port made gets its list once: a new career the list of
    ///   `mc4_start_races`; a career that was under way before the list was kept, every
    ///   race of the table besides (what it had: every race was listed).
    /// - Every block: what the mission scripts unlock when their mission starts
    ///   (`MISSION_UNLOCKS`).
    pub(crate) fn mc4_races_apply(&self, m: &mut Mc4) -> bool {
        let mut changed = false;
        // (not before the race table is read: the list is made from it)
        if m.made_by_port() && !races_counted(m) && !self.race_table.is_empty() {
            m.career.extra.insert("races_counted".into(), Value::Bool(true));
            for r in self.mc4_start_races() {
                m.career.unlock_race(&r);
            }
            if !self.fresh_game {
                for r in self.race_table.clone() {
                    m.career.unlock_race(&r);
                }
            }
            changed = true;
        }
        if !m.made_by_port() && m.career.races_unlocked.is_empty() {
            return changed;
        }
        for (mission, names, district) in MISSION_UNLOCKS {
            if !self.career.mission_started(mission) {
                continue;
            }
            for n in names {
                changed |= m.career.unlock_race(n);
            }
            if let Some((d, t, s)) = district {
                for r in self.mc4_district_races(d, t, s) {
                    changed |= m.career.unlock_race(&r);
                }
            }
        }
        // (the launchers a mission's script starts unlock their races when they start - the
        // series and wager hangouts' core scripts, the tournament and time trial hookmen's:
        // step 17b)
        for (mission, launcher) in MISSION_LAUNCHERS {
            if !self.career.mission_started(mission) {
                continue;
            }
            for (_, races) in LAUNCHERS.iter().filter(|(l, _)| *l == launcher) {
                for r in races.iter() {
                    changed |= m.career.unlock_race(&self.mc4_race_name(r));
                }
            }
        }
        // (and the launchers the career's list of scripts holds: running when the save was
        // made, started again by the load - A-23923 -> F-33294; step 20)
        let running: Vec<String> = m.career.scripts.iter().chain(m.career.addon.scripts.iter()).map(|s| s.name.rsplit('/').next().unwrap_or("").to_ascii_lowercase()).collect();
        for (launcher, races) in LAUNCHERS.iter() {
            if running.iter().any(|n| n == launcher) {
                for r in races.iter() {
                    changed |= m.career.unlock_race(&self.mc4_race_name(r));
                }
            }
        }
        changed
    }

    /// `mc4_races_apply` on the block in hand (a game without one is left without).
    pub fn mc4_races_sync(&mut self) {
        if let Some(mut m) = self.mc4.take() {
            if self.mc4_races_apply(&mut m) {
                self.dirty = true;
            }
            self.mc4 = Some(m);
        }
        self.mc4_tt_sync();
    }

    /// A race of the race table starts: it joins the history of the last twelve
    /// (mcCareer_AddRaceToHistory with the race's index, by the scripts that run a race:
    /// RaceWrapper.sco 0x11a6, TournamentCore.sco 0x10d2, OrderedSeriesCore.sco 0x36df,
    /// PaybackCore.sco 0xb93, TelephoneChallenge.sco, the hookmen's and the wager's; the
    /// delivery's script has no such call, and an editor race is not of the table).
    pub fn mc4_race_started(&mut self, idx: usize) {
        let Some(d) = self.race_defs.get(idx) else { return };
        if d.id.starts_with("user/") || d.sub_type.eq_ignore_ascii_case("Delivery") {
            return;
        }
        // (a time trial: the hookman has been visited - `tTTA` with his id is set when his
        // panel is filled, generic_timetrial_hookman.sco 0x72b..0x74d; the port has no panel,
        // the start of the run stands for it)
        if let Some(tt) = self.mc4_tt_of(&d.id) {
            self.career.ostats.insert(format!("tTTA/{}", TIME_TRIALS[tt].hookman), 1.0);
        }
        let Some(k) = self.mc4_race_index(&d.id) else { return };
        let name = self.race_table[k].clone();
        let mut m = self.mc4.take().unwrap_or_else(|| Mc4::seeded(self.fresh_game));
        m.career.add_race_to_history(k as i64, Some(name));
        self.mc4 = Some(m);
        self.dirty = true;
    }

    /// The difficulty of a race started from the ARCADE menu: the menu's DIFFICULTY, which
    /// its launcher puts in the race data (F-28426 A-78403: `nDifficulty` to +3100).
    pub fn mc4_arcade_difficulty(&self) -> Option<usize> {
        self.arcade_restore.is_some().then(|| self.arcade_opts[1].min(3))
    }

    /// Is the race driven on its easy layout (`EasyRace`, more checkpoints)? The race
    /// scripts take it when the race data's layout word +3112 is 0 (TournamentCore.sco 0x10ed,
    /// and the race has one); the word is 0 for a difficulty below 2 - Easy, Medium - and 1
    /// from Hard on: the ARCADE launcher (F-28426 A-88893), the hookmen's scripts
    /// (e.g. `game/career/Story/0x156c2b36` 0x7e6, generic_local_hookman.sco 0x70a) and the
    /// wager's (wager_hangout_core.sco 0x2079). An event entered from a panel by the same
    /// rule from the difficulty its core script was given (+3100): the series'
    /// OrderedSeriesCore (`game/race/0xb8b948b6` 0x4c7..0x4f5), the tournament's
    /// TournamentCore (`game/race/0x039dd301` 0x54b, before each race). (A payback's panel
    /// writes 1, `game/Hangouts/0xfc2c6c0a` 0xdce, and a delivery has no layout: their races
    /// are neither Ordered nor Unordered.) None: the difficulty is not known before the
    /// start (a race begun from the map or a test), the caller's own rule.
    pub fn mc4_layout_easy(&self) -> Option<bool> {
        // (a pink slip race takes the hard layout: wager_hangout_core +3112 = 1, 0x566e)
        if self.mc4_pink_on() {
            return Some(false);
        }
        if let Some(d) = self.event_diff {
            return Some(d < 2);
        }
        if let Some((_, d)) = self.wager {
            return Some(d < 2);
        }
        if let Some(d) = self.mc4_arcade_difficulty() {
            return Some(d < 2);
        }
        if self.hookman_race {
            // (the story cores that write +3112 as a constant; the others `+3100 < 2 ? 0 : 1`,
            // e.g. MUSCLE_CHAMP_CORE 0x7e6..0x80d)
            if let Some(&(_, easy)) = self.career.hookman_mission().and_then(|m| STORY_LAYOUT.iter().find(|r| r.0 == m)) {
                return Some(easy);
            }
            if let Some(d) = self.mc4_hookman_diff() {
                return Some(d < 2);
            }
        }
        // (a slot hookman's race: generic_local_hookman 0x70a, +3112 = d < 2 ? 0 : 1)
        if let Some(d) = self.man_diff.filter(|_| self.mc4_local_man_race()) {
            return Some(d < 2);
        }
        None
    }

    /// A reward is about to be worked out: the lists of the rewards screen start empty.
    pub fn mc4_rewards_begin(&mut self) {
        self.career.perf_now.clear();
        self.career.rewarded_now.clear();
        self.career.cars_now.clear();
        self.career.reward_page = 0;
    }

    /// The text of an unlocked performance level on the rewards screen: the string of the
    /// reward's description `Performance level <id>` (0x820BBA08; "GROUP 1 VEHICLES:
    /// PERFORMANCE LEVEL 1"). A key the string table has not shows as the original shows a
    /// missing string: `!!key!!` (F-12531).
    pub fn mc4_perf_label(&self, id: &str) -> String {
        let key = format!("Performance level {id}");
        match self.career.strings.get(&key).filter(|v| !v.contains('<')) {
            Some(v) => v.to_uppercase(),
            None => format!("!!{key}!!"),
        }
    }

    /// The pages of the rewards screen after the first (`reward_pages`): REWARDED - the
    /// ability levels the race gave -, UNLOCKED - the performance levels its experience
    /// unlocked (the Unlocking system is added to before the class system: F-48818
    /// A-53944, A-82913), then the body part types of the class point.
    pub fn mc4_reward_pages(&self) -> Vec<Vec<RewardRow>> {
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_string()).filter(|v| !v.contains('<')).unwrap_or_else(|| d.to_string());
        let unlocked: Vec<String> = self.career.perf_now.iter().map(|p| self.mc4_perf_label(p)).chain(self.career.unlocked_now.iter().map(|p| self.mc4_part_type_label(p))).collect();
        let mut pages = reward_pages(&self.career.rewarded_now, &unlocked, &st("EOR_UNLOCKED_PARTS", "UNLOCKED"), &st("EOR_REWARDED_PARTS", "REWARDED"));
        // (the Car rewards - type 4 - come before them, on a page of their own: a row of
        // kind 14 each, eight at the most - F-36204 A-54775..A-89564)
        // (MCLA_REWARD_CAR=<set-up>: a test hook, a car row without a car given)
        let hook = std::env::var("MCLA_REWARD_CAR").ok();
        let cars: Vec<&String> = self.career.cars_now.iter().chain(hook.iter()).take(8).collect();
        if !cars.is_empty() {
            pages.insert(0, cars.into_iter().map(|c| (14, self.mc4_car_reward_label(c), String::new())).collect());
        }
        pages
    }

    /// The rewards screen has something beyond its first page.
    pub fn mc4_rewards_listed(&self) -> bool {
        !self.career.perf_now.is_empty() || !self.career.rewarded_now.is_empty() || !self.career.unlocked_now.is_empty() || !self.career.cars_now.is_empty()
    }

    /// The page of the rewards screen to show: None, the page of the reputation and the
    /// money; else the rows of a later page. `first`: the first page has rows (with none
    /// the form's page count does not step, and the groups begin on page 0).
    /// (MCLA_REWARD_PAGE: a test hook, the page to open on.)
    pub fn mc4_reward_page(&self, first: bool) -> Option<Vec<RewardRow>> {
        let hook = std::env::var("MCLA_REWARD_PAGE").ok().and_then(|v| v.parse::<usize>().ok());
        let at = hook.unwrap_or(self.career.reward_page);
        let pages = self.mc4_reward_pages();
        let k = if first { at.checked_sub(1)? } else { at };
        Some(pages.get(k.min(pages.len().saturating_sub(1))).cloned().unwrap_or_default()).filter(|_| !pages.is_empty())
    }

    /// CONTINUE on the rewards screen (F-48737): the next page while there is one -
    /// true -, else the screen is done.
    pub fn mc4_reward_continue(&mut self, first: bool) -> bool {
        let total = self.mc4_reward_pages().len() + first as usize;
        if self.career.reward_page + 1 < total {
            self.career.reward_page += 1;
            true
        } else {
            self.career.reward_page = 0;
            false
        }
    }
}

// ---- events of the career: completion counters and their rewards ----
//
// An event is an entry of the event reward tables (`tune/career/0x24184df9`, South Central
// `0xf5cd9d2b`): `Event { Name  Difficulty { Name Easy|Medium|Hard|Impossible  Reward { .. } .. } }`,
// fifteen of them: the hangouts' series, the five tournaments, the two deliveries and the
// Beaches payback. The career completes the event the UI names (`szEventName`) at the
// difficulty of the race data (results +3100) on SERIES_COMPLETE, TOURNAMENT_COMPLETE,
// DELIVERY_RACE_COMPLETE and PAYBACK_COMPLETE (F-20860 -> F-47248 -> F-67065):
// the reward of that difficulty's list at index min(completions so far, last) is given
// and the difficulty's counter goes up by one.

/// One reward of an event's list.
#[derive(Clone, Debug, PartialEq)]
pub enum EventReward {
    /// `DynamicMoney`: the race money of the player's car class at the event's difficulty,
    /// times the factor of the panel the event is entered from (F-86598)
    Money,
    /// `DynamicExperience SystemName <system>`: an event's experience at that difficulty
    /// (F-43293), added to the system (F-88785)
    Experience(String),
    /// `Car Name <set-up>`
    Car(String),
    Other(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventDef {
    pub name: String,
    /// the reward lists of Easy, Medium, Hard, Impossible
    pub rewards: [Vec<EventReward>; 4],
}

/// What completing an event handed out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventPay {
    /// the event is one of the tables' (else nothing is counted)
    pub counted: bool,
    pub money: i64,
    pub unlocking: i64,
    pub progression: i64,
    pub car: Option<String>,
}

pub const EVENT_DIFFICULTIES: [&str; 4] = ["Easy", "Medium", "Hard", "Impossible"];
/// The showroom's DUB cars and the tournament whose Hard completion puts them there
/// (F-96690).
pub const DUB_TOURNAMENTS: [(&str, &str); 4] = [("vp_chv_camaro_06_dub", "hw_tournament1"), ("vp_dge_challenger_06_dub", "hl_tournament1"), ("vp_mbz_s600_07_dub", "be_tournament1"), ("vp_lam_murcielago_06_dub", "dt_tournament1")];

impl ExpTables {
    /// The events of one event reward table, added to what is here.
    pub fn parse_events(&mut self, text: &str) {
        let text = text.replace('{', " { ").replace('}', " } ");
        let tok: Vec<&str> = text.split_whitespace().collect();
        let mut path: Vec<&str> = Vec::new();
        let mut ev: Option<EventDef> = None;
        let mut diff: Option<usize> = None;
        let mut reward: std::collections::HashMap<&str, &str> = Default::default();
        let mut i = 0;
        while i < tok.len() {
            let t = tok[i];
            if tok.get(i + 1) == Some(&"{") {
                path.push(t);
                match t {
                    "Event" => ev = Some(EventDef::default()),
                    "Difficulty" => diff = None,
                    "Reward" => reward.clear(),
                    _ => {}
                }
                i += 2;
                continue;
            }
            if t == "}" {
                match path.pop() {
                    Some("Reward") => {
                        if let (Some(e), Some(d)) = (ev.as_mut(), diff) {
                            let kind = reward.get("Type").copied().unwrap_or("");
                            e.rewards[d].push(match kind {
                                "DynamicMoney" => EventReward::Money,
                                "DynamicExperience" => EventReward::Experience(reward.get("SystemName").copied().unwrap_or("").to_string()),
                                "Car" => EventReward::Car(reward.get("Name").copied().unwrap_or("").to_string()),
                                other => EventReward::Other(other.to_string()),
                            });
                        }
                    }
                    Some("Event") => {
                        if let Some(e) = ev.take() {
                            if !e.name.is_empty() && !self.events.iter().any(|x| x.name.eq_ignore_ascii_case(&e.name)) {
                                self.events.push(e);
                            }
                        }
                    }
                    _ => {}
                }
                i += 1;
                continue;
            }
            let v = tok.get(i + 1).copied().unwrap_or("");
            match (path.last().copied(), t) {
                (Some("Event"), "Name") => {
                    if let Some(e) = ev.as_mut() {
                        e.name = v.to_string();
                    }
                }
                (Some("Difficulty"), "Name") => diff = EVENT_DIFFICULTIES.iter().position(|d| d.eq_ignore_ascii_case(v)),
                (Some("Reward"), k) => {
                    reward.insert(k, v);
                }
                _ => {}
            }
            i += 2;
        }
    }

    pub fn event(&self, name: &str) -> Option<&EventDef> {
        self.events.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }
}

impl Game {
    /// The completions of an event at a difficulty, as the block has them.
    pub fn mc4_event_completions(&self, name: &str, diff: usize) -> u32 {
        self.mc4.as_ref().and_then(|m| m.career.events.iter().find(|e| e.name.eq_ignore_ascii_case(name))).map_or(0, |e| e.completions.get(diff))
    }

    /// The reward the event would give now at `diff` (F-90222): what its panel shows.
    pub fn mc4_event_reward(&self, name: &str, diff: usize) -> Option<EventReward> {
        let def = self.career.exp_tables.event(name)?;
        let list = &def.rewards[diff.min(3)];
        list.get((self.mc4_event_completions(name, diff) as usize).min(list.len().checked_sub(1)?)).cloned()
    }

    /// An event of the tables is completed at `diff` (F-67065): the reward at index
    /// min(completions, last) of that difficulty's list is given, then the counter goes up.
    /// `money_factor`: what the event's panel puts on its DynamicMoney - 1.5 on the event
    /// panels (F-23231, F-38970), 1.0 on the delivery menu (F-15204).
    /// Money comes back in the answer for the caller to add; experience and cars are given
    /// here.
    pub fn mc4_event_complete(&mut self, name: &str, diff: usize, money_factor: f32) -> EventPay {
        self.mc4_event_complete_at(name, diff, money_factor, diff)
    }

    /// The same, with the difficulty the DynamicMoney was last computed for (`money_diff`):
    /// the reward holds what its panel put into it. The event panels compute it for the
    /// difficulty shown; the delivery menu (F-15204, A-38963..A-83966) runs the
    /// one reward through all four difficulties to write its four "X - 2X" lines and leaves
    /// the last - Impossible - in it.
    pub fn mc4_event_complete_at(&mut self, name: &str, diff: usize, money_factor: f32, money_diff: usize) -> EventPay {
        let diff = diff.min(3);
        let Some(def) = self.career.exp_tables.event(name).cloned() else { return EventPay::default() };
        let reward = self.mc4_event_reward(&def.name, diff);
        let mut m = self.mc4.take().unwrap_or_else(|| Mc4::seeded(self.fresh_game));
        m.career.event_mut(&def.name).completions.bump(diff);
        self.mc4 = Some(m);
        // (step 23: a district's tournament complete - TournamentCore 0x12c4,
        // mcCareer_SetTournamentWinStatus(district, 1); aBTO with the fourth)
        if def.name.to_ascii_lowercase().contains("tournament") {
            if let Some(d) = ["hw_", "be_", "hl_", "dt_"].iter().position(|p| def.name.to_ascii_lowercase().starts_with(p)) {
                self.mc4_tournament_won(d);
            }
        }
        let mut pay = EventPay { counted: true, ..Default::default() };
        match reward {
            Some(EventReward::Money) => {
                let class = crate::rewards::perf_class(self.player_perf());
                pay.money = (crate::rewards::race_money(self.rep, class, money_diff.min(3), 10.0) * money_factor) as i64;
            }
            Some(EventReward::Experience(system)) => {
                // (the amount of a DynamicExperience is computed when the reward is made -
                // at boot, F-20115 -> F-64159 -> F-83600 -> F-10118, with
                // no player: 0 - and again by the tournament, delivery and series forms for
                // the reward they show. The only event that has one is `be_payback`, whose
                // form (F-75259) does not: the reward is given with amount 0.)
                let amount: i64 = 0;
                if system.eq_ignore_ascii_case("Unlocking") {
                    self.career.unlocking = Some(self.mc4_unlocking() + amount);
                    pay.unlocking = amount;
                } else if system.eq_ignore_ascii_case("Progression") {
                    self.rep += amount;
                    pay.progression = amount;
                }
            }
            Some(EventReward::Car(car)) => {
                self.career.give_car(&car);
                pay.car = Some(car);
            }
            _ => {}
        }
        log::info!("event {} completed at {}: {:?}", def.name, EVENT_DIFFICULTIES[diff], pay);
        pay
    }

    /// What a finished delivery or payback leg pays, with the events they complete:
    /// (money, reputation).
    ///
    /// Delivery (DELIVERY_RACE_COMPLETE): the manager gives the race money of the player's
    /// own car class at the delivery's difficulty, less the share of the damage (F-96289),
    /// and the career completes the district's delivery event, whose DynamicMoney is that
    /// race money again - so the menu's "X - 2X". No experience.
    ///
    /// Payback (PaybackLegCore, `script_obj/game/0x3ae26fc8/0xb3f9732f`): a leg pays
    /// ROUND(f x the race money of class A in steps of 20), f = 0.10 .. 0.30 for the first
    /// to the fifth win; the last leg (F-66865) gives needed wins x an event's
    /// experience to Progression and Unlocking; then PAYBACK_COMPLETE completes the event
    /// `be_payback`. Needed wins: 1 while the mission PAYBACK_GH_18 or PAYBACK_INTRO is
    /// being done, else 5 (`script_obj/game/Hangouts/0xfc2c6c0a`).
    pub fn mc4_mission_pay(&mut self, delivered: Option<f32>) -> (i64, i64) {
        self.mc4_rewards_begin();
        self.career.unlocked_now.clear();
        let diff = self.event_diff.unwrap_or(self.mission_diff).min(3);
        if let Some(damage) = delivered {
            let class = crate::rewards::perf_class(self.player_perf());
            let base = crate::rewards::race_money(self.rep, class, diff, 10.0);
            let mut money = if base > 1.0 { ((1.0 - damage.clamp(0.0, 1.0)) * base) as i64 } else { 0 };
            let south = self.current_race.and_then(|i| self.race_defs.get(i)).map_or(false, |d| d.district == "SouthCentral" || d.id.to_ascii_lowercase().contains("southcentral"));
            money += self.mc4_event_complete_at(if south { "sc_delivery" } else { "hw_delivery" }, diff, 1.0, 3).money;
            return (money, 0);
        }
        let wins = self.career.payback_wins as usize;
        let needed: usize = self.mc4_payback_needed();
        let share = 0.10 + 0.05 * wins.min(4) as f32;
        let money = (share * crate::rewards::race_money(self.rep, 0, diff, 20.0)).round() as i64;
        let mut rep = 0;
        if wins + 1 == needed {
            let exp = ((needed as f32 * crate::rewards::dynamic_exp(self.rep, diff)) * 0.1) as i32 as f32 * 10.0;
            if exp > 1.0 {
                rep = exp as i64;
                self.mc4_unlocking_add(self.rep, rep);
            }
        }
        if wins + 1 >= needed {
            self.mc4_payback_end();
            self.career.payback_done = true;
            // (PAYBACK_COMPLETE: the event is counted; its reward's amount is 0)
            self.mc4_event_complete("be_payback", diff, 1.5);
        } else {
            self.career.payback_wins = wins as u32 + 1;
            // (the run goes on with the next client: `mc4_payback_chain`)
            self.career.payback_next = self.career.payback_run;
        }
        (money, rep)
    }

    /// The factor on a race's money and experience (F-48818, A-44799..A-67293):
    /// 0.6 for a race started from the pause menu's ARCADE (its launcher, F-28426 at
    /// A-42138, sets the results' word +3132); else 1. (0.35 in a network game, with the
    /// experience to the Unlocking system alone: not built - online is postponed. The flag
    /// games, which the original only has on line, are the port's own off line and pay as
    /// any race here: the user's decision of 2026-10-13.)
    pub fn mc4_race_factor(&self) -> f32 {
        if self.arcade_restore.is_some() {
            0.6
        } else {
            1.0
        }
    }

    /// A DUB car is not in the showroom before its tournament has been completed on Hard
    /// (F-96690: the event's third counter). Asked of a block that lists the event.
    pub fn mc4_showroom_locked(&self, car: &str) -> bool {
        let Some((_, event)) = DUB_TOURNAMENTS.iter().find(|(c, _)| c.eq_ignore_ascii_case(car)) else { return false };
        self.mc4.as_ref().and_then(|m| m.career.events.iter().find(|e| e.name.eq_ignore_ascii_case(event))).map_or(false, |e| e.completions.Hard == 0)
    }
}

// ---- step 15b: the event panels, the payback's run, the upgrade percentage, the run to the line ----

/// A set-up's upgrade percentage (F-76051, recomputed for every racer's vehicle when it
/// is made - F-51384): 100 x the stages fitted to the twelve performance parts (all
/// but the nitrous) / (2 x the parts the vehicle has - twelve, a motorcycle five: exhaust,
/// engine, electronics, gear kit, tires), cut to a whole number. `stages`: the thirteen
/// parts in the port's order (`PERF_PARTS`).
pub fn upgrade_percent(stages: &[u8], bike: bool) -> i32 {
    let sum: i32 = stages.iter().enumerate().filter(|(k, _)| *k != 6 && *k < 13).map(|(_, v)| *v as i32).sum();
    let own = if bike { 5 } else { 12 };
    (sum as f32 * 100.0 / (2 * own) as f32) as i32
}

/// The run to the start line once somebody is there (the hookmen's scripts, e.g.
/// `script_obj/game/race/0xb8b948b6` 0x000f8a..0x0011c0): the player first ends it at once;
/// an opponent first shows "<NAME> FINISHED" and gives the others thirty seconds
/// (`Race_SetTargetTime(race, results +3148 = 30000)`); the player's place is his finishing
/// order, or his rank on the road when the time is up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BmtClose {
    /// seconds since the first opponent finished
    pub since: Option<f32>,
    /// opponents over the line
    pub finished: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BmtStep {
    Running,
    /// the first opponent is at the line now: the thirty seconds begin
    FirstIn,
    /// the run is over: the player's place
    Over(usize),
}

pub const BMT_CLOSE_SECS: f32 = 30.0;

impl BmtClose {
    /// `player` / `rivals`: distances to the line; `radius`: what counts as there.
    pub fn step(&mut self, player: f32, rivals: &[f32], radius: f32, dt: f32) -> BmtStep {
        if player < radius {
            return BmtStep::Over(1 + self.finished);
        }
        let there = rivals.iter().filter(|d| **d < radius).count();
        self.finished = self.finished.max(there);
        match self.since {
            None if self.finished > 0 => {
                self.since = Some(0.0);
                BmtStep::FirstIn
            }
            None => BmtStep::Running,
            Some(t) => {
                let t = t + dt;
                self.since = Some(t);
                if t >= BMT_CLOSE_SECS {
                    BmtStep::Over(1 + rivals.iter().filter(|d| **d < player).count())
                } else {
                    BmtStep::Running
                }
            }
        }
    }
}

/// What an event's panel shows (mcTournamentForm F-20832 / F-23231, mcPaybackForm
/// F-12424 / F-75259): string ids and values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventPanel {
    pub title: String,
    pub desc: String,
    pub reward_label: String,
    /// money (shown with "$") or a car's short name
    pub reward_money: Option<i64>,
    pub reward_car: Option<String>,
    pub count_label: String,
    pub count: usize,
}

impl Game {
    /// The share (0..1) of the upgrades the player's vehicle `i` carries, as the original
    /// counts it for the performance value (F-92180 -> F-34010).
    pub fn mc4_upgrade_share(&self, i: usize) -> f32 {
        let bike = self.catalog.get(&self.mc4_car_name(i)).map_or(false, |c| c.class.eq_ignore_ascii_case("SportBike"));
        upgrade_percent(&self.perf_stages(i), bike).clamp(0, 100) as f32 / 100.0
    }

    /// The upgrade percentage of a set-up file (`resources/vehicle/configs/<config>.xcc`:
    /// the stages at 0x1C..0x28); 0 without the file.
    pub fn mc4_setup_percent(&self, car: &str, config: &str) -> i32 {
        if config.is_empty() {
            return 0;
        }
        let Some(Ok(b)) = crate::render::car_lamps::setup_file(std::path::Path::new(&self.data_dir), config).map(std::fs::read) else { return 0 };
        if b.len() < 0x29 {
            return 0;
        }
        let bike = self.catalog.get(car).map_or(false, |c| c.class.eq_ignore_ascii_case("SportBike"));
        upgrade_percent(&b[0x1C..0x29], bike)
    }

    /// How hard a racer in `car` with set-up `config` is for the player (F-25301): his
    /// car's performance value at his set-up's upgrade percentage against the player's.
    pub fn mc4_difficulty_of_setup(&self, car: &str, config: &str) -> i32 {
        let share = self.mc4_setup_percent(car, config).clamp(0, 100) as f32 / 100.0;
        match self.samples.value(car, share) {
            Some(p) => crate::rewards::difficulty(p, self.player_perf()),
            None => 1,
        }
    }

    /// What `price_of` answers as the reputation a vehicle needs: the showroom's lock
    /// (`fs_lock`, F-31112 A-67545) is F-96690 - for a DUB car its tournament's
    /// Hard completion, for the others the level of their group.
    pub fn mc4_car_need(&self, car: &str, group: &str) -> i64 {
        if self.mc4_showroom_locked(car) && std::env::var("MCLA_UNLOCK_ALL").is_err() {
            return i64::MAX;
        }
        self.mc4_group_need(group)
    }

    /// The wins a payback asks for (`script_obj/game/Hangouts/0xfc2c6c0a` 0x000ad4..0x000baa).
    pub fn mc4_payback_needed(&self) -> usize {
        if self.career.mission_active("PAYBACK_GH_18") || self.career.mission_active("PAYBACK_INTRO") { 1 } else { 5 }
    }

    /// The payback panel's COMPLETION REWARD (F-75259): the legs' shares added up the
    /// way the code does - i x 0.05 x the race money of class A in steps of 20, i = 2 ..
    /// wins + 1, cut to a whole number at every step.
    pub fn mc4_payback_total(&self, diff: usize) -> i64 {
        let money = crate::rewards::race_money(self.rep, 0, diff.min(3), 20.0) as i64 as f64;
        let mut total: i64 = 0;
        for i in 2..=self.mc4_payback_needed() + 1 {
            total = (i as f64 * 0.05 * money + total as f64) as i64;
        }
        total
    }

    /// The panel of an event (`hw_tournament1` .., `be_payback`) at `diff`.
    pub fn mc4_event_panel(&self, key: &str, diff: usize) -> EventPanel {
        if key.eq_ignore_ascii_case("be_payback") {
            let (title, desc) = if self.career.mission_active("PAYBACK_GH_18") {
                ("PAYBACK_GH_18_TITLE", "PAYBACK_GH_18_OBJ_02")
            } else if self.career.mission_active("PAYBACK_INTRO") {
                ("PAYBACK_INTRO_TITLE", "PAYBACK_INTRO_OBJ_01")
            } else {
                ("PAYBACK_MENU_TITLE", "PAYBACK_MENU_DESC")
            };
            return EventPanel { title: title.into(), desc: desc.into(), reward_label: "REWARD_PAYBACK_LABEL".into(), reward_money: Some(self.mc4_payback_total(diff)), reward_car: None, count_label: "DESTROY_RACERS_LABEL".into(), count: self.mc4_payback_needed() };
        }
        let (reward_money, reward_car) = match self.mc4_event_reward(key, diff) {
            Some(EventReward::Car(car)) => (None, Some(car)),
            Some(EventReward::Money) => (Some((crate::rewards::race_money(self.rep, crate::rewards::perf_class(self.player_perf()), diff.min(3), 10.0) * 1.5) as i64), None),
            _ => (None, None),
        };
        EventPanel { title: key.to_string(), desc: "tournament_desc".into(), reward_label: "REWARD_TOURNAMENT_LABEL".into(), reward_money, reward_car, count_label: "RACES_TOTAL_LABEL".into(), count: self.career.event_races(key).len() }
    }

    /// A race is about to start: a tournament's first race and a payback not yet running
    /// open their panel first (the original shows it when the tournament's man is flashed -
    /// `script_obj/game/CruisingHookmen/0xdcd7fe26`, form 66 - and at the payback's place -
    /// `Hangouts/0xfc2c6c0a`, form 68). True: the panel is up and the race waits.
    pub fn mc4_event_panel_first(&mut self) -> bool {
        if std::mem::take(&mut self.career.panel_ok) {
            return false;
        }
        // (step 21b: a pink slip race is the wager core's own - Race_Create on the set-up's
        // race, 0x56a4 - whatever event the race also belongs to)
        if self.mc4_pink_on() {
            return false;
        }
        let Some(def) = self.current_race.and_then(|i| self.race_defs.get(i)).cloned() else { return false };
        if self.flash_menu.is_none() {
            return false;
        }
        if let Some((event, done, _, hard)) = self.career.tournament_of(&def.id) {
            let stored = self.career.stats.get(&format!("tour_diff:{}", event.to_ascii_lowercase())).copied();
            if done == 0 {
                // (opens on MEDIUM while the man's mission is open, else on HARD: mcTournamentForm
                // +1896 from the script, generic_tournament_hookman 0x115..0x148 / 0x4e3)
                let _ = hard;
                self.hangout_diff = self.mc4_tournament_panel_diff(&event);
                self.hangout_key = Some(event);
                self.hangout_page = (0, 0);
                self.mode = crate::game::Mode::RaceSelect;
                return true;
            }
            // (the legs run at the difficulty the tournament was entered with: struct +40
            // of the hookman's script -> results +3100 in TournamentCore)
            self.event_diff = Some(stored.map_or(if hard { 2 } else { 1 }, |d| d.clamp(0, 3) as usize));
            return false;
        }
        if def.race_type == "Payback" && !self.career.payback_run {
            // (opens on MEDIUM: mcPaybackForm sets its combo to 1 on every showing, F-60038)
            self.hangout_diff = 1;
            self.hangout_key = Some("be_payback".to_string());
            self.hangout_page = (0, 0);
            self.mode = crate::game::Mode::RaceSelect;
            return true;
        }
        false
    }

    /// ACCEPT on an event's panel: the difficulty chosen goes with the event; true = start
    /// the race that was waiting.
    pub fn mc4_event_panel_accept(&mut self, key: &str) -> bool {
        if self.current_race.is_none() {
            return false;
        }
        let diff = self.hangout_diff.min(3);
        if key.eq_ignore_ascii_case("be_payback") {
            self.career.payback_run = true;
            self.career.payback_wins = 0;
            self.career.payback_next = false;
            // (step 19d: PaybackCore's locals 12..34 - nothing of the two tables used yet)
            let s19d = &mut self.opt_x.arcade.d;
            (s19d.cars_used, s19d.cars_n, s19d.drivers_used, s19d.drivers_n, s19d.lost_leg, s19d.next_call) = ([false; 12], 0, [false; 5], 0, false, false);
            self.career.payback_used = self.current_race.and_then(|i| self.race_defs.get(i)).map(|d| d.id.to_ascii_lowercase()).into_iter().collect();
        } else {
            self.career.stats.insert(format!("tour_diff:{}", key.to_ascii_lowercase()), diff as i64);
        }
        self.event_diff = Some(diff);
        self.career.panel_ok = true;
        true
    }

    /// The payback is over (won, lost or left): its count starts again.
    pub fn mc4_payback_end(&mut self) {
        self.career.payback_wins = 0;
        self.career.payback_run = false;
        self.career.payback_next = false;
        self.career.payback_used.clear();
    }

    /// After a leg that was won and was not the last: the next client at once, without a
    /// menu (PaybackCore, `script_obj/game/0x3ae26fc8/0x422e62e2` 0x000b78..0x000bd0: the
    /// payback race nearest the player that this run has not had -
    /// `mcRaceArray_SortByDistance`, `mcRaceArray_FindNextRaceIndex`). Sets the race and
    /// answers true when the run goes on.
    pub fn mc4_payback_chain(&mut self, from: glam::Vec3) -> bool {
        if !std::mem::take(&mut self.career.payback_next) || !self.career.payback_run {
            return false;
        }
        let mut list: Vec<usize> = (0..self.race_defs.len()).filter(|&i| self.race_defs[i].race_type == "Payback").collect();
        if list.is_empty() {
            self.mc4_payback_end();
            return false;
        }
        if list.iter().all(|&i| self.career.payback_used.contains(&self.race_defs[i].id.to_ascii_lowercase())) {
            self.career.payback_used.clear();
        }
        list.retain(|&i| !self.career.payback_used.contains(&self.race_defs[i].id.to_ascii_lowercase()));
        list.sort_by(|a, b| self.race_defs[*a].beacon.distance(from).partial_cmp(&self.race_defs[*b].beacon.distance(from)).unwrap_or(std::cmp::Ordering::Equal));
        let next = list[0];
        self.career.payback_used.push(self.race_defs[next].id.to_ascii_lowercase());
        self.current_race = Some(next);
        true
    }
}

// ---- step 19c: the ARCADE options at work, the time trial's forms and clock, set-ups on the
// player's vehicle, the payback leg ----
//
// The ARCADE launcher (F-28426) hands the race an options record of 28 bytes (the race
// data's +3160; made by F-15488(28), its defaults those of F-87205):
//   byte 0, bits 7..0: NITRO, ICE, REVERSE STEERING, STEALTH, DISRUPTER, SHIELD, STOP, PULSE
//   byte 1, bits 7..5: MIRROR, AGRO, RANDOM (the eleven rows of POWER UPS' Custom page -
//           widgets form + 5856 + 240 k, read whatever POWER UPS itself says)
//   byte 4 bit 7 heavy flag, byte 5 the flag limit, byte 6 bit 7 flag transfer, bit 6 force
//           balance, word 8 the time limit in ms (the flag games' rows; their defaults 1, 10,
//           1, 1, 10 minutes)
//   word 12 TRAFFIC (0 none, 1 low, 2 default), word 16 TIME OF DAY (the row's index),
//   byte 20 LAST MAN OUT, byte 21 POWER UPS (off / anything else)
// Readers: the script commands EventOptions_* (F-28012..F-94827) and three routines of
// the network lobby (F-53909 / 70 / A8). Off line:
//   - TRAFFIC: OrderedCheckpointCore (`game/0x3ae26fc8/0xf637b18a` 0x90..0x224) switches the
//     traffic off for level 0 and on for anything above; the launcher's own ACCEPT
//     (F-15258 A-82006..A-16151) writes the ambient manager's density scale
//     (manager + 0x64240): 0 for none, 0.5 for low, 1 for default.
//   - TIME OF DAY: the same ACCEPT warps the clock (F-42257: Midnight 0 h, Dawn 6 h,
//     Noon 12 h, Dusk 17.4 h; Dynamic: the hour the menu was opened at) - the clock runs on
//     from there.
//   - WEATHER: F-99588 sets the weather of the row (table 0x82826E28: Clear 0, Cloudy 1,
//     Foggy 3, Stormy 2; Dynamic: the weather the menu was opened in). The weather's own
//     change (F-18159, every 300 s) is not switched off.
//   - LAST MAN OUT: RaceWrapper (`game/race/0xc4e26a20` 0x2bd1) makes the laps the number of
//     opponents; OrderedCheckpointCore 0x1987..0x285d eliminates (`Lmo`).
//   - POWER UPS: the core makes the pick-up sources from the race's checkpoints
//     (PickUpManager_CreatePowerUpSourcesFromCheckPoints, F-28343) - built in
//     `powerups.rs` (step 19e), from the race script's own list where it has one.

/// The eleven rows of POWER UPS' Custom page, in the form's order (F-93221; keys of
/// the string table - their texts begin with the list's sub-row mark, U+00B1).
pub const POWER_UP_ROWS: [(&str, &str); 11] = [
    ("RO_Nitro_PowerUp", "NITRO"),
    ("RO_Ice_PowerUp", "ICE"),
    ("RO_Reverse_PowerUp", "REVERSE STEERING"),
    ("RO_Stealth_PowerUp", "STEALTH"),
    ("RO_Disrupter_PowerUp", "DISRUPTER"),
    ("RO_Shield_PowerUp", "SHIELD"),
    ("RO_Stop_PowerUp", "STOP"),
    ("RO_Pulse_PowerUp", "PULSE"),
    ("RO_Mirror_PowerUp", "MIRROR"),
    ("RO_Agro_PowerUp", "AGRO"),
    ("RO_Mystery", "RANDOM"),
];

/// The first of the Custom rows' places (`ArcadeRow::slot`): 8 .. 18.
pub const ARCADE_CUSTOM_SLOT: usize = 8;

/// What the running game keeps for this step (in `OptLive`: made with the game, not saved).
#[derive(Clone, Debug, Default)]
pub struct ArcadeLive {
    /// the Custom rows that are OFF (the form's default is ON for all eleven: F-84256)
    pub custom_off: [bool; 11],
    /// LAST MAN OUT's state while its race runs
    pub lmo: Option<Lmo>,
    /// the time trial's clock has run out and said so
    pub tt_expired: bool,
    /// the set-up the borrowed vehicle wears: (vehicle, set-up file)
    pub setup: Option<(String, String)>,
    /// the payback leg under way
    pub pb: Option<PaybackLeg>,
    /// a run to the line is in its last thirty seconds: the seconds left (`roamers`)
    pub bmt_left: std::cell::Cell<Option<f32>>,
    /// step 19d: the payback's chase and client, the borrowed vehicle's look, the time
    /// trial's sounds and banner, the hookman's meeting
    pub d: S19d,
    /// step 21: the pink slip panel's cursor and the race under way
    pub pink: PinkLive,
}

/// The options record of an ARCADE race as its launcher packs it (see above). `custom`: the
/// eleven power-up rows that are ON.
pub fn event_options(last_man_out: bool, power_ups: usize, traffic: usize, time_of_day: usize, custom: [bool; 11]) -> [u8; 28] {
    let mut r = [0u8; 28];
    for (k, on) in custom.iter().enumerate().take(8) {
        r[0] |= (*on as u8) << (7 - k);
    }
    // (the low five bits of byte 1 are whatever the allocation left: 0 here)
    r[1] = (custom[8] as u8) << 7 | (custom[9] as u8) << 6 | (custom[10] as u8) << 5;
    r[4] = 0x80;
    r[5] = 10;
    r[6] = 0xC0;
    r[8..12].copy_from_slice(&600_000u32.to_be_bytes());
    r[12..16].copy_from_slice(&(traffic as u32).to_be_bytes());
    r[16..20].copy_from_slice(&(time_of_day as u32).to_be_bytes());
    r[20] = last_man_out as u8;
    r[21] = (power_ups != 0) as u8;
    r
}

/// An ARCADE row is given a choice (on the game's fields, for the menu's code, which holds
/// other parts of the game while it runs): rows 0..5 `arcade_opts`, 6 and 7 `arcade_x`, 8..18
/// the Custom rows.
pub fn arcade_set(opts: &mut [usize; 6], x: &mut [usize; 2], live: &mut OptLive, slot: usize, v: usize) {
    if (ARCADE_CUSTOM_SLOT..ARCADE_CUSTOM_SLOT + 11).contains(&slot) {
        live.arcade.custom_off[slot - ARCADE_CUSTOM_SLOT] = v == 0;
    } else if slot < 6 {
        opts[slot] = v;
    } else {
        x[(slot - 6).min(1)] = v;
    }
}

/// What ACCEPT does to the world for an ARCADE race (F-15258).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcadeEnv {
    /// the ambient manager's density scale: 0, 0.5 or 1
    pub traffic: f32,
    /// the hour the clock is warped to; None: Dynamic
    pub hour: Option<f32>,
    /// the weather set (0 clear, 1 cloudy, 2 stormy, 3 foggy); None: Dynamic
    pub weather: Option<u8>,
}

pub fn arcade_env(traffic: usize, weather: usize, time_of_day: usize) -> ArcadeEnv {
    ArcadeEnv {
        traffic: match traffic { 0 => 0.0, 1 => 0.5, _ => 1.0 },
        hour: match time_of_day { 1 => Some(0.0), 2 => Some(6.0), 3 => Some(12.0), 4 => Some(17.4), _ => None },
        weather: match weather { 1 => Some(0), 2 => Some(1), 3 => Some(3), 4 => Some(2), _ => None },
    }
}

/// LAST MAN OUT as OrderedCheckpointCore runs it (0x1987..0x285d), one pass of its loop a
/// call of `step`:
///   - no clock running: the leader is the racer of rank 1, the last the racer whose rank is
///     the field less those out; when the last has not as many laps behind him as the leader
///     and the leader's laps are not the number of racers out, the clock is set - ten
///     seconds from now (Race_SetTargetTime; HUD control 6 with properties 0 and 2: the race
///     clock counting down in red);
///   - the clock up: the racer of that last rank (never the leader; nobody found: racer 0,
///     the player) is out - his time -1, the splash LMO_ELEMINATED with his name; a computer
///     racer is taken out of the world (Racer_Deactivate), the player's race is over;
///   - "out" counts every racer eliminated or finished.
#[derive(Clone, Debug, PartialEq)]
pub struct Lmo {
    /// the race clock, in ms, at which the next racer goes; -1: no clock
    pub timer: i64,
    /// racers out: eliminated or finished
    pub out: usize,
    /// the racers eliminated (their places in the field: 0 the player), in order
    pub eliminated: Vec<usize>,
    /// the race clock at the last pass (a clock that went back: another race)
    pub last_ms: i64,
}

impl Default for Lmo {
    fn default() -> Lmo {
        Lmo { timer: -1, out: 0, eliminated: Vec::new(), last_ms: 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LmoEvent {
    /// the clock was set to end at this time
    Clock(i64),
    /// the racer of this place is out
    Out(usize),
}

impl Lmo {
    /// `racers`: per place in the field (0 the player) the rank on the road (1 first) and
    /// the laps behind him; `finished`: the racers that have finished (not eliminated).
    pub fn step(&mut self, now_ms: i64, racers: &[(usize, usize)], finished: usize) -> Option<LmoEvent> {
        self.out = self.eliminated.len() + finished;
        self.last_ms = now_ms;
        let n = racers.len();
        if n < 2 {
            return None;
        }
        let last_rank = n.saturating_sub(self.out);
        if self.timer == -1 {
            let mut leader = 0;
            let mut last = 0;
            for (i, r) in racers.iter().enumerate() {
                if r.0 == 1 {
                    leader = i;
                } else if r.0 == last_rank {
                    last = i;
                }
            }
            if racers[last].1 != racers[leader].1 && racers[leader].1 != self.out {
                self.timer = now_ms + 10_000;
                return Some(LmoEvent::Clock(self.timer));
            }
            return None;
        }
        if self.timer > 0 && now_ms > self.timer {
            let mut who = 0;
            for (i, r) in racers.iter().enumerate() {
                if r.0 != 1 && r.0 == last_rank {
                    who = i;
                }
            }
            self.timer = -1;
            self.eliminated.push(who);
            self.out += 1;
            return Some(LmoEvent::Out(who));
        }
        None
    }
}

/// The parts of a set-up file as the port's model takes them: the garage record's part
/// bytes (the importer's table - `tools/import_mc4_save.py` PART_BYTES - measured in the
/// running game: UI_WORK step 53), the rim's place in the rim table (+0x844) and the front
/// rim's size in inches (+0x83e).
pub fn setup_parts(b: &[u8]) -> Option<(crate::vehicle_model::CarConfig, u32, u32)> {
    if b.len() != 8176 {
        return None;
    }
    let mut cfg = crate::vehicle_model::CarConfig::default();
    cfg.hood = b[0x862] as usize;
    cfg.bumper_r = b[0x863] as usize;
    cfg.bumper_f = b[0x865] as usize;
    cfg.skirts = b[0x868] as usize;
    cfg.spoiler = b[0x869] as usize;
    cfg.exhaust = b[0x877] as usize;
    cfg.seats = b[0x88d] as usize;
    cfg.body = b[0x89b] as usize;
    for (key, at) in [("headlight", 0x86b), ("taillight", 0x86c), ("steer_whl", 0x894), ("post_gauge", 0x895), ("blower", 0x870), ("side_exhaust", 0x878), ("intercooler", 0x89d), ("bike_tail", 0x871), ("bike_tank", 0x872), ("bike_cowl", 0x873), ("carbon_hood", 0x898)] {
        if b[at] != 0 {
            cfg.extra.insert(key.to_string(), b[at] as usize);
        }
    }
    Some((cfg, b[0x844] as u32, b[0x83e] as u32))
}

/// A payback leg as PaybackLegCore runs it (`game/0x3ae26fc8/0xb3f9732f` 0x831..0x1942):
///   0 - the client cruises the loop of the race's checkpoints, the GPS on him; with the
///       player within 50 m: PB_GetHim, the client races the checkpoints round and round
///       (no nitrous, at most 32), the race clock starts from 0 against the race's target
///       time (HUD control 6);
///   1 - his damage at or above the difficulty's share (50 / 75 / 87 / 100 % of
///       GetOpponentDamageCommandLine, 100): the leg is won - money(class A, the
///       difficulty, steps of 20) x (0.10 + 0.05 x wins); the clock past the target:
///       PB_Failed, and after the mechanic's call the client's friends give chase
///       (PoliceManager_StartPursuit with the three cars of `game/0x107ebb16/0x3faa7997`);
///   2 - the pursuit's result: lost them (PB_Lost: the run goes on without a win) or
///       caught (PB_Caught: the run is over).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaybackLeg {
    pub phase: u8,
    /// the race clock (s) when the client was reached
    pub since: f32,
    /// step 19d - the chase: 2 the time is up (a second), 3 the call and the chasers'
    /// making (a second), 4 the pursuit, 5 caught (two seconds), 6 over. Seconds in the
    /// phase, the race timer since the call, the chasers made / on the road (bits), the
    /// timer value the next search taunt waits for, the result (`PB_CAUGHT`, `PB_LOST`)
    pub t: f32,
    pub clock: f32,
    pub made: u8,
    pub on_road: u8,
    pub taunt_at: f32,
    pub result: u8,
    /// the leg's opening call (SIDEKICK_PAYBACK_NEXT) has been made
    pub called: bool,
}

/// The share of the client's damage that wins a leg, by difficulty (0x0cc6..0x0d55).
pub fn payback_threshold(diff: usize) -> f32 {
    [0.5, 0.75, 0.87, 1.0][diff.min(3)]
}

impl Game {
    /// A Custom row's place among the eleven, if `slot` is one.
    fn mc4_custom_row(slot: usize) -> Option<usize> {
        (ARCADE_CUSTOM_SLOT..ARCADE_CUSTOM_SLOT + 11).contains(&slot).then(|| slot - ARCADE_CUSTOM_SLOT)
    }

    /// An ARCADE row is given a choice (the options page's left / right and DEFAULT).
    pub fn mc4_arcade_set(&mut self, slot: usize, v: usize) {
        arcade_set(&mut self.arcade_opts, &mut self.arcade_x, &mut self.opt_x, slot, v);
    }

    /// The options record the launcher would hand this race (F-28426).
    pub fn mc4_arcade_record(&self) -> [u8; 28] {
        let on = self.opt_x.arcade.custom_off.map(|off| !off);
        event_options(self.arcade_x[1] != 0, self.arcade_opts[2], self.arcade_opts[3].min(2), self.arcade_opts[5].min(4), on)
    }

    /// Test hook: MCLA_ARCADE_OPTS="opponents,last man out,difficulty,laps,power ups,
    /// traffic,weather,time of day" (the rows' choices as the menu counts them, laps and
    /// opponents as numbers) makes the race about to start an ARCADE race with those options.
    pub fn mc4_arcade_hook(&mut self, race: usize) {
        let Ok(v) = std::env::var("MCLA_ARCADE_OPTS") else { return };
        let f: Vec<usize> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if f.len() != 8 || self.arcade_restore.is_some() || race >= self.race_defs.len() {
            return;
        }
        self.arcade_opts = [f[0].max(1) - 1, f[2].min(3), f[4].min(2), f[5].min(2), f[6].min(4), f[7].min(4)];
        self.arcade_x = [f[3].clamp(1, 5) - 1, f[1].min(1)];
        self.arcade_restore = Some((race, self.race_defs[race].opponents));
        self.race_defs[race].opponents = f[0].max(1);
    }

    /// What starting a race from the ARCADE menu does to traffic, clock and weather; None:
    /// the race is not ARCADE's.
    pub fn mc4_arcade_env(&self) -> Option<ArcadeEnv> {
        self.arcade_restore.is_some().then(|| arcade_env(self.arcade_opts[3], self.arcade_opts[4], self.arcade_opts[5]))
    }

    /// The race in hand is run LAST MAN OUT: started from ARCADE with the row on. (The row
    /// is a circuit's; the launcher packs its widget whatever the race - a widget left on by
    /// a circuit and then hidden was not followed: here a circuit only.)
    pub fn mc4_lmo_on(&self) -> bool {
        self.arcade_restore.is_some() && self.arcade_x[1] != 0 && self.current_race.and_then(|i| self.race_defs.get(i)).map_or(false, |d| d.sub_type.eq_ignore_ascii_case("Circuit"))
    }

    /// LAST MAN OUT, once a tick while the race runs (before the tick's own work). True: the
    /// player is out and the race over.
    pub fn mc4_lmo_tick(&mut self, vehicles: &mut crate::vehicles::Vehicles) -> bool {
        let on = self.mc4_lmo_on();
        let Some(race) = self.race.as_ref().filter(|r| on && r.kind == crate::game::RaceKind::Race && r.countdown <= 0.0) else {
            self.opt_x.arcade.lmo = None;
            return false;
        };
        let now = (race.time as f64 * 1000.0) as i64;
        let order = self.standings(vehicles);
        let lap_len = race.lap_len.max(1);
        let mut lmo = self.opt_x.arcade.lmo.take().filter(|l| l.last_ms <= now).unwrap_or_default();
        let racers: Vec<(usize, usize)> = race.participants.iter().map(|ci| (order.iter().position(|o| o.0 == *ci).map_or(race.participants.len(), |p| p + 1), race.next.get(*ci).copied().unwrap_or(0) / lap_len)).collect();
        let finished = race.participants.iter().filter(|ci| race.finish_time.get(**ci).map_or(false, |t| t.is_some())).count();
        let ev = lmo.step(now, &racers, finished);
        let who = match ev {
            Some(LmoEvent::Out(k)) => race.participants.get(k).copied().map(|ci| (k, ci)),
            _ => None,
        };
        self.opt_x.arcade.lmo = Some(lmo);
        let Some((k, ci)) = who else { return false };
        // (LMO_ELEMINATED: "<name> ELIMINATED!")
        let name = match vehicles.cars.get(ci) {
            Some(c) if c.is_player => self.career.strings.get("G_User").map(|s| s.to_uppercase()).unwrap_or_else(|| "USER".to_string()),
            Some(c) if !c.driver.is_empty() => self.career.name_of(&c.driver).to_uppercase(),
            Some(c) => crate::game::pretty_car_name(&c.model.name).to_uppercase(),
            None => String::new(),
        };
        let text = self.career.strings.get("LMO_ELEMINATED").map(|s| s.to_string()).unwrap_or_else(|| "<D>PLAYER_ELEMINATED</D> ELIMINATED!".to_string());
        let text = match (text.find("<D>"), text.find("</D>")) {
            (Some(a), Some(b)) if b > a => format!("{}{}{}", &text[..a], name, &text[b + 4..]),
            _ => format!("{name} {text}"),
        };
        self.show_message(&text.to_uppercase());
        log::info!("last man out: racer {} ({}) is out at {:.1} s", k, name, now as f32 / 1000.0);
        if k == 0 {
            return true;
        }
        // (Racer_Deactivate: out of the world - the car stays in the field's lists, far off
        // and never set on the ground again)
        if let Some(c) = vehicles.cars.get_mut(ci) {
            let far = c.state.pos + glam::Vec3::new(1.0e5, -500.0, 1.0e5);
            let rot = c.state.rot;
            c.state.relocate(far, rot);
            c.state.vel = glam::Vec3::ZERO;
            c.state.ang_vel = glam::Vec3::ZERO;
            c.ai = None;
            c.placed = false;
            c.pinned = true;
        }
        false
    }

    /// The results panel next in line is a time trial's (it is shown before the rewards).
    pub fn mc4_tt_panel_first(&self) -> bool {
        self.mission_results.first().map_or(false, |m| m.0.starts_with(TT_PANEL) || m.0.starts_with(crate::game::DL_PANEL))
    }

    /// The HUD's field size: LAST MAN OUT shows the racers still in (PosMax, 0x3208..0x3236).
    pub fn mc4_hud_total(&self, total: usize) -> usize {
        match self.opt_x.arcade.lmo.as_ref() {
            Some(l) if self.mc4_lmo_on() => total.saturating_sub(l.out).max(1),
            _ => total,
        }
    }

    /// The race clock as HUD control 6 shows it when its property 0 is on (the HUD's
    /// update, F-86111 A-97648..A-95951, and the control's, F-87516): the
    /// time left to the race's target - shown without its sign -, the movie's
    /// `TotalCountdown` (the time left in ms x 0.0001: under 1 the pie beside the clock
    /// runs down and the figures blink, under 0 they are red) and `red_countdown`
    /// (property 2: LAST MAN OUT's clock). None: the clock counts up as ever.
    /// A time trial's script turns property 0 on for the run (generic_timetrial_hookman
    /// 0x1db2); LAST MAN OUT turns 0 and 2 on while its ten seconds run (0x1d54).
    pub fn mc4_hud_clock(&self) -> Option<(f32, f32, bool)> {
        let race = self.race.as_ref()?;
        if let Some(l) = self.opt_x.arcade.lmo.as_ref().filter(|l| l.timer > 0) {
            let left = l.timer as f32 / 1000.0 - race.time;
            return Some((left.abs(), left / 10.0, true));
        }
        let id = self.current_race.and_then(|i| self.race_defs.get(i)).map(|d| d.id.clone())?;
        let (_, _, target) = self.mc4_tt_run(&id)?;
        let left = target - race.time;
        Some((left.abs(), left / 10.0, false))
    }

    /// The run to the line once an opponent is in: the race clock from 0 against a target of
    /// thirty seconds (the hookmen's scripts, e.g. `game/race/0xb8b948b6` 0x10a6..0x10f0:
    /// property 0 of HUD control 6 on, Race_SetTargetTime(30000), Race_SetTimer(0), the
    /// control shown) - the same countdown as a time trial's. Nothing else of a race's HUD.
    pub fn mc4_bmt_hud(&self) -> Option<crate::flash_ui::RaceHud> {
        let left = self.opt_x.arcade.bmt_left.get().filter(|_| self.race.is_none())?;
        Some(crate::flash_ui::RaceHud { elapsed: BMT_CLOSE_SECS - left, time_left: Some(left), clock_count: Some((left / 10.0, false)), lap: 1, laps: 1, pos: 1, total: 1, ..Default::default() })
    }

    /// The time trial's clock has passed its target: the state machine's splash
    /// (`tune/gamestate/Racing.sc.xml`, Racing_TimeTrial: game.raceTimeExpired ->
    /// UI.Splash('time_trial_time_expired', 2.0); the core posts the event once, 0x196e).
    pub fn mc4_tt_tick(&mut self) {
        // (step 19d: the counting down clock's sounds - `mc4_clock_sounds`)
        let s19d_left = self.mc4_clock_left_ms();
        self.mc4_clock_sounds(s19d_left);
        let over = match (self.race.as_ref(), self.mc4_hud_clock()) {
            (Some(r), Some((_, c, false))) if r.countdown <= 0.0 => c < 0.0,
            _ => {
                self.opt_x.arcade.tt_expired = false;
                return;
            }
        };
        if over && !self.opt_x.arcade.tt_expired {
            self.opt_x.arcade.tt_expired = true;
            let t = self.career.strings.get("time_trial_time_expired").map(|s| s.to_uppercase()).unwrap_or_else(|| "TIME EXPIRED".to_string());
            self.show_message(&t);
        } else if !over {
            self.opt_x.arcade.tt_expired = false;
        }
    }

    /// The set-up file of a time trial's vehicle as parts for the player's car: the model's
    /// part choices, the rim's resource directory (by the rim table, as an imported save's
    /// rim) and the rim's size. Remembers the set-up for the livery (`mc4_setup_livery`).
    pub fn mc4_setup_dress(&mut self, car: &str, config: &str) -> Option<(crate::vehicle_model::CarConfig, Option<std::path::PathBuf>, u32)> {
        let b = std::fs::read(crate::render::car_lamps::setup_file(std::path::Path::new(&self.data_dir), config)?).ok()?;
        let (cfg, rim, inches) = setup_parts(&b)?;
        let dir = self.rim_shop.entry(rim as i32).and_then(|e| self.rim_by_name.get(&e.resource)).and_then(|at| self.rim_dirs.get(*at)).cloned();
        self.opt_x.arcade.setup = Some((car.to_ascii_lowercase(), config.to_string()));
        Some((cfg, dir, inches))
    }

    /// The livery key of the borrowed vehicle's set-up ("setup:<file>"), while the player
    /// sits in that vehicle; the loop paints its layers (`vinyl::Livery::from_xcc`).
    pub fn mc4_setup_livery(&mut self, car: &str) -> Option<String> {
        if !self.borrowed {
            self.opt_x.arcade.setup = None;
        }
        self.opt_x.arcade.setup.as_ref().filter(|s| s.0.eq_ignore_ascii_case(car)).map(|s| format!("setup:{}", s.1))
    }

    /// The picture of a Car reward: `resources/ui/textures/garage/dyntextures/rewardvehicles/
    /// <texture name>.r9` (the movie's `reward_dyntexture` clip names its texture; the form
    /// gives the file's name as string variable 17 - F-36204). None: no such file.
    pub fn mc4_reward_picture(&self, name: &str) -> Option<std::path::PathBuf> {
        let dir = std::path::Path::new(&self.data_dir).join("resources/ui/textures/garage/dyntextures/rewardvehicles");
        let want = format!("{}.r9", name.to_ascii_lowercase());
        std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().map_or(false, |n| n.to_string_lossy().to_ascii_lowercase() == want))
    }

    /// The pictures of the car rows on the rewards page shown, in the rows' order.
    pub fn mc4_reward_pictures(&self) -> Vec<Option<std::path::PathBuf>> {
        let hook = std::env::var("MCLA_REWARD_CAR").ok();
        self.career.cars_now.iter().chain(hook.iter()).take(8).map(|c| self.mc4_reward_picture(c)).collect()
    }

    /// The difficulty a payback leg is run at (the panel's, results +3100).
    pub fn mc4_payback_diff(&self) -> usize {
        self.event_diff.unwrap_or(self.race_diff).min(3)
    }

    /// A payback leg, once a tick (before the tick's own work): the approach and the
    /// moment the client is reached. Returns the share of damage that wins the leg.
    pub fn mc4_payback_tick(&mut self, vehicles: &mut crate::vehicles::Vehicles) -> f32 {
        let diff = self.mc4_payback_diff();
        let thr = payback_threshold(diff);
        let Some(race) = self.race.as_mut().filter(|r| r.kind == crate::game::RaceKind::Payback) else {
            self.opt_x.arcade.pb = None;
            // (step 19d: the factors go with the race - Racer_Reset, F-15879)
            if std::mem::take(&mut self.opt_x.arcade.d.inflict_on) {
                for c in vehicles.cars.iter_mut() {
                    c.inflict = 1.0;
                }
            }
            return thr;
        };
        // (step 19d: the damage factors - PaybackCore 0x73f..0x79b, PaybackLegCore 0x8bf, 0xa47)
        self.opt_x.arcade.d.inflict_on = true;
        let s19d_phase = self.opt_x.arcade.pb.as_ref().map_or(0, |l| l.phase);
        for (i, c) in vehicles.cars.iter_mut().enumerate() {
            if c.is_player {
                c.inflict = PAYBACK_INFLICT[diff.min(3)];
            } else if race.target == Some(i) {
                c.inflict = if s19d_phase == 0 { PAYBACK_CLIENT_INFLICT.0 } else { PAYBACK_CLIENT_INFLICT.1 };
            }
        }
        if race.countdown > 0.0 {
            self.opt_x.arcade.pb = Some(PaybackLeg::default());
            return thr;
        }
        let mut leg = self.opt_x.arcade.pb.take().unwrap_or_default();
        // (step 19d: a leg after a won or a lost one opens with the mechanic's call -
        // PaybackLegCore 0x730..0x829, Script_PayBackSuccess)
        if leg.phase == 0 && leg.since == 0.0 && (self.career.payback_wins > 0 || self.opt_x.arcade.d.next_call) && !std::mem::replace(&mut leg.called, true) {
            self.opt_x.arcade.d.next_call = true;
            self.opt_x.arcade.pb = Some(leg);
            self.mc4_call("SIDEKICK_PAYBACK_NEXT", "SIDEKICK_PAYBACK_NEXT");
            leg = self.opt_x.arcade.pb.take().unwrap_or_default();
        }
        let Some(race) = self.race.as_mut() else { return thr };
        if leg.phase >= 2 {
            self.opt_x.arcade.pb = Some(leg);
            return f32::INFINITY;
        }
        // (step 19d: the time is up - "Payback: TIMER UP!", 0xd72..0xe17: PB_Failed, the
        // client's icon and the GPS's target go, the clock is hidden; the chase follows:
        // `mc4_payback_chase`)
        if leg.phase == 1 && race.time_limit > 0.0 && race.time > race.time_limit {
            leg.phase = 2;
            leg.t = 0.0;
            race.time_limit = 0.0;
            race.target = None;
            race.checkpoints.clear();
            vehicles.fragile = None;
            self.opt_x.arcade.pb = Some(leg);
            self.mc4_splash("PB_Failed", "OBJECTIVE FAILED!");
            log::info!("payback: the leg's time is up");
            return f32::INFINITY;
        }
        let mut reached = false;
        if leg.phase == 0 {
            let pp = vehicles.cars.iter().find(|c| c.is_player).map(|c| c.state.pos);
            let tp = race.target.and_then(|t| vehicles.cars.get(t)).map(|c| c.state.pos);
            // (the clock does not run before the client is reached: it starts from 0 then)
            if race.time_limit > 0.0 {
                race.time_limit += race.time - leg.since;
            }
            leg.since = race.time;
            if let (Some(p), Some(t)) = (pp, tp) {
                // (test hook MCLA_PB_TIMEOUT=<seconds>: the client counts as reached at once
                // and the leg's time is that many seconds)
                let hook = std::env::var("MCLA_PB_TIMEOUT").ok().and_then(|v| v.parse::<f32>().ok());
                if p.distance(t) < 50.0 || hook.is_some() {
                    leg.phase = 1;
                    reached = true;
                    if let Some(s) = hook {
                        race.time_limit = race.time + s;
                    }
                }
            }
            if let Some(ai) = race.target.and_then(|t| vehicles.cars.get_mut(t)).and_then(|c| c.ai.as_mut()) {
                // (a cruising brain that was given no speed drives at its rail's - `RAIL_SPEED`;
                // BrainRacing_SetMaxSpeed(32) once he runs)
                ai.target_speed = if reached { 32.0 } else { RAIL_SPEED };
            }
        }
        self.opt_x.arcade.pb = Some(leg);
        if reached {
            let t = self.career.strings.get("PB_GetHim").map(|s| s.to_uppercase()).unwrap_or_else(|| "THAT'S HIM, GET HIM!".to_string());
            self.show_message(&t);
        }
        thr
    }

    /// The payback's figures for HUD control 10 (PaybackCore 0x7f2..0x826, 0xcc7): the wins
    /// of the run, the wins needed, the meter's full mark, and whether the race clock is on
    /// show (not before the client is reached).
    pub fn mc4_payback_hud(&self) -> Option<(i32, i32, f32, bool)> {
        let race = self.race.as_ref().filter(|r| r.kind == crate::game::RaceKind::Payback)?;
        let _ = race;
        let reached = self.opt_x.arcade.pb.as_ref().map_or(true, |l| l.phase > 0);
        // (step 19d: the meter's full mark is the table at +3368; the clock is off again
        // once the leg's time is up)
        let reached = reached && self.opt_x.arcade.pb.as_ref().map_or(true, |l| l.phase < 2);
        Some((self.career.payback_wins as i32, self.mc4_payback_needed() as i32, PAYBACK_METER_MAX[self.mc4_payback_diff().min(3)], reached))
    }
}

// ---- step 19d: the payback's chase and factors, set-ups on every car ----
//
// The payback (PaybackCore `game/0x3ae26fc8/0x422e62e2`, PaybackLegCore `0xb3f9732f`, the
// set-up of its data `game/Hangouts/0xbaa0ed14`, the chasers `game/0x107ebb16/0x3faa7997`):
//   - the player drives the garage's car, `pyback_plr_vp_buk_grandnational_87`
//     (mcCarConfig_Load + Racer_ChangeCarByConfig, PaybackCore 0x26..0x38; his own comes back
//     when the run is over, 0x20dc);
//   - the client of a leg: a random one not yet used of the twelve set-ups at data +3292 and
//     of the five drivers at +3240 (0x83d..0xaff: Racer_ChangeCar, Racer_SetModValue(0));
//     while the mission PAYBACK_GH_18 is under way: GH_18 in his RX-7;
//   - the damage factors: the player inflicts 3 times his blows (the table at +3388, 3 at all
//     four difficulties; Racer_SetDamageMultiplierFlag(32) multiplies what he takes by his
//     "difficulty damage factor", which nothing sets: 1), the client 0.65 of his while he
//     cruises and 0.55 once he runs (0x73f..0x79b, PaybackLegCore 0x8bf, 0xa47). The factor
//     is mcCarDamage +20, read for the vehicle that was hit by F-61340;
//   - `payback_meterMax` is the table at +3368: 0.5, 0.75, 0.87, 1.0 - the leg's threshold;
//   - a cruising brain without a speed (BrainCruising +1664 = -1) drives at its rail's own
//     speed (F-90476 A-16530): the road's, as the traffic.
//   - the leg's time is up: PB_Failed, a second, the mechanic's call
//     (SIDEKICK_PAYBACK_WARNING, speech PAYBACK_WARNING), the three chasers are made, a
//     second, PoliceManager_StartPursuit(player, 3, 0, 0, 0), PB_LoseTheseGuys, the taunt
//     TAUNT_GOON_CHASE. Then the tactic's result: 2 - PB_Lost, TAUNT_GOON_END, the leg is
//     over without a win and the run goes on (Script_PayBackSuccess 1: the next leg opens
//     with SIDEKICK_PAYBACK_NEXT); 1 - PB_Caught, two seconds, nPlayerCaught: the run is
//     over. While the three search (PoliceManager_CheckPoliceActivity 3) TAUNT_GOON_SEARCH
//     every 15..30 s of the race timer, which was put back to 0 at the call. A player
//     damaged out: the run is over the same way.
//   - HUD control 10's property 5 (`paybackisvisible`) is 1 from the leg's start and 2 once
//     the time is up: the movie then hides the damage bar and its mark, the wins stay.

/// The three chasers: set-up, vehicle, driver set, id (`game/0x107ebb16/0x3faa7997`; map
/// icon type 13 - `icon_ingame13`, a texture the disc does not have).
pub const PAYBACK_GOONS: [(&str, &str, &str, u32); 3] = [
    ("pyback_gn_vp_dge_challenger_70", "vp_dge_challenger_70", "drv_mb_009_set", 450),
    ("pyback_gn_vp_dge_charger_06", "vp_dge_charger_06", "drv_mb_001_set", 451),
    ("pyback_gn_vp_pon_transam_79", "vp_pon_transam_79", "drv_mb_03_set", 452),
];

/// The clients' set-ups (data +3292) and drivers (+3240), `game/Hangouts/0xbaa0ed14`.
pub const PAYBACK_CLIENTS: [&str; 12] = [
    "pyback_vp_dge_challenger_06",
    "pyback_vp_mzd_rx7_95",
    "pyback_vp_chv_camaross_69",
    "pyback_vp_chv_camaro_06",
    "pyback_vp_cry_300c_06",
    "pyback_vp_chv_impala_96",
    "pyback_vp_chv_cobalt_05",
    "pyback_vp_dge_challenger_70",
    "pyback_vp_mit_3000gt_99",
    "pyback_vp_mzd_rx8_06",
    "pyback_vp_pon_solstice_06",
    "pyback_vp_mit_evo_06",
];
pub const PAYBACK_DRIVERS: [&str; 5] = ["drv_mb_007_set", "drv_mc_06_set", "drv_mh_01_set", "drv_mc_003_set", "drv_fh_001_set"];

/// The player's vehicle and set-up in a payback (data +3212).
pub const PAYBACK_PLAYER: (&str, &str) = ("vp_buk_grandnational_87", "pyback_plr_vp_buk_grandnational_87");

/// The player's "damage inflicted" factor by difficulty (data +3388), the client's while
/// he cruises and once he runs.
pub const PAYBACK_INFLICT: [f32; 4] = [3.0, 3.0, 3.0, 3.0];
pub const PAYBACK_CLIENT_INFLICT: (f32, f32) = (0.65, 0.55);

/// `payback_meterMax` by difficulty (data +3368).
pub const PAYBACK_METER_MAX: [f32; 4] = [0.5, 0.75, 0.87, 1.0];

/// The speed of a cruising brain that was given none: its rail's - the road's speed, 40 mph
/// on nearly every road (the port's traffic drives 0.86..1 of it).
pub const RAIL_SPEED: f32 = 17.88;

/// How a payback leg ended, for the loop: nothing yet, the client's friends caught the
/// player (the run is over), the player lost them (the run goes on without a win).
pub const PB_CAUGHT: u8 = 1;
pub const PB_LOST: u8 = 2;

/// What the running game keeps for this step (in `ArcadeLive`: not saved).
#[derive(Clone, Debug, Default)]
pub struct S19d {
    /// the client of the leg about to start: (vehicle, set-up, driver set)
    pub client: Option<(String, String, String)>,
    /// the table's set-ups and drivers this run has had, and how many (PaybackCore's
    /// locals 16 / 12 and 29 / 13)
    pub cars_used: [bool; 12],
    pub cars_n: usize,
    pub drivers_used: [bool; 5],
    pub drivers_n: usize,
    /// the damage factors are on the cars (taken off when the payback's race is gone)
    pub inflict_on: bool,
    /// speech scenarios for the loop to say
    pub say: Vec<String>,
    /// frontend sounds for the loop to play (`AudioManager_PlayFrontendSound`), and a loop
    /// that is to run (the time trial's tick)
    pub sounds: Vec<&'static str>,
    pub tick_loop: bool,
    /// the last leg ended with the chasers lost (Script_PayBackSuccess without a win)
    pub lost_leg: bool,
    /// the next leg opens with the mechanic's call SIDEKICK_PAYBACK_NEXT
    pub next_call: bool,
    /// the look of the set-up the borrowed vehicle wears: (set-up, look, tire set, brake)
    pub borrowed: Option<(String, crate::vehicles::CarLook, String, Option<String>, [f32; 4])>,
    /// the race clock's time left (ms) at the HUD's last update (the counting down clock's
    /// sounds compare it with the time left now)
    pub clock_prev: Option<i64>,
    /// the event results banner (form 56): seconds left, complete
    pub banner: Option<(f32, bool)>,
    /// the banner's event kind (results +3104): 2 a time trial, 3 a series, 4 a tournament
    /// (F-94174: the icon and the text; 0 is taken as the time trial's)
    pub banner_kind: u8,
    /// the local hookman the race is run with - (character, vehicle, set-up) - and the
    /// three races he keeps (race table indices: mcRaceArray_LocalHookmanFilter)
    pub race_man: Option<(String, String, String)>,
    pub man_races: Vec<usize>,
    /// that race's place among `race_defs`
    pub man_race: Option<usize>,
    /// the red light racer of the race about to start: (character, vehicle, set-up)
    pub redlight_man: Option<(String, String, String)>,
    /// generic_local_hookman's registry value `bNextRace` (the race-over menu offers NEXT
    /// RACE while it is 1) and how often it was taken (its local 857)
    pub next_race: bool,
    pub next_count: u8,
    rng: u32,
}

impl S19d {
    fn rand(&mut self) -> u32 {
        let mut x = if self.rng == 0 { 0x9e37_79b9 } else { self.rng };
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// RAND_INT_RANGE(lo, hi): both ends in (PaybackLegCore picks one of two voices with
    /// `RAND_INT_RANGE(0, 1) == 1`).
    pub fn rand_range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.rand() % (hi - lo + 1) as u32) as i32
    }
}

/// PaybackCore 0x848..0x896: a random entry of `used` that is not used yet - the r-th free
/// one, r in 0 ..= n - used - 1; 0 when there is none.
pub fn payback_pick(used: &[bool], n_used: usize, r: usize) -> usize {
    let free = used.len().saturating_sub(n_used);
    let mut r = if free == 0 { 0 } else { r % free };
    for (i, u) in used.iter().enumerate() {
        if !*u {
            if r == 0 {
                return i;
            }
            r -= 1;
        }
    }
    0
}

/// What a set-up file paints: the body's first colour as the port holds the player's, and
/// the rest as the renderer takes it (`vehicles::CarLook`), the tire set and the brake model.
#[derive(Clone, Debug, PartialEq)]
pub struct SetupLook {
    pub color: [f32; 4],
    pub look: crate::vehicles::CarLook,
    pub tire: String,
    pub brake: Option<String>,
}

/// The look of a set-up file (`resources/vehicle/configs/<name>.xcc`: the garage record's
/// own layout), field by field as a player's record of the block comes to the renderer:
///   - the six paint areas at 0x660 + 0x30 k (first colour, second colour +0x10, type
///     +0x20, fade +0x24): the body's first colour is the car's colour; an area after the
///     body has a colour of its own when its first colour differs from the body's by more
///     than 0.02 (the importer's rule) or its type, fade or second colour differ
///     (`paint_out`); a matte type is drawn as `paint_as_drawn` says;
///   - the rims' three colours at 0x90 / 0xa0 / 0xb0 (spoke A, spoke B, lip) and materials
///     at 0x178 (0 painted, 1 chrome, 2 carbon fibre, 3 brushed metal): `rim_finish`;
///   - the cabin's five groups at 0x5c0 + 0x10 k (darker than 0.08: the stock cabin):
///     `interior_groups`;
///   - the window tint at 0x160 (colour, darkness): `tint_rgba`;
///   - the tire set, 0x845 (`TIRES`), and the brake model, 0x846 (`BRAKES`).
/// Step 19e: a set-up's licence plate text (0x847, seven letters; letters, digits and spaces).
pub fn setup_plate(b: &[u8]) -> String {
    b.get(0x847..0x84e).map_or(String::new(), |l| l.iter().take_while(|c| **c != 0).filter(|c| c.is_ascii_alphanumeric() || **c == b' ').map(|c| *c as char).collect::<String>().trim().to_string())
}

pub fn setup_look(b: &[u8]) -> Option<SetupLook> {
    if b.len() != 8176 {
        return None;
    }
    let f = |o: usize| f32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let u = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let rgb = |o: usize| -> Option<[f32; 3]> {
        let c = [f(o), f(o + 4), f(o + 8)];
        c.iter().all(|v| (0.0..=1.0).contains(v)).then_some(c)
    };
    let area = |k: usize| -> AreaPaint {
        let e = 0x660 + 0x30 * k;
        AreaPaint { ty: u(e + 0x20).min(5) as u8, fade: u(e + 0x24).min(4) as u8, b: rgb(e + 0x10).unwrap_or([0.0; 3]) }
    };
    let body_a = rgb(0x660)?;
    let body = paint_as_drawn(area(0));
    let body_b = to_linear(body.b);
    let mut look = crate::vehicles::CarLook { paint_type: body.ty, color2: [body_b[0], body_b[1], body_b[2], body.fade as f32], tint: [0.0, 0.0035, 0.005, 0.372], interior: [[0.0; 4]; 5], area_a: [[0.0; 4]; 5], area_b: [[0.0; 4]; 5], rim_finish: [[0.0; 4]; 3] };
    for k in 1..6 {
        // (a part's colour goes through the port's 8-bit store, as an imported record's)
        let own_a = rgb(0x660 + 0x30 * k).filter(|h| (0..3).any(|i| (h[i] - body_a[i]).abs() > 0.02)).map(|h| to_linear([(h[0] * 255.0).round() / 255.0, (h[1] * 255.0).round() / 255.0, (h[2] * 255.0).round() / 255.0]));
        let own = own_a.is_some() || area(k) != area(0);
        let a = own_a.unwrap_or(body_a);
        let p = paint_as_drawn(area(k));
        let pb = to_linear(p.b);
        look.area_a[k - 1] = [a[0], a[1], a[2], own as u8 as f32];
        look.area_b[k - 1] = [pb[0], pb[1], pb[2], p.ty as f32 + 8.0 * p.fade as f32];
    }
    for k in 0..3 {
        let c = rgb(0x90 + 0x10 * k).map(to_linear).unwrap_or([0.8, 0.8, 0.8]);
        // (the record's material as the port's finish: 0 painted, 1 chrome, 3 brushed -> 2,
        // 2 carbon fibre -> 3; the renderer's w is that + 1)
        let mat = match u(0x178 + 4 * k) {
            0 => 0,
            1 => 1,
            3 => 2,
            2 => 3,
            _ => 1,
        };
        look.rim_finish[[0, 2, 1][k]] = [c[0], c[1], c[2], (mat + 1) as f32];
    }
    for k in 0..5 {
        if let Some(c) = rgb(0x5c0 + 0x10 * k) {
            if c[0].max(c[1]).max(c[2]) >= 0.08 {
                let l = to_linear(c);
                look.interior[k] = [l[0], l[1], l[2], 1.0];
            }
        }
    }
    if let Some(c) = rgb(0x160) {
        let a = f(0x16c);
        if a.is_finite() && tint_row_of(a as f64) > 0 {
            look.tint = [c[0], c[1], c[2], a];
        }
    }
    let tire = TIRES.get(b[0x845] as usize).filter(|t| t.starts_with("tir_car")).map(|t| t.to_string()).unwrap_or_default();
    let brake = BRAKES.get(b[0x846] as usize).map(|s| s.to_string());
    Some(SetupLook { color: [body_a[0], body_a[1], body_a[2], 1.0], look, tire, brake })
}

impl Game {
    fn mc4_setup_file(&self, config: &str) -> Option<Vec<u8>> {
        if config.is_empty() {
            return None;
        }
        // (step 21b: a held car's own record - `mc4_pink_rival`)
        if let Some(uid) = config.strip_prefix("pinkslip:").and_then(|u| u.parse::<i32>().ok()) {
            return self.mc4_held_bytes(uid);
        }
        std::fs::read(crate::render::car_lamps::setup_file(std::path::Path::new(&self.data_dir), config)?).ok()
    }

    /// A set-up as a car that is not the player's is put into it (`Vehicles::dress_setups`):
    /// parts, the rim of the rim table and its size, tires, brakes, the paints, and the
    /// set-up's share of performance upgrades (a hookman's mod value was given him when he
    /// was made; a car that has a share keeps it).
    pub fn mc4_dress_of(&self, car: &str, config: &str) -> Option<crate::vehicles::Dress> {
        let b = self.mc4_setup_file(config)?;
        let s = setup_look(&b)?;
        let (cfg, rim, inches) = setup_parts(&b)?;
        let rim_dir = self.rim_shop.entry(rim as i32).and_then(|e| self.rim_by_name.get(&e.resource)).and_then(|at| self.rim_dirs.get(*at)).cloned();
        let inches = if (13..=26).contains(&inches) { inches } else { 0 };
        let perf = self.mc4_setup_percent(car, config).clamp(0, 100) as f32 / 100.0;
        // (the payback's clients are stock whatever their set-up says: Racer_SetModValue(0),
        // PaybackCore 0xace)
        let perf = (!PAYBACK_CLIENTS.contains(&config)).then_some(perf);
        Some(crate::vehicles::Dress { cfg, rim_dir, inches, tire: s.tire, brake: s.brake, color: s.color, look: s.look, perf, plate: setup_plate(&b) })
    }

    /// The borrowed vehicle is in set-up `config`: its look is the set-up's (the body's
    /// colour is returned for the vehicle's making).
    pub fn mc4_borrow_look(&mut self, config: &str) -> Option<[f32; 4]> {
        let s = self.mc4_setup_file(config).and_then(|b| setup_look(&b))?;
        self.opt_x.arcade.d.borrowed = Some((config.to_string(), s.look, s.tire, s.brake, s.color));
        Some(s.color)
    }

    /// The set-up's look while the player sits in the borrowed vehicle: what the loop hands
    /// the renderer in place of the player's own record's (look, tire set, brake model).
    pub fn mc4_borrowed_look(&mut self) -> Option<(crate::vehicles::CarLook, String, Option<String>)> {
        if !self.borrowed || self.opt_x.arcade.setup.is_none() {
            self.opt_x.arcade.d.borrowed = None;
            return None;
        }
        let want = self.opt_x.arcade.setup.as_ref().map(|s| s.1.clone())?;
        self.opt_x.arcade.d.borrowed.as_ref().filter(|b| b.0 == want).map(|b| (b.1.clone(), b.2.clone(), b.3.clone()))
    }

    /// Step 19e: the plate of the set-up the borrowed vehicle wears (None: not borrowed in a
    /// set-up, or the set-up has no plate text).
    pub fn mc4_borrowed_plate(&self) -> Option<String> {
        let (_, cfg) = self.opt_x.arcade.setup.as_ref().filter(|_| self.borrowed)?;
        Some(setup_plate(&self.mc4_setup_file(cfg)?)).filter(|p| !p.is_empty())
    }

    /// The vehicle and set-up the player is put into for the race about to start: a time
    /// trial's row (`mc4_tt_run`), the payback's garage car.
    pub fn mc4_borrow_for(&self, def: &crate::races::RaceDef) -> Option<(String, String)> {
        if let Some((car, cfg, _)) = self.mc4_tt_run(&def.id) {
            return Some((car, cfg));
        }
        (def.race_type == "Payback").then(|| (PAYBACK_PLAYER.0.to_string(), PAYBACK_PLAYER.1.to_string()))
    }

    /// The client of the payback leg about to start (PaybackCore 0x187..0x24a, 0x836..0xaff):
    /// GH_18 in his RX-7 while PAYBACK_GH_18 is under way, else the table's.
    pub fn mc4_payback_pick(&mut self) -> (String, String, String) {
        let d = &mut self.opt_x.arcade.d;
        let pick = if self.career.mission_active("PAYBACK_GH_18") {
            ("vp_mzd_rx7_95".to_string(), "GH_18_2_vp_mzd_rx7_95".to_string(), "GH_18".to_string())
        } else {
            let r = d.rand_range(0, 12 - d.cars_n as i32 - 1).max(0) as usize;
            let ci = payback_pick(&d.cars_used, d.cars_n, r);
            let r = d.rand_range(0, 5 - d.drivers_n as i32 - 1).max(0) as usize;
            let di = payback_pick(&d.drivers_used, d.drivers_n, r);
            d.cars_used[ci] = true;
            d.drivers_used[di] = true;
            d.cars_n += 1;
            d.drivers_n += 1;
            if d.cars_n >= 12 {
                d.cars_used = [false; 12];
                d.cars_n = 1;
                d.cars_used[ci] = true;
            }
            if d.drivers_n >= 5 {
                d.drivers_used = [false; 5];
                d.drivers_n = 1;
                d.drivers_used[di] = true;
            }
            let set_up = PAYBACK_CLIENTS[ci];
            (set_up.trim_start_matches("pyback_").to_string(), set_up.to_string(), PAYBACK_DRIVERS[di].to_string())
        };
        log::info!("payback: the leg's client is {} in {} ({} of 12 set-ups, {} of 5 drivers used)", pick.2, pick.1, d.cars_n, d.drivers_n);
        d.client = Some(pick.clone());
        pick
    }

    /// A call of the mechanic's on the phone while an event runs (Sidekick_AppendData: the
    /// caller, the text's key, five seconds; its speech scenario).
    fn mc4_call(&mut self, text_key: &str, scenario: &str) {
        let text = self.career.strings.get(text_key).map(|s| s.to_string()).unwrap_or_default();
        if !text.is_empty() {
            self.tip = Some(("KAROL".to_string(), text, 5.0));
            self.phone_prompt.clear();
        }
        self.opt_x.arcade.d.say.push(scenario.to_string());
    }

    fn mc4_splash(&mut self, key: &str, fallback: &str) {
        let t = self.career.strings.get(key).map(|s| s.to_uppercase()).unwrap_or_else(|| fallback.to_string());
        self.show_message(&t);
    }

    /// The speech scenarios and frontend sounds the game's rules asked for since the last
    /// call, for the loop (which owns the voices).
    pub fn mc4_say_take(&mut self) -> (Vec<String>, Vec<&'static str>, bool) {
        // (the run to the line's thirty seconds - no race: the clock's sounds once a frame)
        if self.race.is_none() {
            let left = self.opt_x.arcade.bmt_left.get().map(|s| (s as f64 * 1000.0).round() as i64);
            self.mc4_clock_sounds(left);
        }
        let d = &mut self.opt_x.arcade.d;
        (std::mem::take(&mut d.say), std::mem::take(&mut d.sounds), d.tick_loop)
    }

    /// `paybackisvisible` (HUD control 10's property 5): 1 from the leg's start, 2 once its
    /// time is up.
    pub fn mc4_payback_vis(&self) -> i32 {
        match self.opt_x.arcade.pb.as_ref() {
            Some(l) if l.phase >= 2 => 2,
            _ => 1,
        }
    }

    /// The payback's chase, once a tick of the loop, right after the police's own update
    /// (`ev`: what that update reported). Returns the report for the ordinary handling -
    /// none while the pursuers are the client's friends: no radio, no fine, no arrest form
    /// (the manager's update skips them for cars of type 0: F-22530 A-38933,
    /// A-69090, A-18979).
    pub fn mc4_payback_chase(&mut self, police: &mut crate::police::Police, vehicles: &mut crate::vehicles::Vehicles, gpu: &mut crate::render::Gpu, world: &mut crate::world::World, ev: Option<crate::police::Event>, dt: f32) -> Option<crate::police::Event> {
        use crate::police::{Event, Heat};
        let running = self.race.as_ref().map_or(false, |r| r.kind == crate::game::RaceKind::Payback) && self.mode == crate::game::Mode::Race;
        let Some(mut leg) = self.opt_x.arcade.pb.clone().filter(|l| l.phase >= 2 && running) else {
            // (their script is over - Shutdown_PaybackOpponents: PoliceManager_DestroyPolice)
            if (police.goons || vehicles.cars.iter().any(|c| c.goon)) && self.race.as_ref().map_or(true, |r| r.kind != crate::game::RaceKind::Payback || self.opt_x.arcade.pb.as_ref().map_or(true, |l| l.phase < 2)) {
                police.end_goons(vehicles);
            }
            return ev;
        };
        leg.t += dt;
        leg.clock += dt;
        match leg.phase {
            2 => {
                // (WAITUNWARPED(1000), then the call; the race timer starts again from 0)
                if leg.t >= 1.0 {
                    self.mc4_call("SIDEKICK_PAYBACK_WARNING", "PAYBACK_WARNING");
                    leg.phase = 3;
                    leg.t = 0.0;
                    leg.clock = 0.0;
                }
            }
            3 => {
                // (the three are made - their vehicles read, one a tick - and a second later
                // the pursuit starts)
                if (leg.made as usize) < PAYBACK_GOONS.len() {
                    let (_, car, _, _) = PAYBACK_GOONS[leg.made as usize];
                    if !vehicles.models.iter().any(|m| m.name.eq_ignore_ascii_case(car)) {
                        let dir = world.assets.root.join("resources/vehicle").join(car);
                        if dir.is_dir() {
                            let _ = vehicles.load_model(gpu, world, &dir);
                        }
                    }
                    leg.made += 1;
                } else if leg.t >= 1.0 {
                    police.start_goons(vehicles);
                    self.mc4_splash("PB_LoseTheseGuys", "LOSE THESE GUYS!");
                    self.opt_x.arcade.d.say.push("TAUNT_GOON_CHASE".to_string());
                    leg.phase = 4;
                    leg.t = 1.0;
                    leg.taunt_at = 0.0;
                    log::info!("payback: the client's friends are after the player");
                }
            }
            4 => {
                // (those not on the road yet come when a place is found: a try a second)
                if leg.t >= 1.0 {
                    leg.t = 0.0;
                    for (k, (set_up, car, driver, _)) in PAYBACK_GOONS.iter().enumerate() {
                        if leg.on_road & (1 << k) != 0 {
                            continue;
                        }
                        let Some(model) = vehicles.models.iter().find(|m| m.name.eq_ignore_ascii_case(car)).cloned() else {
                            leg.on_road |= 1 << k;
                            continue;
                        };
                        let color = self.mc4_setup_file(set_up).and_then(|b| setup_look(&b)).map_or([0.3, 0.3, 0.3, 1.0], |s| s.color);
                        if police.spawn_goon(gpu, world, vehicles, model, color, set_up, driver).is_some() {
                            leg.on_road |= 1 << k;
                        }
                    }
                }
                // (PoliceManager_CheckPoliceActivity(0) == 3 and the next taunt's time has come)
                if police.heat == Heat::Search && leg.taunt_at < leg.clock {
                    self.opt_x.arcade.d.say.push("TAUNT_GOON_SEARCH".to_string());
                    leg.taunt_at = leg.clock + self.opt_x.arcade.d.rand_range(15000, 30000) as f32 / 1000.0;
                }
                // (test hook MCLA_PB_RESULT=lost@<s> | caught@<s>: that result once the pursuit
                // has run so many seconds)
                let ev = match std::env::var("MCLA_PB_RESULT").ok().as_deref().and_then(|v| v.split_once('@')).and_then(|(w, s)| s.parse::<f32>().ok().map(|s| (w == "lost", s))) {
                    Some((lost, s)) if leg.clock >= s + 1.0 => Some(if lost { Event::Escaped } else { Event::Busted(0) }),
                    _ => ev,
                };
                match ev {
                    Some(Event::Escaped) => {
                        self.mc4_splash("PB_Lost", "YOU LOST THEM!");
                        self.opt_x.arcade.d.say.push("TAUNT_GOON_END".to_string());
                        leg.result = PB_LOST;
                        leg.phase = 6;
                        police.end_goons(vehicles);
                        log::info!("payback: the client's friends are lost - the run goes on without a win");
                    }
                    Some(Event::Busted(_)) => {
                        self.mc4_splash("PB_Caught", "CAUGHT!");
                        police.hold_busted(2.5);
                        leg.phase = 5;
                        leg.t = 0.0;
                        log::info!("payback: caught by the client's friends - the run is over");
                    }
                    _ => {}
                }
            }
            5 => {
                // (WAITUNWARPED(2000), nPlayerCaught)
                if leg.t >= 2.0 {
                    leg.result = PB_CAUGHT;
                    leg.phase = 6;
                }
            }
            _ => {}
        }
        self.opt_x.arcade.pb = Some(leg);
        None
    }

    /// Has the payback leg ended by its chase (or by the player's wreck)? Called once a tick
    /// of the race, after `mc4_payback_tick`: a lost pursuit finishes the leg without a win,
    /// a catch and a damaged-out player fail it (`Race::failed`: the run is over).
    pub fn mc4_payback_over(&mut self, vehicles: &crate::vehicles::Vehicles) -> bool {
        let Some(race) = self.race.as_mut().filter(|r| r.kind == crate::game::RaceKind::Payback && r.countdown <= 0.0) else { return false };
        if race.failed.is_some() || race.finish_time.first().map_or(false, |t| t.is_some()) {
            return false;
        }
        let pi = vehicles.cars.iter().position(|c| c.is_player);
        // (Race_IsDamagedOut: "Player has damaged out" - nPlayerCaught, the run is over)
        if pi.map_or(false, |i| vehicles.cars[i].state.damage >= 1.0) {
            race.failed = Some("DAMAGED OUT");
            return true;
        }
        match self.opt_x.arcade.pb.as_ref().map_or(0, |l| l.result) {
            PB_CAUGHT => {
                race.failed = Some("CAUGHT");
                true
            }
            PB_LOST => {
                if let Some(i) = pi {
                    while race.finish_time.len() <= i {
                        race.finish_time.push(None);
                    }
                    race.finish_time[i] = Some(race.time);
                }
                self.opt_x.arcade.d.lost_leg = true;
                true
            }
            _ => false,
        }
    }

    /// The leg that just finished was one the chasers were lost in: no win, no money - the
    /// run goes on with the next client (PaybackCore 0xc30..0xd48: the wins have not come to
    /// the wins needed and the player was not caught). True once per such leg.
    pub fn mc4_payback_take_lost(&mut self) -> bool {
        if !std::mem::take(&mut self.opt_x.arcade.d.lost_leg) {
            return false;
        }
        self.last_reward = (0, 0);
        self.career.payback_next = self.career.payback_run;
        self.opt_x.arcade.d.next_call = true;
        true
    }
}

// The generic local hookman's meeting (generic_local_hookman.sco = `game/CruisingHookmen/
// 0x897fb23f`, after CruisingHookmanCore `0x0d18ab57` has seen him flashed):
//   - there is no form. The core's loop asks nothing: flashing the high beams within reach
//     (Racer_CanFlashHookman) is the acceptance (0x8cf..0xdea), provided the cruise's UI
//     state listens, the game is not paused, no idle camera runs and the police are doing
//     nothing with the player (PoliceManager_CheckPoliceActivity(1) < 2: no pull-over,
//     pursuit, search or arrest - F-35540);
//   - Racer_CanFlashHookman (F-96972): the player's vehicle is in no special state
//     (sim +592 == 0), the distance is at most 30, the speeds differ by at most 25, the
//     player's axis c and the direction to the man make -0.2 or less (he is ahead by 0.2 or
//     more), and a segment from 0.5 above the one vehicle to 0.5 above the other hits
//     nothing (F-20855);
//   - the first time ever (stat `tTBM` 0): the tutorial's form 102 with the speech
//     TUTORIAL_BEAT_ME_THERE (0xc0e..0xe18);
//   - police within 250 of the player (PoliceManager_IsAnyPoliceCloseToPlayer(250)): the
//     man says GREETING_MEET_ME_THERE_COPS, the splash COPS_TOO_CLOSE, and the run to the
//     line is skipped - the race starts at its grid (0xe37..0xee8 -> 0x26f5,
//     SKIP_BEAT_ME_THERE_WITH_CUT); else the splash RACE_TO_START ("BEAT ME THERE") and his
//     greeting GREETING_BEAT_ME_THERE with a gesture (0xef2..0xf8b). Either way every
//     police car goes underground (PoliceManager_SetAllPoliceUnderground(1, 1));
//   - the race is his and three more racers' (the script's argument 3: racers 2..4 are
//     random characters in cars of mcCarArray_Fill for the difficulty, 0x926..0xb6c);
//   - after it the race-over menu has NEXT RACE while `bNextRace` is 1 (set when the man is
//     flashed): the race is mcRaceArray_FindNextRaceIndex of his three, he says
//     GREETING_MEET_ME_THERE_EVENT, the race starts at its grid without a run to the line;
//     the second NEXT RACE puts `bNextRace` to 0 (0x3ed6..0x3fa7).
// 24 of the 56 story core scripts carry the same police rule and greeting.

impl Game {
    /// A hookman has been flashed and his race picked: the meeting as the scripts stage it.
    /// `generic`: he is no mission's own man (those greet with their mission's scenario).
    /// Returns true when the run to the line is skipped (police too close).
    pub fn mc4_meet(&mut self, man: Option<(String, String, String)>, race: usize, races: &[usize], cops_close: bool, generic: bool) -> bool {
        // (the first beat-me-there ever: the tutorial - stat tTBM)
        // (step 23: form 102 in the slow motion, its speech, CONTINUE - `mc4_tutorial`)
        if !self.fresh_game {
            self.mc4_tutorial("tTBM");
        } else if !self.mc4_tut_seen("tTBM") {
            self.mc4_stat_set("tTBM", 0, 1.0);
        }
        let d = &mut self.opt_x.arcade.d;
        d.race_man = man.filter(|_| generic);
        d.man_race = d.race_man.is_some().then_some(race);
        d.man_races = if generic { races.to_vec() } else { Vec::new() };
        d.next_race = generic && races.len() > 1;
        d.next_count = 0;
        if cops_close {
            d.say.push("GREETING_MEET_ME_THERE_COPS".to_string());
            self.mc4_splash("COPS_TOO_CLOSE", "COPS ARE TOO CLOSE.  LET'S MEET AT THE RACE START.");
            return true;
        }
        if generic {
            d.say.push("GREETING_BEAT_ME_THERE".to_string());
        }
        self.mc4_splash("raceToStart", "BEAT ME THERE");
        false
    }

    /// The race-over menu offers NEXT RACE: the race was a generic local hookman's and
    /// `bNextRace` stands.
    pub fn mc4_next_race_on(&self) -> bool {
        let d = &self.opt_x.arcade.d;
        d.next_race && d.man_races.len() > 1 && self.mc4_local_man_race()
    }

    /// Test hook MCLA_NEXT_RACE=<n>: NEXT RACE is taken once the menu has stood 150 frames,
    /// n times in all.
    pub fn mc4_next_race_hook(&self) -> bool {
        use std::sync::atomic::{AtomicU32, Ordering};
        static STOOD: AtomicU32 = AtomicU32::new(0);
        static DONE: AtomicU32 = AtomicU32::new(0);
        let Some(n) = std::env::var("MCLA_NEXT_RACE").ok().and_then(|v| v.parse::<u32>().ok()) else { return false };
        if DONE.load(Ordering::Relaxed) >= n || STOOD.fetch_add(1, Ordering::Relaxed) < 150 {
            return false;
        }
        STOOD.store(0, Ordering::Relaxed);
        DONE.fetch_add(1, Ordering::Relaxed);
        log::info!("test hook: NEXT RACE");
        true
    }

    /// Test hook MCLA_RO_CONTINUE=1: CONTINUE on the REWARDS panel once it has stood 120
    /// frames.
    pub fn mc4_continue_hook(&self) -> bool {
        use std::sync::atomic::{AtomicU32, Ordering};
        static STOOD: AtomicU32 = AtomicU32::new(0);
        if std::env::var("MCLA_RO_CONTINUE").is_err() || STOOD.fetch_add(1, Ordering::Relaxed) < 120 {
            return false;
        }
        STOOD.store(0, Ordering::Relaxed);
        true
    }

    /// Form 56, the event results banner (constructor F-10770, fill F-94174, update
    /// F-98159): a SimpleListForm without rows; its template is the form base's
    /// default - 2, `template_Menu` (F-51209 A-80132: neither the form's constructor
    /// nor its fill nor its update calls SetTemplate, vfunc 476 = F-44479) -; the constructor's title `EOR_eventResultsTitle` (a key the string
    /// tables do not have) is replaced by the fill's text by the race's kind and `bComplete`
    /// - a time trial (race type 2): TIMETRIAL_SUCCESS "NEW TIME POSTED" / TIMETRIAL_FAILURE
    /// "TIME TRIAL FAILED" -, no description, `menu.iconID` 4 (vfunc 648 = F-53926 puts
    /// the number in form +248, which the base writes as `iconID` - F-88697
    /// A-93683: 3 series, 5 tournament, 1 delivery, 4 time trial, 2 payback, 6 wager, 7
    /// pink slip), `menu.select` = complete,
    /// the prompt CONTINUE (table 0x82906FE4). The script shows it 3500 ms. `dt`: the frame's
    /// seconds; None when it is not up.
    pub fn mc4_banner_page(&mut self, dt: f32) -> Option<crate::flash_ui::MenuPage> {
        use crate::flash_ui::{MenuLayout, MenuPage, ICON_A};
        let (left, ok) = self.opt_x.arcade.d.banner?;
        if left <= 0.0 {
            self.opt_x.arcade.d.banner = None;
            return None;
        }
        // (test hook MCLA_BANNER_HOLD: the banner stays)
        let dt = if std::env::var("MCLA_BANNER_HOLD").is_ok() { 0.0 } else { dt };
        self.opt_x.arcade.d.banner = Some((left - dt, ok));
        let st = |k: &str, d: &str| self.career.strings.get(k).map(|v| v.to_uppercase()).filter(|v| !v.contains('<')).unwrap_or_else(|| d.to_string());
        Some(MenuPage {
            layout: MenuLayout::List,
            // (F-94174's jump table A-82231: a time trial 4, a series 3, a tournament 5)
            icon: match self.opt_x.arcade.d.banner_kind {
                3 => 3,
                4 => 5,
                _ => 4,
            },
            // (the fill's vfunc 340 -> 344 puts the text in form +48, which is what the base
            // writes as `menu.name` - vfunc 332, F-41972: the title; the constructor's
            // description is the empty string at 0x82000F9E)
            title: match (self.opt_x.arcade.d.banner_kind, ok) {
                (3, true) => st("Series_SUCCESS", "SERIES WON"),
                (3, false) => st("Series_FAILURE", "SERIES LOST"),
                (4, true) => st("Tournament_SUCCESS", "TOURNAMENT WON"),
                (4, false) => st("Tournament_FAILURE", "TOURNAMENT LOST"),
                (_, true) => st("TIMETRIAL_SUCCESS", "NEW TIME POSTED"),
                (_, false) => st("TIMETRIAL_FAILURE", "TIME TRIAL FAILED"),
            },
            description: String::new(),
            rows: Vec::new(),
            select: ok as usize,
            prompts: vec![(st("prompt_continue", "CONTINUE"), ICON_A)],
            list_max: 6,
            ..MenuPage::default()
        })
    }

    /// The race clock's time left in ms while it counts down on the HUD (control 6 with its
    /// property 0 and the race timer on): a time trial's, LAST MAN OUT's ten seconds, a
    /// payback leg's once the client is reached.
    pub fn mc4_clock_left_ms(&self) -> Option<i64> {
        let race = self.race.as_ref()?;
        if race.countdown > 0.0 {
            return None;
        }
        if let Some((_, c, _)) = self.mc4_hud_clock() {
            return Some((c as f64 * 10_000.0).round() as i64);
        }
        let leg = self.opt_x.arcade.pb.as_ref().filter(|l| l.phase == 1 && race.kind == crate::game::RaceKind::Payback && race.time_limit > 0.0)?;
        let _ = leg;
        Some(((race.time_limit - race.time) as f64 * 1000.0).round() as i64)
    }

    /// The counting down clock's sounds, once an update of the HUD (F-86111
    /// A-97648..A-58621): with the time left under 20 s, changed since the last update
    /// and the last one above 0, the tick loop RACE_TIMER_TICK_LOOP_MASTER runs (sounds.dat:
    /// UI_MENUS_STOPWATCH_FIN, the wave `ui/menus/stopwatch_fin`); at 10 s and under, each
    /// time the whole seconds change, RACE_TIMER_SECOND_BEEP_MASTER (UI_MENUS_BEEP_FIN,
    /// `ui/menus/beep_fin`), and when the last update was still above 10 s the player says
    /// PLAYER_TIME_TRIAL_TIMES_UP (F-30989). Otherwise the loop stops (F-31667).
    /// `left`: the time left now (None: no clock counting down).
    pub fn mc4_clock_sounds(&mut self, left: Option<i64>) {
        let d = &mut self.opt_x.arcade.d;
        let prev = std::mem::replace(&mut d.clock_prev, left);
        match (left, prev) {
            (Some(now), Some(before)) if now < 20_000 && now != before && before > 0 => {
                d.tick_loop = true;
                if now <= 10_000 && now / 1000 != before / 1000 {
                    d.sounds.push("ui/menus/beep_fin");
                    if before > 10_000 {
                        log::info!("race clock: 10 s left - PLAYER_TIME_TRIAL_TIMES_UP");
                        d.say.push("PLAYER_TIME_TRIAL_TIMES_UP".to_string());
                    }
                }
            }
            _ => d.tick_loop = false,
        }
    }

    /// Is the race in hand a generic local hookman's own (the one he was flashed for, or a
    /// NEXT RACE of his)?
    pub fn mc4_local_man_race(&self) -> bool {
        let d = &self.opt_x.arcade.d;
        d.race_man.is_some() && d.man_race.is_some() && d.man_race == self.current_race && !self.hookman_race && self.series.is_none() && self.wager.is_none() && self.arcade_restore.is_none()
    }

    /// NEXT RACE was chosen: the next of the man's three (mcRaceArray_FindNextRaceIndex -
    /// `Career::next_race_of`: the first not in the history, else the one run longest ago),
    /// his line, and the count that takes the row away the second time. The race's place
    /// among `race_defs`.
    pub fn mc4_next_race_take(&mut self) -> Option<usize> {
        if !self.mc4_next_race_on() {
            return None;
        }
        let kept: Vec<i64> = self.opt_x.arcade.d.man_races.iter().map(|t| *t as i64).collect();
        let next = match self.mc4.as_ref() {
            Some(m) => m.career.next_race_of(&kept),
            None => kept.first().copied(),
        }? as usize;
        let (i, _) = self.mc4_table_race(next)?;
        let d = &mut self.opt_x.arcade.d;
        d.say.push("GREETING_MEET_ME_THERE_EVENT".to_string());
        d.next_count += 1;
        if d.next_count >= 2 {
            d.next_race = false;
        }
        d.man_race = Some(i);
        self.current_race = Some(i);
        Some(i)
    }
}

// ---- step 16: the body part types the career unlocks, and their "new" marks ----
//
// The career keeps two bytes for every vehicle class and body part type (9 x 38, the
// block's `body_parts_unlocked` / `body_parts_new`): "unlocked" and "new". Both are set by
// one thing only, the reward PartUnlock of a class system's threshold (F-70978 ->
// F-67691, F-10782(.., 1)), for the class of the vehicle the player is in.
// The garage's part list (F-77984, script `garage.` PartMenu) reads them:
//   - opening the list of a part type clears its "new" mark (A-83193..A-29913);
//   - a list whose type is not unlocked keeps its first three parts open when it has more
//     than three, else its first (the stock part) only; the others are closed - item flag
//     16 (A-55738..A-54002, A-87109..A-46767). The door list (menu 16) is never
//     closed, nor is anything with the garage's "all open" byte (+21670) set.
// The area menu that leads to the lists (F-99914, A-71639..A-51632) gives a row
// whose type wears the mark `type` 50 in the movie.

/// The part type a body slot's list belongs to (the garage's menu -> category -> type
/// tables, F-92682 / F-13246); None = a list the flags do not reach.
pub fn slot_part_type(slot: crate::vehicle_model::Slot) -> Option<&'static str> {
    use crate::vehicle_model::Slot;
    Some(match slot {
        Slot::BumperF => "FrontBumper",
        Slot::BumperR => "RearBumper",
        Slot::Skirts => "Skirts",
        Slot::Spoiler => "Spoiler",
        Slot::Seats => "FrontSeat",
        Slot::Hood => "Hood",
        Slot::Exhaust => "Exhaust",
        Slot::Intercooler => "Intercooler",
        Slot::Body => "WideBody",
        _ => return None,
    })
}

/// The same for a section of the vehicle's part file (`XPARTS`): the side exhaust's list
/// goes by the Exhaust type (menu 47 -> category 24); doors, tires and brakes by none.
pub fn section_part_type(section: &str) -> Option<&'static str> {
    Some(match section {
        "headlight" => "Headlight",
        "taillight" => "Taillight",
        "steer_whl" => "SteeringWheel",
        "post_gauge" => "PostGauge",
        "blower" => "Blower",
        "side_exhaust" => "Exhaust",
        "intercooler" => "Intercooler",
        "bike_cowl" => "BikeCowl",
        "bike_tail" => "BikeTail",
        "bike_tank" => "BikeTank",
        _ => return None,
    })
}

/// A row of a part list is open (F-77984): every row of an unlocked type; of a
/// locked one the first three when the list has more than three, else the first.
pub fn part_row_open(locked: bool, row: usize, count: usize) -> bool {
    !locked || if count > 3 { row < 3 } else { row < 1 }
}

/// The garage's meter of an ability (F-91415, A-65412..A-14800): the system's
/// amount, at most MaxAmount, as a percentage of the level it is on - 5 per point up to
/// 20, 5 per point from 20 to 40, then 100.
pub fn ability_meter(amount: i64, max: i64) -> i32 {
    let a = amount.min(max);
    (if a < 20 { a * 5 } else if a < 40 { (a - 20) * 5 } else { 100 }) as i32
}

impl Game {
    /// The class system of the vehicle in the garage.
    fn mc4_garage_class(&self) -> Option<&'static str> {
        let class = self.catalog.get(&self.mc4_car_name(self.current_car)).map(|c| c.class.clone()).unwrap_or_default();
        Game::mc4_class_system(&class)
    }

    /// The list of a part type is locked for the vehicle in the garage: the type's flag
    /// (F-57000) where the class systems are counted; else the port's earlier rule,
    /// races won in the class against the threshold that names the type.
    pub fn mc4_part_type_locked(&self, ptype: Option<&str>) -> bool {
        let Some(ptype) = ptype else { return false };
        if std::env::var("MCLA_UNLOCK_ALL").is_ok() {
            return false;
        }
        let class = self.catalog.get(&self.mc4_car_name(self.current_car)).map(|c| c.class.clone()).unwrap_or_default();
        match self.mc4_garage_class().filter(|_| self.mc4_classes_tracked()) {
            Some(sys) => !self.mc4.as_ref().and_then(|m| m.career.body_parts_unlocked.get(sys)).map_or(false, |v| v.iter().any(|p| p.eq_ignore_ascii_case(ptype))),
            None => {
                let (need, wins) = self.career.part_wins_needed(&class, &ptype.to_ascii_uppercase());
                need > 0 && wins < need
            }
        }
    }

    /// Row `row` of the `count` rows of a part type's list can be chosen.
    pub fn mc4_part_row_open(&self, ptype: Option<&str>, row: usize, count: usize) -> bool {
        part_row_open(self.mc4_part_type_locked(ptype), row, count)
    }

    /// The part type wears the "new" mark for the vehicle in the garage (F-71120).
    pub fn mc4_part_new(&self, ptype: Option<&str>) -> bool {
        let (Some(ptype), Some(sys)) = (ptype, self.mc4_garage_class()) else { return false };
        self.mc4.as_ref().and_then(|m| m.career.body_parts_new.get(sys)).map_or(false, |v| v.iter().any(|p| p.eq_ignore_ascii_case(ptype)))
    }

    /// The list of a part type is opened: its "new" mark goes (F-10782(.., 0)).
    pub fn mc4_part_list_opened(&mut self, ptype: Option<&str>) {
        let (Some(ptype), Some(sys)) = (ptype, self.mc4_garage_class()) else { return };
        if let Some(v) = self.mc4.as_mut().and_then(|m| m.career.body_parts_new.get_mut(sys)) {
            v.retain(|p| !p.eq_ignore_ascii_case(ptype));
        }
    }

    /// A part type's name as the rewards form shows it: the string of the reward's own
    /// name (`mcRewardPartUnlock`'s description is the name, F-23081).
    pub fn mc4_part_type_label(&self, ptype: &str) -> String {
        let s = &self.career.strings;
        let alt = if ptype.eq_ignore_ascii_case("Headlight") { "HeadLight".to_string() } else { ptype.to_ascii_lowercase() };
        [ptype.to_string(), alt, ptype.to_ascii_uppercase()].iter().find_map(|k| s.get(k).map(|v| v.to_string())).filter(|v| !v.contains('<')).unwrap_or_else(|| ptype.to_string()).to_uppercase()
    }

    /// The garage's SPECIAL ABILITIES row of ability `k` (0 ZONE, 1 AGRO, 2 ROAR, 3 EMP):
    /// (stars = level + 1, meter in percent) - F-91415.
    pub fn mc4_ability_panel(&self, k: usize) -> (i32, i32) {
        let k = k.min(3);
        let (sys, stat) = (ABILITY_SYSTEMS[k], ["gZON", "gAGR", "gROR", "gPUL"][k]);
        let amount = self.career.ability_amounts.get(sys).copied().unwrap_or_else(|| self.career.stats.get(stat).copied().unwrap_or(0));
        let max = self.career.exp_tables.system(sys).map_or(40, |d| d.max);
        (self.career.ability_level(sys, stat) as i32 + 1, ability_meter(amount, max))
    }
}

// ---- step 19b: the cruising slot system, the hookmen's cars and their choice of race ----

/// The steps of the performance gap (racer - player) between the difficulties: the career's
/// floats +8472.. (constructor A-13457..A-56187; the scale +8468 is 1): below Easy,
/// Easy, Medium, Hard, Impossible, [unused], above Impossible.
pub const DIFF_STEPS: [f32; 6] = [-0.15, 0.025, 0.15, 0.3, 0.0, 0.45];

/// F-44143: the difficulty a performance gap is: -1 below Easy, 0 Easy, 1 Medium, 2 Hard,
/// 3 Impossible, 5 beyond it (the numbers of mc::LookupDifficulty, F-64420).
pub fn gap_difficulty(gap: f32) -> i32 {
    if gap >= DIFF_STEPS[5] {
        5
    } else if gap >= DIFF_STEPS[3] {
        3
    } else if gap >= DIFF_STEPS[2] {
        2
    } else if gap >= DIFF_STEPS[1] {
        1
    } else if gap >= DIFF_STEPS[0] {
        0
    } else {
        -1
    }
}

/// The band of performance values a racer of difficulty `d` has for a player at `perf`
/// (the head of F-44797 / F-83091).
pub fn diff_window(perf: f32, d: i32) -> (f32, f32) {
    match d {
        -1 => (perf - 1.0, DIFF_STEPS[0] + perf),
        5 => (DIFF_STEPS[5] + perf, perf + 1.0),
        3 => (DIFF_STEPS[3] + perf, DIFF_STEPS[5] + perf),
        // (d 0, 1, 2 and 4 - the steps d and d + 1: F-83091)
        _ => {
            let k = d.clamp(0, 4) as usize;
            (DIFF_STEPS[k] + perf, DIFF_STEPS[k + 1] + perf)
        }
    }
}

/// F-44797: a vehicle whose samples (stock, half, fully upgraded) are `s` can be set up
/// to be of difficulty `d` for the player.
pub fn setup_can_reach(s: [f32; 3], perf: f32, d: i32) -> bool {
    let (lo, hi) = diff_window(perf, d);
    s[2] >= lo && s[0] <= hi
}

/// F-83091: the upgrade percentage ("mod value", 0..100) at which the vehicle sits in
/// the middle of the part of the band it can reach.
pub fn setup_mod_value(s: [f32; 3], perf: f32, d: i32) -> i32 {
    let (lo, hi) = diff_window(perf, d);
    let mut a = lo;
    if lo < s[0] {
        a = if hi > s[0] { s[0] } else { hi };
    }
    let mut b = hi;
    if hi > s[2] {
        b = if lo < s[2] { s[2] } else { lo };
    }
    let target = (b - a) * 0.5 + lo;
    let (pa, va, pb, vb) = if target < s[1] { (0.0, s[0], 50.0, s[1]) } else { (50.0, s[1], 100.0, s[2]) };
    if vb == va {
        return pa as i32;
    }
    let t = target.max(va).min(vb);
    ((t - va) / (vb - va) * (pb - pa) + pa + 0.5) as i32
}

/// F-34010: the vehicle's performance value at a mod value.
pub fn setup_perf(s: [f32; 3], m: i32) -> f32 {
    let (pa, va, pb, vb) = if m < 50 { (0.0, s[0], 50.0, s[1]) } else { (50.0, s[1], 100.0, s[2]) };
    // (the original's fmadds: the product and the sum rounded once)
    ((m as f32 - pa) / (pb - pa)).mul_add(vb - va, va)
}

/// Racer_CanFlashHookman (F-57747 -> F-96972): the hookman is within 30 m, the two
/// speeds are within 25 of each other, and he is ahead - the player's forward axis and the
/// direction to him make more than 0.2. (The routine's last test, a clear line from one
/// vehicle to the other 0.5 m up, is the caller's.)
pub fn can_flash(dist: f32, speed_gap: f32, ahead: f32) -> bool {
    dist <= 30.0 && speed_gap.abs() <= 25.0 && ahead >= 0.2
}

pub const PRIO_FREE: i32 = -1;
pub const PRIO_MISSION: i32 = 1;
pub const PRIO_GENERIC: i32 = 5;
pub const CORE_GENERIC: &str = "Game/CruisingHookmen/generic_local_hookman";

/// A hookman of the slot system (the career's pool of forty, 200 bytes each, manager +456):
/// name +4, set-up +36, core script +100, mod value +168, difficulty +180, district +184.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotHookman {
    pub name: String,
    pub config: String,
    pub car: String,
    pub core: String,
    pub mod_value: i32,
    pub difficulty: i32,
    pub district: usize,
    pub mission: String,
}

/// One cruising slot (84 bytes): hookman +4, priority +8 (-1 free), the script of its loop
/// +16, hidden +80.
#[derive(Clone, Debug, PartialEq)]
pub struct CruisingSlot {
    pub script: String,
    pub default_id: i32,
    pub hookman: Option<SlotHookman>,
    pub priority: i32,
    pub hidden: bool,
}

/// mcSlotSystem (career manager +8596): per district the slots (+44), the thresholds with
/// their difficulty types (+4), the characters with their "used" marks (+84), the lock (+145).
#[derive(Clone, Debug, Default)]
pub struct SlotSystem {
    pub slots: Vec<Vec<CruisingSlot>>,
    pub thresholds: Vec<Vec<(f32, Vec<i32>)>>,
    pub characters: Vec<Vec<(String, bool)>>,
    pub locked: [bool; 5],
    pub rng: u32,
}

fn tune_blocks<'a>(tok: &[&'a str], name: &str) -> Vec<Vec<&'a str>> {
    // (the "Name { ... }" blocks of a tune file, each as its tokens; not those nested in
    // another block of the same name)
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < tok.len() {
        if tok[i] == name && tok[i + 1] == "{" {
            let mut depth = 0;
            let mut j = i + 1;
            while j < tok.len() {
                match tok[j] {
                    "{" => depth += 1,
                    "}" => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            out.push(tok[i + 2..j.min(tok.len())].to_vec());
            i = j;
        }
        i += 1;
    }
    out
}

impl SlotSystem {
    /// F-52390 (`tune/Career/districtSlots.lst` = `0xc72d4489`) and F-85712
    /// (`tune/Career/CarCfg/<district>.lst`: the characters the free slots of a district
    /// draw from).
    pub fn load(data_dir: &str) -> SlotSystem {
        let mut s = SlotSystem { rng: 0x9e37_79b9, ..Default::default() };
        let dir = std::path::Path::new(data_dir).join("tune/career");
        let text = std::fs::read_to_string(dir.join("0xc72d4489")).unwrap_or_default();
        let tok: Vec<&str> = text.split_whitespace().collect();
        let blocks = tune_blocks(&tok, "District");
        let field = |t: &[&str], k: &str| t.iter().position(|x| *x == k).and_then(|i| t.get(i + 1)).map(|v| v.to_string());
        for name in DISTRICTS {
            let b = blocks.iter().find(|b| b.len() > 1 && b[0] == "Name" && b[1].eq_ignore_ascii_case(name)).cloned().unwrap_or_default();
            let slots = tune_blocks(&b, "Slot").iter().map(|t| CruisingSlot { script: field(t, "Script").unwrap_or_default(), default_id: field(t, "Default").and_then(|v| v.parse().ok()).unwrap_or(0), hookman: None, priority: PRIO_FREE, hidden: false }).collect();
            let th = tune_blocks(&b, "Threshold")
                .iter()
                .map(|t| {
                    let types = tune_blocks(t, "Type").iter().filter_map(|x| field(x, "Difficulty")).map(|d| match d.as_str() { "Easy" => 0, "Medium" => 1, "Hard" => 2, "Impossible" => 3, _ => -2 }).collect();
                    (field(t, "Value").and_then(|v| v.parse().ok()).unwrap_or(0.0), types)
                })
                .collect();
            s.slots.push(slots);
            s.thresholds.push(th);
            let file = format!("0x{:08x}", joaat(&format!("{}.lst", name.to_ascii_lowercase())));
            let t = std::fs::read_to_string(dir.join("carcfg").join(file)).unwrap_or_default();
            let ct: Vec<&str> = t.split_whitespace().collect();
            s.characters.push(tune_blocks(&ct, "Character").iter().filter_map(|c| field(c, "Name")).map(|n| (n, false)).collect());
        }
        s
    }

    /// F-90759: the threshold of the district for the player's vehicle - the last whose
    /// value its performance is above (the first when none is).
    pub fn threshold_index(&self, d: usize, perf: f32) -> usize {
        let mut at = 0;
        for (i, t) in self.thresholds.get(d).map_or(&[][..], |v| &v[..]).iter().enumerate() {
            if perf > t.0 {
                at = i;
            }
        }
        at
    }

    /// F-66870 (mcSlotSystem_ToggleCharacterUsed).
    pub fn toggle_character(&mut self, d: usize, name: &str, used: bool) {
        if let Some(c) = self.characters.get_mut(d).and_then(|l| l.iter_mut().find(|c| c.0.eq_ignore_ascii_case(name))) {
            c.1 = used;
        }
    }

    /// F-32224 (mcSlotSystem_FindRandomAvailableCharacter): from a random place of the
    /// list of the district the first character not in use, else the first not in use of the
    /// whole list; with all in use the marks are cleared.
    pub fn random_character(&mut self, d: usize) -> Option<String> {
        let n = self.characters.get(d)?.len();
        if n == 0 {
            return None;
        }
        let r = crate::oppick::career_index(n);
        let l = &self.characters[d];
        if let Some(c) = l.iter().skip(r).find(|c| !c.1).or_else(|| l.iter().find(|c| !c.1)) {
            return Some(c.0.clone());
        }
        self.characters[d].iter_mut().for_each(|c| c.1 = false);
        Some(self.characters[d][r].0.clone())
    }

    fn put(&mut self, d: usize, i: usize, h: SlotHookman, priority: i32) {
        // (F-94535 -> F-40941: the character is marked used and the script of the
        // slot is started for the hookman; a hookman who was there is reset - F-66721:
        // his character is free again, his state 4, his script ends)
        if let Some(old) = self.slots[d][i].hookman.take() {
            self.toggle_character(d, &old.name, false);
        }
        self.toggle_character(d, &h.name, true);
        self.slots[d][i].hookman = Some(h);
        self.slots[d][i].priority = priority;
    }

    /// F-98487, the slot part of mcSlotSystem_RequestSlot: (1) a slot that holds a
    /// hookman of the same name and is not of priority 1; (2) the first free slot; (3) a slot
    /// whose hookman has the same difficulty and a priority number above the asked one;
    /// (4) from the last slot down the first with a priority number above it. False: none.
    pub fn request(&mut self, h: SlotHookman, priority: i32) -> bool {
        let d = h.district;
        let Some(n) = self.slots.get(d).map(|s| s.len()) else { return false };
        let at = (0..n)
            .find(|&i| self.slots[d][i].hookman.as_ref().map_or(false, |o| o.name == h.name) && self.slots[d][i].priority != 1)
            .or_else(|| (0..n).find(|&i| self.slots[d][i].priority == PRIO_FREE))
            .or_else(|| if h.difficulty == -2 { None } else { (0..n).find(|&i| self.slots[d][i].hookman.as_ref().map_or(false, |o| o.difficulty == h.difficulty) && self.slots[d][i].priority > priority) })
            .or_else(|| (0..n).rev().find(|&i| self.slots[d][i].priority > priority));
        match at {
            Some(i) => {
                self.put(d, i, h, priority);
                true
            }
            None => false,
        }
    }

    /// F-80442 (mcSlotSystem_RequestSlotRemoval) without its closing evaluation: the slot
    /// that holds the hookman of the mission is emptied. The district when there was one.
    pub fn remove_mission(&mut self, mission: &str) -> Option<usize> {
        for d in 0..self.slots.len() {
            for i in 0..self.slots[d].len() {
                if self.slots[d][i].hookman.as_ref().map_or(false, |h| h.mission.eq_ignore_ascii_case(mission)) {
                    let h = self.slots[d][i].hookman.take().unwrap();
                    self.toggle_character(d, &h.name, false);
                    self.slots[d][i].priority = PRIO_FREE;
                    return Some(d);
                }
            }
        }
        None
    }

    /// F-92355 (the evaluation of a district): nothing in a locked district. For each
    /// difficulty type of the threshold the player is at, in the order of the file: a slot
    /// that already holds a hookman of that difficulty answers for it (each slot once); else
    /// the first free slot gets a generic hookman - a random free character of the district
    /// in the car `car_of` gives for (character, difficulty) -, priority 5; with no free slot
    /// the evaluation ends.
    pub fn evaluate(&mut self, d: usize, perf: f32, car_of: &mut dyn FnMut(&str, i32) -> Option<(String, String, i32)>) {
        if d >= self.slots.len() || self.locked[d.min(4)] {
            return;
        }
        let n = self.slots[d].len();
        let types = self.thresholds[d].get(self.threshold_index(d, perf)).map(|t| t.1.clone()).unwrap_or_default();
        let mut taken = vec![false; n];
        let have: Vec<Option<i32>> = self.slots[d].iter().map(|s| if s.priority == PRIO_FREE { None } else { s.hookman.as_ref().map(|h| h.difficulty) }).collect();
        for t in types {
            if let Some(i) = (0..n).find(|&i| !taken[i] && have[i] == Some(t) && t != -2) {
                taken[i] = true;
                continue;
            }
            let Some(i) = (0..n).find(|&i| self.slots[d][i].priority == PRIO_FREE) else { break };
            let Some(name) = self.random_character(d) else { break };
            // (the set-up of the man's car is a random one of the car's configurations (F-18603, one more career step, no NULL
            // test); the disc gives every car exactly one, so the draw changes the generator and nothing else)
            let _ = crate::oppick::career_index(1);
            let Some((car, config, mod_value)) = car_of(&name, t) else { continue };
            self.put(d, i, SlotHookman { name, config, car, core: CORE_GENERIC.to_string(), mod_value, difficulty: t, district: d, mission: String::new() }, PRIO_GENERIC);
        }
    }

    /// F-11072 over the unlocked districts (mcSlotSystem_ClearAllDistricts): every
    /// hookman whose slot is not of priority 1 is reset. (His script then gives the slot back,
    /// generic_local_hookman.sco 0x2c..0x48; the port frees it here.)
    pub fn clear_all(&mut self) {
        for d in 0..self.slots.len() {
            if self.locked[d.min(4)] {
                continue;
            }
            for i in 0..self.slots[d].len() {
                if self.slots[d][i].priority != 1 {
                    if let Some(h) = self.slots[d][i].hookman.take() {
                        self.toggle_character(d, &h.name, false);
                    }
                    self.slots[d][i].priority = PRIO_FREE;
                }
            }
        }
    }

    /// F-58249 (mcSlotSystem_HideDistrict).
    pub fn hide_district(&mut self, d: usize, hide: bool) {
        if let Some(l) = self.slots.get_mut(d) {
            l.iter_mut().for_each(|s| s.hidden = hide);
        }
    }

    /// F-82555 (mcSlotSystem_UnlockSlot): the slot that holds the hookman is shown.
    pub fn unlock_slot(&mut self, d: usize, name: &str) {
        if let Some(s) = self.slots.get_mut(d).and_then(|l| l.iter_mut().find(|s| s.hookman.as_ref().map_or(false, |h| h.name == name))) {
            s.hidden = false;
        }
    }
}

/// mcRaceArray_LocalHookmanFilter (F-15609): the `n` races a local hookman keeps - the
/// race he is going to, then, in the order of the array (it was sorted by distance), the
/// races that are not in the history; when those do not fill the list, the ones run longest
/// ago.
pub fn local_hookman_filter(sorted: &[usize], first: usize, n: usize, history: &dyn Fn(usize) -> Option<usize>) -> Vec<usize> {
    let mut fresh: Vec<usize> = vec![first];
    let mut old: Vec<(usize, usize)> = Vec::new();
    for &r in sorted {
        if r == first {
            continue;
        }
        match history(r) {
            None => {
                fresh.push(r);
                if fresh.len() == n {
                    break;
                }
            }
            Some(p) => {
                let at = old.iter().position(|o| o.1 > p).unwrap_or(old.len());
                old.insert(at, (r, p));
            }
        }
    }
    fresh.extend(old.iter().map(|o| o.0));
    fresh.truncate(n);
    fresh
}

pub const DISTRICT_CODES: [&str; 5] = ["hw", "be", "hl", "dt", "sc"];

/// The mission a district's tournament man looks at (generic_tournament_hookman's static +460),
/// written as a constant by `<code>_tournament_hk1` before it starts him: hw `0xddd89a62`, be
/// `0x40c110a8`, hl `0x1f2efed7`, dt `0x2cb25834`, sc `sc/CruisingHookmen/0x5025ae01`.
/// The beat-me-there sort's comparator (F-33494, on the float at +8): 0 when equal, 1 when `a` is
/// greater, else -1 (also for NaN) - lowest first.
pub fn bmt_order(a: f32, b: f32) -> i32 {
    if a == b {
        0
    } else if a > b {
        1
    } else {
        -1
    }
}

pub const TOURNAMENT_MISSIONS: [&str; 5] = ["TOURNAMENT_HOLLYWOOD", "TOURNAMENT_BEACHES", "TOURNAMENT_HILLS", "TOURNAMENT_DOWNTOWN", "TOURNAMENT_SC"];

impl Game {
    /// The "hidden" mark of a cruising slot (slot +80; the save's `slots_hidden`): set for a
    /// district's slots by mcSlotSystem_HideDistrict, cleared when its hookman is found
    /// (mcSlotSystem_UnlockSlot). Without the block: hidden while the district is locked.
    pub fn mc4_slot_hidden(&self, d: usize, i: usize) -> bool {
        match self.mc4.as_ref().and_then(|m| m.career.cruising_slots.get(d)) {
            Some(s) => s.slots_hidden.get(i).copied().unwrap_or(false),
            None => !self.mc4_district_open(DISTRICTS[d.min(4)]),
        }
    }

    /// The hookman of a hidden slot has been found (mcSlotSystem_UnlockSlot, F-82555): the
    /// mark of the slot is cleared in the block.
    pub fn mc4_slot_found(&mut self, d: usize, i: usize) {
        if let Some(h) = self.mc4.as_mut().and_then(|m| m.career.cruising_slots.get_mut(d)).and_then(|s| s.slots_hidden.get_mut(i)) {
            *h = false;
        }
    }

    /// The three samples of a vehicle (stock, half, fully upgraded).
    pub fn mc4_samples_of(&self, car: &str) -> Option<[f32; 3]> {
        Some([self.samples.value(car, 0.0)?, self.samples.value(car, 0.5)?, self.samples.value(car, 1.0)?])
    }

    /// mcCharacterConfigData_GetCarConfigData (F-70276) with the class mask 247 - every
    /// class but the motorcycles - as the slot system and the scripts of the hookmen call it:
    /// of the cars of the character the first that can be set up to the difficulty for the
    /// vehicle of the player; when none can, the one whose difficulty at its best mod value
    /// is nearest. (vehicle, set-up, mod value.) The original walks the cars in the order of
    /// a hash table; the port in the order of the file.
    pub fn mc4_character_car(&self, character: &str, diff: i32) -> Option<(String, String, i32)> {
        let perf = self.player_perf();
        let ch = self.rivals.characters.iter().find(|c| c.id.eq_ignore_ascii_case(character))?;
        let mut best: Option<(usize, i32, i32)> = None;
        for (k, c) in ch.cars.iter().enumerate() {
            if c.car.contains("_duc_") || c.car.contains("_kaw_") {
                continue;
            }
            let Some(s) = self.mc4_samples_of(&c.car) else { continue };
            let m = setup_mod_value(s, perf, diff);
            if setup_can_reach(s, perf, diff) {
                return Some((c.car.clone(), c.config.clone(), m));
            }
            let off = (gap_difficulty(setup_perf(s, m) - perf) - diff).abs();
            if best.map_or(true, |b| off < b.2) {
                best = Some((k, m, off));
            }
        }
        best.map(|(k, m, _)| (ch.cars[k].car.clone(), ch.cars[k].config.clone(), m))
    }

    /// The hookman record mcSlotSystem_RequestSlot (F-25974) makes for a mission: the
    /// vehicle of the set-up is the name after `_vp_` / `_vpd_`; its mod value the one of
    /// the difficulty asked (F-83091); his difficulty the gap at that mod value - Easy
    /// and below count as Medium, beyond Impossible as Impossible (A-10257..A-22231).
    pub fn mc4_mission_hookman(&self, row: &(&str, usize, &str, &str, i32, i32, &str)) -> SlotHookman {
        let (mission, district, name, config, _, level, core) = *row;
        let low = config.to_ascii_lowercase();
        let car = low.find("_vp_").or_else(|| low.find("_vpd_")).map(|i| low[i + 1..].to_string()).unwrap_or_default();
        let perf = self.player_perf();
        let (mod_value, difficulty) = match self.mc4_samples_of(&car) {
            Some(s) => {
                let m = setup_mod_value(s, perf, level);
                (m, match gap_difficulty(setup_perf(s, m) - perf) { d if d <= 0 => 1, 5 => 3, d => d })
            }
            None => (0, -2),
        };
        SlotHookman { name: name.to_string(), config: config.to_string(), car, core: core.to_string(), mod_value, difficulty, district, mission: mission.to_string() }
    }

    /// The difficulty word (+180) a story man's core script gives him as it starts (the race's
    /// +3100 and his icon's colour), docs/reference/ai/race_difficulty.md 1.4. The native
    /// mcCareer_GetDifficultyPlayerVsRacer (F-69029) = his car at his mod value against the
    /// player's: below Easy and Easy both 0, the fifth step 3. Most cores wrap it, `x == 0 ? 1 : x`
    /// (so they are never Easy). The FIND_* cores and RACE_SH_01 take it raw (RACE_SH_01's race
    /// works it out again at its start, 0xa3f, raw into +3100); FIND_SH_01 and RACE_GH_02 set it
    /// raw too, but then start CruisingHookmanCore, which wraps it for a type-0 man as it starts
    /// (0x596..0x5db) - RACE_GH_02's race reads mcHookman_GetDifficulty after that (0xd20). The
    /// other raw cores start no CruisingHookmanCore. RACE_GH_06 sets 0 (`0x40a56c35` 0x1b7; his
    /// type is 1, no wrap), FIND_SH_05 / BEAT_SH_05 1 (0x3b); CITY_CHAMP and the two BESTOF_SH_01
    /// keep mcSlotSystem_RequestSlot's (F-25974: at most Easy -> Medium; generic_local_hookman
    /// takes it, 0x34f, before it starts CruisingHookmanCore, 0x398). None: not a story man of
    /// `MISSION_HOOKMEN`.
    pub fn mc4_story_difficulty(&self, mission: &str) -> Option<usize> {
        match mission {
            "RACE_GH_06" => return Some(0),
            "FIND_SH_05" | "BEAT_SH_05" => return Some(1),
            _ => {}
        }
        let row = MISSION_HOOKMEN.iter().find(|r| r.0 == mission && !r.2.is_empty())?;
        let h = self.mc4_mission_hookman(row);
        if matches!(mission, "CITY_CHAMP" | "BESTOF_SH_01_MUSCLE" | "BESTOF_SH_01_TUNER") {
            return (h.difficulty >= 0).then_some(h.difficulty as usize);
        }
        let s = self.mc4_samples_of(&h.car)?;
        let native = match gap_difficulty(setup_perf(s, h.mod_value) - self.player_perf()) {
            d if d < 0 => 0,
            5 => 3,
            d => d as usize,
        };
        let raw = (mission.starts_with("FIND_") && mission != "FIND_SH_01") || mission == "RACE_SH_01";
        Some(if raw || native != 0 { native } else { 1 })
    }

    /// The difficulty of the hookman out on the road now (`mc4_story_difficulty`, else the
    /// set-up named by his event).
    pub fn mc4_hookman_diff(&self) -> Option<usize> {
        let m = self.career.hookman_mission()?;
        self.mc4_story_difficulty(m).or_else(|| self.career.hookman_diff())
    }

    /// The race definition of a race of the table, and where it starts (the start beacon:
    /// RaceDescription_GetStartBeaconPosition).
    pub fn mc4_table_race(&self, table_index: usize) -> Option<(usize, glam::Vec3)> {
        let name = self.race_table.get(table_index)?;
        let i = self.race_defs.iter().position(|d| d.id.eq_ignore_ascii_case(name))?;
        let d = &self.race_defs[i];
        Some((i, if d.beacon.length_squared() > 1.0 { d.beacon } else { self.race_start(i)? }))
    }

    /// mcRaceArray_Fill (F-50395): the races of the table - in its order - of the
    /// district (6 = all), the type mask and the sub type mask that are unlocked and are not
    /// test-only. (With none the original puts one random race in; the port leaves the array
    /// empty.)
    pub fn mc4_race_array(&self, district: i32, types: i32, subs: i32) -> Vec<usize> {
        (0..self.race_table.len())
            .filter(|&k| race_passes(self.mc4_race_record(&self.race_table[k]), district, types, subs))
            .filter(|&k| self.mc4_table_race(k).map_or(false, |(i, _)| !self.race_defs[i].dev_only && self.mc4_race_unlocked(&self.race_defs[i].id)))
            .collect()
    }

    /// The district argument the local hookman gives mcRaceArray_Fill
    /// (generic_local_hookman.sco 0x5ae..0x7b8): all (6); the Beaches (1) while RACES_IN_BEACHES
    /// is in state 3, else Downtown (3) while RACES_IN_DOWNTOWN is, else South Central (4)
    /// while RACES_IN_SC is. (State 3 is taken as "under way" - the "active" of the port.)
    pub fn mc4_local_district(&self) -> i32 {
        [("RACES_IN_BEACHES", 1), ("RACES_IN_DOWNTOWN", 3), ("RACES_IN_SC", 4)].iter().find(|(m, _)| self.career.mission_active(m)).map_or(6, |x| x.1)
    }

    fn mc4_history_at(&self, table_index: usize) -> Option<usize> {
        self.mc4.as_ref().and_then(|m| m.career.history_position(table_index as i64))
    }

    /// mcRaceArray_FindBeatMeThereRaceIndex (F-73402): of the races of the array in the
    /// district of the racer (6: any), sorted by the dot product of row c of his matrix
    /// (+32 - a vehicle drives along -c, so the smallest is the race most straight ahead)
    /// and the direction to the race, lowest first: among those more than `min_dist` away
    /// (on the map, x / z) the first that is not in the history, else the one run longest
    /// ago. None further away: a random one (`rnd`); the index is taken of the candidates of
    /// the district and used on the whole array (A-76493..A-63524), as here.
    pub fn mc4_bmt_race(&self, arr: &[usize], district: i32, pos: glam::Vec3, forward: glam::Vec3, min_dist: f32, rnd: u32) -> Option<usize> {
        if arr.is_empty() {
            return None;
        }
        let back = -forward.normalize_or_zero();
        let mut cands: Vec<(usize, f32, f32)> = Vec::new();
        for (k, &t) in arr.iter().enumerate() {
            if district != 6 && self.mc4_race_record(&self.race_table[t]).0 != district {
                continue;
            }
            let Some((_, p)) = self.mc4_table_race(t) else { continue };
            let flat = ((pos.x - p.x).powi(2) + (pos.z - p.z).powi(2)).sqrt();
            cands.push((k, flat, back.dot((p - pos).normalize_or_zero())));
        }
        cands.sort_by(|a, b| bmt_order(a.2, b.2).cmp(&0));
        let mut best: Option<(usize, usize)> = None;
        for c in cands.iter().filter(|c| c.1 > min_dist) {
            match self.mc4_history_at(arr[c.0]) {
                None => return Some(arr[c.0]),
                Some(p) => {
                    if best.map_or(true, |b| p < b.1) {
                        best = Some((c.0, p));
                    }
                }
            }
        }
        match best {
            Some((k, _)) => Some(arr[k]),
            None if !cands.is_empty() => Some(arr[rnd as usize % cands.len()]),
            None => Some(arr[rnd as usize % arr.len()]),
        }
    }

    /// The race a local hookman who has been flashed goes to, and the three he keeps
    /// (generic_local_hookman.sco 0x73d..0x853): the array is filled with the ordered-type
    /// races (type mask 2) of the sub types Ordered and Circuit (mask 6); the first race is
    /// the beat-me-there pick in the district the hookman is in, 400 m; then the array is
    /// sorted by the distance from him (mcRaceArray_SortByDistance, F-25597: on the map,
    /// nearest first) and cut to three (mcRaceArray_LocalHookmanFilter).
    pub fn mc4_local_races(&self, pos: glam::Vec3, forward: glam::Vec3, hookman_district: i32, rnd: u32) -> Vec<usize> {
        let arr = self.mc4_race_array(self.mc4_local_district(), 2, 6);
        let Some(first) = self.mc4_bmt_race(&arr, hookman_district, pos, forward, 400.0, rnd) else { return Vec::new() };
        let mut sorted: Vec<(usize, f32)> = arr.iter().filter_map(|&t| self.mc4_table_race(t).map(|(_, p)| (t, ((pos.x - p.x).powi(2) + (pos.z - p.z).powi(2)).sqrt()))).collect();
        sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let order: Vec<usize> = sorted.iter().map(|s| s.0).collect();
        local_hookman_filter(&order, first, 3, &|t| self.mc4_history_at(t))
    }

    /// The tournament the man of a district offers (`<d>_tournament_hk1`, hookman ids 200..):
    /// he is out once a mission that starts his launcher has started (the table of step
    /// 17b); the key of the event.
    pub fn mc4_tournament_man(&self, d: usize) -> Option<String> {
        let code = DISTRICT_CODES[d.min(4)];
        let launcher = format!("{code}_tournament_hk1");
        if !MISSION_LAUNCHERS.iter().any(|(m, l)| *l == launcher && self.career.mission_started(m)) {
            return None;
        }
        let key = format!("{code}_tournament1");
        (!self.career.event_races(&key).is_empty()).then_some(key)
    }

    /// The tournament man's difficulty and his icon's colour (`game/CruisingHookmen/0xdcd7fe26`
    /// 0x115..0x17e: Racer_SetIconColorByDifficulty, mcHookman_SetDifficulty) - also the start of
    /// his panel (form 66 +1896, 0x4e3): 1 while his mission (static +460) exists and is not
    /// COMPLETE, else 2. +460 is a constant of the district's launcher script, whichever mission
    /// started it (TOURNAMENT_<district>, its B, REPEAT_TOURNAMENT_<district>):
    /// `TOURNAMENT_MISSIONS`.
    pub fn mc4_tournament_man_diff(&self, d: usize) -> i32 {
        let m = TOURNAMENT_MISSIONS[d.min(4)];
        if self.career.mission_no(m).is_some() && !self.career.mission_complete(m) { 1 } else { 2 }
    }

    /// The man of a tournament has been flashed: his panel (form 66) opens over the game for
    /// the race the tournament is at; ACCEPT starts it (`mc4_event_panel_accept`).
    pub fn mc4_tournament_open(&mut self, key: &str) -> bool {
        let races = self.career.event_races(key);
        let at = races.iter().filter_map(|r| self.career.tournament_of(r)).map(|t| t.1).next().unwrap_or(0).min(races.len().saturating_sub(1));
        let Some(race) = races.get(at).map(|r| r.to_ascii_lowercase()) else { return false };
        let Some(i) = self.race_defs.iter().position(|d| d.id.to_ascii_lowercase().ends_with(&race)) else { return false };
        if self.flash_menu.is_none() {
            return false;
        }
        self.current_race = Some(i);
        // (opens on the man's own difficulty: MEDIUM while his mission is open, else HARD -
        // mcTournamentForm +1896 from the script, generic_tournament_hookman 0x4e3)
        self.hangout_diff = self.mc4_tournament_panel_diff(key);
        self.hangout_key = Some(key.to_string());
        self.hangout_page = (0, 0);
        self.mode = crate::game::Mode::RaceSelect;
        true
    }

    /// The tournament panel's start value for an event (`hw_tournament1` ...): the script's L5,
    /// set into the form with PropertyCtrl_SetPropertyInt(form, 0, L5) (generic_tournament_hookman
    /// 0x115..0x148, 0x4e3): the man's own difficulty, `mc4_tournament_man_diff` - 1 while his
    /// district's TOURNAMENT_<district> (static +460, the launcher script's constant) is there
    /// and not COMPLETE, else 2; 2 for a key of no district.
    pub fn mc4_tournament_panel_diff(&self, key: &str) -> usize {
        let k = key.to_ascii_lowercase();
        DISTRICT_CODES.iter().position(|c| k.starts_with(&format!("{c}_"))).map_or(2, |d| self.mc4_tournament_man_diff(d) as usize)
    }

    /// Whether an event's panel shows its DIFFICULTY row - the forms' refresh, Ctrl_SetVisible
    /// of the combo: the tournament's only when it opened on HARD, its mission COMPLETE
    /// (mcTournamentForm F-10216); a series' after one MEDIUM completion of its event
    /// (event+36 > 0, mcLocalHangoutForm F-48938); the delivery's after more than four
    /// MEDIUM deliveries (event+36 > 4, mcDeliveryForm F-17823); the payback's and the
    /// wager's always. Hidden, the choice cannot change.
    pub fn mc4_panel_diff_shown(&self, key: &str) -> bool {
        if key.contains("delivery") {
            self.mc4_event_completions(key, 1) > 4
        } else if key.contains("_tournament") {
            self.mc4_tournament_panel_diff(key) == 2
        } else if key == "be_payback" || key.contains("_wager_") {
            true
        } else {
            self.mc4_event_completions(&format!("{}_series", key), 1) > 0
        }
    }

    /// The time trial man has been flashed: form 65 opens over the game.
    pub fn mc4_tt_panel_open(&mut self, d: usize) -> bool {
        if self.flash_menu.is_none() {
            return false;
        }
        // (the panel is filled: the first visit is noted - generic_timetrial_hookman.sco
        // 0x72b..0x74d, `tTTA` with the id of the hookman, mode 3)
        self.career.ostats.insert(format!("tTTA/{}", 250 + d.min(4)), 1.0);
        self.hangout_key = Some(format!("timetrial_{}", DISTRICT_CODES[d.min(4)]));
        self.hangout_page = (0, 0);
        self.mode = crate::game::Mode::RaceSelect;
        true
    }

    /// The district of a time trial panel key (`timetrial_hw` ..).
    pub fn mc4_tt_panel_district(key: &str) -> Option<usize> {
        DISTRICT_CODES.iter().position(|c| key.eq_ignore_ascii_case(&format!("timetrial_{c}")))
    }

    /// ACCEPT on form 65 with row `row` (SimpleListForm_GetSelectedIndex, 0x92a): the race
    /// of the district is set as the time trial with the vehicle of that row. False: the row
    /// is not enabled.
    pub fn mc4_tt_panel_accept(&mut self, key: &str, row: usize) -> bool {
        let Some(d) = Self::mc4_tt_panel_district(key) else { return false };
        if !self.mc4_tt_rows(d).get(row).map_or(false, |r| r.enabled) {
            return false;
        }
        let Some(i) = self.race_defs.iter().position(|r| r.id.eq_ignore_ascii_case(TIME_TRIALS[d].race)) else { return false };
        self.career.tt_pick = row;
        self.current_race = Some(i);
        self.career.panel_ok = true;
        true
    }
}

/// (mission, easy layout): the story cores whose race record's +3112 is a constant -
/// RACE_GH_02_CORE 0 (0xf40), RACE_GH_06_CORE 0, RACE_SH_01_CORE 1 (0xa4f), BEAT_SH_04B_CORE,
/// BEAT_SH_04_BCLASS_CORE, BEAT_GH_04_MUSCLE_CORE, BEAT_GH_05_TUNER_CORE 1 (e.g. `0x7efbea76`
/// 0xa44); docs/reference/ai/race_difficulty.md 1.4.
pub const STORY_LAYOUT: [(&str, bool); 7] = [
    ("RACE_GH_02", true),
    ("RACE_GH_06", true),
    ("RACE_SH_01", false),
    ("BEAT_SH_04B", false),
    ("BEAT_SH_04_BCLASS", false),
    ("BEAT_GH_04_MUSCLE", false),
    ("BEAT_GH_05_TUNER", false),
];

/// The objective a mission's script points at its man (mcRequirement_SetTarget(mcMission_GetObjective(m, k),
/// mcHookman_GetId(h)) in `game/career/missions/*.sco`): objective 0 in all 53 scripts but BESTOF_SH_05 (1)
/// and DISTRICT_CHAMPS (GH_20 0x101 objective 0, GH_11 0x21d 1, GH_10 0x339 2).
pub fn target_objective(mission: &str, man: &str) -> usize {
    match (mission, man) {
        ("BESTOF_SH_05", _) => 1,
        ("DISTRICT_CHAMPS", "GH_11") => 1,
        ("DISTRICT_CHAMPS", "GH_10") => 2,
        _ => 0,
    }
}

/// (mission, district, character, set-up, priority, the difficulty asked of the set-up, core script)
pub const MISSION_HOOKMEN: &[(&str, usize, &str, &str, i32, i32, &str)] = &[
    ("BEAT_GH_04_MUSCLE", 0, "GH_04", "GH_04_1_vp_chv_camaross_69", 1, 2, "Game/Career/Story/BEAT_GH_04_MUSCLE_CORE"),
    ("BEAT_GH_05_TUNER", 0, "GH_05", "GH_05_0_vp_frd_focus_04", 1, 2, "Game/Career/Story/BEAT_GH_05_TUNER_CORE"),
    ("BEAT_SH_02_BCLASS", 1, "SH_02", "SH_02_2_vp_dge_challenger_06", 1, 2, "Game/Career/Story/BEAT_SH_02_BCLASS_CORE"),
    ("BEAT_SH_02_CCLASS", 0, "SH_02", "SH_02_1_vp_frd_boss302_69", 1, 2, "Game/Career/Story/BEAT_SH_02_CCLASS_CORE"),
    ("BEAT_SH_02_DCLASS", 0, "SH_02", "SH_02_0_vp_dsn_280z_75", 1, 2, "Game/Career/Story/BEAT_SH_02_DCLASS_CORE"),
    ("BEAT_SH_02_EXOTIC", 1, "SH_02", "SH_02_3_vp_lam_miura_71", 1, 2, "Game/Career/Story/BEAT_SH_02_EXOTIC_CORE"),
    ("BEAT_SH_03", 1, "SH_03", "SH_03_2_vp_aud_rs4_07", 1, 2, "Game/Career/Story/BEAT_SH_03_CORE"),
    ("BEAT_SH_03_ACLASS", 1, "SH_03", "SH_03_3_vp_frd_cobra_04", 1, 2, "Game/Career/Story/BEAT_SH_03_ACLASS_CORE"),
    ("BEAT_SH_03_CCLASS", 1, "SH_03", "SH_03_1_vp_pon_solstice_06", 1, 2, "Game/Career/Story/BEAT_SH_03_CCLASS_CORE"),
    ("BEAT_SH_04", 3, "SH_04", "SH_04_1_vp_mzd_rx8_06", 1, 2, "Game/Career/Story/BEAT_SH_04_CORE"),
    ("BEAT_SH_04B", 3, "SH_01", "SH_01_3_vp_mbz_s600_07", 1, 2, "Game/Career/Story/BEAT_SH_04B_CORE"),
    ("BEAT_SH_04_BCLASS", 3, "SH_04", "SH_04_2_vp_mzd_rx7_95", 1, 2, "Game/Career/Story/BEAT_SH_04_BCLASS_CORE"),
    ("BEAT_SH_04_MOTORCYCLE", 2, "SH_04", "SH_04_3_vp_duc_999r_06", 1, 2, "Game/Career/Story/BEAT_SH_04_MOTORCYCLE_CORE"),
    ("BEAT_SH_05", 3, "SH_05", "SH_05_3_vp_frd_gt_06", 1, 2, "Game/Career/Story/BEAT_SH_05_CORE"),
    ("BEAT_SH_05_ACLASS", 3, "SH_05", "SH_05_3_vp_frd_gt_06", 1, 2, "Game/Career/Story/BEAT_SH_05_ACLASS_CORE"),
    ("BESTOF_GH_04", 0, "GH_04", "GH_04_1_vp_chv_camaross_69", 1, 2, "Game/Career/Story/BESTOF_GH_04_CORE"),
    ("BESTOF_GH_05", 0, "GH_05", "GH_05_0_vp_frd_focus_04", 1, 2, "Game/Career/Story/BESTOF_GH_05_CORE"),
    ("BESTOF_GH_22", 3, "GH_22", "GH_22_3_vp_lam_gallardo_06", 1, 2, "Game/Career/Story/BESTOF_GH_22_CORE"),
    ("BESTOF_SH_01_MUSCLE", 0, "SH_01", "SH_01_2_vp_dge_charger_06", 1, 2, "Game/CruisingHookmen/generic_local_hookman"),
    ("BESTOF_SH_01_TUNER", 0, "SH_01", "SH_01_1_vp_mit_eclipse_99", 1, 2, "Game/CruisingHookmen/generic_local_hookman"),
    ("BESTOF_SH_02", 1, "SH_02", "SH_02_2_vp_dge_challenger_06", 1, 2, "Game/Career/Story/BESTOF_SH_02_CORE"),
    ("BESTOF_SH_03", 0, "SH_03", "SH_03_1_vp_pon_solstice_06", 1, 2, "Game/Career/Story/BESTOF_SH_03_CORE"),
    ("BESTOF_SH_03_LUXURY", 0, "SH_03", "SH_03_2_vp_cry_300c_06", 1, 2, "Game/Career/Story/BESTOF_SH_03_LUXURY_CORE"),
    ("BESTOF_SH_05", 3, "SH_05", "SH_05_2_vp_vw_golf_07", 1, 2, "Game/Career/Story/BESTOF_SH_05_CORE"),
    ("CITY_CHAMP", 3, "SH_01", "SH_01_3_vp_mbz_s600_07", 1, 2, "Game/Career/Story/CITY_CHAMP_CORE"),
    ("DISTRICT_CHAMPS", 3, "GH_20", "GH_20_3_vp_lam_miura_71", 1, 2, "Game/Career/Story/DISTRICT_CHAMPS_DOWNTOWN_CORE"),
    ("DISTRICT_CHAMPS", 2, "GH_11", "GH_11_3_vp_lam_murcielago_06", 1, 2, "Game/Career/Story/DISTRICT_CHAMPS_HILLS_CORE"),
    ("DISTRICT_CHAMPS", 1, "GH_10", "GH_10_3_vp_lam_gallardo_06", 1, 2, "Game/Career/Story/DISTRICT_CHAMPS_BEACHES_CORE"),
    ("DISTRICT_CHAMPS_BEACHES", 1, "GH_10", "GH_10_3_vp_lam_gallardo_06", 1, 2, "Game/Career/Story/DISTRICT_CHAMPS_BEACHES_CORE"),
    ("DISTRICT_CHAMPS_DOWNTOWN", 3, "GH_20", "GH_20_3_vp_lam_miura_71", 1, 2, "Game/Career/Story/DISTRICT_CHAMPS_DOWNTOWN_CORE"),
    ("DISTRICT_CHAMPS_HILLS", 2, "GH_11", "GH_11_3_vp_lam_murcielago_06", 1, 2, "Game/Career/Story/DISTRICT_CHAMPS_HILLS_CORE"),
    ("EXOTIC_CHAMP", 0, "GH_03", "GH_03_3_vp_frd_gt_06", 1, 2, "Game/Career/Story/EXOTIC_CHAMP_CORE"),
    ("FIND_SH_01", 3, "SH_01", "SH_01_3_vp_mbz_s600_07", 1, 2, "Game/Career/Story/FIND_SH_01_CORE"),
    ("FIND_SH_01B", 3, "SH_01", "SH_01_3_vp_mbz_s600_07", 1, 2, "Game/Career/Story/FIND_SH_01B_CORE"),
    ("FIND_SH_02", 1, "SH_02", "SH_02_2_vp_dge_challenger_06", 1, 2, "Game/Career/Story/FIND_SH_02_CORE"),
    ("FIND_SH_02B", 1, "SH_02", "SH_02_3_vp_lam_miura_71", 1, 2, "Game/Career/Story/FIND_SH_02B_CORE"),
    ("FIND_SH_02C", 3, "SH_02", "SH_02_3_vp_lam_miura_71", 1, 2, "Game/Career/Story/FIND_SH_02C_CORE"),
    ("FIND_SH_03", 0, "SH_03", "SH_03_2_vp_cry_300c_06", 1, 2, "Game/Career/Story/FIND_SH_03_CORE"),
    ("FIND_SH_03B", 1, "SH_03", "SH_03_1_vp_pon_solstice_06", 1, 2, "Game/Career/Story/FIND_SH_03B_CORE"),
    ("LUXURY_CHAMP", 3, "GH_07", "GH_07_3_vp_mbz_sl65_07", 1, 2, "Game/Career/Story/LUXURY_CHAMP_CORE"),
    ("MOTORCYCLE_CHAMP", 2, "GH_23", "GH_23_4_vp_kaw_ninja_zx14_06", 1, 2, "Game/Career/Story/MOTORCYCLE_CHAMP_CORE"),
    ("MUSCLE_CHAMP", 1, "GH_05", "GH_05_2_vp_chv_camaro_06", 1, 2, "Game/Career/Story/MUSCLE_CHAMP_CORE"),
    ("RACE_GH_02", 0, "GH_02", "GH_02_0_vp_dsn_280z_75", 1, 1, "Game/Career/Story/RACE_GH_02_CORE"),
    ("RACE_GH_06", 0, "GH_06", "GH_06_0_vp_dsn_280z_75", 1, 0, "Game/Career/Story/RACE_GH_06_CORE"),
    ("RACE_SH_01", 0, "SH_01", "SH_01_0_vp_nsn_240sx_98", 1, 2, "Game/Career/Story/RACE_SH_01_CORE"),
    ("RACE_SH_04_MOTORCYCLE", 2, "SH_04", "SH_04_3_vp_duc_999r_06", 1, 2, "Game/Career/Story/RACE_SH_04_MOTORCYCLE_CORE"),
    ("REPEAT_BESTOF_BEACHES", 1, "", "", 1, 1, "Game/Career/Story/REPEAT_BESTOF_BEACHES_CORE"),
    ("REPEAT_BESTOF_DOWNTOWN", 3, "", "", 1, 1, "Game/Career/Story/REPEAT_BESTOF_DOWNTOWN_CORE"),
    ("REPEAT_BESTOF_HILLS", 2, "", "", 1, 1, "Game/Career/Story/REPEAT_BESTOF_HILLS_CORE"),
    ("REPEAT_BESTOF_HOLLYWOOD", 0, "", "", 1, 1, "Game/Career/Story/REPEAT_BESTOF_HOLLYWOOD_CORE"),
    ("TUNER_CHAMP", 0, "GH_16", "GH_16_2_vp_mit_evo_06", 1, 2, "Game/Career/Story/TUNER_CHAMP_CORE"),
    ("BEAT_GH_26_SUV", 4, "GH_26", "SC_GH_26_3_vpd_lr_hse_08", 1, 2, "sc/Career/Story/BEAT_GH_26_SUV_CORE"),
    ("BEAT_SH_06_BCLASS", 4, "SH_06", "SH_06_1_vp_frd_boss302_69", 1, 2, "sc/Career/Story/BEAT_SH_06_BCLASS_CORE"),
    ("BEAT_SH_06_SUV", 4, "SH_02", "SH_02_2_vp_dge_challenger_06", 1, 2, "sc/Career/Story/BEAT_SH_06_SUV_CORE"),
    ("BESTOF_GH_26", 4, "GH_26", "SC_GH_26_1_vpd_chv_malibu_65", 1, 2, "sc/Career/Story/BESTOF_GH_26_CORE"),
    ("BESTOF_GH_27", 4, "GH_27", "SC_GH_27_3_vpd_mbz_clk63amg_08", 1, 2, "sc/Career/Story/BESTOF_GH_27_CORE"),
    ("BESTOF_GH_31", 4, "GH_31", "SC_GH_31_2_vpd_lr_hse_08", 1, 2, "sc/Career/Story/BESTOF_GH_31_CORE"),
    ("BESTOF_SC", 4, "GH_08", "GH_08_1_vp_mit_eclipse_99", 1, 2, "sc/Career/Story/BESTOF_SC_CORE"),
    ("BESTOF_SH_06", 4, "GH_08", "GH_08_1_vp_mit_eclipse_99", 1, 2, "sc/Career/Story/BESTOF_SH_06_CORE"),
    ("BESTOF_SH_07", 4, "SH_02", "SH_02_2_vp_dge_challenger_06", 1, 2, "sc/Career/Story/BESTOF_SH_07_CORE"),
    ("FIND_GH_27", 4, "SH_03", "SH_03_1_vp_pon_solstice_06", 1, 2, "sc/Career/Story/FIND_GH_27_CORE"),
    ("REPEAT_BESTOF_SC", 4, "", "", 1, 1, "sc/Career/Story/REPEAT_BESTOF_SC_CORE"),
    ("TEST_MISSION", 4, "SH_02", "SH_02_2_vp_dge_challenger_06", 1, 2, "sc/Career/Story/TEST_MISSION_CORE"),
];


// ---- step 19b: the district of a place ----

/// The box of the district map (the vectors at 0x8282BCB0 / 0x8282BCC0, F-31430): x from
/// -3089 to 2881, z from -2542 to 2121; a place outside is moved onto its edge.
pub const DISTRICT_BOX: (f32, f32, f32, f32) = (-3089.0, 2881.0, -2542.0, 2121.0);

/// The district of a place as Racer_GetCurrentDistrict has it (F-42321 -> F-35346 ->
/// F-84136): the texel of the city's zone map (`tune/ambients/<city>.dds`, 256 x 256, one
/// grey level a zone) at the place names a zone of the ambient tree (`iMapColor` of its
/// `mcAmbientDistributionTreeData.xml`), and the zone's parent is the district. A grey level
/// that is no zone's answers Hollywood (A-46870).
pub struct DistrictMap {
    map: Vec<u8>,
    zones: std::collections::HashMap<u8, usize>,
}

impl DistrictMap {
    pub fn load(data_dir: &str) -> Option<DistrictMap> {
        let amb = std::path::Path::new(data_dir).join("tune/ambients");
        let b = std::fs::read(amb.join("sc.dds")).or_else(|_| std::fs::read(amb.join("LA.dds"))).ok()?;
        if b.len() < 128 + 65536 {
            return None;
        }
        let mut zones = std::collections::HashMap::new();
        // (the traffic tree; the pedestrians' has the same zones)
        let tree = amb.join("0xea020850");
        let value = |t: &str| t.find("iMapColor").and_then(|i| t[i..].find("value=\"").map(|j| i + j + 7)).and_then(|i| t[i..].split('"').next().and_then(|v| v.parse::<u8>().ok()));
        for e in std::fs::read_dir(&tree).ok()?.flatten() {
            let Ok(t) = std::fs::read_to_string(e.path().join("mcAmbientDistributionTreeData.xml")) else { continue };
            let name: String = t.split("<Name>").nth(1).and_then(|s| s.split('<').next()).unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
            let Some(d) = DISTRICTS.iter().position(|n| n.eq_ignore_ascii_case(&name)) else { continue };
            for z in std::fs::read_dir(e.path()).ok()?.flatten() {
                if let Some(c) = std::fs::read_to_string(z.path().join("mcAmbientDistributionTreeData.xml")).ok().and_then(|t| value(&t)) {
                    zones.insert(c, d);
                }
            }
        }
        (!zones.is_empty()).then(|| DistrictMap { map: b[128..128 + 65536].to_vec(), zones })
    }

    pub fn district(&self, p: glam::Vec3) -> usize {
        let (x0, x1, z0, z1) = DISTRICT_BOX;
        let u = ((p.x.clamp(x0, x1) - x0) / (x1 - x0) * 256.0).min(255.0) as usize;
        let v = ((p.z.clamp(z0, z1) - z0) / (z1 - z0) * 256.0).min(255.0) as usize;
        self.zones.get(&self.map[v * 256 + u]).copied().unwrap_or(0)
    }
}

/// The district (0 Hollywood .. 4 South Central) of a place; None without the map.
pub fn district_of_place(data_dir: &str, p: glam::Vec3) -> Option<usize> {
    static MAP: std::sync::OnceLock<(String, Option<DistrictMap>)> = std::sync::OnceLock::new();
    let (dir, map) = MAP.get_or_init(|| (data_dir.to_string(), DistrictMap::load(data_dir)));
    if dir != data_dir {
        return DistrictMap::load(data_dir).map(|m| m.district(p));
    }
    map.as_ref().map(|m| m.district(p))
}

// ---- step 20: completion order, hidden requirement flags, baselines, running scripts ----

/// What the career's start-up script starts for a new career (`Game/Career/CareerStartup`,
/// `game/career/0x400221ca`: 0x43 / 0xb8, then 0x862 .. 0x8fe outside the `unlockcareer`
/// branch): the first scripts of the career's list (manager+424), in the new game sample's
/// order. (script, stack)
pub const STARTUP_SCRIPTS: [(&str, u32); 6] = [
    ("game/Career/CareerSetupPlayer", 1500),
    ("game/Garages/ratemyride_setup", 1500),
    ("game/MiscTasks/GasStations", 1500),
    ("Game/MiscTasks/Collectables", 1500),
    ("Game/MiscTasks/SoundTriggers", 1500),
    ("Game/Flow/BootDebug", 1500),
];

/// The scripts whose saved variables are none (all the others save one word, 0 in every
/// sample: the program's static tail, F-78990 A-82765).
pub const SCRIPTS_WITHOUT_DATA: [&str; 4] = ["game/MiscTasks/GasStations", "Game/MiscTasks/Collectables", "Game/MiscTasks/SoundTriggers", "Game/Flow/BootDebug"];

/// What a mission's own script (`Game/Career/Missions/<ID>`, started when the mission enters
/// the log state: F-58560 -> F-68362, and again when a save is loaded) starts with
/// mcCareer_StartNewScript - each only when it is not running (the scripts ask the registry,
/// `<name>_state`, or mcCareer_GetHookman(300 + n)). (mission, script, stack, South Central
/// list: mcCareer_StartNewScript_SC, manager+428.) From the native calls of
/// `script_obj/game/career/missions/*.sco` and `sc/career/missions/*.sco`.
pub const MISSION_SCRIPTS: [(&str, &str, u32, bool); 69] = [
    ("BEAT_FREEWAY", "Game/CruisingHookmen/freeway_hookman1", 1500, false),
    ("BEAT_FREEWAY", "Game/CruisingHookmen/freeway_hookman2", 1500, false),
    ("BEAT_FREEWAY", "Game/CruisingHookmen/freeway_hookman3", 1500, false),
    ("BEAT_FREEWAY", "Game/CruisingHookmen/freeway_hookman4", 1500, false),
    ("BEAT_FREEWAY", "Game/CruisingHookmen/freeway_hookman5", 1500, false),
    ("BEAT_FREEWAY", "Game/CruisingHookmen/freeway_hookman6", 1500, false),
    ("DELIVERY", "Game/Garages/hw_garage_setup", 1500, false),
    ("DELIVERY", "Game/Hangouts/hw_delivery_setup", 1500, false),
    ("FIND_SH_04", "Game/Hangouts/dt_wager_hangout1_setup", 1500, false),
    ("FIND_SH_05", "Game/Hangouts/dt_local_hangout1_setup", 1500, false),
    ("GARAGE_BEACHES", "Game/Hangouts/be_payback_setup", 1500, false),
    ("GARAGE_HOLLYWOOD", "Game/Garages/hw_garage_setup", 1500, false),
    ("HANGOUT_PINKSLIP_BEACHES", "Game/Hangouts/be_wager_hangout1_setup", 1500, false),
    ("HANGOUT_PINKSLIP_DOWNTOWN", "Game/Hangouts/dt_wager_hangout1_setup", 1500, false),
    ("HANGOUT_PINKSLIP_HILLS", "Game/Hangouts/hl_wager_hangout1_setup", 1500, false),
    ("HANGOUT_PINKSLIP_HOLLYWOOD", "Game/Hangouts/hw_wager_hangout1_setup", 1500, false),
    ("HANGOUT_SERIES_BEACHES", "Game/Hangouts/be_local_hangout1_setup", 1500, false),
    ("HANGOUT_SERIES_DOWNTOWN", "Game/Hangouts/dt_local_hangout1_setup", 1500, false),
    ("HANGOUT_SERIES_HILLS", "Game/Hangouts/hl_local_hangout1_setup", 1500, false),
    ("HANGOUT_SERIES_HOLLYWOOD", "Game/Hangouts/hw_local_hangout1_setup", 1500, false),
    ("HANGOUT_SERIES_HOLLYWOODB", "Game/Hangouts/hw_local_hangout2_setup", 1500, false),
    ("HANGOUT_SERIES_HOLLYWOODC", "Game/Hangouts/hw_local_hangout3_setup", 1500, false),
    ("HANGOUT_WAGER_BEACHES", "Game/Hangouts/be_wager_hangout1_setup", 1500, false),
    ("HANGOUT_WAGER_DOWNTOWN", "Game/Hangouts/dt_wager_hangout1_setup", 1500, false),
    ("HANGOUT_WAGER_HILLS", "Game/Hangouts/hl_wager_hangout1_setup", 1500, false),
    ("HANGOUT_WAGER_HOLLYWOOD", "Game/Hangouts/hw_wager_hangout1_setup", 1500, false),
    ("INSTALL_ABILITY", "Game/Garages/hw_garage_setup", 1500, false),
    ("PAYBACK", "Game/Garages/be_garage_setup", 1500, false),
    ("PAYBACK", "Game/Hangouts/be_payback_setup", 1500, false),
    ("PAYBACK_GH_18", "Game/Garages/be_garage_setup", 1500, false),
    ("PAYBACK_GH_18", "Game/Hangouts/be_payback_setup", 1500, false),
    ("PAYBACK_INTRO", "Game/Garages/be_garage_setup", 1500, false),
    ("PAYBACK_INTRO", "Game/Hangouts/be_payback_setup", 1500, false),
    ("RACE_FREEWAY", "Game/CruisingHookmen/freeway_hookman1", 1500, false),
    ("RACE_FREEWAY", "Game/CruisingHookmen/freeway_hookman2", 1500, false),
    ("RACE_FREEWAY", "Game/CruisingHookmen/freeway_hookman3", 1500, false),
    ("RACE_FREEWAY", "Game/CruisingHookmen/freeway_hookman4", 1500, false),
    ("RACE_FREEWAY", "Game/CruisingHookmen/freeway_hookman5", 1500, false),
    ("RACE_FREEWAY", "Game/CruisingHookmen/freeway_hookman6", 1500, false),
    ("REDLIGHT_GH_01", "Game/CruisingHookmen/hw_redlight_hk1", 3800, false),
    ("REDLIGHT_GH_01", "Game/CruisingHookmen/be_redlight_hk1", 3800, false),
    ("REDLIGHT_GH_01", "Game/CruisingHookmen/hl_redlight_hk1", 3800, false),
    ("REDLIGHT_GH_01", "Game/CruisingHookmen/dt_redlight_hk1", 3800, false),
    ("REDLIGHT_GH_01", "Game/CruisingHookmen/freeway_hookman4", 1500, false),
    ("REPEAT_HANGOUT_WAGER_BEACHES", "Game/Hangouts/be_wager_hangout1_setup", 1500, false),
    ("REPEAT_HANGOUT_WAGER_DOWNTOWN", "Game/Hangouts/dt_wager_hangout1_setup", 1500, false),
    ("REPEAT_HANGOUT_WAGER_HILLS", "Game/Hangouts/hl_wager_hangout1_setup", 1500, false),
    ("REPEAT_HANGOUT_WAGER_HOLLYWOOD", "Game/Hangouts/hw_wager_hangout1_setup", 1500, false),
    ("REP_ON_HW_STRIP", "Game/CruisingHookmen/freeway_hookman1", 1500, false),
    ("REP_ON_HW_STRIP", "Game/CruisingHookmen/freeway_hookman2", 1500, false),
    ("REP_ON_HW_STRIP", "Game/CruisingHookmen/freeway_hookman3", 1500, false),
    ("REP_ON_HW_STRIP", "Game/CruisingHookmen/freeway_hookman4", 1500, false),
    ("REP_ON_HW_STRIP", "Game/CruisingHookmen/freeway_hookman5", 1500, false),
    ("REP_ON_HW_STRIP", "Game/CruisingHookmen/freeway_hookman6", 1500, false),
    ("BESTOF_GH_31", "sc/CruisingHookmen/sc_redlight_hk1", 3800, true),
    ("BESTOF_GH_31", "sc/CruisingHookmen/freeway_hookman7", 1500, true),
    ("BESTOF_GH_31", "sc/CruisingHookmen/freeway_hookman8", 1500, true),
    ("DELIVERY_SC", "sc/Garages/sc_garage_setup", 1500, true),
    ("DELIVERY_SC", "sc/Hangouts/sc_delivery_setup", 1500, true),
    ("DELIVERY_SC", "sc/CruisingHookmen/sc_redlight_hk1", 3800, true),
    ("DELIVERY_SC", "sc/CruisingHookmen/freeway_hookman7", 1500, true),
    ("DELIVERY_SC", "sc/CruisingHookmen/freeway_hookman8", 1500, true),
    ("FIND_GH_27", "sc/CruisingHookmen/sc_redlight_hk1", 3800, true),
    ("FIND_GH_27", "sc/CruisingHookmen/freeway_hookman7", 1500, true),
    ("FIND_GH_27", "sc/CruisingHookmen/freeway_hookman8", 1500, true),
    ("GARAGE_SC", "sc/Garages/sc_garage_setup", 1500, true),
    ("GARAGE_SC", "sc/CruisingHookmen/sc_redlight_hk1", 3800, true),
    ("GARAGE_SC", "sc/CruisingHookmen/freeway_hookman7", 1500, true),
    ("GARAGE_SC", "sc/CruisingHookmen/freeway_hookman8", 1500, true),
];

/// The South Central hangouts' set-ups, started by their missions' scripts the same way.
pub const MISSION_SCRIPTS_SC_HANGOUTS: [(&str, &str); 4] = [
    ("HANGOUT_PINKSLIP_SC", "sc/Hangouts/sc_wager_hangout1_setup"),
    ("HANGOUT_SERIES_SC", "sc/Hangouts/sc_local_hangout1_setup"),
    ("HANGOUT_WAGER_SC", "sc/Hangouts/sc_wager_hangout1_setup"),
    ("REPEAT_HANGOUT_WAGER_SC", "sc/Hangouts/sc_wager_hangout1_setup"),
];

/// What is started when a mission is complete: RACE_GH_02's story script starts the
/// telephone challenges after the race (`Game/Career/Story/RACE_GH_02_CORE` 0x42fa, once the
/// mission is COMPLETE); the third time trial of a district hands out the reward script
/// `<d>_unlock_time_trial` (mcRewardScript of TIMETRIAL_<D>C.xml), which starts the
/// district's time trial hookman after its call (`game/career/hw_unlock_time_trial.sco`
/// 0x1d5) and ends. (mission, script, stack, South Central list)
pub const COMPLETE_SCRIPTS: [(&str, &str, u32, bool); 6] = [
    ("RACE_GH_02", "Game/Race/TelephoneChallenge", 1500, false),
    ("TIMETRIAL_HOLLYWOODC", "Game/CruisingHookmen/hw_time_trial_hk1", 1500, false),
    ("TIMETRIAL_BEACHESC", "Game/CruisingHookmen/be_time_trial_hk1", 1500, false),
    ("TIMETRIAL_DOWNTOWNC", "Game/CruisingHookmen/dt_time_trial_hk1", 1500, false),
    ("TIMETRIAL_HILLSC", "Game/CruisingHookmen/hl_time_trial_hk1", 1500, false),
    ("TIMETRIAL_SCC", "sc/CruisingHookmen/sc_time_trial_hk1", 1500, true),
];

/// A script record as the career saves it (F-78990 A-68894..).
fn script_record(name: &str, stack: u32) -> Script {
    let none = SCRIPTS_WITHOUT_DATA.iter().any(|n| n.eq_ignore_ascii_case(name));
    Script { name: name.to_string(), stack_size: stack, saved_bytes: if none { 0 } else { 4 }, saved_words: if none { Vec::new() } else { vec![0] }, extra: Extra::new() }
}

/// mcCareer_StartNewScript(_SC) for a script that is not running: it joins the end of the
/// career's list (a child of the start-up script, manager+424 / +428). True when added.
pub fn start_career_script(c: &mut Career, name: &str, stack: u32, sc: bool) -> bool {
    let running = c.scripts.iter().chain(c.addon.scripts.iter()).any(|s| s.name.eq_ignore_ascii_case(name));
    if running {
        return false;
    }
    let list = if sc { &mut c.addon.scripts } else { &mut c.scripts };
    list.push(script_record(name, stack));
    true
}

/// What a mission's script starts when it enters the log, and what its completion starts.
pub fn mission_starts(c: &mut Career, id: &str, entered: bool) -> bool {
    let mut changed = false;
    if entered {
        for (m, s, stack, sc) in MISSION_SCRIPTS {
            if m.eq_ignore_ascii_case(id) {
                changed |= start_career_script(c, s, stack, sc);
            }
        }
        for (m, s) in MISSION_SCRIPTS_SC_HANGOUTS {
            if m.eq_ignore_ascii_case(id) {
                changed |= start_career_script(c, s, 1500, true);
            }
        }
    } else {
        for (m, s, stack, sc) in COMPLETE_SCRIPTS {
            if m.eq_ignore_ascii_case(id) {
                changed |= start_career_script(c, s, stack, sc);
            }
        }
    }
    changed
}

/// The `global` mission's record (`tune/career/missions/global.xml`: always in
/// CHECKING_PREREQUISITES, its one prerequisite waits for a mission that does not exist).
/// The port does not load it; a block of the port gets it as the original writes it.
fn global_record(index: i32) -> Mission {
    Mission {
        name: "global".to_string(),
        index,
        active_slot: None,
        completed_slot: None,
        state: MISSION_STATES[1].to_string(),
        state_id: 1,
        addon: false,
        prerequisites: vec![Requirement { hidden: true, satisfied: false, conditions: vec![Condition { kind: Value::String("Mission".into()), met: false, ..Default::default() }], extra: Extra::new() }],
        objectives: Vec::new(),
        extra: Extra::new(),
    }
}

/// A mission's place in the main mission list (`tune/career/0x68d5d432`, "Mission { Name
/// <file> }"): the record's index (mission+696). -1 when it is not on it.
fn list_index_of(data_dir: &str, name: &str) -> i32 {
    std::fs::read(std::path::Path::new(data_dir).join("tune/career/0x68d5d432"))
        .map(|d| String::from_utf8_lossy(&d).lines().filter_map(|l| l.trim().strip_prefix("Name").map(|n| n.trim().to_string())).position(|n| n.eq_ignore_ascii_case(name)).map_or(-1, |k| k as i32))
        .unwrap_or(-1)
}

/// A condition record as the port can give it (`armed`: its requirement has been armed, so a
/// statistic's value is the one in hand - else the constructor's 0).
fn condition_record(kind: &str, r: &crate::career::ReqSave, armed: bool) -> Condition {
    let mut c = Condition { kind: Value::String(kind.to_string()), met: r.met, ..Default::default() };
    match kind {
        "Stat" => {
            c.start_value = Some(r.base.unwrap_or(0));
            c.value = Some(if armed { r.value.unwrap_or(0) } else { 0 });
        }
        "Experience" => c.start_amount = Some(r.base.unwrap_or(0)),
        "TallyRace" | "TallyMission" | "Tally" => c.count = Some(r.count as i64),
        _ => {}
    }
    c
}

fn requirement_record(r: &crate::career::ReqSave, armed: bool) -> Requirement {
    Requirement { hidden: r.hidden, satisfied: r.met, conditions: r.kinds.iter().map(|k| condition_record(k, r, armed)).collect(), extra: Extra::new() }
}

/// A requirement record brought up to the port's state. `live`: the requirement is being
/// checked (or its mission changed state), so its verdict, counts and statistic are the
/// port's; else only what the port keeps all the time (the hidden byte) is written.
/// True when something changed.
fn requirement_out(rec: &mut Requirement, v: &crate::career::ReqSave, live: bool, armed: bool, objective: bool) -> bool {
    let mut changed = false;
    if objective && rec.hidden != v.hidden {
        rec.hidden = v.hidden;
        changed = true;
    }
    if !live {
        return changed;
    }
    if rec.satisfied != v.met {
        rec.satisfied = v.met;
        changed = true;
    }
    if rec.conditions.len() != v.kinds.len() {
        rec.conditions = v.kinds.iter().map(|k| condition_record(k, v, armed)).collect();
        return true;
    }
    for c in rec.conditions.iter_mut() {
        let was_met = c.met;
        if c.met != v.met {
            c.met = v.met;
            changed = true;
        }
        match c.kind.as_str().unwrap_or("") {
            "Stat" => {
                if let Some(b) = v.base.filter(|b| c.start_value != Some(*b)) {
                    c.start_value = Some(b);
                    changed = true;
                }
                // (the statistic's last value: followed while the condition waits, frozen when
                // it is met - F-16469)
                if armed && !was_met && v.value.is_some() && c.value != v.value {
                    c.value = v.value;
                    changed = true;
                }
            }
            "Experience" => {
                if let Some(b) = v.base.filter(|b| c.start_amount != Some(*b)) {
                    c.start_amount = Some(b);
                    changed = true;
                }
            }
            "TallyRace" | "TallyMission" | "Tally" => {
                if c.count != Some(v.count as i64) {
                    c.count = Some(v.count as i64);
                    changed = true;
                }
            }
            _ => {}
        }
    }
    changed
}

impl Game {
    /// Step 20, the load: the save was read and the career restored from the port's own
    /// fields; the block's mission records go over it as the original's load does
    /// (F-98950 -> F-23748 -> F-60562 for each record):
    /// - the mission is put back into the log's list and the completed list at its slots
    ///   (F-49159): the port's log order and completion order;
    /// - its requirements' hidden bytes and its conditions' saved values are read
    ///   (F-47948), and it re-enters its state with the loading flag (F-44473), so
    ///   the baselines are kept, not taken again: the port's baselines;
    /// - re-entering the state posts `<ID>_<STATE>_ON_ENTER` (F-83568): a mission saved in
    ///   READY_TO_START runs its READY_TO_START_ON_ENTER script again - the port's
    ///   `ready_now` (what the port does with those: the time trials' target, step 18);
    /// - a mission in the log starts its own script again (F-58560 -> F-68362); the
    ///   event it runs (a series' wins, a tournament's points) lives in script variables that
    ///   are not saved, so it starts over - the port drops a String objective's partial count;
    ///   what the script starts that is not running is started (the journal, applied when the
    ///   block is written);
    /// - the career's scripts are restarted from the list with their saved variables
    ///   (A-23923 -> F-33294): the list is kept as it is.
    /// A record whose state is not the port's stage (an older block, a career the port moved
    /// on) gives nothing: the port's own state stands, and the record is rewritten at the next
    /// save. Without the block nothing changes: the port's lists and flags as before.
    pub fn mc4_career_resume(&mut self) {
        if !self.career.loaded() {
            return;
        }
        let Some(m) = self.mc4.as_ref() else { return };
        let c = &m.career;
        let by_slot = |slot: fn(&Mission) -> Option<i32>| {
            let mut v: Vec<(i32, String)> = c.missions.iter().filter_map(|r| slot(r).map(|s| (s, r.name.clone()))).collect();
            v.sort();
            v.into_iter().map(|x| x.1).collect::<Vec<String>>()
        };
        let log = by_slot(|r| r.active_slot);
        let done = by_slot(|r| r.completed_slot);
        let recs: Vec<Mission> = c.missions.clone();
        if !log.is_empty() {
            self.career.set_log_order(&log);
        }
        self.career.set_done_order(&done);
        for rec in &recs {
            let Some(i) = self.career.mission_no(&rec.name) else { continue };
            let stage = self.career.stage_of(i);
            if crate::career::Career::state_stage(rec.state_id) != stage {
                continue;
            }
            let v = self.career.save_view(i);
            let base = |r: &Requirement| {
                r.conditions.first().and_then(|c| match c.kind.as_str().unwrap_or("") {
                    "Stat" => c.start_value,
                    "Experience" => c.start_amount,
                    _ => None,
                })
            };
            let hidden: Vec<bool> = if rec.objectives.len() == v.objectives.len() { rec.objectives.iter().map(|r| r.hidden).collect() } else { Vec::new() };
            let ob: Vec<Option<i64>> = if rec.objectives.len() == v.objectives.len() { rec.objectives.iter().map(base).collect() } else { Vec::new() };
            let pb: Vec<Option<i64>> = if rec.prerequisites.len() == v.prerequisites.len() { rec.prerequisites.iter().map(base).collect() } else { Vec::new() };
            self.career.ext_from_save(i, &hidden, &ob, &pb);
            // (a met Stat condition's value stopped where it was met: step 20b)
            let last = |r: &Requirement| r.conditions.first().filter(|c| c.met && c.kind.as_str() == Some("Stat")).and_then(|c| c.value);
            let ol: Vec<Option<i64>> = if rec.objectives.len() == v.objectives.len() { rec.objectives.iter().map(last).collect() } else { Vec::new() };
            let pl: Vec<Option<i64>> = if rec.prerequisites.len() == v.prerequisites.len() { rec.prerequisites.iter().map(last).collect() } else { Vec::new() };
            self.career.last_from_save(i, &ol, &pl);
            if stage == crate::career::Stage::Active {
                let sat: Vec<bool> = if rec.objectives.len() == v.objectives.len() { rec.objectives.iter().map(|r| r.satisfied).collect() } else { Vec::new() };
                self.career.drop_event_progress(i, &sat);
                // (saved in READY_TO_START: re-entered, its ON_ENTER script - the call - again)
                if rec.state_id == 2 {
                    self.career.resume_ready(i);
                }
            }
        }
        self.opt_x.arcade.pink.counted.set(0);
        // (k%AG is posted when the missions have been read: F-23748 -> F-20532)
        self.career.post_percent();
        // (the missions in the log start their scripts again)
        let open: Vec<usize> = self.career.log_order().to_vec();
        for i in open {
            self.career.journal.push((true, i));
        }
    }

    /// Step 20, the save: every mission of the career into the block's records - state, the
    /// log's slot and the completed slot, the requirements' hidden bytes, verdicts, counts and
    /// baselines (what F-56966 / F-57390 write). A record whose state the port has
    /// not moved keeps every value the port does not follow, so an untouched save is written
    /// back as it was read; a mission without a record (a block of the port) gets one.
    fn mc4_missions_out(&self, m: &mut Mc4) {
        if !self.career.loaded() {
            return;
        }
        use crate::career::{Career as C, Stage};
        let port_made = m.made_by_port();
        let c = &mut m.career;
        let n = self.career.mission_count();
        let (order, done) = (self.career.log_order(), &self.career.done);
        for i in 0..n {
            let v = self.career.save_view(i);
            let active_slot = order.iter().position(|&x| x == i).map(|p| p as i32);
            let completed_slot = done.iter().position(|&x| x == i).map(|p| p as i32);
            // (the original's state number: READY_TO_START apart - step 20b)
            let state_id = v.state_id;
            let _ = C::stage_state_id(v.stage);
            let armed_obj = matches!(v.stage, Stage::Active | Stage::Complete);
            let armed_pre = v.stage != Stage::Locked;
            match c.missions.iter().position(|r| r.name.eq_ignore_ascii_case(&v.id)) {
                Some(p) => {
                    let rec = &mut c.missions[p];
                    let same = rec.state_id == state_id || (C::state_stage(rec.state_id) == v.stage && v.stage != Stage::Active);
                    if !same {
                        rec.state_id = state_id;
                        rec.state = MISSION_STATES[state_id as usize].to_string();
                    }
                    if rec.active_slot != active_slot {
                        rec.active_slot = active_slot;
                    }
                    if rec.completed_slot != completed_slot {
                        rec.completed_slot = completed_slot;
                    }
                    if rec.objectives.len() != v.objectives.len() {
                        rec.objectives = v.objectives.iter().map(|r| requirement_record(r, armed_obj)).collect();
                    } else {
                        let live = !same || v.stage == Stage::Active;
                        for (r, x) in rec.objectives.iter_mut().zip(v.objectives.iter()) {
                            requirement_out(r, x, live, armed_obj, true);
                        }
                    }
                    if rec.prerequisites.len() != v.prerequisites.len() {
                        rec.prerequisites = v.prerequisites.iter().map(|r| requirement_record(r, armed_pre)).collect();
                    } else {
                        let live = !same || v.stage == Stage::Armed;
                        for (r, x) in rec.prerequisites.iter_mut().zip(v.prerequisites.iter()) {
                            requirement_out(r, x, live, armed_pre, false);
                        }
                    }
                }
                None => {
                    let rec = Mission {
                        name: v.id.clone(),
                        index: v.index,
                        active_slot,
                        completed_slot,
                        state: MISSION_STATES[state_id as usize].to_string(),
                        state_id,
                        addon: v.addon,
                        prerequisites: v.prerequisites.iter().map(|r| requirement_record(r, armed_pre)).collect(),
                        objectives: v.objectives.iter().map(|r| requirement_record(r, armed_obj)).collect(),
                        extra: Extra::new(),
                    };
                    let at = c.missions.iter().position(|r| r.index > v.index).unwrap_or(c.missions.len());
                    c.missions.insert(at, rec);
                }
            }
        }
        if port_made {
            if !c.missions.iter().any(|r| r.name.eq_ignore_ascii_case("global")) {
                let idx = list_index_of(&self.data_dir, "global");
                let at = c.missions.iter().position(|r| r.index > idx).unwrap_or(c.missions.len());
                c.missions.insert(at, global_record(idx));
            }
            let sc: Vec<String> = c.missions.iter().filter(|r| r.addon).map(|r| r.name.clone()).collect();
            if !sc.is_empty() && c.addon.missions != sc {
                c.addon.missions = sc;
                c.addon.present = 1;
            }
        }
        // (the decoder's two views of the lists)
        let view = |slot: fn(&Mission) -> Option<i32>, list: &Vec<Mission>| {
            let mut v: Vec<(i32, String)> = list.iter().filter_map(|r| slot(r).map(|s| (s, r.name.clone()))).collect();
            v.sort();
            v.into_iter().map(|x| x.1).collect::<Vec<String>>()
        };
        let a = view(|r| r.active_slot, &c.missions);
        if c.active_order != a {
            c.active_order = a;
        }
        let d = view(|r| r.completed_slot, &c.missions);
        if c.completed_order != d {
            c.completed_order = d;
        }
    }

    /// Step 20, the career's scripts: a career begun in the port gets the start-up script's
    /// six; then what the missions' scripts start, in the order the missions entered the log
    /// and were completed (`career.journal`). A block of the port made for a career under way
    /// (a save without the block) gets the start-up six and what its started missions start
    /// (complete ones in their order, then the open ones) - when they were started cannot be
    /// told. An original's list is kept as it is, with what this session's missions start.
    fn mc4_scripts_apply(&self, m: &mut Mc4) {
        if !self.career.loaded() {
            return;
        }
        let port_made = m.made_by_port();
        let c = &mut m.career;
        if port_made && c.scripts.is_empty() && c.addon.scripts.is_empty() {
            for (s, stack) in STARTUP_SCRIPTS {
                start_career_script(c, s, stack, false);
            }
            if !self.career.journal_complete {
                for &i in self.career.done.iter() {
                    mission_starts(c, self.career.mission_id(i), true);
                    mission_starts(c, self.career.mission_id(i), false);
                }
                for &i in self.career.log_order() {
                    mission_starts(c, self.career.mission_id(i), true);
                }
            }
        }
        for &(entered, i) in self.career.journal.iter() {
            mission_starts(c, self.career.mission_id(i), entered);
        }
        // (a bCareerScript READY_TO_START_ON_ENTER script is in the list while its mission waits
        // for the call, and leaves it when the script ends: step 20b)
        let waiting = self.career.waiting_career_scripts();
        for s in self.career.career_on_enter_scripts() {
            let here = waiting.iter().any(|w| w.eq_ignore_ascii_case(&s));
            let sc = s.to_ascii_lowercase().starts_with("sc/");
            if here {
                start_career_script(c, &s, 1500, sc);
            } else {
                c.scripts.retain(|x| !x.name.eq_ignore_ascii_case(&s));
                c.addon.scripts.retain(|x| !x.name.eq_ignore_ascii_case(&s));
            }
        }
    }
}

// ---- step 20b: the running scripts' markers, the hangouts' restarts ----

/// The set-up script of a map marker: a hangout by its key (`hw_local_hangout1` ...: the
/// hangout's set-up `Game/Hangouts/<key>_setup`), a garage or a red light by its district
/// (`<d>_garage_setup`, `<d>_redlight_hk1`). The set-ups draw these markers while they run
/// (mcHudMapServer_AddElement after mcCareer_GetLocationId in `<key>_setup.sco`; the red light
/// hookmen's own element).
pub fn marker_script(kind: u8, key_or_district: &str) -> String {
    match kind {
        0 => format!("{key_or_district}_setup"),
        1 => format!("{key_or_district}_garage_setup"),
        _ => format!("{key_or_district}_redlight_hk1"),
    }
}

/// A hangout's key (its set-up's name without `_setup`) by its pull-in scene, in the order
/// main.rs names them (`cut_hangout_01` = hw_local_hangout1 .. `sc_cut_hangout_02`).
pub fn hangout_key_of_scene(scene: &str) -> Option<&'static str> {
    const KEYS: [(&str, &str); 12] = [("/cut_hangout_01", "hw_local_hangout1"), ("/cut_hangout_02", "hw_local_hangout2"), ("/cut_hangout_03", "hw_local_hangout3"), ("/cut_hangout_04", "hw_wager_hangout1"), ("/cut_hangout_05", "be_local_hangout1"), ("/cut_hangout_06", "be_wager_hangout1"), ("/cut_hangout_07", "dt_local_hangout1"), ("/cut_hangout_08", "dt_wager_hangout1"), ("/cut_hangout_09", "hl_local_hangout1"), ("/cut_hangout_10", "hl_wager_hangout1"), ("/sc_cut_hangout_01", "sc_local_hangout1"), ("/sc_cut_hangout_02", "sc_wager_hangout1")];
    let l = scene.to_ascii_lowercase();
    KEYS.iter().find(|k| l.ends_with(k.0)).map(|k| k.1)
}

impl Game {
    /// A hangout (by its pull-in scene) is there to pull into: its set-up runs (step 20b).
    pub fn mc4_hangout_open(&self, scene: &str) -> bool {
        // (step 21b: not while its restarter waits for the player - the set-up has ended)
        let open = hangout_key_of_scene(scene).map_or(true, |k| self.mc4_setup_shown(&marker_script(0, k)) && !self.car_x.hangout_wait.iter().any(|w| w == k));
        // (step 22b: not while Downtown's scripted pink slip goes from its scene to its race)
        let open = open && !self.mc4_dt_pink_busy();
        // (test hook, step 21b: MCLA_MARKER_TRACE logs each hangout's answer once)
        if std::env::var("MCLA_MARKER_TRACE").is_ok() {
            static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
            if let Ok(mut s) = SEEN.lock() {
                if !s.iter().any(|x| x == scene) {
                    s.push(scene.to_string());
                    log::info!("marker trace: {} key {:?} script {:?} open {} (list known {}, career {})", scene, hangout_key_of_scene(scene), hangout_key_of_scene(scene).map(|k| marker_script(0, k)), open, self.mc4_running_scripts().is_some(), self.career.loaded());
                }
            }
        }
        open
    }

    /// The career's running scripts as a save written now would hold them (the block's list
    /// with what this session's missions have started), last names only, lower case. None:
    /// not known - a save of the port without the block, whose list is not known before its
    /// first save (`mc4_scripts_apply` builds it then); the markers keep the port's own rules.
    pub fn mc4_running_scripts(&self) -> Option<Vec<String>> {
        if !self.career.loaded() {
            return None;
        }
        let mut c = match self.mc4.as_ref() {
            Some(m) if !m.career.scripts.is_empty() || !m.career.addon.scripts.is_empty() => Career { scripts: m.career.scripts.clone(), addon: Addon { scripts: m.career.addon.scripts.clone(), ..Default::default() }, ..Default::default() },
            _ if self.career.journal_complete => {
                let mut c = Career::default();
                for (s, stack) in STARTUP_SCRIPTS {
                    start_career_script(&mut c, s, stack, false);
                }
                c
            }
            _ => return None,
        };
        for &(entered, i) in self.career.journal.iter() {
            mission_starts(&mut c, self.career.mission_id(i), entered);
        }
        Some(c.scripts.iter().chain(c.addon.scripts.iter()).map(|s| s.name.rsplit('/').next().unwrap_or("").to_ascii_lowercase()).collect())
    }

    /// A marker of the map (and a hangout's pull-in) is there when its set-up runs; with the
    /// list not known, as the port had it.
    pub fn mc4_setup_shown(&self, script: &str) -> bool {
        self.mc4_running_scripts().map_or(true, |l| l.iter().any(|n| n.eq_ignore_ascii_case(script)))
    }

    /// The district code (hw, be, hl, dt, sc) of a place.
    pub fn mc4_district_code(&self, p: glam::Vec3) -> &'static str {
        DISTRICT_CODES[district_of_place(&self.data_dir, p).unwrap_or(0).min(4)]
    }

    /// The player has left a hangout: its set-up had ended with the visit (the hangout's core
    /// script finishes, then the set-up) and is started again once the player is away from it
    /// (`game/Hangouts/0xf975d2ad` and its kind: Registry `<key>_state` 1, Racer_DistanceToPoint,
    /// mcCareer_StartNewScript(`Game/Hangouts/<key>_setup`)) - at the end of the list. The
    /// delivery's and the payback's restarters name other scripts (`Game/Delivery/...`,
    /// `Game/Payback/...`) and are left out.
    pub fn mc4_hangout_left(&mut self, key: &str) {
        // (step 22b: the place left - its location events are posted again at the next visit)
        self.career.visit_events.clear();
        // (step 21b: the restarter now waits for the player at its own point -
        // `mc4_hangout_restart_check` starts the set-up again)
        if HANGOUT_RESTARTERS.iter().any(|r| r.0 == key) {
            if !self.car_x.hangout_wait.iter().any(|w| w == key) {
                self.car_x.hangout_wait.push(key.to_string());
            }
        }
    }

    /// The restarters of the places left (`HANGOUT_RESTARTERS`): the player within the
    /// script's distance of its point (Racer_DistanceToPoint < 5 / 25, in three dimensions)
    /// starts the set-up again - by the restarter's own name, at the end of the list.
    pub fn mc4_hangout_restart_check(&mut self, p: [f32; 3]) {
        let waiting = self.car_x.hangout_wait.clone();
        for key in waiting {
            let Some(r) = HANGOUT_RESTARTERS.iter().find(|r| r.0 == key) else { continue };
            let d = ((p[0] - r.2[0]).powi(2) + (p[1] - r.2[1]).powi(2) + (p[2] - r.2[2]).powi(2)).sqrt();
            if d < r.3 {
                self.car_x.hangout_wait.retain(|w| *w != key);
                self.mc4_restart_setup(r.1, r.4);
                log::info!("hangout: {} started again at {:.1} m of its restarter's point", r.1, d);
            }
        }
    }

    /// A set-up started again by a restarter: the ended one out of the list (whatever path it
    /// was started by), the restarter's start at the end (saved variables as the start's: one
    /// word 0, as the list's others).
    fn mc4_restart_setup(&mut self, path: &str, addon: bool) {
        let Some(m) = self.mc4.as_mut() else { return };
        let last = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
        let c = &mut m.career;
        let list = if addon { &mut c.addon.scripts } else { &mut c.scripts };
        let old = list.iter().position(|s| s.name.rsplit('/').next().map_or(false, |n| n.eq_ignore_ascii_case(&last)));
        let mut s = match old {
            Some(k) => list.remove(k),
            None => return,
        };
        s.name = path.to_string();
        list.push(s);
        self.dirty = true;
    }

    /// (the step 20b form, kept for the tests of the list's order: the set-up to the end at once)
    pub fn mc4_hangout_left_now(&mut self, key: &str) {
        if key.contains("delivery") || key.contains("payback") || key.is_empty() {
            return;
        }
        let Some(m) = self.mc4.as_mut() else { return };
        let c = &mut m.career;
        let name = format!("{key}_setup");
        for (list, sc) in [(&mut c.scripts, false), (&mut c.addon.scripts, true)] {
            if let Some(k) = list.iter().position(|s| s.name.rsplit('/').next().map_or(false, |n| n.eq_ignore_ascii_case(&name))) {
                let s = list.remove(k);
                list.push(s);
                let _ = sc;
                self.dirty = true;
                return;
            }
        }
    }
}

/// The hangouts' restarters (`game/Hangouts/0x...`, `sc/Hangouts/0x...`, started as a
/// visit's set-up ends): (place key, the script they start, their point, the distance,
/// add-on list). Each sets `<key>_state` 1, waits until Racer_DistanceToPoint(player, point)
/// < distance, then mcCareer_StartNewScript(_SC). As on the disc: the South Central ones
/// wait at Hollywood's points (sc_local at hw_local_hangout1's, sc_wager at
/// hw_wager_hangout1's), and the delivery / payback ones at the Hollywood garage lot.
pub const HANGOUT_RESTARTERS: [(&str, &str, [f32; 3], f32, bool); 15] = [
    ("hw_local_hangout1", "Game/Hangouts/hw_local_hangout1_setup", [727.875, 7.438, -279.156], 25.0, false), // 0xf975d2ad
    ("hw_local_hangout2", "Game/Hangouts/hw_local_hangout2_setup", [-467.733, 2.019, -176.592], 5.0, false), // 0x6d9b71c9
    ("hw_local_hangout3", "Game/Hangouts/hw_local_hangout3_setup", [-1679.459, -5.666, 277.073], 5.0, false), // 0x25646de1
    ("hw_wager_hangout1", "Game/Hangouts/hw_wager_hangout1_setup", [-140.0, 7.0, -470.595], 5.0, false), // 0xbbe119f7
    ("be_local_hangout1", "Game/Hangouts/be_local_hangout1_setup", [-2728.656, -4.631, -201.7], 5.0, false), // 0xc4742b35
    ("be_wager_hangout1", "Game/Hangouts/be_wager_hangout1_setup", [-2728.656, -4.631, -201.7], 5.0, false), // 0xd932824b
    ("dt_local_hangout1", "Game/Hangouts/dt_local_hangout1_setup", [2122.176, -1.473, -229.034], 5.0, false), // 0x3853c2ca
    ("dt_wager_hangout1", "Game/Hangouts/dt_wager_hangout1_setup", [1837.804, -9.997, 611.44], 5.0, false), // 0xfe50b979
    ("hl_local_hangout1", "Game/Hangouts/hl_local_hangout1_setup", [-989.404, 30.1, -1851.548], 5.0, false), // 0x69fce42b
    ("hl_wager_hangout1", "Game/Hangouts/hl_wager_hangout1_setup", [131.282, 99.646, -1768.193], 5.0, false), // 0x89babab6
    ("sc_local_hangout1", "sc/Hangouts/sc_local_hangout1_setup", [727.875, 7.438, -279.156], 25.0, true), // sc 0x994ef3b9
    ("sc_wager_hangout1", "sc/Hangouts/sc_wager_hangout1_setup", [-140.0, 7.0, -470.595], 5.0, true), // sc 0xbb47551d
    ("hw_delivery", "Game/Delivery/hw_delivery_setup", [824.34, 16.75, -791.74], 25.0, false), // 0x40d08bf5
    ("be_payback", "Game/Payback/be_payback_setup", [824.34, 16.75, -791.74], 25.0, false), // 0x5d90c2f7
    ("sc_delivery", "sc/Delivery/sc_delivery_setup", [824.34, 16.75, -791.74], 25.0, true), // sc 0x6d60215d
];

// ---- step 21: pink slips ----

/// The wager hangouts' pink slip race and man (their set-ups' data: +1400 the race's name,
/// +1404 the hookman - `Game/Hangouts/<key>_setup` 0x3c2 / 0x3d9 for Hollywood; the same
/// places in the others): (hangout key, race, hookman).
pub const PINK_SLIP_RACES: [(&str, &str, &str); 5] = [
    ("hw_wager_hangout1", "game/Race/Hollywood/Ordered/hw_ordered_jp_04", "GH_13"),
    ("be_wager_hangout1", "game/Race/Beaches/Ordered/be_ordered_ds_02", "GH_07"),
    ("hl_wager_hangout1", "Game/Race/Hills/Ordered/hl_ordered_ts_01", "GH_03"),
    ("dt_wager_hangout1", "game/Race/Downtown/Ordered/dt_ordered_jp_01", "GH_18"),
    ("sc_wager_hangout1", "game/Race/SouthCentral/Ordered/sc_ordered_mb_03", "GH_29"),
];

/// The wager hangouts' pink slip places: (hangout key, location id, the mission whose script
/// loads the hangout's own pink slip car, that car's set-up file). The mission scripts
/// `HANGOUT_PINKSLIP_<D>.sco` call mcCareer_LoadPinkSlipProfile(location, set-up, 0) and
/// mcCareer_TogglePinkslipHangoutDirty(location, 1) when they start (0x21 / 0x2d); Downtown's
/// pink slip is a story race of its own (HANGOUT_PINKSLIP_DOWNTOWN_HANGOUT_ON_ENTER.sco), no
/// profile.
pub const PINK_HANGOUTS: [(&str, u32, &str, &str); 5] = [
    ("hw_wager_hangout1", 503, "HANGOUT_PINKSLIP_HOLLYWOOD", "pinkslip_vp_pon_transam_79"),
    ("be_wager_hangout1", 511, "HANGOUT_PINKSLIP_BEACHES", "pinkslip_vp_mzd_rx8_06"),
    ("hl_wager_hangout1", 521, "HANGOUT_PINKSLIP_HILLS", "pinkslip_vp_mit_evo_06"),
    ("dt_wager_hangout1", 531, "", ""),
    ("sc_wager_hangout1", 541, "HANGOUT_PINKSLIP_SC", "SC_pinkslip_vpd_pon_gto_66"),
];

/// The career's pink slip lot: five profiles (manager + 50128, 8272 bytes each).
pub const PINK_SLOTS: usize = 5;

/// A pink slip profile as the hangout offers it: the hangout's own car (kind 0, not saved:
/// manager + 8768 + h * 8272) or one of the player's cars held in the lot (kind 1).
#[derive(Clone, Debug, Default)]
pub struct PinkProfile {
    pub uid: i32,
    pub kind: i32,
    /// the vehicle's folder
    pub car: String,
    /// its set-up file (the hangout's car), or the record's own set-up
    pub setup: String,
    /// a held car's record
    pub record: Option<Car>,
}

/// A pink slip race under way: the hangout, the profile raced for, the player's stake.
#[derive(Clone, Debug, Default)]
pub struct PinkRun {
    pub key: String,
    pub profile: PinkProfile,
    pub stake_uid: i32,
    pub stake_car: String,
}

/// What the running game keeps for pink slips (not saved).
#[derive(Clone, Debug, Default)]
pub struct PinkLive {
    /// the profile under the panel's cursor
    pub pick: usize,
    pub run: Option<PinkRun>,
    /// journal entries already counted for the hangouts' own cars
    pub counted: std::cell::Cell<usize>,
    /// (step 21b) the race that has just ended was a pink slip race (not a leg of any event)
    pub just_ended: bool,
}

fn pink_empty(k: usize) -> PinkSlip {
    PinkSlip { slot: k as u32, kind: -1, empty: true, name: String::new(), uid: -1, extra: Extra::new() }
}

/// The vehicle folder of a set-up file's name (`pinkslip_vp_pon_transam_79` -> `vp_pon_transam_79`).
pub fn setup_vehicle(setup: &str) -> String {
    let l = setup.to_ascii_lowercase();
    l.find("vpd_").or_else(|| l.find("vp_")).map(|i| l[i..].to_string()).unwrap_or(l)
}

impl PinkSlips {
    fn ensure(&mut self) {
        while self.slots.len() < PINK_SLOTS {
            let k = self.slots.len();
            self.slots.push(pink_empty(k));
        }
    }

    /// The id counter (manager + 0x10000 + 25960): one up for every profile added or loaded,
    /// back to 0 past the top (F-58746 A-29345..A-89255).
    pub fn next_uid(&mut self) -> i32 {
        let n = (self.uid_counter as i32).wrapping_add(1);
        self.uid_counter = if n < 0 { 0 } else { n as u32 };
        self.uid_counter as i32
    }

    /// mcCareer_AddPinkSlipProfile(location, record, kind != 0) (F-58746): slots 3..0 move
    /// to 4..1 (slot 4 is lost), slot 0 takes the record, its name (record + 8109) and a new id.
    pub fn add(&mut self, kind: i32, car: &Car) -> i32 {
        self.ensure();
        for k in (0..PINK_SLOTS - 1).rev() {
            if !self.slots[k].empty {
                let mut s = self.slots[k].clone();
                s.slot = (k + 1) as u32;
                self.slots[k + 1] = s;
            }
        }
        let uid = self.next_uid();
        let mut s = pink_empty(0);
        s.kind = kind;
        s.empty = false;
        s.name = car.TypeName.clone();
        s.uid = uid;
        s.extra.insert("vehicle_name".into(), Value::String(car.vehicle.clone()));
        s.extra.insert("car".into(), serde_json::to_value(car).unwrap_or(Value::Null));
        self.slots[0] = s;
        uid
    }

    /// mcCareer_RemovePinkSlipProfile(location, uid) (F-85350): the slot is cleared and
    /// the later ones move up.
    pub fn remove(&mut self, uid: i32) -> Option<PinkSlip> {
        self.ensure();
        let k = self.slots.iter().position(|s| !s.empty && s.uid == uid)?;
        let out = self.slots[k].clone();
        for j in k..PINK_SLOTS - 1 {
            let mut s = self.slots[j + 1].clone();
            s.slot = j as u32;
            self.slots[j] = s;
        }
        self.slots[PINK_SLOTS - 1] = pink_empty(PINK_SLOTS - 1);
        Some(out)
    }

    /// The held cars, slot 0 first.
    pub fn held(&self) -> Vec<&PinkSlip> {
        self.slots.iter().filter(|s| !s.empty && s.kind != -1).collect()
    }
}

/// A held car's record: the port's own (`car`), or the original's bytes (`vehicle_record_hex`
/// of the importer) read as a set-up file is (the same record).
pub fn pink_record(s: &PinkSlip) -> Option<Car> {
    if let Some(c) = s.extra.get("car").and_then(|v| Car::deserialize(v).ok()) {
        return Some(c);
    }
    let hex = s.extra.get("vehicle_record_hex")?.as_str()?;
    let b: Vec<u8> = (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()).collect();
    let name = s.extra.get("vehicle_name").and_then(|v| v.as_str()).unwrap_or(&s.name).to_ascii_lowercase();
    Car::from_xcc(&b, &name, &name, 0)
}

impl Game {
    /// The profiles a wager hangout offers (mcCareer_GetNumPinkSlipProfiles / GetPinkSlipUID,
    /// F-54647 / F-50697): first its own car while its pink slip mission's script
    /// runs (the mission in the log), then the held cars of the lot - the same at every wager
    /// hangout.
    pub fn mc4_pink_profiles(&self, key: &str) -> Vec<PinkProfile> {
        let mut out = Vec::new();
        if let Some((h, (_, _, mission, setup))) = PINK_HANGOUTS.iter().enumerate().find(|(_, p)| p.0.eq_ignore_ascii_case(key)) {
            if !mission.is_empty() && self.career.mission_no(mission).map_or(false, |i| self.career.log_order().contains(&i)) {
                out.push(PinkProfile { uid: -(h as i32) - 2, kind: 0, car: setup_vehicle(setup), setup: setup.to_string(), record: None });
            }
        } else {
            return out;
        }
        if let Some(m) = self.mc4.as_ref() {
            for s in m.career.pink_slips.held() {
                let rec = pink_record(s);
                let car = rec.as_ref().map(|c| c.vehicle.clone()).unwrap_or_else(|| s.name.to_ascii_lowercase());
                let setup = rec.as_ref().and_then(|c| c.source_config.clone()).unwrap_or_else(|| car.clone());
                out.push(PinkProfile { uid: s.uid, kind: s.kind, car, setup, record: rec });
            }
        }
        out
    }

    /// The lot is full: the confirmation warns that the oldest car goes for good
    /// (`pinkslip_confirmation_max`).
    pub fn mc4_pink_lot_full(&self) -> bool {
        self.mc4.as_ref().map_or(false, |m| m.career.pink_slips.held().len() >= PINK_SLOTS)
    }

    /// A pink slip race starts (wager_hangout_core, `game/Hangouts/0x16e89a34` 0x5011..0x5177):
    /// the player's vehicle is copied, taken out of the garage (LocalOptions_RemoveVehicleFrom
    /// Player) and put into the lot as a held car (AddPinkSlipProfile(location, copy, 1)), then
    /// the career is saved. The port takes the vehicle out of the garage when the race is lost
    /// (it is driven meanwhile). False without a profile or a second vehicle.
    pub fn mc4_pink_start(&mut self, key: &str, pick: usize) -> bool {
        let profiles = self.mc4_pink_profiles(key);
        let Some(profile) = profiles.get(pick.min(profiles.len().saturating_sub(1))).cloned() else { return false };
        if self.owned.len() < 2 {
            return false;
        }
        self.mc4_leave_car();
        let car = self.mc4_car_name(self.current_car);
        let fresh = self.fresh_game;
        let mut m = self.mc4.take().unwrap_or_else(|| Mc4::seeded(fresh));
        // (the hangouts' own cars loaded before: their ids come first)
        self.mc4_pink_count(&mut m);
        let rec = m.car(&car).cloned().unwrap_or_else(|| Car { TypeName: car.clone(), vehicle: car.clone(), ..Default::default() });
        let stake_uid = m.career.pink_slips.add(1, &rec);
        // (step 22b: the record's bytes as the lot keeps them - its opponent races it in its
        // parts and paint, `mc4_pink_rival`)
        if let Some(b) = self.mc4_record_bytes(&rec) {
            let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
            if let Some(s) = m.career.pink_slips.slots.iter_mut().find(|s| !s.empty && s.uid == stake_uid) {
                s.extra.insert("vehicle_record_hex".into(), Value::String(hex));
            }
        }
        self.mc4 = Some(m);
        // (xGAP counts the pink slip races, tTPS the tutorial's flag: 0x4b40, 0x50ae)
        *self.career.stats.entry("xGAP".into()).or_insert(0) += 1;
        log::info!("pink slip: {} staked (id {}) for {} (id {}, kind {})", car, stake_uid, profile.car, profile.uid, profile.kind);
        self.opt_x.arcade.pink.run = Some(PinkRun { key: key.to_string(), profile, stake_uid, stake_car: car });
        self.dirty = true;
        true
    }

    /// The opponent of the pink slip race under way: (character, vehicle, set-up) - he drives
    /// the profile's car (OpponentManager_LoadOpponentByCarConfig, 0x5566) and is the
    /// hangout's man of +1404 of the set-up's data (mcCareer_GetHookmanCharacterSet, 0x553a;
    /// `PINK_SLIP_RACES`). A held car of an original save is driven as its record has it
    /// (the record's bytes, `pinkslip:<id>`); step 22b: one the port staked too (its bytes are
    /// written at the stake, `car_to_xcc`).
    pub fn mc4_pink_rival(&self) -> Option<(String, String, String)> {
        // (step 22b: Downtown's scripted pink slip - GH_18 in the 350Z)
        if let Some(r) = self.mc4_dt_pink_rival() {
            return Some(r);
        }
        let r = self.opt_x.arcade.pink.run.as_ref()?;
        let who = PINK_SLIP_RACES.iter().find(|p| p.0.eq_ignore_ascii_case(&r.key)).map(|p| p.2.to_string())?;
        let setup = if r.profile.kind != 0 && self.mc4_held_bytes(r.profile.uid).is_some() { format!("pinkslip:{}", r.profile.uid) } else { r.profile.setup.clone() };
        Some((who, r.profile.car.clone(), setup))
    }

    /// (step 23b) The set-up a profile's car is built in where it stands at the hangout
    /// (LoadOpponentByCarConfig with the profile, wager_hangout_core 0xb80): a held car with the
    /// original's bytes in its record, else its set-up file.
    pub fn mc4_pink_setup_name(&self, p: &PinkProfile) -> String {
        if p.kind != 0 && self.mc4_held_bytes(p.uid).is_some() { format!("pinkslip:{}", p.uid) } else { p.setup.clone() }
    }

    /// A held car's record as the original wrote it (`vehicle_record_hex`, the set-up file's
    /// layout).
    fn mc4_held_bytes(&self, uid: i32) -> Option<Vec<u8>> {
        let m = self.mc4.as_ref()?;
        let s = m.career.pink_slips.slots.iter().find(|s| !s.empty && s.uid == uid)?;
        let hex = s.extra.get("vehicle_record_hex")?.as_str()?;
        let b: Vec<u8> = (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()).collect();
        (b.len() == 8176).then_some(b)
    }

    /// (step 21b) The vehicles the save holds: while a pink slip race runs, not the staked one
    /// (wager_hangout_core 0x513c LocalOptions_RemoveVehicleFromPlayer).
    pub fn mc4_owned_out(&self) -> Vec<String> {
        match self.opt_x.arcade.pink.run.as_ref() {
            Some(r) if self.owned.len() > 1 => self.owned.iter().filter(|o| !o.eq_ignore_ascii_case(&r.stake_car)).cloned().collect(),
            _ => self.owned.clone(),
        }
    }

    /// (step 21b) The vehicle in use the save holds: while a pink slip race runs, the garage's
    /// first (0x5144 LocalOptions_SetCurrentVehicleProfileIndex(0)).
    pub fn mc4_current_out(&self) -> Option<String> {
        self.opt_x.arcade.pink.run.as_ref()?;
        self.mc4_owned_out().first().map(|s| s.to_ascii_lowercase())
    }

    /// The race of a pink slip at hangout `key`: of the hangout's races (its set-up's list,
    /// `LAUNCHERS`) the first not run lately, else the one run longest ago (the race array's
    /// FindNextRaceIndex, F-18968 - which array call the wager core makes was not
    /// followed). The race definition's index.
    pub fn mc4_pink_race(&self, key: &str) -> Option<usize> {
        // (step 21b: the race is the one of +1400 of the set-up's data - CityDescription_
        // GetRaceIndexByName(city, data + 1400), wager_hangout_core 0x5467)
        let race = PINK_SLIP_RACES.iter().find(|p| p.0.eq_ignore_ascii_case(key))?.1;
        let k = self.mc4_race_index(race)?;
        self.mc4_table_race(k).map(|(i, _)| i)
    }

    /// (test hook, step 21b: MCLA_PINK_SAVE writes the save copy as a pink slip race ends)
    fn mc4_pink_test_save(&mut self, how: &str) {
        if std::env::var("MCLA_PINK_SAVE").is_ok() {
            let keep = self.no_save;
            self.no_save = false;
            let dir = self.data_dir.clone();
            self.write_save(&dir);
            self.no_save = keep;
            log::info!("pink slip: {} - save written (MCLA_PINK_SAVE)", how);
        }
    }

    /// (step 21b) The race just over was a pink slip race (taken once).
    pub fn mc4_pink_was(&mut self) -> bool {
        std::mem::take(&mut self.opt_x.arcade.pink.just_ended)
    }

    pub fn mc4_pink_on(&self) -> bool {
        self.opt_x.arcade.pink.run.is_some()
    }

    /// The race is over (0x5851..0x6037). Won: the player's vehicle back
    /// (LocalOptions_AddConfigToPlayer), the opponent's car given - the hangout's own by its
    /// set-up name with PINK_SLIP_COMPLETE for the mission (BuildRewardCarB), a held car with
    /// its record (BuildRewardCarA) - and both profiles out of the lot. Lost (or quit, or
    /// damaged out: `pinkslip_confirmation`): the vehicle stays in the lot, gone from the
    /// garage. Returns the splash's text key.
    pub fn mc4_pink_end(&mut self, won: bool) -> Option<&'static str> {
        // (step 22b: Downtown's scripted pink slip - no stake, the 350Z on a win, no splash)
        if self.mc4_dt_pink_end(won) {
            return None;
        }
        let run = self.opt_x.arcade.pink.run.take()?;
        self.opt_x.arcade.pink.just_ended = true;
        let fresh = self.fresh_game;
        if won {
            // (the player's vehicle back: LocalOptions_AddConfigToPlayer - at the garage's end -
            // and in use, SetCurrentVehicleProfileIndex(the new slot): 0x586d / 0x5885)
            if let Some(k) = self.owned.iter().position(|o| o.eq_ignore_ascii_case(&run.stake_car)) {
                let c = self.owned.remove(k);
                self.owned.push(c);
            }
            let m = self.mc4.get_or_insert_with(|| Mc4::seeded(fresh));
            m.career.pink_slips.remove(run.stake_uid);
            if run.profile.kind == 0 {
                self.career.give_car(&run.profile.setup);
                self.career.pink_slip_won(&run.key);
            } else {
                m.career.pink_slips.remove(run.profile.uid);
                let car = run.profile.car.to_ascii_lowercase();
                if let Some(mut rec) = run.profile.record.clone() {
                    if m.car(&car).is_none() {
                        rec.slot = m.garage.len() as u32;
                        m.garage.push(rec);
                    }
                }
                if !self.owned.iter().any(|o| o.eq_ignore_ascii_case(&car)) && self.car_dirs.iter().any(|d| d.file_name().map_or(false, |n| n.to_string_lossy().eq_ignore_ascii_case(&car))) {
                    self.owned.push(car);
                }
            }
            self.dirty = true;
            self.mc4_pink_test_save("won");
            return Some("PINKSLIP_SUCCESS");
        }
        // (lost: out of the garage; the next vehicle is the one in use, as when one is sold)
        let car = run.stake_car.clone();
        if self.owned.len() > 1 {
            self.owned.retain(|o| !o.eq_ignore_ascii_case(&car));
            if let Some(next) = (0..self.car_dirs.len()).find(|&k| self.is_owned(k)) {
                self.selected_car = next;
                self.current_car = next;
                // (main stands the next vehicle in when the race is left)
                self.borrowed = true;
            }
        }
        self.dirty = true;
        // (step 23b: logged, and the test hook's save as on a win)
        log::info!("pink slip: lost - {} (id {}) stays in the lot, out of the garage", car, run.stake_uid);
        self.mc4_pink_test_save("lost");
        Some("PINKSLIP_FAILURE")
    }

    /// The hangouts' own cars loaded this session (their missions' scripts start - at the
    /// log, and again at a load): the id counter, once each (F-53651).
    fn mc4_pink_count(&self, m: &mut Mc4) {
        let j = &self.career.journal;
        let from = self.opt_x.arcade.pink.counted.get().min(j.len());
        for &(entered, i) in &j[from..] {
            if entered && PINK_HANGOUTS.iter().any(|p| !p.2.is_empty() && p.2.eq_ignore_ascii_case(self.career.mission_id(i))) {
                m.career.pink_slips.next_uid();
            }
        }
        self.opt_x.arcade.pink.counted.set(j.len());
        if m.career.pink_slips.slots.is_empty() && m.made_by_port() {
            m.career.pink_slips.ensure();
        }
    }
}

// ---------------------------------------------------------------------------------------
// step 22: vinyl packages - VINYL PACKAGES and STORE VINYL PACKAGES of the vinyl editor
// ---------------------------------------------------------------------------------------

/// The vehicles whose own preset (`tune/garage/<vehicle>_<zone>.vgp`) heads VINYL PACKAGES
/// (F-30784, called by F-95994 and F-34888 with the vehicle's name).
pub const OWN_PRESET_CARS: [&str; 9] = ["vp_dge_challenger_70", "vp_frd_boss302_69", "vp_pon_transam_79", "vp_frd_cobra_04", "vp_frd_gt_06", "vp_sln_s281_06", "vp_duc_monster_s4r_07", "vp_duc_999r_06", "vpd_chv_chevelle_70"];

/// The content mask words (`tDL1`, `tDL2` at 0x827E9170) as F-68286 sets them at
/// start-up: word 0 |= 0x13F, word 1 |= 0x13D. Bit 4 of word 1 lists the South Central
/// presets (`sc_vinyl_preset_names.vgp`) for a car (F-95994 A-98299); it is
/// always set, so the list is never short of them. `tms_vinyl_preset_names.vgp` (asked
/// for after them) is not on the disc.
pub const CONTENT_MASK_START: [u32; 2] = [0x13F, 0x13D];
pub const CONTENT_SC_PRESETS: u32 = 4;

/// A row of VINYL PACKAGES: a preset of the disc (its `userdefined` -1) or one of the three
/// stored packages (`userdefined` = the slot).
#[derive(Clone, Debug, PartialEq)]
pub enum PkgRow {
    Preset(String),
    User(usize),
}

/// What a vinyl editor box asks: put a row of VINYL PACKAGES on the vehicle
/// (`OverwriteVinylsConfirmation`), store the vehicle's layers in a slot
/// (`OverwriteVinylsPresetConfirmation`), take every layer off (`RemoveVinylsConfirmation`).
#[derive(Clone, Debug, PartialEq)]
pub enum VinylAsk {
    Load(PkgRow),
    Store(usize),
    RemoveAll,
}

/// The rows of VINYL PACKAGES (`garage.CreateVinylPresetMenu` = F-34888, the names from
/// F-95994): the vehicle's own preset when it has one, the disc's list for its kind
/// (`vinyl_preset_names` then - content bit 4 - `sc_vinyl_preset_names` for a car,
/// `vinyl_bk_preset_names` for a bike; `presets` is that list as the port reads it), then
/// "Package<n>" for every stored slot whose byte 0 is set, in slot order.
pub fn vinyl_package_rows(car: &str, presets: &[String], used: &[bool], content: u32) -> Vec<PkgRow> {
    let mut out = Vec::new();
    let car = car.to_ascii_lowercase();
    if OWN_PRESET_CARS.contains(&car.as_str()) {
        out.push(PkgRow::Preset(car.clone()));
    }
    let bike = presets.iter().any(|p| p.starts_with("bikepreset"));
    out.extend(presets.iter().filter(|p| !p.eq_ignore_ascii_case(&car)).filter(|p| bike || content & CONTENT_SC_PRESETS != 0 || !p.starts_with("sc_preset")).map(|p| PkgRow::Preset(p.clone())));
    // (F-34888 returns before the list when the disc's count is 0: no rows at all)
    if out.is_empty() {
        return out;
    }
    out.extend(used.iter().take(3).enumerate().filter(|(_, u)| **u).map(|(k, _)| PkgRow::User(k)));
    out
}

impl Game {
    /// The three slots' "used" bytes (UserVinylData + 5764 k).
    pub fn mc4_vinyl_used(&self) -> [bool; 3] {
        let mut u = [false; 3];
        if let Some(m) = self.mc4.as_ref() {
            for p in &m.vinyl_packages {
                if (p.slot as usize) < 3 {
                    u[p.slot as usize] = p.used;
                }
            }
        }
        u
    }

    /// VINYL PACKAGES of the vehicle in the editor (see `vinyl_package_rows`): the disc's
    /// list is the one the port made for this vehicle's kind (`vinyl_list`).
    pub fn mc4_vinyl_rows(&self) -> Vec<PkgRow> {
        let presets: Vec<String> = self.vinyl_list.iter().filter_map(|v| v.strip_prefix("preset:")).map(|s| s.to_string()).collect();
        vinyl_package_rows(&self.vinyl_for, &presets, &self.mc4_vinyl_used(), CONTENT_MASK_START[1])
    }

    /// A row's text: the movie looks the row's name up in the string table ("preset_bboy"
    /// -> B-BOY, the vehicle's own -> its name, "Package1" -> PACKAGE 1).
    pub fn mc4_vinyl_row_label(&self, r: &PkgRow) -> String {
        let key = match r {
            PkgRow::Preset(p) => p.clone(),
            PkgRow::User(k) => format!("Package{}", k + 1),
        };
        [key.clone(), key.to_uppercase(), format!("VE_{}", key)].iter().find_map(|k| self.career.strings.get(k).map(|v| v.to_uppercase())).unwrap_or_else(|| key.to_uppercase())
    }

    /// The vehicle has layers (F-74954: a layer in any of the five zones) - REMOVE ALL
    /// VINYLS and STORE VINYL PACKAGES are open (EnterTopLevelVinylMenu, F-96638
    /// A-55810: flag 16 of both rows cleared; LoadPresetVinyl(-1) sets it again).
    pub fn mc4_vinyl_has_layers(&self) -> bool {
        match self.vinyls.get(&self.vinyl_for).map(|s| s.as_str()) {
            Some("custom") => self.liveries.get(&self.vinyl_for).map_or(false, |l| !l.is_empty()),
            Some(_) => true,
            None => false,
        }
    }

    /// A vinyl editor box: `ask` is done on its YES (`mc4_vinyl_finish`). Without the popup
    /// movie the port does it at once.
    pub fn mc4_vinyl_ask(&mut self, ask: VinylAsk) {
        let id = match ask {
            VinylAsk::Load(_) => "OverwriteVinylsConfirmation",
            VinylAsk::Store(_) => "OverwriteVinylsPresetConfirmation",
            VinylAsk::RemoveAll => "RemoveVinylsConfirmation",
        };
        if self.open_message_box(id, crate::game::MenuAction::None) {
            self.car_x.vinyl_ask = Some(ask);
        } else {
            self.car_x.vinyl_ask = Some(ask);
            self.mc4_vinyl_finish();
        }
    }

    /// YES in a vinyl editor box: `garage.LoadPresetVinyl` (F-58362: a stored slot
    /// through F-57683, a preset of the disc from its five `.vgp` files, index -1 every
    /// layer off) or `garage.SavePresetVinyl` (F-59911 on the slot of
    /// SetVinylPresetIndex).
    pub fn mc4_vinyl_finish(&mut self) {
        // (step 23: the YES of a box of this step - FIND_SH_02C's money)
        self.mc4_s23_box_yes();
        let Some(ask) = self.car_x.vinyl_ask.take() else { return };
        let car = self.vinyl_for.clone();
        match ask {
            VinylAsk::Load(PkgRow::Preset(p)) => {
                self.vinyls.insert(car, format!("preset:{p}"));
                self.dirty = true;
            }
            VinylAsk::Load(PkgRow::User(k)) => {
                self.mc4_vinyl_load(k);
                if !self.vinyl_list.iter().any(|v| v == "custom") {
                    self.vinyl_list.insert(0, "custom".to_string());
                }
            }
            VinylAsk::Store(k) => {
                // (a preset on the vehicle is its layers: the original copies a preset's layers
                // onto the vehicle when it is put on; the port keeps the preset's name)
                if let Some(p) = self.vinyls.get(&car).and_then(|v| v.strip_prefix("preset:")).map(|s| s.to_string()) {
                    let l = self.vinyl_presets.livery(&p);
                    self.liveries.insert(car.clone(), l);
                    self.vinyls.insert(car.clone(), "custom".to_string());
                    self.livery_rev += 1;
                }
                self.mc4_vinyl_store(k);
            }
            VinylAsk::RemoveAll => {
                if self.vinyls.remove(&car).is_some() {
                    self.dirty = true;
                }
            }
        }
    }

    /// The STORE VINYL PACKAGES page (`VinylPresetSave`: PACKAGE 1 .. 3, SetVinylPresetIndex
    /// 0 .. 2 as the cursor enters a row; ACCEPT asks, BACK goes back to the editor's menu).
    /// Returns true while the page takes the input.
    pub fn mc4_vinyl_save_page(&mut self, up: bool, down: bool, enter: bool, back: bool) -> bool {
        let Some(sel) = self.car_x.vinyl_save else { return false };
        if self.message_box.is_some() {
            return true;
        }
        let sel = if up { (sel + 2) % 3 } else if down { (sel + 1) % 3 } else { sel.min(2) };
        self.car_x.vinyl_save = Some(sel);
        if back {
            self.car_x.vinyl_save = None;
        } else if enter {
            self.mc4_vinyl_ask(VinylAsk::Store(sel));
        }
        true
    }
}

// ---------------------------------------------------------------------------------------
// step 23: the race editor's radius / height / city, achievements, tutorial flags
// ---------------------------------------------------------------------------------------
//
// The race editor. A slot (EditorData, 1056 bytes) keeps per checkpoint its position, its
// activation height (byte 16) and radius (byte 17), and the city it was made in (byte 1043).
//   - Net_EditorRaceCreateCheckpoints (native F-96995 -> F-70973) builds the race of
//     a slot: count - 1 race checkpoints (F-81860), checkpoint k of the race from the
//     slot's k + 1 - the position (F-47850), +180 the activation height = byte 16 and
//     +168 the activation radius = byte 17 (A-46936..A-72657); the rest of the race
//     checkpoint is the constructor's (F-64962: radius 10, height 100, activation
//     height 50 ...) before those two are set. The slot's checkpoint 0 is where the grid goes
//     (Net_EditorPopulateGrid). EditorRace.sco runs it as OrderedCheckpointCore.
//   - The editor gives a new checkpoint the radius and activation height of the statics at
//     0x827DCB34 / 0x827DCB38 (F-52268 A-15149..A-97683; 25 and 25 in the image),
//     which take the values of the checkpoint the cursor stands on (F-75970 A-41895..
//     A-15301) - no control of the editor's menus (tune/ui/RaceEditor.sc.xml) changes them,
//     so every saved checkpoint has 25 / 25 (empty slots 20 / 20).
//   - A slot is offered in the current city when its byte 1043 is that city, or when it is 0
//     or 0xCD (never saved) and the city is 0 or 3 (F-34172, caller F-71344). The
//     city's number is its place in the table of 80-byte entries at 0x828CCE98 (built at run
//     time): Los Angeles is 3 (the 100 % save's own race, made there).
//   - Saving a race (F-60473) awards `aSRE` (A-70341).
//
// Achievements. The statistics with an `a` id are the achievements (value 1 = awarded); the
// stats manager hands such a record to the console (F-79933). Who sets them:
//   - F-56238, every 60th update of the stats manager at count 30 (F-50381):
//     23 thresholds of statistics (`ACHIEVEMENT_STATS`; five compare for equality);
//   - the race-end statistics (F-98556): aLOW / aSUV at 10 class wins;
//   - scripts at a mission's end: RACE_GH_06_CORE aCPL (0x13cf), the class champions'
//     CORE scripts aUTU / aUMU / aUEX / aULU / aUMO, CITY_CHAMP_CORE aBCC; mission rewards of
//     type mcRewardAchievement (CITY_CHAMP aBCC, BESTOF_GH_27 aBSC, GARAGE_SC aMSC,
//     HANGOUT_WAGER_SC aWSC, TIMETRIAL_SCC aLSC, TOURNAMENT_SC aTSC) - `ACHIEVEMENT_MISSIONS`;
//     generic_timetrial_hookman aB1T / aBAT (step 18);
//   - the career tables' ability thresholds (Reward Type Achievement aZON / aAGR / aROR /
//     aPUL with the level 2 of their ability, `tune/career/0x3b8ff776`);
//   - code: the purse (F-71075: aMIL when the money after an addition is 1,000,000 or
//     more), mcCareer_SetTournamentWinStatus (F-67674: aBTO when the four districts'
//     tournaments are won), the police (F-72120 aRUN as a ticket is run from,
//     F-22530 aEVA with gCPL as the police are lost), the race editor (aSRE), the photo
//     mode (aFAB, tPTS >= 16 - F-67304), the goal attack goals (F-96807: aAOR,
//     aAAR, aARC), the showroom's test drive of a South Central vehicle (F-22067: aTDV),
//     and the online ones (aNJR, aNRE, aNRS, aTMF; postponed).
// The game shows nothing of its own: the pause menu's ACHIEVEMENTS row calls
// XamShowAchievementsUI (F-13098 -> F-82612 -> 0x827BCDC4), the console's list, and
// an award is announced by the console's overlay. The port records the award (the
// statistic, the block's `achievements`) and logs it.
//
// Tutorial flags. A `tT..` statistic is 0 until its tutorial has been shown. The scripts'
// pattern (generic_local_hookman 0xc0e..0xe18, RACE_GH_02_READY_TO_START_ON_ENTER 0x3d5..
// 0x8b3, garage_core 0x15ff..0x1b70 ...): unless GetNoTutorialCommandLine, flag 0 ->
// StatsManager_UpdateStatInt(flag, 3 (set), 1); AudioManager_SetActiveTimeSlow(1); the
// "ease to pause" - form 108 visible, PerspectiveForm_SetSlowMotion(form 108, 0, 1 / 0.5,
// 0, 1, 1), 500 ms; Form_ShowForm(-1, <form>) with the speech scenario (mcCareerSpeech_
// PlayScenario(<scenario>, drv_mb_04_set, PLAYER)), wait until it is closed;
// SetActiveTimeSlow(0); "ease from pause" - SetSlowMotion(form 108, 1, 1 / 0.5, 0, 1, 1),
// form 108 hidden. Some show the form without the slow motion (the hangouts' panels, the
// hookman in reach, the police). The forms (F-26445 A-49179..: one class each,
// title / description keys `tutorial_<name>_title / _desc`): `TUTORIAL_FORMS`.
//
// The slow motion (F-84653 sets, F-82413 updates once a frame): target +960, rate
// +964 (per second), ease flags +968 / +969, power +972, t +976, value +980, start +984 =
// the global scale (0x827E1C34) when set. While t < 1: t += rate x frame time (0x827D7558);
// e = t eased - both flags (1 - cos(pi t)) / 2 (F-39265), +968 only 1 - cos(t pi / 2)
// (F-86323), +969 only sin(t pi / 2) (F-17888), else t; e ^= power when it is not
// 1; value = start + (target - start) e; the global scale = value; t >= 1: value = target. A
// value equal to the target sets t = 1 at once; rate 0 sets the value at once.

use crate::checkpoints::CpSpec;

/// Byte 1043 of a slot made in Los Angeles (F-34172's city 3).
pub const EDITOR_CITY_LA: u32 = 3;

/// Whether a slot of city `city` is offered in city `current` (F-34172).
pub fn editor_city_ok(city: u32, current: u32) -> bool {
    city == current || ((city == 0 || city == 0xCD) && (current == 0 || current == 3))
}

/// The achievements: id, the console's number, title (the table F-53715 builds with
/// F-47825; titles from the executable's achievement resource - `mc4save/small.py`).
pub const ACHIEVEMENTS: [(&str, u32, &str); 55] = [
    ("aRUN", 18, "Wanted Man"),
    ("aEVA", 19, "Out On Bail"),
    ("aJDS", 20, "Neither Bird nor Plane"),
    ("aJTM", 21, "Jordan Jumpman"),
    ("aWHL", 22, "Unicycle"),
    ("a2WD", 23, "Just Get A Motorcycle"),
    ("aROR", 24, "Hear me Roar!"),
    ("aAGR", 27, "Anger Management"),
    ("aPUL", 28, "Set To Stun"),
    ("aZON", 29, "Zoning Out"),
    ("aULU", 30, "In the Lap of..."),
    ("aUTU", 31, "Name that Tune(r)"),
    ("aUMU", 33, "World's Strongest"),
    ("aUMO", 34, "Two Wheels of Fury"),
    ("aUEX", 35, "How Exotic"),
    ("aCPL", 36, "The Basics"),
    ("aFWW", 37, "What's a Speed Limit?"),
    ("aw5C", 38, "Streaker"),
    ("aBIK", 39, "Winning With Less"),
    ("aRRR", 40, "Go on Green"),
    ("aD10", 41, "Destroyer of Worlds"),
    ("aC5D", 42, "Here for a Pickup"),
    ("aB1T", 43, "Father Time"),
    ("aBAT", 44, "Uncle Time"),
    ("aBTO", 45, "Competitive Glory"),
    ("a200", 46, "It's How You Play The Game"),
    ("aW18", 47, "The Winner"),
    ("aMIL", 48, "Rags To Riches"),
    ("aDST", 49, "Have Car, Will Travel"),
    ("aNJR", 50, "Getting Off the Bench"),
    ("aNRE", 51, "Who Made This?"),
    ("aSRE", 52, "Saving Los Angeles"),
    ("aFAB", 53, "Paparazzi"),
    ("aTMF", 55, "T-Mobile myFaves"),
    ("aNRS", 56, "Turn in the Work"),
    ("aNRR", 57, "Everyone's a Critic"),
    ("aNRC", 58, "Crossing the Line"),
    ("aNST", 59, "Master Thief"),
    ("aNFL", 60, "Postman"),
    ("aNRW", 61, "Ranking Up"),
    ("aCOH", 62, "Where in the World?"),
    ("aCOL", 64, "Waldo's Still Missing"),
    ("aBCC", 65, "Retire the Master"),
    ("aAOR", 66, "Mild Dedication"),
    ("aAAR", 67, "True Dedication"),
    ("aMSC", 69, "Goin' South"),
    ("aTDV", 70, "South Central Test Drive"),
    ("aSUV", 71, "Big Ballin'"),
    ("aLOW", 72, "Low Ballin'"),
    ("aDSC", 73, "Sign Here"),
    ("aTSC", 74, "Easier than the Hills"),
    ("aWSC", 75, "Money for nothing"),
    ("aLSC", 76, "Brother Time"),
    ("aBSC", 77, "Been there, Done that"),
    ("aARC", 78, "GOOOOAAAAALLLLL!!!!!"),
];

/// F-56238 in its order: (achievement, statistic, sub id, equal (`bne`) - else at least
/// (`blt`) -, the value). Floats for g?2W / gkWL / gkDS / gmJD / gsJT (vfunc 80), the others
/// whole numbers (vfunc 76).
pub const ACHIEVEMENT_STATS: [(&str, &str, u32, bool, f64); 23] = [
    ("a2WD", "gk2W", 0, false, 50.0),
    ("aCOH", "gCOL", 0, false, 30.0),
    ("aCOL", "gCOL", 0, true, 60.0),
    ("aRRR", "kRLW", 0, true, 15.0),
    ("aFWW", "kFWN", 0, true, 15.0),
    ("aW18", "kWIN", 0, true, 180.0),
    ("a200", "kRCP", 0, true, 300.0),
    ("aWHL", "gkWL", 0, false, 50.0),
    ("aw5C", "kCWN", 0, false, 5.0),
    ("aDST", "gkDS", 0, false, 5000.0),
    ("aJDS", "gmJD", 0, false, 310.896),
    ("aJTM", "gsJT", 0, false, 600.0),
    ("aBIK", "kMOW", 0, false, 10.0),
    ("aD10", "kPDS", 0, false, 15.0),
    ("aNFL", "nFLG", 0, false, 100.0),
    ("aNST", "nFST", 0, false, 80.0),
    ("aNRC", "nRCP", 0, false, 50.0),
    ("aNRW", "nWIN", 0, false, 20.0),
    ("aNRR", "tGRR", 0, false, 25.0),
    ("aC5D", "kDLT", 0, false, 15.0),
    ("aDSC", "xCDL", 542, false, 10.0),
    ("aSUV", "kSUW", 0, false, 10.0),
    ("aLOW", "kLOW", 0, false, 10.0),
];

/// The achievements a mission's completion gives: its CORE script's (RACE_GH_06_CORE 0x13cf,
/// TUNER / MUSCLE / EXOTIC / LUXURY / MOTORCYCLE / CITY_CHAMP_CORE, after the race that
/// completes the mission) and the missions' mcRewardAchievement rewards (bHidden, the XMLs).
pub const ACHIEVEMENT_MISSIONS: [(&str, &str); 12] = [
    ("RACE_GH_06", "aCPL"),
    ("TUNER_CHAMP", "aUTU"),
    ("MUSCLE_CHAMP", "aUMU"),
    ("EXOTIC_CHAMP", "aUEX"),
    ("LUXURY_CHAMP", "aULU"),
    ("MOTORCYCLE_CHAMP", "aUMO"),
    ("CITY_CHAMP", "aBCC"),
    ("BESTOF_GH_27", "aBSC"),
    ("GARAGE_SC", "aMSC"),
    ("HANGOUT_WAGER_SC", "aWSC"),
    ("TIMETRIAL_SCC", "aLSC"),
    ("TOURNAMENT_SC", "aTSC"),
];

/// The tutorial forms by id (places in the main form's table; constructors F-10143 ..
/// F-86881, the hydraulics' F-51796 built later): `tutorial_<name>_title / _desc`.
pub const TUTORIAL_FORMS: [(u32, &str); 31] = [
    (76, "mission_log"),
    (77, "target"),
    (78, "slipstream"),
    (79, "slipstream_use"),
    (80, "police"),
    (81, "race_difficulty"),
    (82, "challenge"),
    (83, "flash"),
    (84, "quick_fix"),
    (85, "gas_station"),
    (86, "red_light_racer"),
    (87, "freeway_racer"),
    (88, "motorcycle"),
    (89, "telephone_challenge"),
    (90, "zone"),
    (91, "agro"),
    (92, "roar"),
    (93, "emp"),
    (94, "collectables"),
    (95, "tournament"),
    (96, "time_trial"),
    (97, "series"),
    (98, "wager"),
    (99, "pinkslip"),
    (100, "delivery"),
    (101, "payback"),
    (102, "beat_me_there"),
    (103, "police_pursuit"),
    (104, "police_pursuit_live"),
    (105, "sidekick"),
    (111, "hydraulics"),
];

/// The tutorial flags of the save: (flag, form, speech scenario, slow motion, who shows it).
pub const TUTORIAL_FLAGS: [(&str, u32, &str, bool, &str); 22] = [
    ("tTBM", 102, "TUTORIAL_BEAT_ME_THERE", true, "generic_local_hookman 0xc0e (the first hookman flashed)"),
    ("tTFL", 82, "", false, "CruisingHookmanCore 0x8f0 (a hookman in reach; flag set when he is flashed, 0xd91)"),
    ("tTFB", 83, "", false, "CruisingHookmanCore 0x918 (the next hookman in reach after tTFL; 0xdbe)"),
    ("tTRL", 86, "TUTORIAL_REDLIGHT_INGAME", true, "CruisingHookmanCore 0x95c (a red light racer in reach, icon 2)"),
    ("tTFR", 87, "TUTORIAL_FREEWAY_INGAME", true, "CruisingHookmanCore 0xb72 (freeway racer, icon 5)"),
    ("tTTO", 95, "TUTORIAL_TOURNAMENT_INGAME", true, "CruisingHookmanCore 0xc10 (tournament man, icon 4)"),
    ("tTTT", 96, "TUTORIAL_TIME_TRIAL_INGAME", true, "CruisingHookmanCore 0xcb1 (time trial man, icon 3)"),
    ("tTMO", 88, "TUTORIAL_MOTORCYCLE_INGAME", true, "garage_core 0x160c (out of the garage on a motorcycle)"),
    ("tTZO", 90, "TUTORIAL_ZONE_INGAME", true, "garage_core 0x187d (out of the garage with ZONE fitted)"),
    ("tTAG", 91, "TUTORIAL_AGRO_INGAME", true, "garage_core 0x1915 (AGRO)"),
    ("tTRO", 92, "TUTORIAL_ROAR_INGAME", true, "garage_core 0x19ad (ROAR)"),
    ("tTEM", 93, "TUTORIAL_EMP_INGAME", true, "garage_core 0x1a45 (EMP)"),
    ("tTHY", 111, "TUTORIAL_HYDRAULICS_INGAME", true, "garage_core 0x1ad7 (hydraulics fitted)"),
    ("tTSE", 97, "TUTORIAL_SERIES_HANGOUT_INGAME", false, "local_hangout_core 0x1169 (before the panel)"),
    ("tTWA", 98, "TUTORIAL_WAGER_HANGOUT_INGAME", false, "wager_hangout_core 0x166b (before the panel)"),
    ("tTPS", 99, "TUTORIAL_PINKSLIP_HANGOUT_INGAME", false, "wager_hangout_core 0x4b17 (PINKSLIP RACE chosen)"),
    ("tTDE", 100, "TUTORIAL_DELIVERY_INGAME", true, "delivery_hangout_core 0x600 (before the panel)"),
    ("tTPA", 101, "TUTORIAL_PAYBACK_INGAME", true, "payback_hangout_core 0x80f (before the panel)"),
    ("tTGS", 85, "", false, "GasStations 0x6a7 (a station entered in cruising)"),
    ("tTCH", 89, "", false, "TelephoneChallenge 0x2c4 (the first telephone challenge)"),
    ("tTPP", 103, "", false, "sub_82700108 from sub_82704228 (the first pursuit)"),
    ("tTPL", 104, "", false, "sub_82700108 (the second pursuit)"),
];

/// The form's string keys.
pub fn tutorial_keys(form: u32) -> Option<(String, String)> {
    TUTORIAL_FORMS.iter().find(|f| f.0 == form).map(|f| (format!("tutorial_{}_title", f.1), format!("tutorial_{}_desc", f.1)))
}

/// The perspective form's slow motion (form 108, +960 .. +984; F-84653 / F-82413).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlowMo {
    pub target: f32,
    pub rate: f32,
    pub ease_in: bool,
    pub ease_out: bool,
    pub power: f32,
    pub t: f32,
    pub value: f32,
    pub start: f32,
    /// the global scale 0x827E1C34 (1 at the start)
    pub global: f32,
    pub visible: bool,
}

impl Default for SlowMo {
    fn default() -> Self {
        SlowMo { target: 1.0, rate: 0.0, ease_in: false, ease_out: false, power: 1.0, t: 1.0, value: 1.0, start: 1.0, global: 1.0, visible: false }
    }
}

impl SlowMo {
    /// PerspectiveForm_SetSlowMotion(form, target, rate, a3 != 0, a4 != 0, power).
    pub fn set(&mut self, target: f32, rate: f32, a3: bool, a4: bool, power: f32) {
        self.target = target;
        self.rate = rate;
        self.ease_in = a3;
        self.ease_out = a4;
        self.power = power;
        self.t = 0.0;
        if rate == 0.0 {
            self.value = target;
        }
        self.start = self.global;
    }

    /// The scripts' "ease to pause" (target 0) and "ease from pause" (1), 0.5 s each.
    pub fn ease(&mut self, to: f32) {
        self.visible = to == 0.0;
        self.set(to, 1.0 / 0.5, false, true, 1.0);
    }

    /// Once a frame (`dt` the frame's time, not scaled).
    pub fn update(&mut self, dt: f32) {
        if !(self.visible || self.t < 1.0) {
            return;
        }
        if self.value == self.target {
            self.t = 1.0;
        } else if self.rate != 0.0 && self.t < 1.0 {
            self.t += self.rate * dt;
        }
        if self.rate == 0.0 {
            self.global = self.value;
            return;
        }
        if self.t < 1.0 {
            let t = self.t;
            let mut e = match (self.ease_in, self.ease_out) {
                (true, true) => (1.0 - (std::f32::consts::PI * t).cos()) * 0.5,
                (true, false) => 1.0 - (t * std::f32::consts::FRAC_PI_2).cos(),
                (false, true) => (t * std::f32::consts::FRAC_PI_2).sin(),
                _ => t,
            };
            if self.power != 1.0 {
                e = e.powf(self.power);
            }
            self.value = self.start + (self.target - self.start) * e;
            self.global = self.value;
        } else {
            self.value = self.target;
            self.global = self.target;
        }
    }
}

/// A tutorial waiting or under way.
#[derive(Clone, Debug, PartialEq)]
pub struct TutRun {
    pub flag: String,
    pub form: u32,
    pub speech: String,
    pub slow: bool,
    /// 0 waiting, 1 easing to pause (seconds left in `wait`), 2 the form shown
    pub phase: u8,
    pub wait: f32,
    /// a form the player does not answer: closed by the script (hookman out of reach / flashed)
    pub modeless: bool,
}

/// Game/Career/SST_Tutorial (`game/career/0xc3c7949c`), started by RACE_GH_06's core
/// (`Story/0x40a56c35` 0x1031: mcCareer_StartNewScript after EvaluatePolice, before "missionRace").
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SstTut {
    #[default]
    Idle,
    /// 0x69..0xe3: every 100 ms (WAITUNPAUSED) until racing and the turbo level > 0.3
    Charge,
    /// 0xe8..0x14e: AudioManager_SetActiveTimeSlow(1), the ease to pause (form 108, 1 / 0.5 s), 500 ms
    Ease(f32),
    /// 0x156..0x22d: form 78 with TUTORIAL_SST_INGAME (drv_mb_04_set, PLAYER) until it is closed
    Box78,
    /// 0x2dc..0x356: the ease from pause done; every 100 ms until racing and the level > 0.99
    Full,
    /// 0x35b..0x367: form 79, no speech, no slow motion, until it is closed (F-61402: SSTvalue < 0.9)
    Box79,
    Done,
}

/// What the running game keeps for step 23 (in `OptLive`).
#[derive(Clone, Debug, Default)]
pub struct S23Live {
    pub slow: SlowMo,
    pub queue: Vec<TutRun>,
    pub run: Option<TutRun>,
    /// RACE_GH_02's lesson: 0 not begun, 1 eased to pause (box 76), 2 the log's lesson (its
    /// speech said), 3 box 77, 4 over
    pub lesson: u8,
    pub mode_was: Option<crate::game::Mode>,
    pub heat_was: u8,
    pub money_was: Option<i64>,
    /// the stats manager's update counter (+656: the thresholds at 30, reset at 60)
    pub stats_frame: u32,
    pub journal_seen: usize,
    pub races_seen: Option<usize>,
    pub xcdl_was: Option<i64>,
    pub pb_wins_was: u32,
    /// a hookman in reach (`near`), the form shown for this approach (CruisingHookmanCore local 8)
    pub near_was: bool,
    pub near_shown: bool,
    pub phone_was: bool,
    /// the hangout panel held back while its tutorial is up (its key), the pink slip page
    pub hangout_hold: Option<String>,
    pub page_was: (u8, usize),
    /// FIND_SH_02C's money box: 1 asked (YES / NO), 2 the restriction (OK); the YES heard;
    /// the player must leave 30 m before he is asked again
    pub f02c_ask: u8,
    pub f02c_yes: bool,
    pub f02c_away: bool,
    /// (step 22b) the pink slip's confirmation box is up: the hangout and the profile chosen
    pub pink_ask: Option<(String, usize)>,
    /// (step 22b) Downtown's scripted pink slip: its race runs (the steps before the
    /// opponent gives up), the frames without a scene after its scene, a race to start
    pub dt_pink: Option<i32>,
    pub dt_wait: u32,
    pub start_race: bool,
    /// (step 22b) the garage was on the vinyl editor's top menu last frame
    pub vyl_was: bool,
    /// Game/Career/SST_Tutorial (`SstTut`), the race seen last frame, the script's 100 ms poll
    pub sst_tut: SstTut,
    pub sst_race_was: bool,
    pub sst_poll: f32,
    /// (step 23b) in a gas station last frame
    pub station_was: bool,
}

/// What the loop hands to the tick: the police state (0 none, 1 pull over, 2 pursuit, 3
/// search, 4 busted), a hookman in reach, he was flashed this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct S23In {
    pub heat: u8,
    pub near: bool,
    pub flashed: bool,
    /// (step 23b) the type of the hookman in reach (mcHookman_GetType; `Roamers::near_type`)
    pub kind: u8,
    /// (step 23b) a gas station entered (mcCareer_IsInGasStation set by the station's trigger)
    pub station: bool,
    /// (step 23b) the player's vehicle this frame, for the driving statistics
    pub drive: crate::mc4b::S23Drive,
    /// the player's slipstream turbo level (mcCarSSTurbo +36, Racer_GetTurboLevel F-55566), None without one
    pub sst: Option<f32>,
}

/// FIND_SH_02C_CORE (`game/career/Story/0xb27d6524`): the place registered (location 605,
/// 0x42) and the meeting loop's radius (0x5b9: 20 m; asked again after 30 m away, 0x6ec).
pub const FIND_SH_02C_AT: [f32; 3] = [12.892, 30.144, -886.856];

impl Game {
    // ---- the race editor ----

    /// The race checkpoints of a race of the editor (Net_EditorRaceCreateCheckpoints): from the
    /// slot whose points are the race's, each with its slot checkpoint's activation radius and
    /// height. None: not an editor race, or no slot record for it (the port's defaults hold).
    pub fn mc4_editor_specs(&self, def: &crate::races::RaceDef) -> Option<Vec<CpSpec>> {
        if !def.id.starts_with("user/") {
            return None;
        }
        let m = self.mc4.as_ref()?;
        let mut want = vec![def.beacon.to_array()];
        want.extend(def.checkpoints.iter().map(|p| p.to_array()));
        let r = m.editor.races.iter().find(|r| r.points() == want)?;
        Some(
            r.checkpoints
                .iter()
                .take(r.count as usize)
                .skip(1)
                .filter(|c| c.pos.len() >= 3)
                .map(|c| CpSpec { radius: c.activation_radius as f32, act_height: c.activation_height as f32, ..CpSpec::at(glam::Vec3::new(c.pos[0].0 as f32, c.pos[1].0 as f32, c.pos[2].0 as f32)) })
                .collect(),
        )
    }

    /// The `k`-th race of the editor is offered in Los Angeles (its slot's city: F-34172).
    /// A race without a slot record is the port's own: offered.
    pub fn mc4_editor_listed(&self, k: usize) -> bool {
        self.mc4.as_ref().and_then(|m| m.editor.races.get(k)).map_or(true, |r| editor_city_ok(r.city, EDITOR_CITY_LA))
    }

    // ---- statistics and achievements ----

    /// A statistic's whole value (the port's count, else the save's).
    pub fn mc4_stat(&self, code: &str, sub: u32) -> f64 {
        self.career.stat(code, &sub.to_string()).unwrap_or(0.0)
    }

    /// StatsManager_UpdateStatInt(code, sub, 3 (set), v).
    pub fn mc4_stat_set(&mut self, code: &str, sub: u32, v: f64) {
        self.career.ostats.insert(format!("{code}/{sub}"), v);
        // (the port's own counts are whole numbers: a float statistic lives in `ostats` alone)
        if sub == 0 && v.fract() == 0.0 {
            self.career.stats.insert(code.to_string(), v as i64);
        } else if sub == 0 {
            self.career.stats.remove(code);
        }
    }

    /// StatsManager_UpdateStatInt(code, sub, 0 (add), n).
    pub fn mc4_stat_add(&mut self, code: &str, sub: u32, n: f64) {
        let v = self.mc4_stat(code, sub) + n;
        self.mc4_stat_set(code, sub, v);
    }

    pub fn mc4_awarded(&self, id: &str) -> bool {
        self.mc4_stat(id, 0) != 0.0
    }

    /// An achievement awarded (operation 4, set once): the statistic, the block's list. The
    /// game shows nothing of its own (the console announces it). True when it is new.
    pub fn mc4_award(&mut self, id: &str) -> bool {
        if self.mc4_awarded(id) {
            return false;
        }
        self.mc4_stat_set(id, 0, 1.0);
        let fresh = self.fresh_game;
        let m = self.mc4.get_or_insert_with(|| Mc4::seeded(fresh));
        if !m.achievements.iter().any(|a| a == id) {
            m.achievements.push(id.to_string());
            m.achievements.sort();
        }
        self.dirty = true;
        let (n, title) = ACHIEVEMENTS.iter().find(|a| a.0 == id).map(|a| (a.1, a.2)).unwrap_or((0, ""));
        log::info!("achievement {} ({} {}) awarded", id, n, title);
        true
    }

    /// F-56238: the thresholds of the statistics.
    pub fn mc4_achievements_check(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, stat, sub, eq, v) in ACHIEVEMENT_STATS {
            let have = self.mc4_stat(stat, sub);
            let hit = if eq { (have - v).abs() < 1e-6 } else { have >= v };
            if hit && self.mc4_award(id) {
                out.push(id.to_string());
            }
        }
        out
    }

    /// The race-end statistics (F-98556) of a finished race: races completed; a win - the
    /// wins, the streak and the longest streak, the last race won; second / third places; any
    /// place but the first ends the streak. A red light race won counts kRLW (the red light
    /// hookmen's scripts). The class wins are the career's (`Ev::Race`); aLOW / aSUV at ten.
    pub fn mc4_race_stats(&mut self, place: usize) {
        self.mc4_stat_add("gRCP", 0, 1.0);
        self.mc4_stat_add("kRCP", 0, 1.0);
        if place == 1 {
            self.mc4_stat_set("xLRW", 0, 1.0);
            self.mc4_stat_add("kWIN", 0, 1.0);
            self.mc4_stat_add("gWIN", 0, 1.0);
            self.mc4_stat_add("kCWN", 0, 1.0);
            self.mc4_stat_add("gCWN", 0, 1.0);
            let (cwn, ccw) = (self.mc4_stat("kCWN", 0), self.mc4_stat("kCCW", 0));
            if cwn > ccw {
                self.mc4_stat_set("kCCW", 0, cwn);
            }
            let red = self.current_race.and_then(|i| self.race_defs.get(i)).map_or(false, |d| d.sub_type == "RedLight" || d.id.ends_with("red_light_race"));
            if red {
                self.mc4_stat_add("kRLW", 0, 1.0);
            }
        } else {
            // (GOAL ATTACK's branch, A-77758..A-58950, counts no second or third place)
            if self.goal_attack {
            } else if place == 2 {
                self.mc4_stat_add("k2ND", 0, 1.0);
            } else if place == 3 {
                self.mc4_stat_add("k3RD", 0, 1.0);
            }
            self.mc4_stat_set("kCWN", 0, 0.0);
            self.mc4_stat_set("gCWN", 0, 0.0);
        }
        for (id, stat) in [("aLOW", "kLOW"), ("aSUV", "kSUW")] {
            if self.mc4_stat(stat, 0) >= 10.0 {
                self.mc4_award(id);
            }
        }
        self.dirty = true;
    }

    /// mcCareer_SetTournamentWinStatus(district, 1) (TournamentCore 0x12c4, the tournament
    /// complete): the flag, and aBTO once the four are set (F-67674).
    pub fn mc4_tournament_won(&mut self, district: usize) {
        if district > 3 {
            return;
        }
        let fresh = self.fresh_game;
        let m = self.mc4.get_or_insert_with(|| Mc4::seeded(fresh));
        m.career.tournaments_won.insert(DISTRICTS[district].to_string(), true);
        let all = (0..4).all(|d| m.career.tournaments_won.get(DISTRICTS[d]).copied().unwrap_or(false));
        self.dirty = true;
        if all {
            self.mc4_award("aBTO");
        }
    }

    // ---- tutorials ----

    /// A tutorial flag has been set (its tutorial shown).
    pub fn mc4_tut_seen(&self, flag: &str) -> bool {
        self.mc4_stat(flag, 0) != 0.0
    }

    /// GetNoTutorialCommandLine (the port's command line: MCLA_NO_TUTORIAL).
    pub fn mc4_no_tutorial() -> bool {
        std::env::var("MCLA_NO_TUTORIAL").is_ok()
    }

    /// The scripts' test and set of a tutorial flag; the tutorial queued. False: seen already.
    pub fn mc4_tutorial(&mut self, flag: &str) -> bool {
        if self.mc4_tut_seen(flag) || Self::mc4_no_tutorial() {
            return false;
        }
        let Some(&(_, form, speech, slow, _)) = TUTORIAL_FLAGS.iter().find(|t| t.0 == flag) else { return false };
        self.mc4_stat_set(flag, 0, 1.0);
        let fresh = self.fresh_game;
        let m = self.mc4.get_or_insert_with(|| Mc4::seeded(fresh));
        if !m.tutorials_shown.iter().any(|t| t == flag) {
            m.tutorials_shown.push(flag.to_string());
            m.tutorials_shown.sort();
        }
        self.dirty = true;
        let modeless = matches!(flag, "tTFL" | "tTFB");
        self.opt_x.s23.queue.push(TutRun { flag: flag.to_string(), form, speech: speech.to_string(), slow, phase: 0, wait: 0.0, modeless });
        log::info!("tutorial {} (form {}) queued", flag, form);
        true
    }

    /// The box of a tutorial form answers CONTINUE (A) (the forms of `TUTORIAL_FLAGS`).
    pub fn mc4_tut_continue(&self, title: &str) -> bool {
        self.opt_x.s23.run.as_ref().map_or(false, |r| r.phase == 2 && !r.modeless && tutorial_keys(r.form).map_or(false, |k| k.0 == title))
    }

    /// The race's end after the line, in real seconds (EndOfRaceSequence and its forms, decoded
    /// 2026-10-19 and timed against a recording of the original - CAREER_RACES.md): after
    /// WAITUNWARPED(200) forms 60 (mcEndOfRaceForm) and 54 (mcStandingsForm, the place) come up
    /// and the camera swings to the standings form's PauseCamera3.
    pub const RACE_END_PANEL: f32 = 0.2;
    /// mcEndOfRaceForm's update (F-79093) pauses the game when its own clock reaches 1 s.
    pub const RACE_END_FREEZE: f32 = 1.2;
    /// The standings panel's WAITUNWARPED(2200) ends: forms 54 and 60 close, the race-over menu.
    pub const RACE_END_MENU: f32 = 2.4;

    /// The global time scale (0x827E1C34): what the simulation's time is multiplied by.
    pub fn mc4_time_scale(&self) -> f32 {
        // (the race's end: the main loop never lets the scale under 0.1 - F-75502)
        // (and a second after its panels came up, form 60 pauses the game: UILogic_PauseGame)
        if self.race_end_t.map_or(false, |t| t >= Self::RACE_END_FREEZE) && self.replay.is_none() {
            return 0.0;
        }
        // (a hangout entry's ease to pause: held at the main loop's floor of 0.1 - F-75502 A-91484)
        self.opt_x.s23.slow.global.clamp(0.0, 1.0).min(self.race_slow.global.clamp(0.1, 1.0)).min(self.hangout_slow.global.clamp(0.1, 1.0))
    }

    /// The race's end in slow motion, once a frame (`dt` real time). EndOfRaceSequence
    /// (RaceOver.sch, every race type's script): the end-of-race speech, WAITUNWARPED(200),
    /// `raceEnding`, form 60 shown, then - when race +3120 is 0 - PerspectiveForm_SetSlowMotion(form 60,
    /// 0, 1 / 0.5, 0, 1, 0.4): to a standstill in half a second, eased out with power 0.4, held at
    /// the main loop's floor of 0.1 until the race is left.
    pub fn race_slow_tick(&mut self, dt: f32) {
        let finished = self.race.as_ref().map_or(false, |r| r.finish_time.first().map_or(false, |t| t.is_some()));
        // (a beat me there run's end slows down as a race's: its EndOfRaceSequence - `Game::bmt_end`)
        let bmt = self.mode == crate::game::Mode::FreeRoam && self.bmt_end.as_ref().map_or(false, |e| !e.wrecked);
        if !bmt && (!matches!(self.mode, crate::game::Mode::Race | crate::game::Mode::Results) || (self.mode == crate::game::Mode::Race && !finished)) {
            if self.race_slow_wait.is_some() || self.race_slow.global != 1.0 {
                self.race_slow = SlowMo::default();
                self.race_slow_wait = None;
            }
            self.race_end_t = None;
            return;
        }
        if let Some(t) = self.race_end_t.as_mut() {
            *t += dt;
        }
        if let Some(w) = self.race_slow_wait.as_mut() {
            *w -= dt;
            if *w <= 0.0 {
                self.race_slow_wait = None;
                self.race_slow.set(0.0, 1.0 / 0.5, false, true, 0.4);
            }
        }
        self.race_slow.update(dt);
    }

    fn mc4_tut_tick(&mut self, dt: f32) {
        if self.fresh_game {
            return;
        }
        let x = &mut self.opt_x.s23;
        // (step 23b: a hangout's own runs over its scene, in its panel's mode)
        let here = self.mode == crate::game::Mode::FreeRoam || (self.mode == crate::game::Mode::RaceSelect && x.hangout_hold.is_some());
        if x.run.is_none() && !x.queue.is_empty() && self.tutorial_box.is_none() && here && self.message_box.is_none() {
            x.run = Some(x.queue.remove(0));
        }
        let Some(mut run) = x.run.take() else { return };
        match run.phase {
            0 => {
                if run.slow {
                    x.slow.ease(0.0);
                    run.wait = 0.5;
                }
                run.phase = 1;
            }
            1 => {
                run.wait -= dt;
                if run.wait <= 0.0 {
                    if let Some((t, d)) = tutorial_keys(run.form) {
                        self.tutorial_box = Some((t, d, 0.0, 600.0));
                    }
                    if !run.speech.is_empty() {
                        self.opt_x.arcade.d.say.push(run.speech.clone());
                    }
                    run.phase = 2;
                }
            }
            _ => {
                let mine = tutorial_keys(run.form).map(|k| k.0);
                let up = self.tutorial_box.as_ref().map_or(false, |b| Some(&b.0) == mine.as_ref());
                if !up {
                    if run.slow {
                        self.opt_x.s23.slow.ease(1.0);
                    }
                    log::info!("tutorial {} over", run.flag);
                    // (a hangout's panel held back for it comes up now)
                    if let Some(k) = self.opt_x.s23.hangout_hold.take() {
                        if self.hangout_key.as_deref() == Some(k.as_str()) {
                            self.mode = crate::game::Mode::RaceSelect;
                        }
                    }
                    return;
                }
            }
        }
        self.opt_x.s23.run = Some(run);
    }

    /// Game/Career/SST_Tutorial (`SstTut`). Started at each start of RACE_GH_06's race (its core runs
    /// the start once per race: hw_ordered_jp_11 against GH_06) unless GetNoTutorialCommandLine. "Racing" is
    /// the script's test: the UI in its Racing state (vhsmState_IsListener), no FlashNavigator transition, not
    /// networked, not paused, no warp, no menu - the port's race under way, no box or menu up.
    fn mc4_sst_tut_tick(&mut self, dt: f32, sst: Option<f32>) {
        use crate::game::Mode;
        let race = self.race.is_some() && self.mode == Mode::Race;
        let was = std::mem::replace(&mut self.opt_x.s23.sst_race_was, race);
        if race && !was && matches!(self.opt_x.s23.sst_tut, SstTut::Idle | SstTut::Done) && !Self::mc4_no_tutorial() && !self.fresh_game {
            let id = self.current_race.and_then(|i| self.race_defs.get(i)).map(|d| d.id.to_ascii_lowercase()).unwrap_or_default();
            if self.hookman_race && self.career.hookman_mission() == Some("RACE_GH_06") && id.ends_with("hw_ordered_jp_11") {
                self.opt_x.s23.sst_tut = SstTut::Charge;
                self.opt_x.s23.sst_poll = 0.0;
                log::info!("SST_Tutorial started (RACE_GH_06)");
            }
        }
        // (the UI's Racing state, tune/gamestate/Racing.sc.xml: entered on mc.missionRace - the core's event right after
        // it starts this script, so the grid and countdown are in it - and left on game.raceEnd / raceEndNoMenu; the game is
        // paused once the end of race's form 60 freezes it, RACE_END_FREEZE after the line)
        let racing = race && self.race_end_t.map_or(true, |t| t < Self::RACE_END_FREEZE) && self.message_box.is_none() && self.tutorial_box.is_none() && !self.in_menu();
        self.mc4_sst_tut_step(dt, sst.unwrap_or(0.0), racing);
    }

    /// SST_Tutorial's steps once it runs (`SstTut`): `racing` the script's test, `level` Racer_GetTurboLevel.
    fn mc4_sst_tut_step(&mut self, dt: f32, level: f32, racing: bool) {
        let title = self.tutorial_box.as_ref().map(|b| b.0.clone());
        let x = &mut self.opt_x.s23;
        // (the script's WAITUNPAUSED(100) between its tests)
        let poll = matches!(x.sst_tut, SstTut::Charge | SstTut::Full) && {
            x.sst_poll += dt;
            if x.sst_poll >= 0.1 {
                x.sst_poll = 0.0;
                true
            } else {
                false
            }
        };
        match x.sst_tut {
            SstTut::Charge if poll && racing && level > 0.3 => {
                x.slow.ease(0.0);
                x.sst_tut = SstTut::Ease(0.5);
                log::info!("SST_Tutorial: level {:.3} > 0.3 - the ease to pause", level);
            }
            SstTut::Ease(t) => {
                if t - dt <= 0.0 {
                    if let Some((t, d)) = tutorial_keys(78) {
                        self.tutorial_box = Some((t, d, 0.0, 600.0));
                    }
                    self.opt_x.arcade.d.say.push("TUTORIAL_SST_INGAME".to_string());
                    self.opt_x.s23.sst_tut = SstTut::Box78;
                } else {
                    x.sst_tut = SstTut::Ease(t - dt);
                }
            }
            SstTut::Box78 if title.as_deref() != Some("tutorial_slipstream_title") => {
                x.slow.ease(1.0);
                x.sst_tut = SstTut::Full;
                x.sst_poll = 0.0;
                log::info!("SST_Tutorial: form 78 closed - the ease from pause");
            }
            SstTut::Full if poll && racing && level > 0.99 => {
                if let Some((t, d)) = tutorial_keys(79) {
                    self.tutorial_box = Some((t, d, 0.0, 600.0));
                }
                self.opt_x.s23.sst_tut = SstTut::Box79;
                log::info!("SST_Tutorial: level {:.3} > 0.99 - form 79", level);
            }
            SstTut::Box79 => {
                // (mcTutorialSlipstreamUseForm's update F-61402: closed once SSTvalue < 0.9 - the burn)
                if title.as_deref() == Some("tutorial_slipstream_use_title") {
                    if level < 0.9 {
                        self.tutorial_box = None;
                        self.opt_x.s23.sst_tut = SstTut::Done;
                        log::info!("SST_Tutorial: level {:.3} < 0.9 - form 79 closed, over", level);
                    }
                } else {
                    x.sst_tut = SstTut::Done;
                }
            }
            _ => {}
        }
    }

    /// RACE_GH_02's tutorial (READY_TO_START_ON_ENTER 0x44f..0x8b3) about the port's lesson:
    /// box 76 after the ease to pause, the log's lesson (GOTO_MISSIONLOG_TUTORIAL ->
    /// MissionLogTutorialScene) with the speech TUTORIAL_MISSION_LOG_GPS, box 77 with
    /// TUTORIAL_TARGET_INGAME, then the ease from pause.
    fn mc4_lesson_tick(&mut self) {
        let title = self.tutorial_box.as_ref().map(|b| b.0.clone()).unwrap_or_default();
        let st = &self.career.stats;
        let in_lesson = title.contains("tutorial_mission_log") || title.contains("tutorial_target") || self.mlog_guided || (st.contains_key("tut:mission_log") && !st.contains_key("tut:target"));
        let x = &mut self.opt_x.s23;
        match x.lesson {
            0 if title.contains("tutorial_mission_log") => {
                x.slow.ease(0.0);
                x.lesson = 1;
                log::info!("RACE_GH_02 lesson: box 76 - SetActiveTimeSlow(1), the ease to pause");
                // (the box comes after the 500 ms of the ease: 0x4a7)
                if let Some(b) = self.tutorial_box.as_mut() {
                    b.2 = b.2.max(0.5);
                }
            }
            1 if self.mlog_guided => {
                x.lesson = 2;
                self.opt_x.arcade.d.say.push("TUTORIAL_MISSION_LOG_GPS".to_string());
            }
            1 | 2 if title.contains("tutorial_target") => {
                x.lesson = 3;
                self.opt_x.arcade.d.say.push("TUTORIAL_TARGET_INGAME".to_string());
            }
            1..=3 if !in_lesson || (x.lesson == 3 && !title.contains("tutorial_target")) => {
                x.slow.ease(1.0);
                x.lesson = 4;
                log::info!("RACE_GH_02 lesson: over - the ease from pause");
            }
            _ => {}
        }
    }

    /// The hookman in reach (CruisingHookmanCore 0x8b1..0xe1f): the first time form 82
    /// (CHALLENGE) else, while tTFB is 0, form 83 (FLASH), shown while he is in reach; flashed,
    /// the form closes and the flag of the one shown is set.
    fn mc4_near_tick(&mut self, near: bool, flashed: bool, kind: u8) {
        // (step 23b: in reach, after the CHALLENGE / FLASH form, the tutorial of the man's type
        // once - CruisingHookmanCore 0x95c..0xd49: type 2 tTRL form 86, 5 tTFR 87, 4 tTTO 95, 3
        // tTTT 96, with the slow motion and its speech; the form takes the place of the
        // CHALLENGE / FLASH one and the man's flash is not taken while it is up, 0xb69)
        if near && self.mode == crate::game::Mode::FreeRoam && !Self::mc4_no_tutorial() {
            let flag = match kind { 2 => "tTRL", 5 => "tTFR", 4 => "tTTO", 3 => "tTTT", _ => "" };
            if !flag.is_empty() && !self.mc4_tut_seen(flag) && self.opt_x.s23.run.is_none() {
                let near_form = |g: &Game| [82u32, 83].iter().any(|f| tutorial_keys(*f).map_or(false, |k| g.tutorial_box.as_ref().map_or(false, |b| b.0 == k.0)));
                if near_form(self) {
                    self.tutorial_box = None;
                }
                self.opt_x.s23.near_shown = true;
                if self.mc4_tutorial(flag) {
                    // (shown at once: ahead of anything queued)
                    if let Some(r) = self.opt_x.s23.queue.pop() {
                        self.opt_x.s23.queue.insert(0, r);
                    }
                }
            }
        }
        let x = &mut self.opt_x.s23;
        if near && !x.near_was {
            x.near_shown = false;
        }
        x.near_was = near;
        let shown = |g: &Game, form: u32| tutorial_keys(form).map_or(false, |k| g.tutorial_box.as_ref().map_or(false, |b| b.0 == k.0));
        // (Form_ShowForm(Form_GetForm(-1), form) 0x946: shown over whatever tutorial box is up - the
        // walkthrough at 1:40.8 has CHALLENGE RACER in the place of the SIDEKICK tip, the phone still
        // up; a slow-motion tutorial of the runner keeps its place)
        if near && !self.opt_x.s23.near_shown && !Self::mc4_no_tutorial() && self.opt_x.s23.run.is_none() && self.mode == crate::game::Mode::FreeRoam {
            let form = if !self.mc4_tut_seen("tTFL") { Some(82) } else if !self.mc4_tut_seen("tTFB") { Some(83) } else { None };
            if let Some((t, d)) = form.and_then(tutorial_keys) {
                log::info!("tutorial form {} shown: a hookman can be flashed", form.unwrap_or(0));
                self.tutorial_box = Some((t, d, 0.0, 600.0));
            }
            self.opt_x.s23.near_shown = true;
        }
        let on = if shown(self, 82) { Some("tTFL") } else if shown(self, 83) { Some("tTFB") } else { None };
        if let Some(flag) = on {
            if flashed {
                self.tutorial_box = None;
                self.mc4_stat_set(flag, 0, 1.0);
                let fresh = self.fresh_game;
                let m = self.mc4.get_or_insert_with(|| Mc4::seeded(fresh));
                if !m.tutorials_shown.iter().any(|t| t == flag) {
                    m.tutorials_shown.push(flag.to_string());
                    m.tutorials_shown.sort();
                }
                self.dirty = true;
            } else if !near {
                self.tutorial_box = None;
            }
        }
    }

    /// A tutorial flag set without its runner (the forms the scripts leave up and close
    /// themselves): the statistic and the block's list.
    fn mc4_tut_flag(&mut self, flag: &str) {
        self.mc4_stat_set(flag, 0, 1.0);
        let fresh = self.fresh_game;
        let m = self.mc4.get_or_insert_with(|| Mc4::seeded(fresh));
        if !m.tutorials_shown.iter().any(|t| t == flag) {
            m.tutorials_shown.push(flag.to_string());
            m.tutorials_shown.sort();
        }
        self.dirty = true;
    }

    /// (step 23b) GasStations (`game/MiscTasks/<station>`, 0x438..0x6b3): in the station (its
    /// trigger of radius 12 sets mcCareer_IsInGasStation), not damaged out, in the cruise
    /// (vhsm state Cruising_Offline; or Racing_Payback), tTGS 0 and the tutorials on: tTGS set,
    /// form 85 (`tutorial_gas_station`) shown - no slow motion, no wait; the script closes it
    /// after the station's menu (form 61) has been up. The port's station has no menu (it fills
    /// on rolling in): the form is closed as the player leaves the station.
    pub(crate) fn mc4_station_tick(&mut self, station: bool) {
        let was = std::mem::replace(&mut self.opt_x.s23.station_was, station);
        let keys = tutorial_keys(85);
        let up = keys.as_ref().map_or(false, |k| self.tutorial_box.as_ref().map_or(false, |b| b.0 == k.0));
        if station && !was && self.mode == crate::game::Mode::FreeRoam && self.race.is_none() && !self.mc4_tut_seen("tTGS") && !Self::mc4_no_tutorial() && self.tutorial_box.is_none() {
            self.mc4_tut_flag("tTGS");
            if let Some((t, d)) = keys {
                self.tutorial_box = Some((t, d, 0.0, 600.0));
            }
            log::info!("tutorial tTGS (form 85) shown at the station");
        } else if !station && was && up {
            self.tutorial_box = None;
        }
    }

    /// FIND_SH_02C (its CORE, 0x56d..0x7b9): within 20 m of his place, not chased, with
    /// $1,000,000 to spend: the box FIND_SH_02C_SPEND_MONEY (YES / NO) - YES spends it, sets
    /// tPAR (every part free) and the meeting's scene plays, the objective met
    /// (FIND_SH_02C_OBJ_01_COMPLETE); NO, or without the money the box _MONEY_RESTRICTION (OK):
    /// he waits until the player has been 30 m away. Returns true when the scene is due.
    pub fn mc4_find02c_meet(&mut self, p: [f32; 3], pursued: bool) -> bool {
        let open = self.career.mission_no("FIND_SH_02C").map_or(false, |i| self.career.log_order().contains(&i));
        if !open {
            return false;
        }
        let d = ((p[0] - FIND_SH_02C_AT[0]).powi(2) + (p[1] - FIND_SH_02C_AT[1]).powi(2) + (p[2] - FIND_SH_02C_AT[2]).powi(2)).sqrt();
        let x = &mut self.opt_x.s23;
        if x.f02c_away {
            if d >= 30.0 {
                x.f02c_away = false;
            }
            return false;
        }
        if x.f02c_ask != 0 || pursued || d >= 20.0 || self.message_box.is_some() {
            return false;
        }
        let rich = self.money >= 1_000_000;
        let def = if rich {
            crate::flash_ui::MessageBoxDef { title: "FIND_SH_02C_SPEND_MONEY_TITLE".into(), description: "FIND_SH_02C_SPEND_MONEY_DESC".into(), options: vec![("yes".into(), "Common_Yes".into()), ("no".into(), "Common_No".into())] }
        } else {
            crate::flash_ui::MessageBoxDef { title: "FIND_SH_02C_MONEY_RESTRICTION_TITLE".into(), description: "FIND_SH_02C_MONEY_RESTRICTION_DESC".into(), options: Vec::new() }
        };
        self.message_box = Some((def, 0, crate::game::MenuAction::None));
        self.opt_x.s23.f02c_ask = if rich { 1 } else { 2 };
        self.opt_x.s23.f02c_yes = false;
        false
    }

    /// The box's YES (from the message box's YES hooks).
    pub fn mc4_s23_box_yes(&mut self) {
        if self.opt_x.s23.f02c_ask == 1 {
            self.opt_x.s23.f02c_yes = true;
        }
        // (step 22b: the pink slip's YES stakes the vehicle before the race starts)
        if self.opt_x.s23.pink_ask.is_some() {
            self.mc4_pink_confirmed();
        }
    }

    /// The box answered: YES pays and the meeting goes on.
    fn mc4_find02c_tick(&mut self) {
        let x = &mut self.opt_x.s23;
        if x.f02c_ask == 0 || self.message_box.is_some() {
            return;
        }
        let yes = x.f02c_ask == 1 && std::mem::take(&mut x.f02c_yes);
        x.f02c_ask = 0;
        if !yes {
            x.f02c_away = true;
            return;
        }
        self.money -= 1_000_000;
        self.mc4_stat_set("tPAR", 0, 1.0);
        self.career.news.push(crate::career::News::Scene("FIND_SH_02C".to_string()));
        self.career.mission_event("FIND_SH_02C_OBJ_01_COMPLETE");
        self.dirty = true;
        log::info!("FIND_SH_02C: $1,000,000 spent, every part free (tPAR)");
    }

    /// Once a frame (the loop, beside the clock's sync): the slow motion, the tutorials, the
    /// stats manager's thresholds and the counters that follow the port's own events.
    pub fn mc4_s23_tick(&mut self, dt: f32, i: S23In) {
        // (test hook, step 23: MCLA_EDITOR_RACE=<n> puts the editor's race number n under
        // --race's start)
        if let Some(n) = std::env::var("MCLA_EDITOR_RACE").ok().and_then(|v| v.parse::<usize>().ok()) {
            if self.race.is_none() {
                if let Some(k) = self.race_defs.iter().position(|d| d.id == format!("user/race_editor_{n}")) {
                    self.race_sel = k;
                    self.current_race = Some(k);
                }
            }
        }
        self.opt_x.s23.slow.update(dt);
        self.mc4_lesson_tick();
        self.mc4_find02c_tick();
        self.mc4_pink_box_tick();
        self.mc4_dt_pink_tick();
        // (a race starts away from the place: its events may be posted again at the next visit)
        if self.mode == crate::game::Mode::Race {
            self.career.visit_events.clear();
        }
        let mode = self.mode;
        let was = self.opt_x.s23.mode_was.replace(mode);
        use crate::game::Mode;
        if !self.fresh_game && self.career.loaded() {
            // (out of the garage: garage_core's tutorials after the autosave - motorcycle,
            // the ability fitted, hydraulics)
            if was == Some(Mode::Garage) && mode == Mode::FreeRoam && self.test_drive.is_none() {
                let car = self.mc4_car_name(self.current_car);
                if self.catalog.get(&car).map_or(false, |c| c.class.eq_ignore_ascii_case("SportBike")) {
                    self.mc4_tutorial("tTMO");
                }
                if self.mc4_abilities_unlocked() && !self.mc4_ability_none() {
                    let k = self.car_x.ability.unwrap_or(self.ability_sel).min(3);
                    self.mc4_tutorial(["tTZO", "tTAG", "tTRO", "tTEM"][k]);
                }
                if self.hydraulics {
                    self.mc4_tutorial("tTHY");
                }
            }
            // (a hangout's panel: its tutorial first - the panel waits)
            if mode == Mode::RaceSelect && was != Some(Mode::RaceSelect) && self.opt_x.s23.hangout_hold.is_none() {
                if let Some(k) = self.hangout_key.clone() {
                    let flag = if k.contains("delivery") { "tTDE" } else if k.contains("payback") { "tTPA" } else if k.contains("wager") { "tTWA" } else if k.contains("local") { "tTSE" } else { "" };
                    // (step 23b: over the hangout's scene - the panel waits under the box,
                    // `mc4_hangout_tut_ui`)
                    if !flag.is_empty() && self.mc4_tutorial(flag) {
                        self.opt_x.s23.hangout_hold = Some(k);
                    }
                }
            }
            // (PINKSLIP RACE chosen: its tutorial before the cars)
            let page = self.hangout_page;
            if mode == Mode::RaceSelect && page.0 == 2 && self.opt_x.s23.page_was.0 != 2 {
                if let Some(k) = self.hangout_key.clone() {
                    if self.mc4_tutorial("tTPS") {
                        self.opt_x.s23.hangout_hold = Some(k);
                    }
                }
            }
            self.opt_x.s23.page_was = page;
            // (the police: the first pursuit form 103, the second 104 - F-24639)
            let heat_was = std::mem::replace(&mut self.opt_x.s23.heat_was, i.heat);
            if i.heat == 2 && heat_was != 2 && heat_was != 3 {
                if !self.mc4_tutorial("tTPP") {
                    self.mc4_tutorial("tTPL");
                }
            }
            // (aRUN: a ticket run from - the pull over turned into a pursuit, F-72120;
            // aEVA and gCPL: the police lost, F-22530)
            if heat_was == 1 && i.heat == 2 {
                self.mc4_award("aRUN");
            }
            if matches!(heat_was, 2 | 3) && i.heat == 0 {
                self.mc4_stat_add("gCPL", 0, 1.0);
                self.mc4_award("aEVA");
            }
            // (the telephone challenge's first call: form 89)
            if self.phone_challenge && !self.opt_x.s23.phone_was {
                self.mc4_tutorial("tTCH");
            }
            self.opt_x.s23.phone_was = self.phone_challenge;
            self.mc4_near_tick(i.near, i.flashed, i.kind);
            // (step 23b: a gas station entered in cruising or a payback race - GasStations
            // 0x438..0x51f: tTGS, form 85, shown and left up without the slow motion; closed by
            // the script once the station's menu has been shown - `mc4_station_tick`)
            self.mc4_station_tick(i.station);
        }
        self.mc4_tut_tick(dt);
        self.mc4_sst_tut_tick(dt, i.sst);
        // (a mission's completion: its script's or its reward's achievement)
        let j = self.career.journal.clone();
        let from = self.opt_x.s23.journal_seen.min(j.len());
        for &(entered, k) in &j[from..] {
            if !entered {
                let id = self.career.mission_id(k).to_string();
                if let Some((_, a)) = ACHIEVEMENT_MISSIONS.iter().find(|m| m.0 == id) {
                    self.mc4_award(a);
                }
            }
        }
        self.opt_x.s23.journal_seen = j.len();
        // (the race editor saved a race: aSRE)
        let n = self.user_races.len();
        if let Some(was) = self.opt_x.s23.races_seen.replace(n) {
            if n > was {
                self.mc4_award("aSRE");
            }
        }
        // (the money: aMIL as the purse grows to $1,000,000 or more)
        // (the purse is held in [-2,140,000,000, 2,140,000,000] and mirrored into the statistic `tPUR` at every change:
        // AddMoney F-71075, SpendMoney F-38145)
        self.money = self.money.clamp(-PURSE_CAP, PURSE_CAP);
        self.career.stats.insert("tPUR".to_string(), self.money);
        if let Some(was) = self.opt_x.s23.money_was.replace(self.money) {
            if self.money > was && self.money >= 1_000_000 {
                self.mc4_award("aMIL");
            }
        }
        // (deliveries: kDLT with the port's count; payback vehicles destroyed: kPDS with a leg won)
        let xcdl = self.career.stats.get("xCDL").copied().unwrap_or(0);
        if let Some(was) = self.opt_x.s23.xcdl_was.replace(xcdl) {
            if xcdl > was {
                self.mc4_stat_add("kDLT", 0, (xcdl - was) as f64);
            }
        }
        let pb = self.career.payback_wins;
        if pb > self.opt_x.s23.pb_wins_was {
            self.mc4_stat_add("kPDS", 0, (pb - self.opt_x.s23.pb_wins_was) as f64);
        }
        self.opt_x.s23.pb_wins_was = pb;
        // (step 22b: garage.EnterTopLevelVinylMenu posts tVYL, add 1 - F-96638 A-55810:
        // the vinyl editor's top menu, garage menu 15, entered)
        let vyl = self.mode == crate::game::Mode::Garage && self.garage_menu == 15 && self.car_x.vinyl_save.is_none() && self.vinyl_pkg.is_none();
        if vyl && !std::mem::replace(&mut self.opt_x.s23.vyl_was, vyl) {
            self.mc4_stat_add("tVYL", 0, 1.0);
        } else {
            self.opt_x.s23.vyl_was = vyl;
        }
        // (the collectables found: gCOL is their count)
        let col = self.collected.len() as f64;
        if col > self.mc4_stat("gCOL", 0) {
            self.mc4_stat_set("gCOL", 0, col);
        }
        // (step 23b: the driving statistics, the showroom's test drive)
        self.mc4b_tick(dt, i.drive);
        // (the stats manager's update: the thresholds at 30 of every 60)
        let f = &mut self.opt_x.s23.stats_frame;
        if *f == 30 {
            self.mc4_achievements_check();
        } else if *f == 60 {
            self.opt_x.s23.stats_frame = 0;
        }
        self.opt_x.s23.stats_frame += 1;
    }
}


// ---- step 22b, item 4: a held car the port staked, written as the original keeps it ----
//
// The lot keeps a held car's whole vehicle record (the garage record's layout, the same as a
// set-up file's: AddPinkSlipProfile copies it, F-58746); its opponent races it as it
// is (OpponentManager_LoadOpponentByCarConfig, wager_hangout_core 0x5566). The port's lot
// keeps the decoded record (`car`); `car_to_xcc` writes it back into the record's bytes - the
// inverse of `Car::from_xcc` field by field, the layers of the vehicle's livery at 0x904
// (the inverse of `vinyl::Livery::from_xcc`) - over the bytes of the set-up file the car was
// made from (what no field of the record names stays as that file has it).

/// f32 -> IEEE half (round to nearest), as the records keep the layers' floats.
pub fn f32_to_half(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = (man | 0x80_0000) >> (1 - e);
        let r = (m + 0x1000) >> 13;
        return sign | r as u16;
    }
    let r = ((e as u32) << 10) + ((man + 0x1000) >> 13);
    sign | r.min(0x7bff) as u16
}

fn xcc_put_f(b: &mut [u8], o: usize, v: &F) {
    b[o..o + 4].copy_from_slice(&(v.0 as f32).to_be_bytes());
}

fn xcc_put_v(b: &mut [u8], o: usize, v: &[F]) {
    for (k, x) in v.iter().enumerate().take(4) {
        xcc_put_f(b, o + 4 * k, x);
    }
}

fn xcc_put_u(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_be_bytes());
}

fn xcc_put_s(b: &mut [u8], o: usize, n: usize, s: &str) {
    let raw = s.as_bytes();
    for k in 0..n {
        b[o + k] = raw.get(k).copied().unwrap_or(0);
    }
}

fn xcc_named(names: &[&str], v: &Value) -> Option<u32> {
    enum_of(names, v).map(|k| k as u32).or_else(|| v.as_u64().map(|x| x as u32))
}

/// A record written into set-up bytes (`base`: the set-up file the car was made from).
pub fn car_to_xcc(c: &Car, base: &[u8], livery: Option<&crate::vinyl::Livery>) -> Option<Vec<u8>> {
    if base.len() != 8176 {
        return None;
    }
    let mut b = base.to_vec();
    let p = &c.performance;
    for (k, n) in PERF_PARTS.iter().enumerate() {
        if let Some(s) = p.stages.get(*n) {
            b[28 + k] = *s;
        }
        if let Some(s) = p.brand.get(*n) {
            b[43 + k] = *s;
        }
    }
    b[41] = p.airbags_fitted;
    b[42] = p.hydraulics_fitted;
    b[59] = p.upgrade_percent as i8 as u8;
    b[60] = p.upgrade_percent_override as i8 as u8;
    for (k, v) in p.unk_percent_61.iter().enumerate().take(3) {
        b[61 + k] = *v as u8;
    }
    let v = &c.visual;
    xcc_put_f(&mut b, 68, &v.m_vInteriorCharacterNeonLightFalloff);
    xcc_put_f(&mut b, 72, &v.m_vInteriorCharacterGaugeConeAng);
    xcc_put_f(&mut b, 76, &v.m_vInteriorCharacterGaugeConeFalloff);
    for (o, x) in [
        (0x50, &v.ExtNeonColor),
        (0x60, &v.IntNeonColor),
        (0x70, &v.DashGaugeColor),
        (0x80, &v.DashGaugeColorB),
        (0x90, &v.RimColorA),
        (0xa0, &v.RimColorB),
        (0xb0, &v.RimColorC),
        (0xc0, &v.RimColorD),
        (0xd0, &v.RimColorE),
        (0xe0, &v.m_vBrushedMetalColorA),
        (0xf0, &v.m_vBrushedMetalColorB),
        (0x100, &v.m_vBrushedMetalColorC),
        (0x110, &v.m_vBrakeCaliperColor),
        (0x120, &v.m_vHeadlightGlassColorNew),
        (0x130, &v.m_vTaillightGlassColorNew),
        (0x140, &v.m_vBumpedHeadlightColor),
        (0x150, &v.m_vBumpedTaillightColor),
        (0x160, &v.m_vWindowTintColorNew),
        (0x800, &v.m_vInteriorCharacterNeonLightPos),
        (0x810, &v.m_vInteriorCharacterGaugeLightPos),
        (0x820, &v.m_vInteriorCharacterGaugeConeDir),
    ] {
        xcc_put_v(&mut b, o, x);
    }
    xcc_put_f(&mut b, 0x170, &v.ExtNeonIntensity);
    xcc_put_f(&mut b, 0x174, &v.IntNeonIntensity);
    let m = &v.RimMaterial;
    for (k, x) in [&m.A, &m.B, &m.C, &m.D, &m.E].iter().enumerate() {
        if let Some(n) = xcc_named(&RIM_MATERIAL, x) {
            xcc_put_u(&mut b, 0x178 + 4 * k, n);
        }
    }
    b[0x18c] = v.ExtNeonFlashRate;
    b[0x18d] = v.IntNeonFlashRate;
    for (k, g) in v.gauge_text.iter().enumerate().take(5) {
        let o = 0x190 + 0x20 * k;
        xcc_put_v(&mut b, o, &g.m_fColor);
        xcc_put_f(&mut b, o + 16, &g.m_fSkew);
        xcc_put_u(&mut b, o + 20, g.m_iFontIdx as u32);
    }
    for (k, l) in v.lights.iter().enumerate().take(19) {
        let o = 0x230 + 0x30 * k;
        xcc_put_v(&mut b, o, &l.LightBulbColor);
        xcc_put_v(&mut b, o + 16, &l.LightCoverColor);
        if let Some(n) = xcc_named(&KLIGHT, &l.kind) {
            xcc_put_u(&mut b, o + 32, n);
        }
    }
    for (k, g) in v.m_vBaseColor.iter().enumerate().take(10) {
        xcc_put_v(&mut b, 0x5c0 + 16 * k, &g.rgb);
    }
    for (k, a) in v.paint.iter().enumerate().take(6) {
        let o = 0x660 + 0x30 * k;
        xcc_put_v(&mut b, o, &a.ColorA);
        xcc_put_v(&mut b, o + 16, &a.ColorB);
        if let Some(n) = xcc_named(&PAINT_TYPE, &a.PaintType) {
            xcc_put_u(&mut b, o + 32, n);
        }
        if let Some(n) = xcc_named(&PAINT_FADE, &a.PaintFade) {
            xcc_put_u(&mut b, o + 36, n);
        }
        b[o + 40] = a.unpainted_flag;
    }
    if let [a, c2] = &v.Camber[..] {
        xcc_put_f(&mut b, 0x830, a);
        xcc_put_f(&mut b, 0x834, c2);
    }
    for (o, x) in [(0x838, &v.TireOffset), (0x83a, &v.RideHeight), (0x83c, &v.TireWidth), (0x83e, &v.RimSize), (0x840, &v.TireProfile)] {
        for (k, y) in x.iter().enumerate().take(2) {
            b[o + k] = *y;
        }
    }
    b[0x844] = v.RimMdlIdx;
    b[0x845] = v.TireTexIdx;
    b[0x846] = v.BrakeMdlIdx;
    if v.plate_letters_raw.len() == 7 {
        b[0x847..0x84e].copy_from_slice(&v.plate_letters_raw);
    }
    b[0x84f] = v.LicenseBG;
    xcc_put_u(&mut b, 0x854, v.m_iCarPaintOverride as u32);
    xcc_put_u(&mut b, 0x858, v.vinyl_preset_group as u32);
    xcc_put_u(&mut b, 0x85c, v.vinyl_preset_index as u32);
    for (k, n) in PARTS.iter().enumerate() {
        if let Some(x) = v.parts.get(*n) {
            b[0x860 + k] = *x;
        }
    }
    b[8068] = v.vinyl_is_user_slot;
    xcc_put_u(&mut b, 8096, c.unk_8096);
    b[8108] = c.ability_idx().map_or(-1i8, |k| k as i8) as u8;
    if !c.TypeName.is_empty() {
        xcc_put_s(&mut b, 8109, 32, &c.TypeName);
    }
    if !c.ProfileName.is_empty() {
        xcc_put_s(&mut b, 8141, 34, &c.ProfileName);
    }
    // the layers, the zones one after the other from 0x904; a free record's shape 0xffff
    if let Some(l) = livery {
        let mut o = 0x904;
        for (z, cap) in crate::vinyl::ZONE_CAPACITY.iter().enumerate() {
            for k in 0..*cap {
                let r = &mut b[o..o + 20];
                match l.zones[z].get(k) {
                    Some(y) => {
                        r[0..2].copy_from_slice(&f32_to_half(y.scale[0]).to_be_bytes());
                        r[2..4].copy_from_slice(&f32_to_half(y.scale[1]).to_be_bytes());
                        r[4..6].copy_from_slice(&f32_to_half(y.pos[0]).to_be_bytes());
                        r[6..8].copy_from_slice(&f32_to_half(y.pos[1]).to_be_bytes());
                        r[8] = y.color[3];
                        r[9] = y.color[0];
                        r[10] = y.color[1];
                        r[11] = y.color[2];
                        r[12..14].copy_from_slice(&f32_to_half(y.skew).to_be_bytes());
                        r[14..16].copy_from_slice(&f32_to_half(y.rot).to_be_bytes());
                        r[16..18].copy_from_slice(&y.shape.to_be_bytes());
                        r[18] = match y.blend {
                            crate::vinyl::Blend::AlphaCutout => 1,
                            crate::vinyl::Blend::NegativeCutout => 2,
                            _ => 0,
                        };
                        r[19] = 0;
                    }
                    None => {
                        r[16] = 0xff;
                        r[17] = 0xff;
                    }
                }
                o += 20;
            }
        }
    }
    Some(b)
}

impl Game {
    /// (step 22b) The bytes of a vehicle the player stakes: its record over its set-up file,
    /// its livery's layers.
    pub fn mc4_record_bytes(&self, rec: &Car) -> Option<Vec<u8>> {
        let base = rec.source_config.as_deref().and_then(|s| self.mc4_setup_file(s)).or_else(|| self.mc4_setup_file(&rec.vehicle))?;
        car_to_xcc(rec, &base, self.liveries.get(&rec.vehicle))
    }
}


// ---- step 22b, item 2: the pink slip's confirmation box ----
//
// wager_hangout_core after the car is chosen (0x4ff8..0x508e): UILogic_SetPrompt(0); then
// MessageBox_ShowMessageBox(0, desc, <the hangout's location key> (title), 3, 0) through its
// helper 0x1900, waiting while the box is visible - desc `pinkslip_confirmation_max` when
// mcCareer_GetNumPinkSlipProfiles(location) is 4 (F-54647: the hangout's own car, if
// loaded, plus the held cars), else `pinkslip_confirmation`. A result other than 3 (YES)
// goes back to the choice; YES leaves the hangout's state and starts the race (0x5095 ->
// 0x63f6, the stake at 0x50a0..).

impl Game {
    /// ACCEPT on the pink slip page: the confirmation box (YES / NO). False always - the race
    /// starts from the box's YES (`mc4_pink_confirmed`). Nothing is asked when the stake
    /// cannot be made (no profile, a single vehicle).
    pub fn mc4_pink_ask(&mut self, key: &str, pick: usize) -> bool {
        let profiles = self.mc4_pink_profiles(key);
        if profiles.is_empty() || self.owned.len() < 2 || self.message_box.is_some() || self.opt_x.s23.pink_ask.is_some() {
            return false;
        }
        let desc = if profiles.len() == 4 { "pinkslip_confirmation_max" } else { "pinkslip_confirmation" };
        let def = crate::flash_ui::MessageBoxDef { title: key.to_string(), description: desc.into(), options: vec![("yes".into(), "Common_Yes".into()), ("no".into(), "Common_No".into())] };
        self.message_box = Some((def, 0, crate::game::MenuAction::StartRace));
        self.opt_x.s23.pink_ask = Some((key.to_string(), pick));
        false
    }

    /// The box's YES: the stake, the race (what ACCEPT did before the box - step 21), the
    /// hangout's panel left; the box's own action starts the race.
    fn mc4_pink_confirmed(&mut self) {
        let Some((key, pick)) = self.opt_x.s23.pink_ask.take() else { return };
        if self.mc4_pink_start(&key, pick) {
            self.current_race = self.mc4_pink_race(&key).or(Some(self.race_sel.min(self.race_defs.len().saturating_sub(1))));
            self.wager_here = false;
            log::info!("hangout: {} pink slip race (confirmed)", key);
            self.hangout_key = None;
            self.hangout_page = (0, 0);
            if let Some(fm) = self.flash_menu.as_mut() {
                fm.hide();
            }
        }
    }

    /// A box of this step answered NO (or put away): what waited for it is dropped.
    fn mc4_pink_box_tick(&mut self) {
        if self.opt_x.s23.pink_ask.is_some() && self.message_box.is_none() {
            self.opt_x.s23.pink_ask = None;
        }
    }
}


// ---- step 22b, item 3: Downtown's scripted pink slip (HANGOUT_PINKSLIP_DOWNTOWN) ----
//
// HANGOUT_PINKSLIP_DOWNTOWN_HANGOUT_ON_ENTER.sco (the mission's DT_WAGER_HANGOUT1_ENTER event
// in CHECKING_OBJECTIVES, every visit while it is open):
//   0x2e5 dt_wager_hangout1_state 1; 0x3c8..0x455 two opponents - [0] SH_04_1_vp_mzd_rx8_06
//   with drv_ma_03_set (SH_04), [1] pinkslip_vp_nsn_350z_06 with drv_ma_001_set (GH_18);
//   0x602 the scene cut_story_hangout_pinkslip_downtown; 0x638 the RX-8 deleted; state 3;
//   0x66e the race (fn 0x688) against opponent [1] with the prize set-up [1]:
//   - two racers (+3080 = 2), the player and GH_18 in pinkslip_vp_nsn_350z_06, difficulty
//     mcCareer_GetDifficultyPlayerVsRacer; the race game/Race/Hollywood/Ordered/hw_ordered_jp_11
//     (0x935);
//   - EvaluatePolice(race, probability 0.0, ...) (0xb85 -> 0xb99): with the probability 0
//     the random draw is above it, so every police car is destroyed - no police in the race
//     (PoliceManager_CreatePolice / SetRaceChaseTarget are in the branch never taken; the
//     call dump's names belong to the creation function's tables);
//   - 0x1180: RAND_INT_RANGE(400, 600) steps; each step of the race (WAITUNWARPED 10, not
//     paused, not warping, no scene) counts one down; below 0 the opponent is deactivated
//     (Racer_Deactivate, 0x1292) and says DAMAGE_HANGOUT_PINKSLIP_DOWNTOWN to the player;
//   - the race over (0x138e..0x1448): the race unlocked, GENERIC_RACE_COMPLETE; the player
//     first: mcRewardManager_BuildRewardCarB(pinkslip_vp_nsn_350z_06) - the 350Z is the
//     prize - and MISSION_EVENT HANGOUT_PINK_DT_OBJ_01_COMPLETE. No vehicle is staked.

pub const DT_PINK_RACE: &str = "game/Race/Hollywood/Ordered/hw_ordered_jp_11";
pub const DT_PINK_SETUP: &str = "pinkslip_vp_nsn_350z_06";
pub const DT_PINK_MAN: &str = "GH_18";

impl Game {
    /// The opponent of Downtown's scripted pink slip while its race runs.
    pub fn mc4_dt_pink_rival(&self) -> Option<(String, String, String)> {
        self.opt_x.s23.dt_pink.as_ref()?;
        Some((DT_PINK_MAN.to_string(), setup_vehicle(DT_PINK_SETUP), DT_PINK_SETUP.to_string()))
    }

    /// The hangout's own pull-in waits while Downtown's scripted pink slip is under way.
    pub fn mc4_dt_pink_busy(&self) -> bool {
        self.career.dt_pink_due || self.opt_x.s23.dt_pink.is_some()
    }

    /// The scene over (no scene for a second): the race starts.
    fn mc4_dt_pink_tick(&mut self) {
        if self.career.dt_pink_due {
            if self.cine.is_some() {
                self.opt_x.s23.dt_wait = 0;
            } else {
                self.opt_x.s23.dt_wait += 1;
            }
            if self.opt_x.s23.dt_wait >= 60 && self.mode == crate::game::Mode::FreeRoam {
                self.career.dt_pink_due = false;
                let at = self.mc4_race_index(DT_PINK_RACE).and_then(|k| self.mc4_table_race(k)).map(|(i, _)| i).or_else(|| self.race_defs.iter().position(|d| d.id.to_ascii_lowercase().ends_with(race_rel(DT_PINK_RACE).to_ascii_lowercase().as_str())));
                if let Some(i) = at {
                    self.current_race = Some(i);
                    self.race_sel = i;
                    // (RAND_INT_RANGE(400, 600): the steps before the opponent gives up)
                    let n = 400 + (self.career.journal.len() as u32 * 7919 + self.money.unsigned_abs() as u32) % 201;
                    self.opt_x.s23.dt_pink = Some(n as i32);
                    self.opt_x.s23.start_race = true;
                    log::info!("HANGOUT_PINKSLIP_DOWNTOWN: race {} against {} in {} ({} steps)", DT_PINK_RACE, DT_PINK_MAN, DT_PINK_SETUP, n);
                }
            }
        }
    }

    /// The loop asks whether a race is to start (Downtown's scripted pink slip).
    pub fn mc4_s23_take_start(&mut self) -> bool {
        std::mem::take(&mut self.opt_x.s23.start_race)
    }

    /// A step of the race (not paused, no scene): the countdown; below 0 the opponent's
    /// racer is deactivated and he speaks. Returns true once, when he stops.
    pub fn mc4_dt_pink_step(&mut self) -> bool {
        let Some(n) = self.opt_x.s23.dt_pink.as_mut() else { return false };
        if self.race.is_none() || *n < 0 {
            return false;
        }
        *n -= 1;
        if *n < 0 {
            self.opt_x.arcade.d.say.push("DAMAGE_HANGOUT_PINKSLIP_DOWNTOWN".to_string());
            log::info!("HANGOUT_PINKSLIP_DOWNTOWN: the opponent is deactivated");
            return true;
        }
        false
    }

    /// The race over: won - the 350Z (BuildRewardCarB) and the objective. True when it was
    /// Downtown's race.
    pub fn mc4_dt_pink_end(&mut self, won: bool) -> bool {
        if self.opt_x.s23.dt_pink.take().is_none() {
            return false;
        }
        self.opt_x.arcade.pink.just_ended = true;
        if won {
            self.career.give_car(DT_PINK_SETUP);
            self.career.mission_event("HANGOUT_PINK_DT_OBJ_01_COMPLETE");
            self.dirty = true;
            log::info!("HANGOUT_PINKSLIP_DOWNTOWN: won - {} given", DT_PINK_SETUP);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    /// F-47840 (the original's rgb to hsv) run offline on its own generated code
    /// (tools/map_hsv_reference.py): 4,716 inputs, every output float bit for bit.
    #[test]
    fn map_hsv_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for l in include_str!("testdata/map_hsv_ref.txt").lines() {
            let t: Vec<&str> = l.split_whitespace().collect();
            let got = super::rgb_hsv([f(t[1]), f(t[2]), f(t[3])]);
            for k in 0..3 {
                assert_eq!(got[k].to_bits(), f(t[4 + k]).to_bits(), "{l} lane {k}");
            }
            n += 1;
        }
        assert_eq!(n, 4716);
    }

    use super::*;

    fn fixture(name: &str) -> Option<Value> {
        let dir = std::env::var("MCLA_SAVE_FIXTURES").unwrap_or_else(|_| "D:/MCLA/work/save/port".to_string());
        let t = std::fs::read_to_string(std::path::Path::new(&dir).join(name)).ok()?;
        serde_json::from_str(&t).ok()
    }

    /// Every key and value of `a` is in `b` (numbers compared as numbers); `path` for the report.
    fn lost(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                for (k, v) in x {
                    match y.get(k) {
                        Some(w) => lost(v, w, &format!("{path}/{k}"), out),
                        None => out.push(format!("{path}/{k} missing")),
                    }
                }
            }
            (Value::Array(x), Value::Array(y)) => {
                if x.len() != y.len() {
                    out.push(format!("{path} length {} -> {}", x.len(), y.len()));
                }
                for (k, (v, w)) in x.iter().zip(y).enumerate() {
                    lost(v, w, &format!("{path}/{k}"), out);
                }
            }
            (Value::Number(x), Value::Number(y)) => {
                if x.as_f64() != y.as_f64() {
                    out.push(format!("{path} {x} -> {y}"));
                }
            }
            _ => {
                if a != b {
                    out.push(format!("{path} {a} -> {b}"));
                }
            }
        }
    }

    /// The block of an imported save through the types and back: nothing lost, nothing added.
    #[test]
    fn block_survives_the_types() {
        for name in ["full_import.json", "newgame_import.json"] {
            let Some(save) = fixture(name) else {
                eprintln!("(no fixture {name}: skipped)");
                continue;
            };
            let block = &save["mc4"];
            let typed = Mc4::from_value(block).expect("the block fits the types");
            let back = typed.to_value();
            let mut diff = Vec::new();
            lost(block, &back, "mc4", &mut diff);
            lost(&back, block, "mc4(added)", &mut diff);
            assert!(diff.is_empty(), "{name}: {:#?}", &diff[..diff.len().min(40)]);
            if name == "full_import.json" {
                assert_eq!(typed.garage.len(), 14);
                assert_eq!(typed.career.missions.len(), 143);
                assert!(typed.car("vpd_chv_impala_64").unwrap().hydraulics());
                assert_eq!(typed.car("vp_sln_s281_06").unwrap().tint_row(), 9);
            }
        }
    }

    /// A game on a copy of `name` in a folder of its own (never the player's save).
    fn game_on(name: &str, tag: &str) -> Option<(Game, String, Value)> {
        let save = fixture(name)?;
        // (the game reads MCLA_SAVE_FILE before its data folder: not in a test)
        std::env::remove_var("MCLA_SAVE_FILE");
        std::env::remove_var("MCLA_NEW_GAME");
        let dir = std::env::temp_dir().join(format!("mcla_mc4_{}_{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(dir.join("mcla_save.json"), serde_json::to_string(&save).ok()?).ok()?;
        let dir = dir.to_string_lossy().to_string();
        let cars: Vec<std::path::PathBuf> = save["owned"].as_array()?.iter().filter_map(|n| n.as_str()).map(std::path::PathBuf::from).collect();
        let mut g = Game::new(cars, 0);
        g.data_dir = dir.clone();
        g.load_save(&dir);
        g.ensure_owned();
        g.sync_car_paint();
        Some((g, dir, save))
    }

    fn saved_block(dir: &str) -> Value {
        let t = std::fs::read_to_string(std::path::Path::new(dir).join("mcla_save.json")).unwrap();
        serde_json::from_str::<Value>(&t).unwrap()["mc4"].clone()
    }

    fn same(a: &Value, b: &Value, what: &str) {
        let mut diff = Vec::new();
        lost(a, b, "mc4", &mut diff);
        lost(b, a, "mc4(added)", &mut diff);
        assert!(diff.is_empty(), "{what}: {:#?}", &diff[..diff.len().min(40)]);
    }

    /// The mandatory one: the 100 % save's import is loaded by the game, saved, loaded again
    /// and saved again - with every vehicle taken out once in between - and the block of the
    /// file is what the importer wrote.
    #[test]
    fn save_cycle_keeps_the_block() {
        for name in ["full_import.json", "newgame_import.json"] {
            let Some((mut g, dir, save)) = game_on(name, "cycle") else {
                eprintln!("(no fixture {name}: skipped)");
                continue;
            };
            assert!(g.mc4.is_some(), "{name}: the block was read");
            g.write_save(&dir);
            same(&save["mc4"], &saved_block(&dir), &format!("{name}, first save"));
            // (every vehicle in use once: its record's state out and in again)
            let n = g.car_dirs.len();
            for k in (0..n).chain([0]) {
                g.current_car = k;
                g.selected_car = k;
                g.sync_car_paint();
            }
            g.write_save(&dir);
            same(&save["mc4"], &saved_block(&dir), &format!("{name}, after every vehicle"));
            let cars = g.car_dirs.clone();
            let mut g2 = Game::new(cars, 0);
            g2.data_dir = dir.clone();
            g2.load_save(&dir);
            g2.ensure_owned();
            g2.sync_car_paint();
            g2.write_save(&dir);
            same(&save["mc4"], &saved_block(&dir), &format!("{name}, second cycle"));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    fn data_dir() -> String {
        std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string())
    }

    /// The set-ups the dev extraction once had under colliding names
    /// (docs/reference/misc/configs_misnamed.md) are read by their real names, and a data folder
    /// built by launcher v0.1.0 / v0.2.0 (the old names) gives the same paint, livery and parts.
    #[test]
    fn misnamed_setups_dress_the_same() {
        let real = std::path::Path::new(&data_dir()).join("resources/vehicle/configs");
        let cars = [
            ("vp_pon_transam_79", "timetrial_vp_pon_transam_79", "1 0 0.82364774 -0.56710166 -0 0.56710166 0.82364774.pt"),
            ("vp_lam_murcielago_06_dub", "vp_lam_murcielago_06_dub", "0.5486715436.wav"),
            ("vp_chv_camaro_06", "SC_GH_36_2_vp_chv_camaro_06", "0x2AA070FD"),
            ("vp_chv_cobalt_05", "SC_GH_36_1_vp_chv_cobalt_05", "0x674FF727"),
        ];
        if !cars.iter().all(|c| real.join(format!("{}.xcc", c.1)).is_file()) {
            return;
        }
        let old = std::env::temp_dir().join(format!("mcla_mc4_oldnames_{}", std::process::id()));
        let _ = std::fs::create_dir_all(old.join("resources/vehicle/configs"));
        for (_, cfg, was) in cars {
            std::fs::copy(real.join(format!("{cfg}.xcc")), old.join("resources/vehicle/configs").join(was)).unwrap();
        }
        let mut g = Game::new(Vec::new(), 0);
        for (car, cfg, _) in cars {
            g.data_dir = data_dir();
            let a = g.mc4_dress_of(car, cfg).unwrap_or_else(|| panic!("{cfg}"));
            let livery = crate::vinyl::Livery::from_xcc(&g.mc4_setup_file(cfg).unwrap());
            g.data_dir = old.to_string_lossy().to_string();
            let b = g.mc4_dress_of(car, cfg).unwrap_or_else(|| panic!("{cfg} (old name)"));
            assert_eq!((a.color, a.tire.clone(), a.brake.clone(), a.inches, a.plate.clone()), (b.color, b.tire, b.brake, b.inches, b.plate), "{cfg}");
            assert_eq!(format!("{:?}", a.cfg), format!("{:?}", b.cfg), "{cfg}");
            assert_eq!(format!("{livery:?}"), format!("{:?}", crate::vinyl::Livery::from_xcc(&g.mc4_setup_file(cfg).unwrap())), "{cfg}");
            // (the files' body colours, 0x660: Trans Am 0.141 0.125 0.094, the DUB Murcielago
            // black, the Camaro 0.639 grey, the Cobalt 0.063 0.094 0.431)
            let body = match cfg { "timetrial_vp_pon_transam_79" => 0.141, "vp_lam_murcielago_06_dub" => 0.0, "SC_GH_36_2_vp_chv_camaro_06" => 0.639, _ => 0.063 };
            let raw = g.mc4_setup_file(cfg).map(|b| f32::from_be_bytes([b[0x660], b[0x661], b[0x662], b[0x663]])).unwrap();
            assert!((raw - body).abs() < 0.001 && a.color[3] == 1.0, "{cfg}: {raw} {:?}", a.color);
        }
        let _ = std::fs::remove_dir_all(&old);
    }

    /// What the game changes reaches the block, per vehicle, and comes back after a save.
    #[test]
    fn changes_reach_the_block() {
        let Some((mut g, dir, _)) = game_on("full_import.json", "changes") else { return };
        let car_no = |g: &Game, name: &str| g.car_dirs.iter().position(|p| p.to_string_lossy() == name).unwrap();
        let take = |g: &mut Game, name: &str| {
            let k = car_no(g, name);
            g.current_car = k;
            g.selected_car = k;
            g.sync_car_paint();
        };
        // (the R8 as the save has it: no hydraulics, stock glass, ZONE)
        assert_eq!((g.hydraulics, g.car_x.airbags, g.tint_level, g.neon_on, g.car_x.ability), (false, false, 0, false, Some(0)));
        assert_eq!(g.tint_rgba(), [0.0, 0.0035, 0.005, 0.372]);
        assert_eq!(g.mc4_brake_name("vp_aud_r8_08").as_deref(), Some("brk_car_brembo_drilled"));
        assert_eq!(g.mc4_brake_name("vp_frd_gt_06").as_deref(), Some("brk_car_brembo_drilled_blank"));
        // (the Impala has both fitted, and its own second colour; the S281 the darkest glass)
        take(&mut g, "vpd_chv_impala_64");
        assert_eq!((g.hydraulics, g.car_x.airbags, g.paint_fade, g.car_x.ability), (true, true, 1, None));
        let b = g.paint_b_linear();
        let want = to_linear([0.686275, 0.14902, 0.07451]);
        assert!((0..3).all(|i| (b[i] - want[i]).abs() < 1e-6), "{b:?}");
        take(&mut g, "vp_sln_s281_06");
        assert_eq!((g.hydraulics, g.tint_level, g.car_x.ability), (false, 9, Some(1)));
        assert_eq!(g.tint_rgba(), [0.0, 0.0035, 0.005, 0.8472]);
        // (changes on the S281: hydraulics and air bags in, neon on in a swatch's colour, tint
        // row 4, the ability ROAR, a second colour and a fade)
        g.hydraulics = true;
        g.car_x.airbags = true;
        g.neon_on = true;
        g.neon_color = 5;
        g.tint_level = 4;
        g.car_x.ability = Some(2);
        g.paint_fade = 2;
        g.paint_b = 4;
        g.mc4_mut().options.airride_pattern = 3;
        take(&mut g, "vp_aud_r8_08");
        assert_eq!((g.hydraulics, g.car_x.airbags, g.neon_on, g.tint_level), (false, false, false, 0), "the R8 keeps its own");
        g.write_save(&dir);
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        let s = m.car("vp_sln_s281_06").unwrap();
        assert_eq!((s.hydraulics(), s.airbags(), s.tint_row(), s.ability_idx()), (true, true, 4, Some(2)));
        assert_eq!(s.visual.m_vWindowTintColorNew[3].0, 0.5832);
        assert_eq!(s.visual.ExtNeonIntensity.0, 1.0);
        let sw = swatch_screen(5);
        assert!((s.visual.ExtNeonColor[0].0 as f32 - sw[0]).abs() < 1e-5);
        assert_eq!(s.visual.paint[0].fade(), Some(2));
        assert_eq!(s.visual.paint[3].fade(), Some(2), "an area that had the body's fade follows it");
        let pb = swatch_screen(4);
        assert!((s.visual.paint[0].ColorB[2].0 as f32 - pb[2]).abs() < 1e-5);
        assert_eq!(m.options.airride_pattern, 3);
        let r8 = m.car("vp_aud_r8_08").unwrap();
        assert_eq!((r8.hydraulics(), r8.tint_row(), r8.neon_ext().0), (false, 0, false));
        assert_eq!(m.car("vpd_chv_impala_64").unwrap().visual.paint[2].fade(), Some(0), "the Impala's hood keeps its own fade");
        // (and a game that reads that save has the S281 as it was left)
        let cars = g.car_dirs.clone();
        let mut g2 = Game::new(cars, 0);
        g2.data_dir = dir.clone();
        g2.load_save(&dir);
        g2.ensure_owned();
        g2.sync_car_paint();
        take(&mut g2, "vp_sln_s281_06");
        assert_eq!((g2.hydraulics, g2.car_x.airbags, g2.neon_on, g2.neon_color, g2.tint_level, g2.car_x.ability, g2.paint_fade, g2.paint_b), (true, true, true, 5, 4, Some(2), 2, 4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The race editor's races keep their slots; a new one takes the first free slot. The
    /// three vinyl packages take a vehicle's layers and give them to another.
    #[test]
    fn editor_races_and_vinyl_packages() {
        let Some((mut g, dir, save)) = game_on("full_import.json", "editor") else { return };
        assert_eq!(g.user_races.len(), 1);
        g.user_races.push(vec![[1.0, 2.0, 3.0], [4.5, 5.0, 6.0], [7.0, 8.0, 9.25]]);
        // (a package of the save onto the R8, and the R8's layers into package 2)
        let before = g.liveries.get("vp_aud_r8_08").cloned();
        assert!(g.mc4_vinyl_load(1));
        let pkg: crate::vinyl::Livery = serde_json::from_value(save["mc4"]["vinyl_packages"][1]["livery"].clone()).unwrap();
        assert_eq!(serde_json::to_value(g.liveries.get("vp_aud_r8_08").unwrap()).unwrap(), serde_json::to_value(&pkg).unwrap());
        assert_eq!(g.vinyls.get("vp_aud_r8_08").map(|s| s.as_str()), Some("custom"));
        assert!(g.mc4_vinyl_store(2));
        g.write_save(&dir);
        let b = saved_block(&dir);
        let m = Mc4::from_value(&b).unwrap();
        assert_eq!(b["editor"]["races"][0], save["mc4"]["editor"]["races"][0], "the save's race is untouched");
        let r = &m.editor.races[1];
        assert_eq!((r.slot, r.name.as_str(), r.count, r.city), (1, "LosAngeles1", 3, 3));
        assert_eq!((r.checkpoints[2].pos[2].0, r.checkpoints[2].activation_radius, r.checkpoints[2].activation_height), (9.25, 25, 25));
        assert_eq!(b["editor"]["received"], save["mc4"]["editor"]["received"]);
        assert_eq!(b["vinyl_packages"][2]["livery"], b["vinyl_packages"][1]["livery"]);
        assert_eq!(b["vinyl_packages"][0], save["mc4"]["vinyl_packages"][0]);
        let _ = before;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With the career's own tables read (the game's data at hand), the 100 % save and the
    /// new game still come back as they went in - and their career state is the game's.
    #[test]
    fn save_cycle_with_the_career_tables() {
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            eprintln!("(no game data: skipped)");
            return;
        }
        for name in ["full_import.json", "newgame_import.json"] {
            let Some((g0, dir, save)) = game_on(name, "career") else { continue };
            let mut g = Game::new(g0.car_dirs.clone(), 0);
            g.data_dir = data_dir();
            g.career = crate::career::Career::load(&data_dir());
            g.load_save(&dir);
            g.ensure_owned();
            g.sync_car_paint();
            let full = name == "full_import.json";
            assert_eq!(g.mc4_police_unlocked(), full, "{name}: police");
            assert_eq!(g.mc4_abilities_unlocked(), full, "{name}: abilities");
            assert_eq!(g.mc4_perf_level_unlocked("A2"), full);
            assert_eq!(g.career.reached_ids.contains("Progression_36"), full);
            if full {
                // (the EMP's level comes from its system's amount, 41: the statistics have no gPUL)
                assert_eq!(g.career.ability_level("Pulse", "gPUL"), 2);
                assert_eq!(g.career.ability_level("Zone", "gZON"), 2);
                assert!(g.mc4_race_unlocked("Hollywood/Ordered/hw_ordered_jp_11"));
                assert_eq!(g.perf_rep(0, 2), 0);
            } else {
                assert_eq!(g.career.ability_level("Zone", "gZON"), 0);
                assert!(g.mc4_race_unlocked("hollywood/ordered/hw_ordered_ds_02"));
            }
            g.write_save(&dir);
            same(&save["mc4"], &saved_block(&dir), &format!("{name}, with the career's tables"));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// The options of the 100 % save reach the game, and the game's changes the block.
    #[test]
    fn options_in_and_out() {
        let Some((mut g, dir, _)) = game_on("full_import.json", "options") else { return };
        // (the save: Boom 1 = the port's far view; idle camera and shake on; midnight)
        let (mut cam, mut hours) = (9u8, 14.0f32);
        g.mc4_camera_sync(&mut cam);
        g.mc4_clock_sync(&mut hours, false);
        assert_eq!(cam, 0);
        assert!((hours - 0.486299).abs() < 1e-6);
        assert!(g.mc4_idle_camera() && g.opts.shake);
        assert_eq!((g.opts.master, g.opts.sens, g.music_genre), (10, 5, 0));
        assert_eq!(g.sens_exact(), 5.0);
        // (an odd step of the original's slider stays, and steers)
        g.mc4_mut().options.set_sensitivity_step(3);
        g.opts.sens = 7;
        assert_eq!(g.sens_exact(), 6.5);
        // (the player goes to the bumper view, turns the idle camera off, the music down)
        cam = 4;
        g.mc4_camera_sync(&mut cam);
        option_step(&mut g.mc4, &mut g.opt_x, 49, true, false);
        g.opts.music = 4;
        g.opts.shake = false;
        g.music_genre = 3;
        hours = 17.25;
        g.mc4_clock_sync(&mut hours, false);
        g.write_save(&dir);
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        let o = &m.options;
        assert_eq!((o.camera, o.camera_name.as_deref()), (0, Some("Bumper")));
        assert_eq!((o.idle_camera, o.camera_shake, o.cinematics), (false, false, 0));
        assert_eq!((o.volume_music.0, o.volume_effects.0), (0.4, 1.0));
        assert_eq!((o.music_genre, o.music_genre_name.as_deref()), (2, Some("MUSIC_GENRE_HARDROCK")));
        assert_eq!(o.steering_sensitivity.0, 0.3);
        assert_eq!(o.time_of_day_hours.0, 17.25);
        // (the rows of CAMERA OPTIONS)
        option_step(&mut g.mc4, &mut g.opt_x, 48, true, false);
        assert_eq!(g.mc4_camera_name(), "POV");
        let mut cam2 = 0u8;
        g.mc4_camera_sync(&mut cam2);
        assert_eq!(cam2, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A save of the port's own (no block) behaves as before and gets a block: one record a
    /// vehicle, made from its set-up file, with the game's state in it.
    #[test]
    fn a_save_without_the_block_gets_one() {
        let Some((mut g, dir, save)) = game_on("port_own_save.json", "own") else { return };
        if !std::path::Path::new(&data_dir()).join("resources/vehicle/configs").is_dir() {
            eprintln!("(no game data: skipped)");
            return;
        }
        g.data_dir = data_dir();
        assert!(g.mc4.is_none());
        let before = (g.hydraulics, g.tint_level, g.neon_on, g.int_neon, g.paint_type, g.paint_fade);
        assert_eq!(before.0, save["hydraulics"].as_bool().unwrap_or(false));
        g.write_save(&dir);
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        assert_eq!(m.garage.len(), g.owned.len());
        let cur = g.mc4_car_name(g.current_car);
        let c = m.car(&cur).expect("the vehicle in use has a record");
        assert_eq!((c.hydraulics(), c.tint_row(), c.neon_ext().0, c.neon_int()), (before.0, before.1.min(9), before.2, before.3));
        assert_eq!(c.origin(), "stock");
        assert!(c.current && m.garage_current_slot == c.slot as i32);
        let cars = g.car_dirs.clone();
        let mut g2 = Game::new(cars, 0);
        g2.data_dir = data_dir();
        g2.load_save(&dir);
        g2.ensure_owned();
        g2.sync_car_paint();
        assert_eq!((g2.hydraulics, g2.tint_level, g2.neon_on, g2.int_neon, g2.paint_type, g2.paint_fade), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A partial block and one with keys the types do not name load, and the keys stay.
    #[test]
    fn partial_block_and_unknown_keys() {
        let v: Value = serde_json::from_str(r#"{"enable_gps": false, "new_key": [1, 2], "options": {"camera": 2, "odd": "x"}, "garage": [{"vehicle": "vp_aud_r8_08", "visual": {"later": 7}}]}"#).unwrap();
        let m = Mc4::from_value(&v).unwrap();
        assert!(!m.enable_gps);
        assert_eq!(m.options.camera, 2);
        assert_eq!(m.options.volume_music.0, 1.0);
        let back = m.to_value();
        assert_eq!(back["new_key"], serde_json::json!([1, 2]));
        assert_eq!(back["options"]["odd"], "x");
        assert_eq!(back["garage"][0]["visual"]["later"], 7);
    }

    #[test]
    fn tint_rows() {
        // (the S281 of the 100 % save: 0.8472 = row 9; stock glass 0.372 reads as row 0)
        assert_eq!(tint_row_of(0.8472), 9);
        assert_eq!(tint_row_of(0.372), 0);
        assert_eq!(tint_alpha(9), 0.8472);
        for row in 0..10 {
            assert_eq!(tint_row_of(tint_alpha(row)), row);
        }
    }

    #[test]
    fn name_hash() {
        // (decode notes: 0x1996E803 Audi R8, 0x040B83A3 Golf)
        assert_eq!(joaat("vp_aud_r8_08"), 0x1996E803);
        assert_eq!(joaat("vp_vw_golf_83"), 0x040B83A3);
    }

    #[test]
    fn purchase_record_sum() {
        let mut r = VehicleRecord::new("vp_aud_r8_08");
        assert_eq!((r.index, r.spent()), (34, 0));
        r.buy("rims", 82, 1200);
        r.buy("rims", 82, 500);
        assert_eq!((r.spent(), r.spent_plus_1), (1200, 1201), "a part already owned adds nothing");
        assert!(r.owns("rims", 82));
        r.buy("tires", 1, 70000);
        assert_eq!(r.spent_plus_1, 0xFFFF);
        // (the 100 % save's records stand at 0: a price p makes the word p)
        let mut z = VehicleRecord { spent_plus_1: 0, ..VehicleRecord::new("vp_frd_gt_06") };
        z.buy("rims", 82, 1200);
        assert_eq!(z.spent_plus_1, 1200);
        // (the trade-in, A-93343..A-99774: (the word + the price - 1) x 0.8, cut)
        assert_eq!(trade_in_value(1201, 147000, false), 118560);
        assert_eq!(trade_in_value(1, 21000, false), 16800);
        assert_eq!(trade_in_value(1201, 147000, true), 0);
    }

    /// Every category of the purchase records by name, as the port's tags and back; the
    /// owned performance stages; a vehicle whose record has no special ability.
    #[test]
    fn owned_parts_and_no_ability() {
        // (a block of the first importer: numbers for most categories)
        let mut r = VehicleRecord::new("vp_aud_r8_08");
        r.owned_parts.insert("cat11".into(), vec![0, 2]);
        r.owned_parts.insert("cat53".into(), vec![0, 1]);
        r.owned_parts.insert("cat5".into(), vec![0]);
        r.owned_parts.insert("cat13".into(), vec![1]);
        r.owned_parts.insert("spoiler".into(), vec![2]);
        r.owned_parts.insert("performance".into(), vec![0, 1, 2, 3, 4, 6]);
        r.upgrade();
        assert_eq!(r.owned_parts.keys().map(|k| k.as_str()).collect::<Vec<_>>(), ["bumper_front", "paint", "performance", "scissor_door", "spoiler"]);
        assert_eq!(r.owned_parts["spoiler"], [1, 2]);
        assert_eq!(r.owned_tags(), ["bumper_front:2", "paint:0", "performance:0", "performance:1", "performance:2", "performance:3", "performance:4", "performance:6", "scissor_door:1", "spoiler:1", "spoiler:2"]);
        // (F-32441: stage 2's bit, else stage 1's)
        assert_eq!((r.perf_owned(0), r.perf_owned(1), r.perf_owned(2), r.perf_owned(3)), (2, 1, 0, 0));
        assert_eq!(PART_CATEGORIES.len(), 27);
        assert_eq!((part_category("rim"), part_category("side_exhaust"), part_category("wheels")), (Some("rims"), Some("side_exhaust"), None));

        let Some((mut g, dir, _)) = game_on("full_import.json", "owned") else { return };
        let has = |g: &Game, car: &str, tag: &str| g.parts_owned.get(car).map_or(false, |v| v.iter().any(|t| t == tag));
        // (the 100 % save: the R8's bought bumpers, doors and exhaust, the Ninja's body parts
        // and its paint shop)
        for tag in ["bumper_front:2", "bumper_rear:2", "skirts:2", "headlight:2", "taillight:2", "spoiler:2", "scissor_door:1", "exhaust:1", "tires:10", "brakes:1", "performance:38"] {
            assert!(has(&g, "vp_aud_r8_08", tag), "R8 {tag}");
        }
        for tag in ["bike_tail:2", "bike_tank:1", "bike_cowl:1", "paint:0", "rim:136", "tires:15"] {
            assert!(has(&g, "vp_kaw_ninja_zx14_06", tag), "Ninja {tag}");
        }
        assert!(!has(&g, "vp_aud_r8_08", "hood:0") && !has(&g, "vp_aud_r8_08", "paint:0"));
        // (the Ninja's fitted stages 2 0 0 0 2 2 2 0 2 0 0 0 2 are what it owns; the S600 DUB
        // owns stage 1 of everything, the Murcielago the stock stages only)
        let ninja: Vec<u8> = (0..13).map(|k| g.perf_owned("vp_kaw_ninja_zx14_06", k)).collect();
        assert_eq!(ninja, [2, 0, 0, 0, 2, 2, 2, 0, 2, 0, 0, 0, 2]);
        assert_eq!((g.perf_owned("vp_mbz_s600_07_dub", 5), g.perf_owned("vp_lam_murcielago_06", 5), g.perf_owned("vp_sln_s7_06", 12)), (1, 0, 2));
        // (the tags come from the block alone: a save imported before the importer wrote them)
        g.parts_owned.clear();
        let block = g.mc4_save();
        g.mc4_load(block);
        assert!(has(&g, "vp_aud_r8_08", "scissor_door:1") && has(&g, "vpd_lr_sport_08", "headlight:1"));
        // (a purchase of the port reaches the record, whatever the section)
        g.mc4_parts_bought("vp_frd_gt_06", &["hood:2".to_string(), "side_exhaust:1".to_string(), "rim:5".to_string(), "wheels:3".to_string()], 100);
        g.mc4_perf_bought("vp_frd_gt_06", &[2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 500);
        let rec = g.mc4.as_ref().unwrap().record("vp_frd_gt_06").unwrap().clone();
        assert!(rec.owns("hood", 2) && rec.owns("side_exhaust", 1) && rec.owns("rims", 5) && rec.owns("rims", 82));
        assert!(rec.owns("performance", 2) && rec.owns("performance", 1) && rec.owns("performance", 4) && !rec.owns("performance", 5));
        assert_eq!((rec.perf_owned(0), rec.perf_owned(1), g.perf_owned("vp_frd_gt_06", 0), g.perf_owned("vp_frd_gt_06", 1), g.perf_owned("vp_frd_gt_06", 2)), (2, 1, 2, 1, 0));
        assert_eq!(rec.spent_plus_1, 600);

        // (the paint shop and the vinyl editor: the 100 % save has the free-parts flag, so
        // nothing is asked or charged - the original's test of the same flag, A-47155)
        let money = g.money;
        // (the flag is the save's statistic tPAR = 1; a game without the career's tables, as here, starts its statistics anew)
        g.career.stats.insert("tPAR".into(), 1);
        assert!(g.parts_free());
        assert!(g.mc4_misc_check(0) && g.mc4_misc_check(1));
        assert_eq!(g.money, money);

        // (the special ability: the R8 has ZONE, the GTO - a prize car nobody gave one - none)
        let take = |g: &mut Game, name: &str| {
            let k = g.car_dirs.iter().position(|p| p.to_string_lossy() == name).unwrap();
            g.current_car = k;
            g.selected_car = k;
            g.sync_car_paint();
        };
        assert!(!g.mc4_ability_none());
        take(&mut g, "vpd_pon_gto_66");
        assert!(g.mc4_ability_none(), "the GTO's record says -1");
        take(&mut g, "vp_sln_s281_06");
        assert!(!g.mc4_ability_none());
        assert_eq!(g.car_x.ability, Some(1));
        take(&mut g, "vp_chv_camaro_06_dub");
        assert!(g.mc4_ability_none());
        // (the garage's SPECIAL ABILITIES page gives it one, and it stays the Camaro's)
        g.car_x.ability = Some(3);
        assert!(!g.mc4_ability_none());
        g.write_save(&dir);
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        assert_eq!((m.car("vp_chv_camaro_06_dub").unwrap().ability_idx(), m.car("vpd_pon_gto_66").unwrap().ability_idx()), (Some(3), None));
        let _ = std::fs::remove_dir_all(&dir);

        // (a new game: the Scirocco's paint shop costs $1000 once - no box to ask in here, so
        // it is bought as chosen - and so does its vinyl editor; with less than $1000, nothing)
        if let Some((mut n, dir, _)) = game_on("newgame_import.json", "owned3") {
            let car = n.mc4_car_name(n.current_car);
            assert_eq!((car.as_str(), n.money, n.parts_free()), ("vp_vw_scirocco_88", 5470, false));
            assert!(n.mc4_misc_check(0));
            assert_eq!(n.money, 4470);
            assert!(n.mc4_misc_check(0), "bought: free from now on");
            assert_eq!(n.money, 4470);
            let rec = n.mc4.as_ref().unwrap().record(&car).unwrap().clone();
            assert!(rec.owns("paint", 0) && !rec.owns("vinyl", 0));
            assert_eq!(rec.spent(), 1000, "the model's sum takes the price: the trade-in counts it");
            n.money = 999;
            assert!(!n.mc4_misc_check(1), "G_OverBudgetErrorMisc");
            assert_eq!(n.money, 999);
            n.money = 1000;
            assert!(n.mc4_misc_check(1));
            assert_eq!(n.money, 0);
            n.write_save(&dir);
            let m = Mc4::from_value(&saved_block(&dir)).unwrap();
            assert!(m.record(&car).unwrap().owns("paint", 0) && m.record(&car).unwrap().owns("vinyl", 0));
            let _ = std::fs::remove_dir_all(&dir);
        }

        // (a save without the block: the ability in hand stays, nothing is owned by a record,
        // and the record its vehicle gets at the first save has that ability)
        let Some((mut p, dir, _)) = game_on("port_own_save.json", "owned2") else { return };
        assert!(p.mc4.is_none() && !p.mc4_ability_none());
        assert_eq!(p.perf_owned(&p.mc4_car_name(p.current_car), 0), 0);
        if std::path::Path::new(&data_dir()).join("resources/vehicle/configs").is_dir() {
            p.data_dir = data_dir();
            p.ability_sel = 2;
            p.write_save(&dir);
            let m = Mc4::from_value(&saved_block(&dir)).unwrap();
            assert_eq!(m.car(&p.mc4_car_name(p.current_car)).unwrap().ability_idx(), Some(2));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The cabin zone tables of the disc's vehicles (their .r83), and the cabin's five colour
    /// groups from the record to the renderer and back.
    #[test]
    fn interior_colour_groups() {
        let dir = std::path::Path::new(&data_dir()).join("resources/vehicle");
        if !dir.join("vp_aud_r8_08").is_dir() {
            eprintln!("(no game data: skipped)");
            return;
        }
        // (the R8: carpet -> C, the plain zones 6 / 7 / 8 -> E / D / C, the leathers 9 / 10 / 11 ->
        // A / B / C, the cloth 16 -> B; 5 = a group the garage does not offer)
        let r8 = zone_groups_dir(&dir.join("vp_aud_r8_08")).expect("the R8 has a table");
        assert_eq!(r8, [2, 5, 5, 5, 5, 5, 4, 3, 2, 0, 1, 2, 1, 2, 5, 2, 1, 5, 5, 5, 5, 5, 5, 5, 5]);
        // (every vehicle of the player's has one, and both of its .r83 hold the same)
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let car = e.file_name().to_string_lossy().to_string();
            if !e.path().is_dir() || !car.starts_with("vp") {
                continue;
            }
            let tables: Vec<[u8; 25]> = std::fs::read_dir(e.path()).unwrap().flatten().map(|f| f.path()).filter(|p| p.extension().map_or(false, |x| x == "r83")).filter_map(|p| zone_groups(&p)).collect();
            if tables.is_empty() {
                // (bikes and the like have no cabin)
                continue;
            }
            assert!(tables.iter().all(|t| *t == tables[0]), "{car}");
            assert!(tables[0].iter().all(|g| *g <= 7), "{car}");
            n += 1;
        }
        assert!(n >= 60, "{n} vehicles with a table");

        let Some((mut g, dir, _)) = game_on("full_import.json", "intcol") else { return };
        // (the save's R8: groups 1..3 black - the stock cabin -, 4 green, 5 white)
        assert_eq!((g.car_x.int_de_idx, g.car_x.int_de_hsv[0].is_some(), g.car_x.int_de_hsv[1].is_some()), ([StripIdx(9), StripIdx(9)], true, true));
        let o = g.interior_groups();
        assert_eq!((o[0][3], o[1][3], o[2][3], o[3][3], o[4][3]), (0.0, 0.0, 0.0, 1.0, 1.0));
        let lin = |v: f32| v.powf(2.2);
        assert!((o[3][0] - lin(0.164706)).abs() < 1e-4 && (o[3][1] - lin(0.517647)).abs() < 1e-4 && o[3][2].abs() < 1e-4, "{:?}", o[3]);
        assert!((o[4][0] - 1.0).abs() < 1e-4);
        // (untouched, the record's colours are what they were)
        g.write_save(&dir);
        let before = fixture("full_import.json").unwrap()["mc4"]["garage"][0]["visual"].clone();
        let after = saved_block(&dir)["garage"][0]["visual"].clone();
        for key in ["m_vBaseColor", "IntNeonColor", "DashGaugeColor", "DashGaugeColorB", "RimColorA", "RimColorB", "RimColorC"] {
            assert_eq!(before[key], after[key], "{key}");
        }
        // (INTERIOR COLOR E mixed to a red, D reset to the stock cabin, A mixed to a blue: the
        // record takes the two that have a colour and keeps what D had)
        g.car_x.int_de_hsv[1] = Some([0.0, 1.0, 0.5]);
        g.car_x.int_de_hsv[0] = None;
        g.slot_hsv[3] = Some([2.0 / 3.0, 1.0, 1.0]);
        let o = g.interior_groups();
        assert_eq!((o[3][3], o[4][3], o[0][3]), (0.0, 1.0, 1.0));
        g.write_save(&dir);
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        let v = &m.car("vp_aud_r8_08").unwrap().visual;
        assert!(close(rgb_of(&v.m_vBaseColor[4].rgb).unwrap(), [0.5, 0.0, 0.0]));
        assert!(close(rgb_of(&v.m_vBaseColor[0].rgb).unwrap(), [0.0, 0.0, 1.0]));
        assert!(close(rgb_of(&v.m_vBaseColor[3].rgb).unwrap(), [0.164706, 0.517647, 0.0]));
        // (and gives E back when the vehicle comes into use again)
        g.mc4_load(Some(saved_block(&dir)));
        g.mc4_take_car("vp_aud_r8_08");
        let h = g.car_x.int_de_hsv[1].unwrap();
        assert!(h[0].abs() < 1e-4 && (h[1] - 1.0).abs() < 1e-4 && (h[2] - 0.5).abs() < 1e-4);
        // (a swatch on the row, no mixed colour)
        g.car_x.int_de_hsv[1] = None;
        g.car_x.int_de_idx[1] = StripIdx(2);
        assert_eq!(g.interior_groups()[4][3], 1.0);
        let _ = std::fs::remove_dir_all(&dir);

        // (a save without the block: D and E are the stock cabin)
        if let Some((p, dir, _)) = game_on("port_own_save.json", "intcol2") {
            let o = p.interior_groups();
            assert_eq!((o[3][3], o[4][3]), (0.0, 0.0));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// The intercooler's switch, the bikes' three parts and the carbon fibre hood: between
    /// the record, the port's `xparts` and the vehicle's model configuration.
    #[test]
    fn intercooler_bike_parts_carbon_hood() {
        // (the lists' names are the string table's by the section's own name)
        let c = crate::career::Career::load(&data_dir());
        if c.strings.get("hood").is_some() {
            assert_eq!((c.strings.get("intercooler"), c.strings.get("bike_cowl"), c.strings.get("bike_tail"), c.strings.get("bike_tank")), (Some("INTERCOOLER"), Some("COWL"), Some("TAIL"), Some("TANK")));
        }
        for (_, sec, _, _) in crate::game::XPARTS.iter().skip(9) {
            assert!(XPART_BYTES.iter().any(|x| x.0 == *sec), "{sec}");
        }
        let Some((mut g, dir, _)) = game_on("full_import.json", "xparts4") else { return };
        let get = |g: &Game, car: &str, key: &str| g.xparts.get(car).and_then(|m| m.get(key)).copied();
        // (the save's Ninja wears tail 2 and tank 2, the stock cowl)
        assert_eq!((get(&g, "vp_kaw_ninja_zx14_06", "bike_tail"), get(&g, "vp_kaw_ninja_zx14_06", "bike_tank"), get(&g, "vp_kaw_ninja_zx14_06", "bike_cowl")), (Some(2), Some(2), Some(0)));
        // (a save imported before the importer wrote these: from the block's records)
        for m in g.xparts.values_mut() {
            for (key, _) in XPART_BYTES {
                m.remove(key);
            }
        }
        let block = g.mc4_save();
        g.mc4_load(block);
        assert_eq!((get(&g, "vp_kaw_ninja_zx14_06", "bike_tail"), get(&g, "vp_kaw_ninja_zx14_06", "bike_tank"), get(&g, "vp_kaw_ninja_zx14_06", "bike_cowl")), (Some(2), Some(2), None));
        assert_eq!(get(&g, "vp_aud_r8_08", "carbon_hood"), None);
        // (X in the hood list: the R8's hood in carbon fibre - the model's configuration has it)
        g.mc4_swap_carbon_fiber();
        assert_eq!(get(&g, "vp_aud_r8_08", "carbon_hood"), Some(1));
        g.sync_extra_parts();
        assert_eq!(g.cfg.extra.get("carbon_hood"), Some(&1));
        // (an intercooler bought for the Range Rover, a cowl for the Ninja)
        g.xparts.entry("vpd_lr_sport_08".into()).or_default().insert("intercooler".into(), 1);
        g.xparts.entry("vp_kaw_ninja_zx14_06".into()).or_default().insert("bike_cowl".into(), 1);
        g.write_save(&dir);
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        assert_eq!(m.car("vp_aud_r8_08").unwrap().part("CarbonFiberHood"), 1);
        // (the switch is UseIntercooler; the intercooler's own byte stays the front bumper's mirror)
        let rr = m.car("vpd_lr_sport_08").unwrap();
        assert_eq!((rr.part("UseIntercooler"), rr.part("intercooler"), rr.part("bumper_f")), (1, 1, 1));
        let nj = m.car("vp_kaw_ninja_zx14_06").unwrap();
        assert_eq!((nj.part("bike_tail"), nj.part("bike_tank"), nj.part("bike_cowl")), (2, 2, 1));
        // (and off again)
        g.mc4_swap_carbon_fiber();
        g.sync_extra_parts();
        assert_eq!(g.cfg.extra.get("carbon_hood"), None);
        g.write_save(&dir);
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        assert_eq!(m.car("vp_aud_r8_08").unwrap().part("CarbonFiberHood"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The record's "vinyl preset" fields are the vinyl generator's pending redraw: at rest
    /// (-1, -1) in every sample's garage and in every set-up file of the disc, and the port
    /// leaves them there.
    #[test]
    fn vinyl_redraw_markers_at_rest() {
        for name in ["full_import.json", "newgame_import.json"] {
            let Some((g, dir, _)) = game_on(name, "vmark") else { continue };
            g.write_save(&dir);
            let m = Mc4::from_value(&saved_block(&dir)).unwrap();
            assert!(!m.garage.is_empty());
            for c in &m.garage {
                assert_eq!((c.visual.vinyl_preset_group, c.visual.vinyl_preset_index), (-1, -1), "{name} {}", c.vehicle);
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
        let dir = std::path::Path::new(&data_dir()).join("resources/vehicle/configs");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut n = 0;
        for e in rd.flatten() {
            let Ok(b) = std::fs::read(e.path()) else { continue };
            if b.len() < 8176 || e.path().extension().map_or(true, |x| x != "xcc") {
                continue;
            }
            let w = |o: usize| i32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
            assert_eq!((w(0x858), w(0x85c)), (-1, -1), "{}", e.path().display());
            n += 1;
        }
        assert!(n == 0 || n > 300, "{n} set-up files");
    }

    /// A vehicle's origin: the hash at record + 8104 is that of the set-up file the record was
    /// made from (F-86225). Every car of the samples resolves to a file of the disc, and
    /// the kinds are the ones the port makes records from (bought, starter, the rewards').
    #[test]
    fn origins_are_setup_files() {
        let configs = std::path::Path::new(&data_dir()).join("resources/vehicle/configs");
        let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
        let mut no_file: Vec<String> = Vec::new();
        for name in ["full_import.json", "newgame_import.json"] {
            let Some(save) = fixture(name) else { continue };
            let m = Mc4::from_value(&save["mc4"]).unwrap();
            for c in &m.garage {
                let cfg = c.source_config.clone().unwrap_or_else(|| panic!("{name} {}: origin not resolved", c.vehicle));
                assert_eq!(c.source_config_hash, format!("0x{:08x}", joaat(&cfg)), "{}", c.vehicle);
                if configs.is_dir() && crate::render::car_lamps::setup_file(std::path::Path::new(&data_dir()), &cfg).is_none() {
                    no_file.push(cfg.to_ascii_lowercase());
                }
                // (the set-up is one of this vehicle)
                assert!(cfg.to_ascii_lowercase().ends_with(&c.vehicle), "{cfg} / {}", c.vehicle);
                *kinds.entry(c.origin()).or_default() += 1;
            }
        }
        // (every one has its file: vp_lam_murcielago_06_dub and vp_sln_s7_06 are among the ten
        // the dev extraction once had under colliding names - docs/reference/misc/configs_misnamed.md)
        if configs.is_dir() && !kinds.is_empty() {
            assert!(no_file.is_empty(), "{no_file:?}");
        }
        if !kinds.is_empty() {
            assert_eq!(kinds.keys().map(|k| k.as_str()).collect::<Vec<_>>(), ["SC_pinkslip", "SC_timetrial", "SC_tournament", "starter", "stock"], "{kinds:?}");
            assert_eq!(kinds["stock"], 11);
        }
    }

    /// The garage's REPAIR boxes: asked before MANAGE / CUSTOMIZE / SHOWROOM for a dented car,
    /// before leaving for a wreck; a row taken without a box repairs for nothing; the way out charges nothing otherwise.
    #[test]
    fn garage_repair_gate() {
        let Some((mut g, dir, _)) = game_on("full_import.json", "repair") else { return };
        let money = g.money;
        let whole = Some(([0.0f32; 4], 0.0f32));
        // (a whole car: every row and the way out pass)
        assert_eq!(g.mc4_repair_gate(true, true, false, false, 1, whole), (true, false));
        assert_eq!(g.mc4_repair_gate(true, false, true, false, 0, whole), (false, true));
        // (scraped, no zone above 0.05, damage 0.4: passes everywhere - and out of the garage as it is)
        let scraped = Some(([0.05f32, 0.0, 0.01, 0.0], 0.4f32));
        assert_eq!(g.mc4_repair_gate(true, true, false, false, 0, scraped), (true, false));
        assert_eq!(g.mc4_repair_gate(true, true, false, true, 3, scraped), (true, false));
        // (a row taken with no zone above 0.05 repairs the car on the way in, free - F-32431 -> F-12181;
        // the way out does not)
        assert_eq!((g.money, g.repair_car, g.car_x.repair_pending), (money, true, None));
        g.repair_car = false;
        assert_eq!(g.mc4_repair_gate(true, false, true, false, 0, scraped), (false, true));
        assert!(!g.repair_car);
        // (a dented zone: CUSTOMIZE VEHICLE asks - no box to ask in here, so it is repaired as
        // chosen, at the fix price - but the way out does not)
        let dented = Some(([0.0f32, 0.3, 0.0, 0.0], 0.2f32));
        assert_eq!(g.mc4_repair_gate(true, false, true, false, 1, dented), (false, true));
        assert_eq!(g.mc4_repair_gate(true, true, false, true, 3, dented), (true, false));
        assert_eq!((g.money, g.repair_car), (money, false));
        let price = g.fix_price(0.2f32.max(0.3 / 4.0), true);
        assert!(price >= 125);
        assert_eq!(g.mc4_repair_gate(true, true, false, false, 1, dented), (true, false));
        assert_eq!((g.money, g.repair_car), (money - price, true));
        // (a wreck: no way out without the repair)
        g.repair_car = false;
        let wreck = Some(([0.0f32; 4], 0.95f32));
        let price = g.fix_price(0.95, false);
        assert_eq!(g.mc4_repair_gate(true, false, true, false, 0, wreck), (false, true));
        assert_eq!((g.money, g.repair_car), (money - g.fix_price(0.2f32.max(0.3 / 4.0), true) - price, true));
        // (not the top menu, or someone else's car: nothing is asked)
        g.repair_car = false;
        assert_eq!(g.mc4_repair_gate(false, true, false, false, 0, dented), (true, false));
        g.borrowed = true;
        assert_eq!(g.mc4_repair_gate(true, true, false, false, 1, dented), (true, false));
        assert!(!g.repair_car);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// STEERING SENS.: twenty-one steps, -10 .. 10, and the steering factor they give.
    #[test]
    fn steering_sensitivity_steps() {
        let Some((mut g, dir, _)) = game_on("full_import.json", "sens") else { return };
        // (the 100 % save: 0.0, the middle)
        assert_eq!((g.mc4_sens_step(), g.opts.sens, g.sens_exact()), (0, 5, 5.0));
        // (up through every step to the end, where it stops)
        let mut seen = vec![g.mc4_sens_step()];
        for _ in 0..12 {
            g.mc4_sens_set(true, false);
            seen.push(g.mc4_sens_step());
        }
        assert_eq!(seen, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 10, 10]);
        assert_eq!((g.opts.sens, g.sens_exact()), (10, 10.0));
        // (down: all twenty-one)
        let mut n = 0;
        while g.mc4_sens_step() > -10 {
            g.mc4_sens_set(false, false);
            n += 1;
        }
        g.mc4_sens_set(false, false);
        assert_eq!((n, g.mc4_sens_step(), g.opts.sens, g.sens_exact()), (20, -10, 0, 0.0));
        // (ACCEPT goes round)
        g.mc4_sens_set(true, true);
        assert_eq!(g.mc4_sens_step(), -9);
        // (an odd step is kept: -9 is 0.5 of the port's old places, and the save has -0.9)
        assert_eq!(g.sens_exact(), 0.5);
        g.write_save(&dir);
        assert_eq!(saved_block(&dir)["options"]["steering_sensitivity"], -0.9);
        for _ in 0..19 {
            g.mc4_sens_set(true, true);
        }
        assert_eq!(g.mc4_sens_step(), 10);
        g.mc4_sens_set(true, true);
        assert_eq!(g.mc4_sens_step(), -10);
        let _ = std::fs::remove_dir_all(&dir);
        // (a save without the block: its place 5 is step 0; the row makes the block)
        if let Some((mut p, dir, _)) = game_on("port_own_save.json", "sens2") {
            let was = p.opts.sens as i32;
            assert_eq!(p.mc4_sens_step(), was * 2 - 10);
            p.mc4_sens_set(true, false);
            assert_eq!(p.mc4_sens_step(), (was * 2 - 10 + 1).min(10));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// The GPS options: the ARROW row's three modes, the top arrow's place and model, and the
    /// switch of the GPS button.
    #[test]
    fn gps_arrow_and_button() {
        let Some((mut g, dir, _)) = game_on("full_import.json", "gps") else { return };
        // (the 100 % save: COMPASS, the button on)
        assert_eq!((g.mc4_gps_arrow(), g.mc4.as_ref().unwrap().enable_gps), (0, true));
        // (GAME OPTIONS, ARROW: compass -> top -> none -> compass; back the other way)
        option_step(&mut g.mc4, &mut g.opt_x, 51, true, false);
        assert_eq!((g.mc4_gps_arrow(), g.mc4.as_ref().unwrap().options.gps_arrow_name.as_deref()), (1, Some("option_arrow_top")));
        option_step(&mut g.mc4, &mut g.opt_x, 51, true, false);
        assert_eq!(g.mc4_gps_arrow(), 2);
        option_step(&mut g.mc4, &mut g.opt_x, 51, true, false);
        assert_eq!(g.mc4_gps_arrow(), 0);
        option_step(&mut g.mc4, &mut g.opt_x, 51, false, false);
        assert_eq!(g.mc4_gps_arrow(), 2);
        g.write_save(&dir);
        assert_eq!(saved_block(&dir)["options"]["gps_arrow"], 2);
        // (the top arrow's square: F-29366 - at 720 lines 256 wide from 512, from line
        // -10 to line 245; the lines' 0.075 less the tuning's 0.09 down from the top, the size
        // by lines / 720, never under 1; a 4:3 screen keeps the 256)
        let a = TopArrowTune::default();
        assert_eq!(top_arrow_rect(&a, 1280.0, 720.0), [512.0, -10.0, 256.0, 255.0]);
        assert_eq!(top_arrow_rect(&a, 1600.0, 900.0), [640.0, -13.0, 320.0, 319.0]);
        assert_eq!(top_arrow_rect(&a, 640.0, 480.0), [192.0, -7.0, 256.0, 255.0]);
        // (the fade: 3 a second up to 1 and down to 0)
        let mut x = CarLive::default();
        top_arrow_fade(&mut x, Some([1.0, 0.0, -2.0]), 0.1);
        assert!((x.top_arrow.0 - 0.3).abs() < 1e-6 && x.top_arrow.1 == [1.0, 0.0, -2.0]);
        top_arrow_fade(&mut x, Some([0.0, 0.0, -5.0]), 1.0);
        assert_eq!(x.top_arrow, (1.0, [0.0, 0.0, -5.0]));
        top_arrow_fade(&mut x, None, 0.25);
        assert!((x.top_arrow.0 - 0.25).abs() < 1e-6 && x.top_arrow.1 == [0.0, 0.0, -5.0], "the arrow keeps its direction while it goes");
        // (the arrow's frame, F-30300: its z axis away from the target, x = z x up, y = x x z)
        let m = top_arrow_axes(glam::Vec3::new(0.0, 0.0, -9.0));
        assert_eq!(m, [glam::Vec3::NEG_X, glam::Vec3::Y, glam::Vec3::Z]);
        let m = top_arrow_axes(glam::Vec3::new(4.0, 0.0, 0.0));
        assert!((m[2] - glam::Vec3::NEG_X).length() < 1e-6 && (m[0] - glam::Vec3::NEG_Z).length() < 1e-6 && (m[1] - glam::Vec3::Y).length() < 1e-6, "{m:?}");
        // (the tuning file and the model of the disc)
        let t = TopArrowTune::load(&data_dir());
        if std::path::Path::new(&data_dir()).join("tune/ui/0xf9e58c68").is_file() {
            assert_eq!((t.scale, t.fovy, t.position, t.pivot, t.lookat, t.blit), (0.18000001, 21.0, [0.0, 0.38500005, -1.8049988], [0.0, 0.0, 0.83333331], [0.0, 0.20000009, -1.0], [0.0, -0.089999989]));
            assert_eq!((t.in_target, t.ambient, t.lights[0].0, t.lights[1].0), (true, [20, 20, 20], [39, 176, 176], [173, 154, 173]));
            assert_eq!((t.color, t.opacity), ([0.0, 0.886275, 0.94902], 0.909804));
            assert_eq!(t, TopArrowTune::default());
            let m = top_arrow_model(&data_dir()).expect("the arrow's model");
            assert_eq!((m.pos.len(), m.tris.len(), m.specular, m.shininess), (162, 84, glam::Vec3::ONE, 30.0));
            // (the ambient shares: the second colour's first byte, 21..255)
            let (lo, hi) = m.amb.iter().fold((f32::MAX, f32::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
            assert!((lo - 21.0 / 255.0).abs() < 1e-6 && hi == 1.0, "{lo} {hi}");
            // (a target straight ahead: the arrow lies over the eye, its point away - the point
            // in the middle of the square and lowest, the feathers above it; all of it inside
            // the square)
            let span = |v: &[TopArrowVertex]| {
                let (x0, x1) = v.iter().fold((f32::MAX, f32::MIN), |a, v| (a.0.min(v.x), a.1.max(v.x)));
                let (y0, y1) = v.iter().fold((f32::MAX, f32::MIN), |a, v| (a.0.min(v.y), a.1.max(v.y)));
                (x0 * 256.0, x1 * 256.0, y0 * 256.0, y1 * 256.0)
            };
            let ahead = top_arrow_vertices(&t, m, [0.0, 0.0, -50.0], 1.0);
            let (x0, x1, y0, y1) = span(&ahead);
            assert!(x0 > 0.0 && x1 < 256.0 && y0 > 0.0 && y1 < 256.0, "{x0} {x1} {y0} {y1}");
            assert!(((x0 + x1) / 2.0 - 128.0).abs() < 1.0 && (y1 - 128.0).abs() < 12.0 && y1 - y0 < 60.0, "{x0} {x1} {y0} {y1}");
            // (the footage's arrow, 1080 lines: 164 x 59 points about (956, 133) - at 720 lines
            // 109 x 39 about line 89; here about line -10 + 100)
            assert!((x1 - x0 - 113.0).abs() < 6.0 && (y1 - y0 - 44.0).abs() < 6.0 && ((y0 + y1) / 2.0 - 10.0 - 90.0).abs() < 4.0, "{x0} {x1} {y0} {y1}");
            // (the point: the model's -z end, the lowest corner, all but on the pivot)
            let tip = (0..m.pos.len()).min_by(|a, b| m.pos[*a].z.total_cmp(&m.pos[*b].z)).unwrap();
            assert!((ahead[tip].x - 0.5).abs() < 0.01 && (ahead[tip].y * 256.0 - y1).abs() < 1.5, "{:?}", ahead[tip]);
            // (a target to the right: the point stays where it was, the feathers swing to the left)
            let right = top_arrow_vertices(&t, m, [50.0, 0.0, 0.0], 1.0);
            let (x0, x1, _, _) = span(&right);
            assert!(x0 < 48.0 && (x1 - 128.0).abs() < 12.0, "{x0} {x1}");
            assert!((right[tip].x - ahead[tip].x).abs() < 0.02 && (right[tip].y - ahead[tip].y).abs() < 0.02, "{:?} {:?}", right[tip], ahead[tip]);
            // (the colours: no red but the ambient's and the gloss; a face away from both
            // lights keeps the ambient share alone; the opacity 0.91..1)
            let c8 = |c: [u8; 3]| glam::Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32) / 255.0;
            for (k, v) in ahead.iter().enumerate() {
                assert!(v.alpha >= t.opacity - 1e-6 && v.alpha <= 1.0 + 1e-6, "{v:?}");
                assert!(v.rgb.cmpge(c8(t.ambient) * m.amb[k] - 1e-6).all(), "{v:?}");
            }
            let [ax, ay, az] = top_arrow_axes(glam::Vec3::new(0.0, 0.0, -50.0));
            let dark = (0..m.pos.len())
                .find(|&k| {
                    let n = ax * m.nrm[k].x + ay * m.nrm[k].y + az * m.nrm[k].z;
                    t.lights.iter().all(|(_, d)| n.dot(-glam::Vec3::from(*d)) <= 0.0)
                })
                .expect("a corner in the shade");
            assert!((ahead[dark].rgb - c8(t.ambient) * m.amb[dark]).length() < 1e-6, "{:?}", ahead[dark]);
            // (the picture: points of the arrow in the middle, nothing in the corners; the
            // brightest face is the material's blue-green, without red)
            let pic = top_arrow_picture(&t, m, 256, [0.0, 0.0, -50.0]);
            assert_eq!(pic.len(), 256 * 256 * 4);
            let at = |x: usize, y: usize| [pic[(y * 256 + x) * 4], pic[(y * 256 + x) * 4 + 1], pic[(y * 256 + x) * 4 + 2], pic[(y * 256 + x) * 4 + 3]];
            assert_eq!((at(2, 2), at(253, 253), at(128, 60), at(128, 140)), ([0; 4], [0; 4], [0; 4], [0; 4]));
            let filled = (0..256 * 256).filter(|k| pic[k * 4 + 3] > 200).count();
            assert!((2200..3800).contains(&filled), "{filled}");
            let wing = at(100, 105);
            assert!(wing[3] > 225 && wing[0] < 40 && wing[1] > 100 && wing[2] > 100, "{wing:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);

        // ---- the GPS button ----
        let on = |base: Option<bool>, gh06: bool, gh02: bool, sh01: bool| gps_button(base, true, gh06, gh02, sh01);
        // (a new game: on until the first race's start switches it off, on again from RACE_GH_02's)
        assert_eq!((on(Some(true), false, false, false), on(Some(true), true, false, false), on(Some(true), true, true, false), on(Some(false), true, false, true)), (true, false, true, true));
        // (a save that says off, before any of the three: off)
        assert!(!on(Some(false), false, false, false));
        // (no career data: as the save says, on without one)
        assert!(gps_button(None, false, false, false, false) && !gps_button(Some(false), false, false, false, false));
        // (a save without the block: the same rule from its career)
        assert_eq!((on(None, true, false, false), on(None, true, true, false)), (false, true));
    }

    /// The music lists: the disc's songs by genre, the orders, shuffle, the custom playlist
    /// and what the save keeps.
    #[test]
    fn music_lists() {
        let Ok(bytes) = std::fs::read(std::path::Path::new(&data_dir()).join("audio/X360/config/game.dat")) else { return };
        let mut l = MusicLists::from_game_dat(&bytes).expect("the songs");
        // (97 songs of the game and 9 of the Social Club's; by genre; ALL has every one)
        assert_eq!(l.songs.len(), 106);
        assert_eq!(l.counts, [12, 16, 16, 17, 17, 13, 15, 106, 0]);
        assert_eq!((l.songs[0].object.as_str(), l.songs[0].id, l.songs[0].genre, l.songs[0].key.as_str()), ("HARD_ROCK_33HZ_IFEELALIVE", 0x32c374dc, 2, "Music_HardRock_33HZ_IFEELALIVE"));
        assert!(l.songs[97..].iter().all(|s| s.key.starts_with("SC_")) && l.songs[..97].iter().all(|s| !s.key.starts_with("SC_")));
        // (made with shuffle on: each order is its list's places, every one once)
        assert!(l.shuffle);
        for g in 0..8 {
            let mut o = l.order[g].arr.clone();
            o.sort();
            assert_eq!((o, l.order[g].count, l.order[g].pos), ((0..l.counts[g]).collect::<Vec<_>>(), l.counts[g], 0));
        }
        assert_eq!((l.order[8].arr.len(), l.order[8].count), (106, 0));
        // (the playing list empty: nothing to step to)
        l.set_genre(8);
        assert_eq!(l.step(true), None);
        l.set_genre(2);
        assert_eq!((l.genre, l.prev_genre), (2, 8));
        // (shuffle off: the list's own order, round and round, both ways)
        l.set_shuffle(false);
        l.play_index(14);
        assert!(std::mem::take(&mut l.play));
        assert_eq!((l.now, l.cur, l.order[2].pos), (Some(l.lists[2][14]), 14, 14));
        let n = l.counts[2];
        assert_eq!((l.step(true), l.cur), (Some(l.lists[2][15]), 15));
        for _ in 0..n - 16 {
            l.step(true);
        }
        assert_eq!(l.cur, n - 1);
        assert_eq!((l.step(true), l.cur), (Some(l.lists[2][0]), 0));
        assert_eq!((l.step(false), l.cur), (Some(l.lists[2][n - 1]), n - 1));
        // (shuffle on: the place of the song that is on becomes the order's head; a round of
        // steps plays every song once)
        l.set_shuffle(true);
        assert_eq!((l.cur, l.order[2].pos), (l.order[2].arr[0], 0));
        let mut seen: Vec<usize> = (0..n).map(|_| l.step(true).unwrap()).collect();
        seen.sort();
        let mut all = l.lists[2].clone();
        all.sort();
        assert_eq!(seen, all);
        // (PLAY with shuffle on: the player stands where that song is in the order)
        l.play_index(3);
        assert_eq!(l.order[2].arr[l.order[2].pos], 3);
        // (the custom playlist: a song once; its order made again each time)
        let (a, b, c) = (l.lists[2][1], l.lists[4][0], l.lists[0][5]);
        assert!(l.add_custom(a) && l.add_custom(b) && !l.add_custom(a) && l.add_custom(c));
        assert_eq!((l.counts[8], l.order[8].count, l.in_custom(b), l.in_custom(l.lists[1][0])), (3, 3, true, false));
        let mut o = l.order[8].arr[..3].to_vec();
        o.sort();
        assert_eq!(o, [0, 1, 2]);
        assert_eq!(l.custom_ids(), [l.songs[a].id, l.songs[b].id, l.songs[c].id]);
        // (a song leaves: the later ones move up; the order keeps its length)
        l.remove_custom(l.songs[a].id);
        assert_eq!((l.counts[8], &l.lists[8][..2], l.order[8].count), (2, &[b, c][..], 3));
        // (it plays, then runs empty: the list before plays again)
        l.set_genre(8);
        assert_eq!((l.genre, l.prev_genre), (8, 2));
        l.remove_custom(l.songs[b].id);
        assert_eq!(l.genre, 8);
        l.remove_custom(l.songs[c].id);
        assert_eq!((l.counts[8], l.genre), (0, 2));
        // (the save: more than 32 songs are kept as the first 32)
        for s in 0..40 {
            l.add_custom(s);
        }
        assert_eq!((l.counts[8], l.custom_ids().len()), (40, 32));
        // (a save read: the ids it has, unknown ones passed over; genre and shuffle)
        let mut m = MusicLists::from_game_dat(&bytes).unwrap();
        m.load(&[0x814a94b9, 0xdeadbeef, 0x32c374dc, 0x814a94b9], 8, false);
        assert_eq!((m.counts[8], m.genre, m.shuffle, m.custom_ids()), (2, 8, false, vec![0x814a94b9, 0x32c374dc]));
        assert_eq!(m.songs[m.find(0x814a94b9).unwrap()].object, "ROCK_BECK_TIMEBOMB");
        // (adding reshuffles the custom order whatever the option says; off straightens it)
        m.set_shuffle(false);
        assert_eq!(&m.order[8].arr[..2], &[0, 1]);
        // (the port's tracks: every song finds its file by the asset's name)
        let assets: Vec<String> = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/music_assets.txt")).map(|t| t.lines().map(|l| l.trim().to_string()).collect()).unwrap_or_default();
        if !assets.is_empty() {
            m.bind_files(&assets);
            assert_eq!(m.songs.iter().filter(|s| s.file.is_none()).map(|s| s.object.as_str()).collect::<Vec<_>>(), Vec::<&str>::new());
            let files: std::collections::HashSet<usize> = m.songs.iter().filter_map(|s| s.file).collect();
            assert_eq!(files.len(), 106);
            m.set_genre(0);
            let f = m.next_file().unwrap();
            assert_eq!(m.title_of_file(f), m.now.map(|s| m.songs[s].title.clone()));
        }
        // (the rows of the menu, the port's numbering: 0 ALL, 1..7 the genres, 8 custom)
        assert_eq!((music_list(0), music_list(1), music_list(7), music_list(8)), (7, 0, 6, 8));
        let x = OptLive { music: Some(std::sync::Arc::new(std::sync::Mutex::new(m))), ..Default::default() };
        assert_eq!((music_rows(&x, 0).unwrap().len(), music_rows(&x, 3).unwrap().len(), music_rows(&x, 8).unwrap().len()), (106, 16, 2));
        assert_eq!((music_shuffle(&x), music_cursor(&x)), (Some(false), Some(0)));
        music_toggle_shuffle(&x);
        assert_eq!(music_shuffle(&x), Some(true));
        // (PLAY on the third song of HARD ROCK; Y takes it into the playlist and out again)
        assert_eq!(music_play(&x, 3, 2), Some(3));
        assert!(!music_in_custom(&x, 3, 2));
        assert_eq!(music_add_remove(&x, 3, 2), Some(3));
        assert!(music_in_custom(&x, 3, 2) && music_rows(&x, 8).unwrap().len() == 3);
        music_add_remove(&x, 3, 2);
        assert!(!music_in_custom(&x, 3, 2));
        {
            let l = x.music.as_ref().unwrap().lock().unwrap();
            assert_eq!((l.genre, l.cur, l.play), (2, 2, true));
        }
        // (a list with nothing in it: no PLAY)
        let y = OptLive { music: Some(std::sync::Arc::new(std::sync::Mutex::new(MusicLists::from_game_dat(&bytes).unwrap()))), ..Default::default() };
        assert_eq!((music_play(&y, 8, 0), music_add_remove(&y, 8, 0)), (None, None));
    }

    /// The options of a save and the music lists: read, changed in the menu, written.
    #[test]
    fn music_options_saved() {
        let Some((mut g, dir, _)) = game_on("full_import.json", "music") else { return };
        let Ok(bytes) = std::fs::read(std::path::Path::new(&data_dir()).join("audio/X360/config/game.dat")) else { return };
        // (the test's game has no disc: the lists are handed to it as `mc4_music` makes them)
        let o = g.mc4.as_ref().unwrap().options.clone();
        assert_eq!((o.music_shuffle, o.music_genre, o.custom_playlist.len()), (true, 7, 0));
        let mut l = MusicLists::from_game_dat(&bytes).unwrap();
        l.load(&[], o.music_genre as usize, o.music_shuffle);
        g.opt_x.music = Some(std::sync::Arc::new(std::sync::Mutex::new(l)));
        // (untouched: the block is written as it came)
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!((b["options"]["music_shuffle"].clone(), b["options"]["music_genre"].clone(), b["options"]["custom_playlist"].as_array().map(|a| a.len())), (Value::Bool(true), Value::from(7), Some(0)));
        // (shuffle off, two songs into the playlist, PLAY in it)
        music_toggle_shuffle(&g.opt_x);
        music_add_remove(&g.opt_x, 5, 0);
        music_add_remove(&g.opt_x, 1, 3);
        g.music_genre = music_play(&g.opt_x, 8, 1).unwrap();
        assert_eq!(g.music_genre, 0);
        assert!(g.mc4_music_take_play() && !g.mc4_music_take_play());
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!((b["options"]["music_shuffle"].clone(), b["options"]["music_genre"].clone(), b["options"]["music_genre_name"].clone()), (Value::Bool(false), Value::from(8), Value::from("MUSIC_GENRE_ON_THE_FLY")));
        let ids: Vec<String> = b["options"]["custom_playlist"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!((ids.len(), ids[0].as_str()), (2, "0x814a94b9"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The language of a save: the string table's block, English where a block is not there.
    #[test]
    fn language_of_the_save() {
        let Ok(d) = std::fs::read(std::path::Path::new(&data_dir()).join("ui/0xa4c1e8fa")) else { return };
        let mut t = mcla_formats::strtbl::StringTable::parse(&d).expect("the string table");
        let text = |t: &mut mcla_formats::strtbl::StringTable, lang: usize| {
            t.set_lang(lang);
            (t.lang, t.get("Common_Yes").unwrap_or("").to_string(), t.get("option_arrow").unwrap_or("").to_string())
        };
        // (the blocks in the option's order: english, spanish, french, german, italian)
        assert_eq!(text(&mut t, 0), (0, "YES".into(), "ARROW".into()));
        assert_eq!(text(&mut t, 1).2, "FLECHA");
        assert_eq!(text(&mut t, 2), (2, "OUI".into(), "FL\u{c8}CHE".into()));
        assert_eq!(text(&mut t, 3), (3, "JA".into(), "PFEIL".into()));
        assert_eq!(text(&mut t, 4).2, "FRECCIA");
        // (a block the table has not: English)
        assert_eq!(text(&mut t, 9), (0, "YES".into(), "ARROW".into()));
        assert_eq!(text(&mut t, 40).0, 0);
        // (a save's language reaches the game's table; the 100 % save is English)
        let Some((mut g, dir, _)) = game_on("full_import.json", "lang") else { return };
        assert_eq!((g.mc4.as_ref().unwrap().options.language, g.career.strings.lang), (0, 0));
        g.career.strings = t;
        g.mc4_mut().options.language = 3;
        g.mc4_options_in();
        assert_eq!((g.career.strings.lang, g.career.strings.get("Common_Yes")), (3, Some("JA")));
        // (and it is kept)
        g.write_save(&dir);
        assert_eq!(saved_block(&dir)["options"]["language"], 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The disc's controller list and maps.
    #[test]
    fn controller_maps_of_the_disc() {
        let maps = control_maps(&data_dir());
        if maps.is_empty() {
            return;
        }
        assert_eq!(maps.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), CONTROL_CONFIGS);
        assert_eq!(maps.iter().map(|m| m.manual).collect::<Vec<_>>(), [false, false, true, true, false, false, true, false, false]);
        // (every map was found, and the custom ones start from their base files)
        assert!(maps.iter().all(|m| m.slots.len() >= 50), "{:?}", maps.iter().map(|m| m.slots.len()).collect::<Vec<_>>());
        assert_eq!((maps[4].slots == maps[0].slots, maps[7].slots == maps[5].slots), (true, true));
        // (the default style: triggers for the pedals, A handbrake, B weight, X nitrous)
        let t = &maps[1].slots;
        assert_eq!((t["GAS"], t["BRAKE"], t["HANDBRAKE"], t["WEIGHT_XFER"], t["NITRO"], t["STEER_AXIS"]), ((8, 1), (8, 0), (8, 6), (8, 5), (8, 7), (9, 0)));
        assert_eq!((t["SHIFT_UP"], t["LOOKBACK"], t["CONVERTIBLE_UP_DN"]), ((0, 0), (9, 27), (0, 0)));
        // (stick: the right stick is the pedals; manual: the gears on the shoulders or the stick)
        assert_eq!((maps[0].slots["GAS"], maps[0].slots["BRAKE"]), ((9, 26), (9, 27)));
        assert_eq!((maps[2].slots["SHIFT_UP"], maps[2].slots["SHIFT_DOWN"], maps[3].slots["SHIFT_UP"], maps[3].slots["SHIFT_DOWN"]), ((8, 1), (8, 0), (9, 26), (9, 27)));
        // (the hydraulics' map: the four wheels on the shoulders and triggers)
        let h = &maps[8].slots;
        assert_eq!((h["WHEEL_FRONT_LEFT"], h["WHEEL_FRONT_RIGHT"], h["WHEEL_BACK_LEFT"], h["WHEEL_BACK_RIGHT"]), ((8, 2), (8, 3), (8, 0), (8, 1)));
    }

    /// The pad through a map: bytes, values, the active map, manual gears, SET.
    #[test]
    fn controller_style_and_pad() {
        // (the poll: sticks' bytes with up = 0, the triggers' bits from 64 on)
        let p = PadState::new([0.5, 1.0, 0.0, -1.0], [0.2, 1.0], 1 << 6 | 1 << 12);
        assert_eq!((p.sticks, p.triggers, p.buttons), ([191, 0, 128, 255], [51, 255], 1 << 6 | 1 << 12 | 2));
        // (buttons: 255 or 0, a trigger its own byte; the left trigger at 51 is not "down")
        assert_eq!((p.byte(8, 6), p.byte(8, 7), p.byte(8, 0), p.byte(8, 1)), (255, 0, 51, 255));
        // (axes: the sticks raw; the half-axes; the d-pad as axes)
        assert_eq!((p.byte(9, 0), p.byte(9, 1), p.byte(9, 22), p.byte(9, 23), p.byte(9, 25), p.byte(9, 24)), (191, 0, 255, 0, 127, 0));
        assert_eq!((p.byte(9, 26), p.byte(9, 27), p.byte(9, 20), p.byte(9, 21)), (0, 255, 128, 0));
        // (the stick as switches: up, not right - 0.5 is not past 0.8)
        assert_eq!((p.byte(9, 4), p.byte(9, 5), p.byte(9, 7), p.byte(9, 13)), (255, 0, 0, 255));
        let maps = control_maps(&data_dir());
        if maps.is_empty() {
            return;
        }
        // (a save without the block: the trigger style)
        let t = control_slots(&maps, None, false);
        assert_eq!(t, maps[1].slots);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
        // (gas on the right trigger, whole; the brake at 51 / 255 past the dead zone of 0.05)
        assert!(near(pad_analog(&p, &t, "GAS"), 1.0) && near(pad_analog(&p, &t, "BRAKE"), (0.2 - 0.05) / 0.95));
        // (steering: (191 - 127.5) / 127.5 past 0.1; the lean axis: the stick up is -1)
        assert!(near(pad_analog(&p, &t, "STEER_AXIS"), (63.5 / 127.5 - 0.1) / 0.9) && near(pad_analog(&p, &t, "STEER_AXIS_UP_DN"), -1.0));
        assert!(pad_down(&p, &t, "HANDBRAKE") && !pad_down(&p, &t, "NITRO") && pad_down(&p, &t, "MAP") && pad_down(&p, &t, "LOOKBACK") && !pad_down(&p, &t, "BRAKE"));
        // (not bound: nothing; CONVERTIBLE_UP_DN is unbound in every file of the disc)
        assert!(!pad_down(&p, &t, "SHIFT_UP") && pad_analog(&p, &t, "CAM_DOWN") == 0.0 && maps.iter().all(|m| m.slots["CONVERTIBLE_UP_DN"] == (0, 0)));
        // (manual gears are the map's: both shifts bound)
        assert_eq!(maps.iter().map(|m| pad_manual(&m.slots)).collect::<Vec<_>>(), [false, false, true, true, false, false, true, false, false]);
        // (the stick style: the right stick is the pedals - pulled back, the brake)
        let s = control_slots(&maps, Some(&Options { control_config_pad: 0, ..Options::default() }), false);
        assert!(near(pad_analog(&p, &s, "BRAKE"), 1.0) && pad_analog(&p, &s, "GAS") == 0.0);
        // (the hydraulics switched on: the ninth map, whatever the style)
        for style in [0, 1, 3, 4] {
            assert_eq!(control_slots(&maps, Some(&Options { control_config_pad: style, ..Options::default() }), true), maps[8].slots);
        }
        // (the custom style: the save's own slots; one never filled starts as the stick's file)
        let mut o = Options { control_config_pad: 4, ..Options::default() };
        assert_eq!(control_slots(&maps, Some(&o), false), maps[0].slots);
        o.custom_mapping_pad.insert("GAS".into(), Mapping { slot: 2, device: "button".into(), key: 6, ..Default::default() });
        o.custom_mapping_pad.insert("BRAKE".into(), Mapping { slot: 3, device: "none".into(), key: 0, ..Default::default() });
        o.custom_mapping_pad.insert("NITRO".into(), Mapping { slot: 8, device: "axis".into(), key: 27, ..Default::default() });
        let c = control_slots(&maps, Some(&o), false);
        assert_eq!((c.len(), c["GAS"], c["BRAKE"], c["NITRO"]), (3, (8, 6), (0, 0), (9, 27)));
        assert!(near(pad_analog(&p, &c, "GAS"), 1.0) && pad_down(&p, &c, "NITRO") && !pad_down(&p, &c, "HANDBRAKE"));
        // (the 100 % save: its player's own layout - the custom style, with the nitrous on LB,
        // the ability on L3, the roof on the d-pad's down and no button for the hydraulics)
        if let Some((mut g, dir, _)) = game_on("full_import.json", "pad") {
            let o = g.mc4.as_ref().unwrap().options.clone();
            assert_eq!((o.control_config_pad, o.control_config_pad_name.as_deref()), (4, Some("Custom_Pad")));
            let custom = control_slots(&maps, Some(&o), false);
            assert_eq!((custom["GAS"], custom["BRAKE"], custom["NITRO"], custom["WEIGHT_XFER"], custom["IN_ZONE"], custom["HEADLIGHTS"], custom["CAM_UP"]), ((8, 1), (8, 0), (8, 2), (8, 7), (8, 9), (8, 5), (8, 4)));
            assert_eq!((custom["CONVERTIBLE_UP_DN"], custom["HYDRAULICS"], custom["SHIFT_UP"], pad_manual(&custom)), ((8, 14), (0, 0), (0, 0), false));
            // (the game's own reading of it, and the hydraulics' map over it)
            g.opt_x.control_maps = Some(maps.clone());
            assert_eq!((g.mc4_controls(false), g.mc4_controls(true)), (custom.clone(), maps[8].slots.clone()));
            // (SET on STICK - MANUAL; again does nothing; a wheel's row does nothing; saved)
            assert!(control_set(&mut g.mc4, 2, false) && !control_set(&mut g.mc4, 2, false) && !control_set(&mut g.mc4, 5, false));
            g.write_save(&dir);
            let b = saved_block(&dir);
            assert_eq!((b["options"]["control_config_pad"].clone(), b["options"]["control_config_pad_name"].clone()), (Value::from(2), Value::from("Throttle_Stick_Manual")));
            let _ = std::fs::remove_dir_all(&dir);
        }
        // (a save without the block: SET makes it)
        let mut none: Option<Mc4> = None;
        assert!(!control_set(&mut none, 1, false) && control_set(&mut none, 0, false));
        assert_eq!(none.unwrap().options.control_config_pad, 0);
    }

    /// The 100 % save's own mapping, by slot (the importer's listing).
    fn custom_100() -> SlotMap {
        let mut m: SlotMap = [(0, 0); 74];
        for (slot, d, k) in [(0, 9, 0), (1, 9, 1), (2, 8, 1), (3, 8, 0), (6, 8, 6), (7, 8, 7), (8, 8, 2), (9, 8, 5), (10, 8, 10), (11, 8, 9), (12, 8, 4), (14, 8, 12), (17, 8, 11), (18, 8, 10), (19, 8, 13), (20, 8, 15), (22, 8, 15), (23, 8, 13), (24, 8, 12), (25, 8, 14), (26, 9, 27), (27, 9, 28), (28, 9, 29), (29, 9, 26), (43, 8, 14), (64, 9, 26), (65, 9, 27), (66, 9, 28), (67, 9, 29), (68, 8, 1), (69, 8, 0), (70, 9, 22), (71, 9, 23), (72, 9, 24), (73, 9, 25)] {
            m[slot] = (d, k);
        }
        m
    }

    /// The mapping editor's rules: rows, the poll's order, the assignment, "must be mapped".
    #[test]
    fn controller_editor_rules() {
        // (the rows: slots 0..44 that are not hidden, 23 of them)
        assert_eq!((0..45).filter(|s| !control_hidden(*s)).collect::<Vec<_>>(), CONTROL_ROWS);
        assert_eq!(CONTROL_ROWS.iter().map(|s| action_name(*s).unwrap()).collect::<Vec<_>>()[..4], ["STEER_AXIS", "GAS", "BRAKE", "SHIFT_UP"]);
        let base = custom_100();
        let on = |m: &SlotMap, d: u32, k: u32, slot: usize| {
            let mut c = *m;
            let ok = control_assign(d, k, slot, &mut c, false);
            (ok, c)
        };
        // (BACK and START never)
        assert!(!on(&base, 8, 11, 8).0 && !on(&base, 8, 8, 8).0 && !on(&base, 8, 8, 2).0);
        // (a button another shown action has: that one is emptied, no swap)
        let (ok, m) = on(&base, 8, 7, 8);
        assert_eq!((ok, m[8], m[7]), (true, (8, 7), (0, 0)));
        // (a hidden slot with the same button is left alone: TALK keeps R3)
        let (ok, m) = on(&base, 8, 10, 44);
        assert_eq!((ok, m[44], m[10], m[18]), (true, (8, 10), (0, 0), (8, 10)));
        // (LOOKBACK not on a d-pad button; on RB it is)
        assert!(!on(&base, 8, 12, 26).0 && on(&base, 8, 3, 26).0);
        // (not the steering's stick; the right stick is free while the pedals are buttons -
        // and takes it from CAMERA_UP)
        assert!(!on(&base, 9, 24, 8).0 && !on(&base, 9, 0, 8).0);
        let (ok, m) = on(&base, 9, 26, 8);
        assert_eq!((ok, m[8], m[29]), (true, (9, 26), (0, 0)));
        // (STEER_AXIS: an axis only; the left stick or the d-pad, with the lean axis; the
        // d-pad's buttons are cleared everywhere, hidden slots too)
        assert!(!on(&base, 8, 6, 0).0 && !on(&base, 9, 2, 0).0 && !on(&base, 9, 26, 0).0 && on(&base, 9, 1, 0).0);
        let (ok, m) = on(&base, 9, 20, 0);
        assert_eq!((ok, m[0], m[1]), (true, (9, 20), (9, 21)));
        assert_eq!([19, 20, 14, 43, 22, 23, 24, 25].map(|s| m[s]), [(0, 0); 8]);
        // (... and with the d-pad steering no action takes a d-pad button, nor the left stick)
        assert!(!on(&m, 8, 13, 10).0 && !on(&m, 9, 24, 8).0 && on(&m, 8, 3, 10).0);
        // (a wheel has no d-pad steering)
        let mut w = base;
        assert!(!control_assign(9, 20, 0, &mut w, true) && w == base);
        // (GAS on the right stick: the pair, GAS up and BRAKE down, the stick's users cleared;
        // then no other action on that stick)
        let (ok, m) = on(&base, 9, 3, 2);
        assert_eq!((ok, m[2], m[3]), (true, (9, 26), (9, 27)));
        assert_eq!([26, 27, 28, 29].map(|s| m[s]), [(0, 0); 4]);
        assert!(!on(&m, 9, 28, 8).0 && !on(&m, 9, 27, 26).0);
        // (not the steering's axes or the d-pad)
        assert!(!on(&base, 9, 0, 2).0 && !on(&base, 9, 1, 3).0 && !on(&base, 9, 20, 2).0 && !on(&base, 9, 21, 3).0);
        // (a trigger: the pair - the pressed one to the edited row - and the triggers' other
        // users cleared; from the stick file, where the triggers are the side looks)
        let maps = control_maps(&data_dir());
        if !maps.is_empty() {
            let stick = slots_array(&maps[0].slots);
            assert_eq!((stick[2], stick[3], stick[27], stick[28]), ((9, 26), (9, 27), (8, 0), (8, 1)));
            let (ok, m) = on(&stick, 8, 1, 3);
            assert_eq!((ok, m[2], m[3], m[27], m[28]), (true, (8, 0), (8, 1), (0, 0), (0, 0)));
            let (ok, m) = on(&stick, 8, 1, 2);
            assert_eq!((ok, m[2], m[3]), (true, (8, 1), (8, 0)));
            // (the defaults of the custom row are the stick file)
            assert_eq!(slots_array(&maps[4].slots), stick);
        }
        // (a pedal alone on LB: NITRO loses it, the other pedal stays; then no action takes LB
        // or RB - and, as the original orders its tests, LOOKBACK is then let onto the d-pad)
        let (ok, m) = on(&base, 8, 2, 2);
        assert_eq!((ok, m[2], m[3], m[8]), (true, (8, 2), (8, 0), (0, 0)));
        assert!(!on(&m, 8, 3, 10).0 && !on(&m, 8, 2, 10).0 && on(&m, 8, 12, 26).0);
        // (the poll. No axis action: the lowest new button first, then the sticks as halves)
        let pad = |sticks: [f32; 4], triggers: [f32; 2], buttons: u16| PadState::new(sticks, triggers, buttons);
        let rest = pad([0.0; 4], [0.0; 2], 0);
        assert_eq!(control_poll(8, &rest, 0), None);
        assert_eq!(control_poll(8, &pad([0.0; 4], [0.0; 2], 1 << 6 | 1 << 7), 0), Some((8, 6)));
        assert_eq!(control_poll(8, &pad([0.0; 4], [0.0; 2], 1 << 6 | 1 << 7), 1 << 6), Some((8, 7)));
        assert_eq!(control_poll(8, &pad([0.0; 4], [0.0; 2], 1 << 6), 1 << 6), None);
        assert_eq!(control_poll(8, &pad([-0.8, 0.0, 0.0, 0.0], [0.0; 2], 0), 0), Some((9, 24)));
        assert_eq!(control_poll(8, &pad([0.0, 0.8, 0.0, 0.0], [0.0; 2], 0), 0), Some((9, 22)));
        assert_eq!(control_poll(8, &pad([0.0, 0.0, 0.8, -0.8], [0.0; 2], 0), 0), Some((9, 29)));
        assert_eq!(control_poll(8, &pad([0.0, 0.0, 0.0, -0.8], [0.0; 2], 0), 0), Some((9, 27)));
        assert_eq!(control_poll(8, &pad([0.4, 0.0, 0.0, 0.0], [0.0; 2], 0), 0), None);
        assert_eq!(control_poll(8, &pad([0.0; 4], [0.0, 0.5], 0), 0), Some((8, 1)));
        // (STEER_AXIS, GAS, BRAKE: LB / RB first for the pedals, a stick 0.3 off as the whole
        // axis, the d-pad as an axis, then a button)
        assert_eq!(control_poll(2, &pad([0.4, 0.0, 0.0, 0.0], [0.0; 2], 1 << 2), 0), Some((8, 2)));
        assert_eq!(control_poll(0, &pad([0.4, 0.0, 0.0, 0.0], [0.0; 2], 1 << 2), 0), Some((9, 0)));
        assert_eq!(control_poll(0, &rest, 0), None);
        assert_eq!(control_poll(3, &pad([0.0, 0.0, 0.0, 0.4], [0.0; 2], 1 << 6), 0), Some((9, 3)));
        assert_eq!(control_poll(2, &pad([0.2, 0.0, 0.4, 0.0], [0.0; 2], 0), 0), Some((9, 2)));
        assert_eq!(control_poll(0, &pad([0.0; 4], [0.0; 2], 1 << 15 | 1 << 6), 0), Some((9, 20)));
        assert_eq!(control_poll(0, &pad([0.0; 4], [0.0; 2], 1 << 12), 0), Some((9, 21)));
        assert_eq!(control_poll(2, &pad([0.0; 4], [0.0; 2], 1 << 6), 0), Some((8, 6)));
        assert_eq!(control_poll(2, &pad([0.0; 4], [0.0, 1.0], 0), 0), Some((8, 1)));
        // ("must be mapped": the steering, pedals, HANDBRAKE, NITRO, HEADLIGHTS, IN_ZONE, MAP;
        // every row while only one shift is bound)
        assert_eq!(CONTROL_ROWS.iter().filter(|s| control_recommended(&base, **s)).copied().collect::<Vec<_>>(), [0, 2, 3, 6, 8, 9, 11, 14]);
        let mut one = base;
        one[4] = (8, 3);
        assert!(CONTROL_ROWS.iter().all(|s| control_recommended(&one, *s)));
        one[5] = (8, 2);
        assert!(!control_recommended(&one, 10));
        // (the 100 % save: every row that must be mapped is - HYDRAULICS is not one of them)
        assert!(!ctrl_unmapped(&base));
        let mut gone = base;
        gone[8] = (0, 0);
        assert!(ctrl_unmapped(&gone));
        // (the BUTTON column: glyphs for the buttons and half-axes, texts for the d-pad's
        // buttons and the whole axes)
        assert_eq!((control_label(8, 1), control_label(8, 6), control_label(8, 9), control_label(9, 27), control_label(9, 20)), (ControlLabel::Glyph(8), ControlLabel::Glyph(1), ControlLabel::Glyph(29), ControlLabel::Glyph(37), ControlLabel::Glyph(21)));
        assert_eq!((control_label(8, 14), control_label(9, 0), control_label(0, 0)), (ControlLabel::Text("DPad Down"), ControlLabel::Text("Left Analog X"), ControlLabel::None));
        // (a mapping in and out of the save's form)
        let mut saved: BTreeMap<String, Mapping> = BTreeMap::new();
        mapping_store(&mut saved, &base);
        assert_eq!((saved.len(), mapping_array(&saved) == base), (57, true));
        let h = &saved["HANDBRAKE"];
        assert_eq!((h.slot, h.device.as_str(), h.key, h.button_guess.as_deref()), (6, "button", 6, Some("RDOWN")));
        assert_eq!((saved["LOOKBACK"].device.as_str(), saved["LOOKBACK"].button_guess.is_none(), saved["SHIFT_UP"].device.as_str()), ("axis", true, "none"));
    }

    /// The look-back rule of the pad, and what ends the idle camera.
    #[test]
    fn controller_lookback_rule() {
        let mut slots = ControlSlots::new();
        for (slot, name) in CONTROL_ACTIONS {
            slots.insert(name.to_string(), custom_100()[slot]);
        }
        // (d-pad right is the next song - and CAM_PAN_RIGHT; the right stick back looks back)
        let ahead = PadState::new([0.0; 4], [0.0; 2], 1 << 13);
        let behind = PadState::new([0.0, 0.0, 0.0, -1.0], [0.0; 2], 1 << 13 | 1 << 12);
        let bit = |a: &str| 1u32 << PAD_ACTS.iter().position(|x| *x == a).unwrap();
        assert!(!pad_lookback(&ahead, &slots) && pad_lookback(&behind, &slots));
        assert_eq!(pad_acts_down(&ahead, &slots), bit("MUSIC_UP"));
        // (looking back: the song and the map on the d-pad read nothing; the pans are untouched)
        assert_eq!(pad_acts_down(&behind, &slots), bit("LOOKBACK"));
        let live = pad_live(&behind, &slots);
        assert_eq!((live["MUSIC_UP"], live["MAP"], live["CAM_PAN_RIGHT"], live["NITRO"]), ((0, 0), (0, 0), (8, 13), (8, 2)));
        // (the stick style: the triggers are the side looks, both of them look back)
        let maps = control_maps(&data_dir());
        if !maps.is_empty() {
            let s = &maps[0].slots;
            assert_eq!((s["CAMERA_LEFT"], s["CAMERA_RIGHT"]), ((8, 0), (8, 1)));
            assert!(pad_lookback(&PadState::new([0.0; 4], [1.0, 1.0], 0), s) && !pad_lookback(&PadState::new([0.0; 4], [1.0, 0.0], 0), s));
        }
        // (the idle camera: the camera, music, map and hydraulics actions end it; the brake does not)
        assert!(pad_wakes_idle(&ahead, &slots) && pad_wakes_idle(&PadState::new([0.0, 0.0, 0.6, 0.0], [0.0; 2], 0), &slots));
        assert!(!pad_wakes_idle(&PadState::new([0.0; 4], [1.0, 0.0], 1 << 6), &slots));
    }

    /// The controller pages of OPTIONS: SET kept until the options are saved, the viewer,
    /// the editor (poll, refusal, clear, defaults), the commit and the save's block.
    #[test]
    fn controller_editor_session() {
        let maps = control_maps(&data_dir());
        if maps.is_empty() {
            return;
        }
        let Some((mut g, dir, _)) = game_on("full_import.json", "ctrl") else { return };
        g.opt_x.control_maps = Some(maps.clone());
        let none = [false; 4];
        let (a, b) = ([false, false, true, false], [false, false, false, true]);
        let rest = PadState::new([0.0; 4], [0.0; 2], 0);
        let row = |slot: usize| CONTROL_ROWS.iter().position(|s| *s == slot).unwrap();
        // (OPTIONS opens: the save's style and mapping)
        g.pause_page = 4;
        assert_eq!(g.mc4_ctrl_input(none, false, false), none);
        assert!(g.opt_x.ctrl.open && g.opt_x.ctrl.style == 4 && g.opt_x.ctrl.work == custom_100());
        // (CONTROLLER > GAMEPAD: the list, the cursor on the style)
        g.pause_page = PAGE_CONTROLLER;
        g.menu_sel = 0;
        assert_eq!(g.mc4_ctrl_page().rows.len(), 3);
        g.mc4_ctrl_input(a, false, false);
        assert_eq!((g.pause_page, g.menu_sel), (PAGE_STYLE, 4));
        let p = g.mc4_ctrl_page();
        assert_eq!((p.rows.len(), p.rows[4].1.as_str(), p.rows[1].1.as_str(), p.rows[5].2, p.prompts.len()), (8, crate::flash_ui::CHECK_ON, crate::flash_ui::CHECK_OFF, false, 3));
        // (SET on STICK - MANUAL: the list's tick moves; the save and the pad are as they were)
        g.menu_sel = 2;
        g.mc4_ctrl_input(a, false, false);
        assert_eq!((g.opt_x.ctrl.style, g.mc4.as_ref().unwrap().options.control_config_pad, g.mc4_ctrl_page().rows[2].1.as_str()), (2, 4, crate::flash_ui::CHECK_ON));
        assert_eq!(slots_array(&g.mc4_controls(false)), custom_100());
        // (a wheel's row: neither SET nor the editor)
        g.menu_sel = 6;
        g.mc4_ctrl_input(a, false, true);
        assert_eq!((g.pause_page, g.opt_x.ctrl.style, g.mc4_ctrl_page().prompts.len()), (PAGE_STYLE, 2, 1));
        // (Y on TRIGGER - AUTOMATIC: the viewer - that map, nothing to change)
        g.menu_sel = 1;
        g.mc4_ctrl_input(none, false, true);
        assert_eq!((g.pause_page, g.opt_x.ctrl.read_only, g.opt_x.ctrl.row), (PAGE_EDIT, true, 1));
        let p = g.mc4_ctrl_page();
        assert_eq!((p.rows.len(), p.headers.len(), p.prompts.len(), p.rows[row(2)].1.clone()), (23, 2, 1, format!("{}8", crate::flash_ui::GLYPH)));
        g.opt_x.pad = Some(rest);
        g.mc4_ctrl_input(a, true, true);
        assert!(!g.opt_x.ctrl.polling && g.message_box.is_none() && g.opt_x.ctrl.work == custom_100());
        g.mc4_ctrl_input(b, false, false);
        assert_eq!((g.pause_page, g.menu_sel), (PAGE_STYLE, 1));
        // (Y on CUSTOM CONTROLLER: the editor on the working copy)
        g.menu_sel = 4;
        g.mc4_ctrl_input(none, false, true);
        assert_eq!((g.pause_page, g.opt_x.ctrl.read_only, g.opt_x.ctrl.row, g.mc4_ctrl_page().prompts.len()), (PAGE_EDIT, false, 4, 4));
        // (A on HYDRAULICS: it waits - nothing else is heard, the first frame is passed over -
        // and takes the d-pad's down; CONVERTIBLE_UP_DN had it and is emptied, the hidden pan keeps it)
        g.menu_sel = row(44);
        g.mc4_ctrl_input(a, false, false);
        assert!(g.opt_x.ctrl.polling);
        let p = g.mc4_ctrl_page();
        assert_eq!((p.prompts.len(), p.prompts[0].1, p.rows[0].2, p.rows[row(44)].2), (1, 0, false, true));
        g.opt_x.pad = Some(PadState::new([0.0; 4], [0.0; 2], 1 << 5));
        assert_eq!(g.mc4_ctrl_input([true, true, true, true], true, true), none);
        assert!(g.opt_x.ctrl.polling && g.pause_page == PAGE_EDIT);
        // (B was down on the frame passed over: it is no new press afterwards)
        g.mc4_ctrl_input(none, false, false);
        assert!(g.opt_x.ctrl.polling);
        g.opt_x.pad = Some(PadState::new([0.0; 4], [0.0; 2], 1 << 5 | 1 << 14));
        g.mc4_ctrl_input(none, false, false);
        let w = g.opt_x.ctrl.work;
        assert_eq!((g.opt_x.ctrl.polling, w[44], w[43], w[25]), (false, (8, 14), (0, 0), (8, 14)));
        // (LOOKBACK on the d-pad: refused, the box, the mapping as it was)
        g.opt_x.pad = Some(rest);
        g.mc4_ctrl_input(none, false, false);
        g.menu_sel = row(26);
        g.mc4_ctrl_input(a, false, false);
        g.mc4_ctrl_input(none, false, false);
        g.opt_x.pad = Some(PadState::new([0.0; 4], [0.0; 2], 1 << 12));
        g.mc4_ctrl_input(none, false, false);
        assert_eq!((g.opt_x.ctrl.polling, g.opt_x.ctrl.box_kind, g.opt_x.ctrl.work == w), (false, Some(CtrlBox::Invalid), true));
        assert_eq!(g.message_box.as_ref().map(|m| m.0.title.as_str()), Some("customcontrol_invalid_title"));
        g.message_box = None;
        g.box_sel = 0;
        g.opt_x.pad = Some(rest);
        g.mc4_ctrl_input(none, false, false);
        assert_eq!(g.opt_x.ctrl.box_kind, None);
        // (Y clears a row; NITRO is one that must be mapped)
        g.menu_sel = row(8);
        g.mc4_ctrl_input(none, false, true);
        let p = g.mc4_ctrl_page();
        assert_eq!((g.opt_x.ctrl.work[8], p.props[row(8)], p.props[row(44)], ctrl_unmapped(&g.opt_x.ctrl.work)), ((0, 0), 20, 1, true));
        assert_eq!(p.rows[row(8)].1, g.career.strings.get("mapping_recommended").map(|v| v.to_uppercase()).unwrap_or_else(|| "MUST BE MAPPED".into()));
        // (X: the defaults box, the cursor on NO; NO leaves the copy, YES makes it the custom row's file)
        g.mc4_ctrl_input(none, true, false);
        assert_eq!((g.opt_x.ctrl.box_kind, g.message_box.as_ref().map(|m| m.1)), (Some(CtrlBox::Defaults), Some(1)));
        g.message_box = None;
        g.box_sel = 1;
        g.mc4_ctrl_input(none, false, false);
        assert_eq!(g.opt_x.ctrl.work[44], (8, 14));
        g.mc4_ctrl_input(none, true, false);
        g.message_box = None;
        g.box_sel = 0;
        g.mc4_ctrl_input(none, false, false);
        assert_eq!(g.opt_x.ctrl.work, slots_array(&maps[4].slots));
        // (put the edit back: the 100 % mapping with HYDRAULICS on the d-pad's down)
        let mut want = custom_100();
        want[44] = (8, 14);
        want[43] = (0, 0);
        g.opt_x.ctrl.work = want;
        // (BACK, BACK to CONTROLLER; nothing has reached the save or the pad yet)
        g.mc4_ctrl_input(b, false, false);
        assert_eq!((g.pause_page, g.menu_sel), (PAGE_STYLE, 4));
        g.mc4_ctrl_input(b, false, false);
        assert_eq!(g.pause_page, PAGE_CONTROLLER);
        assert_eq!(slots_array(&g.mc4_controls(false)), custom_100());
        // (BACK on OPTIONS with the mapping changed. No movie in a test: no box, as if YES.
        // The style is the row that was edited - not the STICK - MANUAL that was SET - and the
        // pad follows at once)
        g.pause_page = 4;
        assert!(ctrl_changed(&g.opt_x.ctrl));
        assert_eq!(g.mc4_ctrl_input(b, false, false), b);
        assert_eq!((g.opt_x.ctrl.open, g.dirty, g.mc4.as_ref().unwrap().options.control_config_pad), (false, true, 4));
        let now = g.mc4_controls(false);
        assert_eq!((slots_array(&now) == want, now["HYDRAULICS"], now["CONVERTIBLE_UP_DN"]), (true, (8, 14), (0, 0)));
        // (the save's block - written by the commit: every action by its name, as the importer writes them)
        let blk = saved_block(&dir);
        let m = blk["options"]["custom_mapping_pad"].as_object().unwrap();
        assert_eq!(m.len(), 57);
        assert_eq!(m["HYDRAULICS"], serde_json::json!({"slot": 44, "device": "button", "key": 14, "button_guess": "LDOWN"}));
        assert_eq!(m["CONVERTIBLE_UP_DN"], serde_json::json!({"slot": 43, "device": "none", "key": 0}));
        assert_eq!(m["PHOTO_CAM_PAN_RT"], serde_json::json!({"slot": 73, "device": "axis", "key": 25}));
        assert_eq!(blk["options"]["control_config_pad_name"], "Custom_Pad");
        // (the box's answers. SET alone, then NO: as it was; then YES: the style, the mapping untouched)
        for (answer, style) in [(1usize, 4), (0, 0)] {
            g.pause_page = 4;
            g.mc4_ctrl_input(none, false, false);
            g.opt_x.ctrl.style = 0;
            g.opt_x.ctrl.box_kind = Some(CtrlBox::Save);
            g.box_sel = answer;
            assert_eq!(g.mc4_ctrl_input([true; 4], false, false), none);
            assert_eq!((g.pause_page, g.opt_x.ctrl.open, g.mc4.as_ref().unwrap().options.control_config_pad), (0, false, style));
        }
        assert_eq!(mapping_array(&g.mc4.as_ref().unwrap().options.custom_mapping_pad), want);
        assert_eq!(g.mc4_controls(false), maps[0].slots);
        // (BACK on the box: OPTIONS stays, nothing is lost)
        g.pause_page = 4;
        g.mc4_ctrl_input(none, false, false);
        g.opt_x.ctrl.style = 3;
        g.opt_x.ctrl.box_kind = Some(CtrlBox::Save);
        g.box_sel = usize::MAX;
        g.mc4_ctrl_input(none, false, false);
        assert_eq!((g.pause_page, g.opt_x.ctrl.open, g.opt_x.ctrl.style), (4, true, 3));
        // (OPTIONS left with nothing changed: nothing happens)
        g.opt_x.ctrl.style = 0;
        assert_eq!(g.mc4_ctrl_input(b, false, false), b);
        assert!(!g.opt_x.ctrl.open);
        // (a save without the block: the editor starts from the custom row's file; the block
        // is made by the commit, not before)
        g.mc4 = None;
        g.pause_page = 4;
        g.mc4_ctrl_input(none, false, false);
        assert_eq!((g.opt_x.ctrl.style, g.opt_x.ctrl.work == slots_array(&maps[4].slots)), (1, true));
        g.pause_page = PAGE_STYLE;
        g.menu_sel = 4;
        g.mc4_ctrl_input(none, false, true);
        g.menu_sel = row(10);
        g.opt_x.pad = Some(rest);
        g.mc4_ctrl_input(a, false, false);
        g.mc4_ctrl_input(none, false, false);
        g.opt_x.pad = Some(PadState::new([0.0; 4], [0.0; 2], 1 << 4));
        g.mc4_ctrl_input(none, false, false);
        assert!(g.mc4.is_none() && g.opt_x.ctrl.work[10] == (8, 4));
        g.pause_page = 4;
        g.mc4_ctrl_input(b, false, false);
        let o = &g.mc4.as_ref().unwrap().options;
        assert_eq!((o.control_config_pad, o.custom_mapping_pad.len(), o.custom_mapping_pad["HORN"].key), (4, 57, 4));
        assert_eq!(g.mc4_controls(false)["HORN"], (8, 4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The keyboard page of OPTIONS (the port's own): rows, the wait for a key, Escape,
    /// a reserved key, the conflict, clear, the working copy and its commit into the save.
    #[test]
    fn keyboard_page_session() {
        use crate::keybinds as kb;
        use winit::keyboard::KeyCode as K;
        let Some((mut g, dir, _)) = game_on("full_import.json", "kbd") else { return };
        g.opt_x.control_maps = Some(Vec::new());
        let none = [false; 4];
        let (a, b) = ([false, false, true, false], [false, false, false, true]);
        let rows = kb::rows();
        let row = |act: usize, place: usize| rows.iter().position(|r| *r == (act, place)).unwrap();
        // (the fixture has no field: the defaults)
        assert_eq!(g.opt_x.ctrl.kbd.binds, kb::KeyBinds::default());
        // (CONTROLLER > KEYBOARD)
        g.pause_page = PAGE_CONTROLLER;
        g.menu_sel = 1;
        g.mc4_ctrl_input(a, false, false);
        assert_eq!((g.pause_page, g.menu_sel), (PAGE_KEYBOARD, 0));
        let p = g.mc4_ctrl_page();
        assert_eq!((p.rows.len(), p.headers.len(), p.prompts.len()), (rows.len(), 2, 4));
        assert_eq!((p.rows[row(kb::GAS, 0)].1.as_str(), p.rows[row(kb::GAS, 1)].1.as_str(), p.rows[row(kb::NITRO, 0)].1.as_str(), p.rows[row(kb::SHIFT_UP, 0)].1.as_str()), ("W", "UP", "LEFT SHIFT", ""));
        assert!(p.rows[row(kb::GAS, 1)].0.ends_with("(2ND KEY)") && p.props.iter().all(|v| *v == 1));
        // (CHANGE on HORN: it waits; the other rows are greyed, nothing else is heard)
        g.menu_sel = row(kb::HORN, 0);
        g.mc4_ctrl_input(a, false, false);
        let p = g.mc4_ctrl_page();
        assert!(g.opt_x.ctrl.kbd.polling && p.prompts.len() == 1 && p.rows[g.menu_sel].1.is_empty() && p.rows[g.menu_sel].2 && !p.rows[0].2);
        assert_eq!(g.mc4_ctrl_input([true, true, true, false], false, false), none);
        assert!(g.opt_x.ctrl.kbd.polling);
        // (Escape gives up)
        g.opt_x.ctrl.kbd.queue.push(K::Escape);
        assert_eq!(g.mc4_ctrl_input(none, false, false), none);
        assert!(!g.opt_x.ctrl.kbd.polling && g.opt_x.ctrl.kbd.work == kb::KeyBinds::default());
        // (a reserved key is refused; E is taken - and IN_ZONE, which had it, is empty)
        for (key, horn, zone) in [(K::F2, Some(K::KeyH), Some(K::KeyE)), (K::KeyE, Some(K::KeyE), None)] {
            g.mc4_ctrl_input(a, false, false);
            g.opt_x.ctrl.kbd.queue.push(key);
            g.mc4_ctrl_input(none, false, false);
            let w = &g.opt_x.ctrl.kbd.work;
            assert_eq!((g.opt_x.ctrl.kbd.polling, w.keys[kb::HORN][0], w.keys[kb::IN_ZONE][0]), (false, horn, zone));
        }
        // (a pad's B gives up too)
        g.mc4_ctrl_input(a, false, false);
        g.mc4_ctrl_input(b, false, false);
        assert_eq!((g.opt_x.ctrl.kbd.polling, g.pause_page), (false, PAGE_KEYBOARD));
        // (Delete clears: GAS loses W, its second key moves up; cleared again it must be mapped)
        g.menu_sel = row(kb::GAS, 0);
        g.opt_x.ctrl.kbd.queue.push(K::Delete);
        g.mc4_ctrl_input(none, false, false);
        assert_eq!(g.opt_x.ctrl.kbd.work.keys[kb::GAS], [Some(K::ArrowUp), None]);
        g.nav_y = true;
        g.mc4_ctrl_input(none, false, false);
        let p = g.mc4_ctrl_page();
        assert_eq!((p.rows[g.menu_sel].1.as_str(), p.props[g.menu_sel], p.rows[g.menu_sel + 1].1.as_str()), ("MUST BE MAPPED", 20, ""));
        // (F5: the defaults - no box in a test; then the edit again)
        g.opt_x.ctrl.kbd.queue.push(K::F5);
        g.mc4_ctrl_input(none, false, false);
        assert_eq!(g.opt_x.ctrl.kbd.work, kb::KeyBinds::default());
        g.menu_sel = row(kb::HORN, 0);
        g.mc4_ctrl_input(a, false, false);
        g.opt_x.ctrl.kbd.queue.push(K::KeyE);
        g.mc4_ctrl_input(none, false, false);
        // (nothing has reached the game yet; BACK, BACK to OPTIONS; leaving it commits - no box
        // in a test, as if YES - and the save has the field)
        assert_eq!(g.opt_x.ctrl.kbd.binds, kb::KeyBinds::default());
        g.mc4_ctrl_input(b, false, false);
        assert_eq!((g.pause_page, g.menu_sel), (PAGE_CONTROLLER, 1));
        g.pause_page = 4;
        assert!(ctrl_changed(&g.opt_x.ctrl));
        let mc4_was = g.mc4_save();
        assert_eq!(g.mc4_ctrl_input(b, false, false), b);
        let want = g.opt_x.ctrl.kbd.work;
        assert_eq!((g.opt_x.ctrl.kbd.binds == want, g.dirty, want.action_of(K::KeyE), want.action_of(K::KeyH)), (true, true, Some(kb::HORN), None));
        assert_eq!(g.mc4_save(), mc4_was);
        let t = std::fs::read_to_string(std::path::Path::new(&dir).join("mcla_save.json")).unwrap();
        let v: Value = serde_json::from_str(&t).unwrap();
        assert_eq!((v["keyboard"]["HORN"].clone(), v["keyboard"]["IN_ZONE"].clone(), v["keyboard"].as_object().unwrap().len()), (serde_json::json!(["E"]), serde_json::json!([]), kb::COUNT));
        // (read again: the same bindings; NO on the box drops an edit)
        g.opt_x.ctrl.kbd.binds = Default::default();
        g.load_save(&dir);
        assert_eq!(g.opt_x.ctrl.kbd.binds, want);
        g.pause_page = PAGE_KEYBOARD;
        g.menu_sel = row(kb::NITRO, 0);
        g.mc4_ctrl_input(a, false, false);
        g.opt_x.ctrl.kbd.queue.push(K::ControlLeft);
        g.mc4_ctrl_input(none, false, false);
        assert_eq!(g.opt_x.ctrl.kbd.work.keys[kb::NITRO][0], Some(K::ControlLeft));
        g.pause_page = 4;
        g.opt_x.ctrl.box_kind = Some(CtrlBox::Save);
        g.box_sel = 1;
        g.mc4_ctrl_input(none, false, false);
        g.pause_page = 4;
        g.mc4_ctrl_input(none, false, false);
        assert_eq!((g.opt_x.ctrl.kbd.binds == want, g.opt_x.ctrl.kbd.work == want), (true, true));
        // (back to the defaults: the field goes from the save)
        g.opt_x.ctrl.kbd.work = Default::default();
        assert_eq!(g.mc4_ctrl_input(b, false, false), b);
        let t = std::fs::read_to_string(std::path::Path::new(&dir).join("mcla_save.json")).unwrap();
        assert!(serde_json::from_str::<Value>(&t).unwrap().get("keyboard").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The convertible top: the options' switch, its row, and the model's roof parts.
    #[test]
    fn convertible_top() {
        use crate::vehicle_model::{roof_part, ROOF_DOWN};
        assert_eq!((roof_part("conv_topup0"), roof_part("conv_topup1"), roof_part("conv_topdown0"), roof_part("conv_top0"), roof_part("conv_hatch"), roof_part("roof0"), roof_part("conv_top")), (Some(0), Some(0), Some(1), Some(2), None, None, None));
        let Some((mut g, dir, save)) = game_on("full_import.json", "roof") else { return };
        // (the 100 % save has the roof up)
        assert_eq!(save["mc4"]["options"]["convertible_top_down"], false);
        g.sync_extra_parts();
        assert!(!g.cfg.extra.contains_key(ROOF_DOWN));
        // (GAME OPTIONS, CONVERTIBLE TOP)
        option_step(&mut g.mc4, &mut g.opt_x, 50, true, false);
        assert!(g.mc4.as_ref().unwrap().options.convertible_top_down);
        g.sync_extra_parts();
        assert_eq!(g.cfg.extra.get(ROOF_DOWN), Some(&1));
        g.write_save(&dir);
        assert_eq!(saved_block(&dir)["options"]["convertible_top_down"], true);
        // (the switch is no part of the vehicle: the record's parts are what they were)
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        assert_eq!((m.car("vp_aud_r8_08").unwrap().part("conv_top"), m.car("vp_aud_r8_08").unwrap().part("conv_topup"), m.car("vp_aud_r8_08").unwrap().part("conv_topdown")), (0, 0, 0));
        option_step(&mut g.mc4, &mut g.opt_x, 50, false, false);
        g.sync_extra_parts();
        assert!(!g.cfg.extra.contains_key(ROOF_DOWN));
        let _ = std::fs::remove_dir_all(&dir);
        // (a save without the block: up, as before; the row makes the block)
        if let Some((mut p, dir, _)) = game_on("port_own_save.json", "roof2") {
            p.sync_extra_parts();
            assert!(p.mc4.is_none() && !p.cfg.extra.contains_key(ROOF_DOWN));
            option_step(&mut p.mc4, &mut p.opt_x, 50, true, false);
            p.sync_extra_parts();
            assert_eq!(p.cfg.extra.get(ROOF_DOWN), Some(&1));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Rim material 2, Carbon Fiber: from the record to the renderer's finish 4 (the rims'
    /// program's flat branch) and back; the shop's row takes it off.
    #[test]
    fn carbon_fibre_rims() {
        let Some((mut g, dir, _)) = game_on("full_import.json", "cf") else { return };
        // (the R8's rims are chrome on all three rows: finish 2 for the renderer)
        assert_eq!((g.rim_mats, g.rim_finish()[0][3]), ([1, 1, 1], 2.0));
        // (a record with spoke A in carbon fibre - no sample has one - while the port's own
        // field still says chrome, as an import from before this did)
        g.mc4.as_mut().unwrap().car_mut("vp_aud_r8_08").unwrap().set_rim_material(0, 2);
        g.mc4.as_mut().unwrap().car_mut("vp_frd_gt_06").unwrap().set_rim_material(2, 2);
        // (the vehicle comes into use, as at a load)
        g.mc4_take_car("vp_aud_r8_08");
        assert_eq!(g.rim_mats, [3, 1, 1]);
        // (rim_finish: zone 1 is spoke A, zone 2 the lip, zone 3 spoke B)
        assert_eq!((g.rim_finish()[0][3], g.rim_finish()[1][3], g.rim_finish()[2][3]), (4.0, 2.0, 2.0));
        g.write_save(&dir);
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        assert_eq!((m.car("vp_aud_r8_08").unwrap().rim_material(0), m.car("vp_aud_r8_08").unwrap().rim_material(1)), (Some(2), Some(1)));
        assert_eq!(m.car("vp_frd_gt_06").unwrap().rim_material(2), Some(2), "a vehicle not in use keeps its record's");
        // (the shop's MATERIAL row: painted, chrome, brushed - carbon fibre is none of its choices)
        g.rim_mats[0] = 2;
        g.write_save(&dir);
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        assert_eq!(m.car("vp_aud_r8_08").unwrap().rim_material(0), Some(3));
        // (the Ford GT comes into use: its lip is carbon fibre)
        let k = g.car_dirs.iter().position(|p| p.to_string_lossy() == "vp_frd_gt_06").unwrap();
        g.current_car = k;
        g.selected_car = k;
        g.sync_car_paint();
        assert_eq!(g.rim_mats[2], 3);
        assert_eq!(g.rim_finish()[1][3], 4.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The six paint areas: from the record to the renderer, and the paint shop's rules.
    #[test]
    fn paint_areas() {
        let Some((mut g, dir, _)) = game_on("full_import.json", "areas") else { return };
        let take = |g: &mut Game, name: &str| {
            let k = g.car_dirs.iter().position(|p| p.to_string_lossy() == name).unwrap();
            g.current_car = k;
            g.selected_car = k;
            g.sync_car_paint();
            g.mc4_paint_sync();
        };
        // (the Impala: a colour shift on the body, the hood - painted by itself - without)
        take(&mut g, "vpd_chv_impala_64");
        let live = g.car_x.paint.clone().unwrap();
        assert_eq!((live.areas[0].fade, live.areas[2].fade), (1, 0));
        assert!(close(live.areas[0].b, [0.686275, 0.14902, 0.07451]), "{:?}", live.areas[0].b);
        let o = g.paint_out();
        assert_eq!((o.ty, o.fade), (live.areas[0].ty, 1));
        // (hood: its own entry - w 1; it is Matte, type 1, which is drawn shifting to the fixed grey: type + 8 x 1)
        assert_eq!(o.area_a[1][3], 1.0);
        assert_eq!((live.areas[2].ty, o.area_b[1][3]), (1, 1.0 + 8.0));
        // (an area with the body's entry and no colour of its own draws as the body)
        let plain = (1..6).filter(|k| live.areas[*k] == live.areas[0] && !g.cfg.part_paint.contains_key(AREA_KEYS[*k - 1])).count();
        assert_eq!((1..6).filter(|k| o.area_a[*k - 1][3] == 0.0).count(), plain);
        // (untouched, the record is what it was)
        g.write_save(&dir);
        let before = fixture("full_import.json").unwrap()["mc4"]["garage"].clone();
        let after = saved_block(&dir)["garage"].clone();
        let pick = |v: &Value| v.as_array().unwrap().iter().find(|c| c["vehicle"] == "vpd_chv_impala_64").unwrap()["visual"]["paint"].clone();
        assert_eq!(pick(&before), pick(&after));

        // ---- the shop, on the R8 (every area the body's) ----
        take(&mut g, "vp_aud_r8_08");
        g.mode = crate::game::Mode::Garage;
        let t0 = g.car_x.paint.as_ref().unwrap().areas[0].ty;
        // ALL PARTS: a type goes into all six entries (F-31035 with the flag set)
        g.paint_parts = false;
        g.paint_type = if t0 == 4 { 5 } else { 4 };
        g.mc4_paint_sync();
        assert!(g.car_x.paint.as_ref().unwrap().areas.iter().all(|a| a.ty == g.paint_type));
        // ... and a fade
        g.paint_fade = 2;
        g.mc4_paint_sync();
        assert!(g.car_x.paint.as_ref().unwrap().areas.iter().all(|a| a.fade == 2));
        // PARTS, PAINT AREA hood: the row shows the hood's type; a type set there is the hood's alone
        g.paint_parts = true;
        g.paint_area = 2;
        g.mc4_paint_sync();
        let all = g.paint_type;
        g.paint_type = 1;
        g.mc4_paint_sync();
        let a = g.car_x.paint.as_ref().unwrap().areas;
        assert_eq!((a[2].ty, a[0].ty, a[1].ty, a[5].ty), (1, all, all, all));
        // (a colour set on the hood takes the hood's fade off: F-40495)
        g.cfg.part_paint.insert("hood".to_string(), [10, 200, 30]);
        g.mc4_paint_sync();
        let a = g.car_x.paint.as_ref().unwrap().areas;
        assert_eq!((a[2].fade, a[0].fade, a[3].fade), (0, 2, 2));
        // (the matte hood is drawn shifting to the fixed grey; the body fades front to back)
        let o = g.paint_out();
        assert_eq!((o.ty, o.fade), (all, 2));
        assert_eq!(o.area_a[1][3], 1.0);
        assert_eq!(o.area_b[1][3], 1.0 + 8.0);
        assert!((o.area_b[1][0] - 0.6f32.powf(2.2)).abs() < 1e-6 && (o.area_b[1][2] - 0.55f32.powf(2.2)).abs() < 1e-6);
        assert!((o.area_a[1][1] - (200.0f32 / 255.0).powf(2.2)).abs() < 1e-6);
        // (back on the body the row shows the body's type again)
        g.paint_area = 0;
        g.mc4_paint_sync();
        assert_eq!(g.paint_type, all);
        // (the record gets the six entries as they stand, and gives them back)
        g.paint_parts = false;
        g.write_save(&dir);
        let m = Mc4::from_value(&saved_block(&dir)).unwrap();
        let p = &m.car("vp_aud_r8_08").unwrap().visual.paint;
        assert_eq!((p[2].paint_type(), p[2].fade(), p[0].paint_type(), p[0].fade(), p[4].fade()), (Some(1), Some(0), Some(all), Some(2), Some(2)));
        let mut g2 = Game::new(g.car_dirs.clone(), 0);
        g2.data_dir = dir.clone();
        g2.load_save(&dir);
        g2.ensure_owned();
        g2.sync_car_paint();
        g2.mc4_paint_sync();
        assert_eq!(g2.car_x.paint.as_ref().unwrap().areas, g.car_x.paint.as_ref().unwrap().areas);
        // ALL PARTS, a new body colour: every part takes it (F-87462 with the flag set)
        g.mode = crate::game::Mode::Garage;
        g.paint_hsv = Some([0.3, 0.9, 0.8]);
        g.mc4_paint_sync();
        assert!(!g.cfg.part_paint.contains_key("hood"));
        let _ = std::fs::remove_dir_all(&dir);

        // (a new game's Scirocco: doors in DirtyMatte by its starter set-up)
        if let Some((mut n, dir, _)) = game_on("newgame_import.json", "areas2") {
            n.mc4_paint_sync();
            let live = n.car_x.paint.clone().unwrap();
            assert_eq!(live.areas[1].ty, 0);
            let o = n.paint_out();
            assert_eq!((o.area_a[0][3], o.area_b[0][3]), (1.0, 0.0 + 8.0));
            let _ = std::fs::remove_dir_all(&dir);
        }
        // (a save without the block: the areas are the body's; a part painted by itself has no fade)
        if let Some((mut p, dir, _)) = game_on("port_own_save.json", "areas3") {
            p.paint_fade = 3;
            p.cfg.part_paint.insert("skirts".to_string(), [1, 2, 3]);
            p.car_x.paint = None;
            p.mc4_paint_sync();
            let a = p.car_x.paint.as_ref().unwrap().areas;
            assert_eq!((a[0].fade, a[1].fade, a[5].fade), (3, 3, 0));
            let o = p.paint_out();
            assert_eq!((o.fade, o.area_a[4][3], o.area_a[0][3]), (3, 1.0, 0.0));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// A record made from a set-up file of the disc equals the importer's decode of the same
    /// bytes in a save (the starter of a new game is `starter_vp_vw_scirocco_88`).
    #[test]
    fn record_from_setup_file() {
        let data = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let Ok(b) = std::fs::read(std::path::Path::new(&data).join("resources/vehicle/configs/starter_vp_vw_scirocco_88.xcc")) else {
            eprintln!("(no set-up file: skipped)");
            return;
        };
        let car = Car::from_xcc(&b, "vp_vw_scirocco_88", "starter_vp_vw_scirocco_88", 0).unwrap();
        assert_eq!(car.origin(), "starter");
        assert_eq!(car.visual.LicenseBG, 11);
        assert_eq!(car.visual.paint[1].paint_type(), Some(0));
        let Some(save) = fixture("newgame_import.json") else { return };
        let theirs = Mc4::from_value(&save["mc4"]).unwrap();
        let t = &theirs.garage[0];
        assert_eq!(car.visual.paint, t.visual.paint);
        assert_eq!(car.visual.m_vWindowTintColorNew, t.visual.m_vWindowTintColorNew);
        assert_eq!(car.visual.RimMaterial, t.visual.RimMaterial);
        assert_eq!(car.visual.m_vBaseColor, t.visual.m_vBaseColor);
        assert_eq!(car.visual.lights, t.visual.lights);
        assert_eq!(car.visual.parts, t.visual.parts);
        assert_eq!(car.source_config_hash, t.source_config_hash);
        assert_eq!(car.performance.stages, t.performance.stages);
    }

    /// Step 14: the experience systems of the career tables and their shares (F-14176,
    /// F-96803, F-57870, F-53104).
    #[test]
    fn experience_tables_and_shares() {
        // (the rule itself, without the game's data)
        let mut t = ExpTables::default();
        t.parse("System\n{\n\tType Tuner\n\tMaxAmount 45\n\tIncrementValue 1\n\tThreshold\n\t{\n\t\tId Tuner_0\n\t\tPercent 0.2\n\t\tReward\n\t\t{\n\t\t\tType PartUnlock\n\t\t\tHidden 0\n\t\t\tGiveImmediately 0\n\t\t\tName FrontBumper\n\t\t}\n\t}\n\tThreshold\n\t{\n\t\tId Tuner_1\n\t\tPercent 1.0\n\t}\n}System\n{\n\tType Unlocking\n\tMaxAmount 33500\n\tThreshold\n\t{\n\t\tId Unlocking_0\n\t\tPercent 0.04\n\t\tReward\n\t\t{\n\t\t\tType PerformanceLevel\n\t\t\tPerformanceId D1\n\t\t}\n\t}\n}\n");
        assert_eq!(t.systems.len(), 2);
        let tu = t.system("TUNER").unwrap();
        assert_eq!((tu.max, tu.thresholds.len(), tu.thresholds[0].parts.clone()), (45, 2, vec!["FrontBumper".to_string()]));
        assert_eq!(t.system("Unlocking").unwrap().thresholds[0].perf, vec!["D1".to_string()]);
        // (9 of 45 is the share 0.2; the amount counts up to MaxAmount)
        assert!(tu.fraction(8) < 0.2 && tu.fraction(9) >= 0.2 && tu.fraction(108) == 1.0);
        assert_eq!((tu.index(0), tu.index(9), tu.index(44), tu.index(45), tu.index(108)), (0, 1, 1, 1, 1));
        assert_eq!((tu.span(2), tu.span(9)), ((0, 9), (9, 45)));
        // (a main system goes through the levels: 4 % of the career is 800 points, not 1340)
        assert_eq!((main_amount_at(0.04), main_amount_at(0.1), main_amount_at(0.05), main_amount_at(0.015)), (800, 2000, 1000, 300));
        assert!(t.system("Unlocking").unwrap().fraction(799) < 0.04 && t.system("Unlocking").unwrap().fraction(800) >= 0.04);
        // (0.4 of 36 is 14.4: fifteen races, not fourteen)
        assert_eq!((class_amount_at(0.4, 36), class_amount_at(0.2, 45), class_amount_at(1.0, 45)), (15, 9, 45));
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            eprintln!("(no game data: the tables themselves skipped)");
            return;
        }
        let c = crate::career::Career::load(&data_dir());
        let x = &c.exp_tables;
        for (name, n) in [("Tuner", 5), ("Muscle", 5), ("Luxury", 5), ("Exotic", 5), ("SUV", 5), ("LowRider", 5), ("SportBike", 3), ("Unlocking", 13), ("Progression", 43)] {
            assert_eq!(x.system(name).map(|s| s.thresholds.len()), Some(n), "{name}");
        }
        // (South Central's two class systems reuse the Exotic ids)
        assert_eq!(x.system("SUV").unwrap().thresholds[0].id, "Exotic_0");
        let sb = x.system("SportBike").unwrap();
        assert_eq!((sb.max, sb.index(14), sb.index(15), sb.index(332)), (36, 0, 1, 2));
        assert_eq!(sb.span(15), (14, 21));
        assert_eq!(x.system("Muscle").unwrap().thresholds[4].parts, vec!["Blower", "Exhaust", "Intercooler", "WideBody"].iter().map(|s| s.to_string()).collect::<Vec<_>>());
        // (the performance levels of the Unlocking system, at the amounts of the levels)
        assert_eq!((c.perf_unlocks.get("D1"), c.perf_unlocks.get("C0"), c.perf_unlocks.get("A2")), (Some(&800), Some(&2000), Some(&main_amount_at(0.84))));
        assert_eq!(c.part_wins_needed("SportBike", "BIKETAIL").0, 15);
    }

    fn career_game(name: &str, tag: &str) -> Option<(Game, String, Value)> {
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            return None;
        }
        let (g0, dir, save) = game_on(name, tag)?;
        let mut g = Game::new(g0.car_dirs.clone(), 0);
        g.data_dir = data_dir();
        g.career = crate::career::Career::load(&data_dir());
        g.catalog = crate::catalog::Catalog::load(&data_dir());
        g.samples = crate::rewards::Samples::load(&data_dir());
        g.load_save(&dir);
        g.ensure_owned();
        g.sync_car_paint();
        Some((g, dir, save))
    }

    /// Step 14: a finished race gives the Unlocking system the race's experience and the
    /// class of the vehicle one point; the thresholds unlock the body part types; all of it
    /// is in the block of the save and comes back.
    #[test]
    fn class_reputation_and_unlocking_grow() {
        use crate::vehicle_model::Slot;
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s14ng") else { return };
        // (a new game after its first races: the Scirocco is a Tuner)
        assert_eq!((g.rep, g.mc4_unlocking(), g.mc4_class_amount("Tuner")), (260, 210, 2));
        assert_eq!(g.mc4_class_rank("TUNER"), (1, 2, 9));
        g.selected_car = g.current_car;
        assert_eq!(g.slot_lock(Slot::BumperF), Some((9, 2, "TUNER".to_string())));
        // (the first performance level is the Unlocking system's, at 800: 590 short of it)
        assert_eq!(g.perf_rep(g.current_car, 1), 260 + 590);
        // (the cars of group C come with the level C0, at 2000 of the Unlocking amount)
        assert_eq!((g.mc4_group_need("C0"), g.mc4_group_need("D0"), g.mc4_group_need("")), (260 + 1790, 0, 0));
        let mut earned = 0;
        for k in 0..7 {
            // (last of four still counts for the class: F-48818 adds the point whatever the place)
            g.award_race(if k == 0 { 4 } else { 1 }, 4);
            earned += g.last_reward.1;
        }
        assert!(earned > 590, "{earned}");
        assert_eq!((g.rep, g.mc4_unlocking(), g.mc4_class_amount("Tuner")), (260 + earned, 210 + earned, 9));
        assert_eq!(g.mc4_class_rank("Tuner"), (2, 0, 9));
        // (Tuner_0: FrontBumper and Skirts, unlocked and marked new; the spoiler waits for 18)
        let m = g.mc4.as_ref().unwrap();
        assert_eq!(m.career.body_parts_unlocked["Tuner"], vec!["FrontBumper".to_string(), "Skirts".to_string()]);
        assert_eq!(m.career.body_parts_new["Tuner"], vec!["FrontBumper".to_string(), "Skirts".to_string()]);
        assert_eq!(m.career.experience.classes["Tuner"].reached("Tuner_0"), Some(true));
        assert_eq!(m.career.experience.classes["Tuner"].reached("Tuner_1"), Some(false));
        assert!(m.career.body_parts_unlocked["Muscle"].is_empty());
        assert_eq!(g.slot_lock(Slot::BumperF), None);
        assert_eq!(g.slot_lock(Slot::Spoiler), Some((18, 9, "TUNER".to_string())));
        assert_eq!(g.perf_rep(g.current_car, 1), 0);
        // (a delivery gives neither system anything and is no race of the class)
        let (r0, u0) = (g.rep, g.mc4_unlocking());
        g.award_mission(true, Some(0.5));
        assert_eq!((g.rep, g.mc4_unlocking(), g.mc4_class_amount("Tuner")), (r0, u0, 9));
        let (rep, unl) = (g.rep, g.mc4_unlocking());
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!(b["career"]["experience"]["main"]["Unlocking"]["amount"], unl);
        assert_eq!(b["career"]["experience"]["main"]["Progression"]["amount"], rep);
        assert_eq!(b["career"]["experience"]["classes"]["Tuner"]["amount"], 9);
        assert_eq!(b["career"]["performance_levels_unlocked"]["D1"], true);
        assert_eq!(b["career"]["performance_levels_unlocked"]["C0"], false);
        // (Unlocking_0 is reached at 800, Unlocking_3 - 10 % - is not)
        let th = |sys: &str, id: &str| b["career"]["experience"]["main"][sys]["thresholds"].as_array().unwrap().iter().find(|t| t["id"] == id).map(|t| t["reached"].as_bool().unwrap());
        assert_eq!((th("Unlocking", "Unlocking_0"), th("Unlocking", "Unlocking_3")), (Some(true), Some(false)));
        assert!(b["career"]["experience"].get("classes_counted").is_none() && b["career"]["experience"].get("unlocking_counted").is_none());
        let mut g2 = Game::new(g.car_dirs.clone(), 0);
        g2.career = crate::career::Career::load(&data_dir());
        g2.catalog = crate::catalog::Catalog::load(&data_dir());
        g2.load_save(&dir);
        assert_eq!((g2.rep, g2.mc4_unlocking(), g2.mc4_class_amount("Tuner")), (rep, unl, 9));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 14: the 100 % save's class systems as the map screen shows them, and a race
    /// past the top of a system still counts.
    #[test]
    fn class_reputation_of_the_full_save() {
        let Some((mut g, dir, save)) = career_game("full_import.json", "s14full") else { return };
        assert_eq!((g.rep, g.mc4_unlocking()), (119080, 121460));
        for (class, amount, rank) in [("Exotic", 70, (5, 9, 9)), ("Tuner", 108, (5, 9, 9)), ("Muscle", 95, (5, 9, 9)), ("Luxury", 54, (5, 9, 9)), ("SportBike", 332, (3, 15, 15)), ("SUV", 7, (1, 7, 9)), ("LowRider", 6, (1, 6, 9)), ("Cop", 0, (1, 0, 100))] {
            assert_eq!(g.mc4_class_amount(class), amount, "{class}");
            assert_eq!(g.mc4_class_rank(class), rank, "{class}");
        }
        // (nothing of it changes the block of a save that was only read)
        g.write_save(&dir);
        same(&save["mc4"], &saved_block(&dir), "step 14, read only");
        // (the R8 is an Exotic: 71, and the Unlocking amount moves with the reputation)
        g.award_race(1, 4);
        let exp = g.last_reward.1;
        assert_eq!((g.mc4_class_amount("Exotic"), g.mc4_unlocking()), (71, 121460 + exp));
        assert_eq!(g.mc4_class_rank("Exotic"), (5, 9, 9));
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!(b["career"]["experience"]["classes"]["Exotic"]["amount"], 71);
        assert_eq!(b["career"]["experience"]["main"]["Unlocking"]["amount"], 121460 + exp);
        assert_eq!(b["career"]["body_parts_unlocked"], save["mc4"]["career"]["body_parts_unlocked"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 14: a save of the port's (no block) goes on by the port's earlier rules - the
    /// Unlocking amount starts from the reputation, the classes from the races won - and
    /// counts them from its first race on.
    #[test]
    fn experience_of_a_save_without_the_block() {
        let Some((mut g, dir, _)) = career_game("port_own_save.json", "s14own") else { return };
        assert!(g.mc4.as_ref().map_or(true, |m| m.made_by_port()));
        assert_eq!(g.career.unlocking, None);
        assert_eq!(g.mc4_unlocking(), g.rep);
        let class = g.catalog.get(&g.mc4_car_name(g.current_car)).map(|c| c.class.clone()).unwrap_or_default();
        let Some(sys) = Game::mc4_class_system(&class) else { return };
        let stat = CLASS_WIN_STATS.iter().find(|(s, _)| *s == sys).map(|(_, st)| *st).unwrap();
        let wins = g.career.stats.get(stat).copied().unwrap_or(0);
        assert_eq!(g.mc4_class_amount(sys), wins);
        // (the lock of a part goes by the wins, as before)
        let need = g.career.part_wins_needed(&class, "FRONTBUMPER").0;
        assert_eq!(g.mc4_part_lock(&class, "FRONTBUMPER").is_some(), wins < need);
        let r0 = g.rep;
        g.award_race(2, 4);
        let exp = g.last_reward.1;
        assert_eq!(g.mc4_class_amount(sys), wins + 1);
        if exp > 1 {
            assert_eq!(g.career.unlocking, Some(r0 + exp));
        }
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!(b["made_by_port"], true);
        assert_eq!(b["career"]["experience"]["classes_counted"], true);
        assert_eq!(b["career"]["experience"]["classes"][sys]["amount"], wins + 1);
        assert_eq!(b["career"]["experience"]["classes"]["Tuner"]["thresholds"].as_array().map(|a| a.len()), Some(5));
        let mut g2 = Game::new(g.car_dirs.clone(), 0);
        g2.career = crate::career::Career::load(&data_dir());
        g2.catalog = crate::catalog::Catalog::load(&data_dir());
        g2.load_save(&dir);
        assert_eq!(g2.mc4_class_amount(sys), wins + 1);
        assert_eq!(g2.mc4_unlocking(), g.mc4_unlocking());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 14b: what gives reputation and what does not - a delivery and a race of the
    /// race editor give none (F-96289; results word +3140), and nothing stops the
    /// reputation at the top of the table (F-55689).
    #[test]
    fn reputation_rules_of_the_original() {
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s14b") else { return };
        // (a delivery: money, no reputation, nothing for the Unlocking system)
        let (r0, u0, m0) = (g.rep, g.mc4_unlocking(), g.money);
        g.award_mission(true, Some(0.0));
        assert_eq!((g.rep, g.mc4_unlocking(), g.last_reward.1), (r0, u0, 0));
        assert!(g.money > m0);
        // (a race of the editor: no money, no reputation, no class point)
        let pts = [glam::Vec3::new(0.0, 0.0, 0.0), glam::Vec3::new(50.0, 0.0, 0.0), glam::Vec3::new(100.0, 0.0, 0.0)];
        let own = g.add_user_race(1, &pts).unwrap();
        g.current_race = Some(own);
        let (m1, t1) = (g.money, g.mc4_class_amount("Tuner"));
        g.award_race(1, 4);
        assert_eq!((g.rep, g.money, g.mc4_unlocking(), g.mc4_class_amount("Tuner"), g.last_reward), (r0, m1, u0, t1, (0, 0)));
        // (an ordinary race just below the top of the table goes past it, the Unlocking
        // amount with it; the share stays at 1)
        g.current_race = None;
        g.rep = 33_490;
        g.career.unlocking = Some(33_440);
        g.award_race(1, 4);
        let exp = g.last_reward.1;
        assert!(exp >= 100, "{exp}");
        assert_eq!((g.rep, g.mc4_unlocking()), (33_490 + exp, 33_440 + exp));
        g.award_race(1, 4);
        assert_eq!(g.rep, 33_490 + exp + g.last_reward.1);
        assert_eq!(crate::rewards::progress(g.rep), (12, 1.0));
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!(b["career"]["experience"]["main"]["Progression"]["amount"], g.rep);
        let all = |sys: &str| b["career"]["experience"]["main"][sys]["thresholds"].as_array().unwrap().iter().all(|t| t["reached"] == true);
        assert!(all("Progression") && all("Unlocking"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 15: the events of the reward tables.
    #[test]
    fn event_tables() {
        let mut t = ExpTables::default();
        t.parse_events("Event\n{\n\tName\t\thw_tournament1\n\tDifficulty\n\t{\n\t\tName Easy\n\t\tReward\n\t\t{\n\t\t\tType DynamicMoney\n\t\t\tHidden 0\n\t\t}\n\t}\n\tDifficulty\n\t{\n\t\tName Hard\n\t\tReward\n\t\t{\n\t\t\tType Car\n\t\t\tName vp_chv_camaro_06_DUB \n\t\t\tTextureName vp_chv_camaro_06_DUB\n\t\t} \n\t\tReward\n\t\t{\n\t\t\tType DynamicMoney\n\t\t}\n\t}\n}\nEvent\n{\n\tName be_payback\n\tDifficulty\n\t{\n\t\tName Medium\n\t\tReward\n\t\t{\n\t\t\tType DynamicExperience\n\t\t\tSystemName Unlocking\n\t\t}\n\t}\n}\n");
        assert_eq!(t.events.len(), 2);
        let e = t.event("HW_TOURNAMENT1").unwrap();
        assert_eq!(e.rewards[0], vec![EventReward::Money]);
        assert_eq!(e.rewards[2], vec![EventReward::Car("vp_chv_camaro_06_DUB".into()), EventReward::Money]);
        assert!(e.rewards[1].is_empty() && e.rewards[3].is_empty());
        assert_eq!(t.event("be_payback").unwrap().rewards[1], vec![EventReward::Experience("Unlocking".into())]);
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            return;
        }
        let c = crate::career::Career::load(&data_dir());
        let names: Vec<&str> = c.exp_tables.events.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names.len(), 15, "{names:?}");
        for n in ["hw_local_hangout1_series", "dt_local_hangout1_series", "sc_local_hangout1_series", "hw_tournament1", "sc_tournament1", "hw_delivery", "sc_delivery", "be_payback"] {
            assert!(names.contains(&n), "{n}");
        }
        assert_eq!(c.exp_tables.event("sc_tournament1").unwrap().rewards[2], vec![EventReward::Car("SC_tournament_vpd_lr_sport_08".into()), EventReward::Money]);
        for d in 0..4 {
            assert_eq!(c.exp_tables.event("be_payback").unwrap().rewards[d], vec![EventReward::Experience("Unlocking".into())]);
            assert_eq!(c.exp_tables.event("hw_delivery").unwrap().rewards[d], vec![EventReward::Money]);
        }
    }

    /// Step 15: an event completes at a difficulty - the reward at min(completions, last)
    /// of that list, then the counter (F-67065); the counters are the block's.
    #[test]
    fn panels_show_difficulty_by_the_forms_rules() {
        let Some((mut g, _dir, _save)) = career_game("full_import.json", "s15panels") else { return };
        // (the 100 % save: fifteen MEDIUM deliveries - more than four - and every tournament
        // mission complete: the tournament panel opens on HARD with its choice shown)
        assert!(g.mc4_panel_diff_shown("hw_delivery"));
        assert_eq!(g.mc4_tournament_panel_diff("hw_tournament1"), 2);
        assert!(g.mc4_panel_diff_shown("hw_tournament1"));
        // (a new career: every district's TOURNAMENT_<district> open, South Central's
        // TOURNAMENT_SC too - MEDIUM, the choice hidden)
        let Some((n, _dir, _save)) = career_game("newgame_import.json", "s15panels_new") else { return };
        for code in DISTRICT_CODES {
            let key = format!("{code}_tournament1");
            assert_eq!((n.mc4_tournament_panel_diff(&key), n.mc4_panel_diff_shown(&key)), (1, false), "{key}");
        }
        assert!(g.mc4_panel_diff_shown("be_payback") && g.mc4_panel_diff_shown("hw_wager_hangout1"));
        // (a series shows it after one MEDIUM completion of its event, not before)
        let medium = g.mc4_event_completions("hw_local_hangout1_series", 1);
        assert_eq!(g.mc4_panel_diff_shown("hw_local_hangout1"), medium > 0);
        if medium == 0 {
            g.mc4_event_complete("hw_local_hangout1_series", 1, 1.5);
            assert!(g.mc4_panel_diff_shown("hw_local_hangout1"));
        }
        // (an event never run at MEDIUM: hidden)
        assert!(!g.mc4_panel_diff_shown("zz_local_hangout9") && !g.mc4_panel_diff_shown("zz_delivery"));
    }

    #[test]
    fn event_completions_and_rewards() {
        let Some((mut g, dir, save)) = career_game("full_import.json", "s15full") else { return };
        // (the 100 % save: fifteen deliveries on Medium, the tournaments once on Medium and once on Hard)
        assert_eq!((g.mc4_event_completions("hw_delivery", 1), g.mc4_event_completions("hw_tournament1", 1), g.mc4_event_completions("hw_tournament1", 2), g.mc4_event_completions("be_payback", 0)), (15, 1, 1, 6));
        // (the car went with the first Hard completion: now the money)
        assert_eq!(g.mc4_event_reward("hw_tournament1", 2), Some(EventReward::Money));
        assert!(!g.mc4_showroom_locked("vp_chv_camaro_06_dub") && !g.mc4_showroom_locked("vp_aud_r8_08"));
        // (the hangout panel's COMPLETION REWARD of the R8, class A, at the top of the
        // career: 7,800 / 9,750 / 11,700 / 13,650 in the original)
        let (rep, unl) = (g.rep, g.mc4_unlocking());
        let pay = g.mc4_event_complete("hw_local_hangout1_series", 1, 1.5);
        assert_eq!((pay.counted, pay.money, pay.car.clone()), (true, 9750, None));
        assert_eq!(g.mc4_event_complete("HW_LOCAL_HANGOUT1_SERIES", 3, 1.5).money, 13650);
        assert_eq!((g.mc4_event_completions("hw_local_hangout1_series", 1), g.mc4_event_completions("hw_local_hangout1_series", 3)), (2, 1));
        // (an event the tables do not have: nothing is counted)
        assert_eq!(g.mc4_event_complete("no_such_event", 1, 1.5), EventPay::default());
        // (the Beaches payback's reward is a DynamicExperience nobody computes: it is counted
        // and gives nothing)
        let p = g.mc4_event_complete("be_payback", 0, 1.5);
        assert_eq!((p.counted, p.unlocking, p.money, g.rep, g.mc4_unlocking()), (true, 0, 0, rep, unl));
        // (the delivery event's money is what the delivery menu left in it: Impossible's)
        assert_eq!(g.mc4_event_complete_at("hw_delivery", 1, 1.0, 3).money, 9100);
        g.write_save(&dir);
        let b = saved_block(&dir);
        let ev = |name: &str| b["career"]["events"].as_array().unwrap().iter().find(|e| e["name"] == name).unwrap()["completions"].clone();
        assert_eq!(ev("hw_local_hangout1_series"), serde_json::json!({"Easy": 0, "Medium": 2, "Hard": 0, "Impossible": 1}));
        assert_eq!(ev("be_payback")["Easy"], 7);
        assert_eq!(b["career"]["events"].as_array().unwrap().len(), save["mc4"]["career"]["events"].as_array().unwrap().len());
        let _ = std::fs::remove_dir_all(&dir);

        // (a new game: the DUB Camaro is the first Hard completion of the Hollywood
        // tournament, and not in the showroom before it)
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s15ng") else { return };
        assert!(g.mc4_showroom_locked("vp_chv_camaro_06_DUB"));
        assert_eq!(g.mc4_event_reward("hw_tournament1", 2), Some(EventReward::Car("vp_chv_camaro_06_DUB".into())));
        let m0 = g.mc4_event_complete("hw_tournament1", 1, 1.5);
        assert!(m0.money > 0 && m0.car.is_none() && g.mc4_showroom_locked("vp_chv_camaro_06_dub"));
        let first = g.mc4_event_complete("hw_tournament1", 2, 1.5);
        assert_eq!((first.money, first.car.as_deref()), (0, Some("vp_chv_camaro_06_DUB")));
        assert!(g.career.news.iter().any(|n| matches!(n, crate::career::News::Car(c) if c.eq_ignore_ascii_case("vp_chv_camaro_06_dub"))));
        assert!(!g.mc4_showroom_locked("vp_chv_camaro_06_dub"));
        let second = g.mc4_event_complete("hw_tournament1", 2, 1.5);
        assert!(second.money > 0 && second.car.is_none());
        assert_eq!((g.mc4_event_completions("hw_tournament1", 1), g.mc4_event_completions("hw_tournament1", 2)), (1, 2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 15: what the race types pay - the ARCADE factor, the legs of a payback, a
    /// delivery, a race of a series, the difficulty steps.
    #[test]
    fn race_type_rewards() {
        use crate::rewards::{difficulty, dynamic_exp, perf_class, race_money};
        // (the career's steps: -0.15, 0.025, 0.15, 0.3)
        assert_eq!((difficulty(0.0, 0.2), difficulty(0.1, 0.2), difficulty(0.22, 0.2), difficulty(0.23, 0.2), difficulty(0.36, 0.2), difficulty(0.55, 0.2)), (-1, 0, 0, 1, 2, 3));
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s15pay") else { return };
        let class = perf_class(g.player_perf());
        // (a race from the ARCADE menu: x 0.6 on money and experience; the class point all the same)
        let (r0, t0) = (g.rep, g.mc4_class_amount("Tuner"));
        let full = crate::rewards::race_reward(r0, g.player_perf(), g.race_diff, 1, 4, 1.0);
        g.arcade_restore = Some((0, 3));
        assert_eq!(g.mc4_race_factor(), 0.6);
        g.award_race(1, 4);
        assert_eq!(g.last_reward, crate::rewards::race_reward(r0, g.player_perf(), g.race_diff, 1, 4, 0.6));
        assert!(g.last_reward.0 < full.0 && g.last_reward.1 < full.1);
        assert_eq!(g.mc4_class_amount("Tuner"), t0 + 1);
        g.arcade_restore = None;
        assert_eq!(g.mc4_race_factor(), 1.0);
        // (a race of a hangout's series pays its money like any race)
        g.series = Some(crate::game::Series { key: "hw_local_hangout1_series".into(), races: Vec::new(), at: 0, wins: Vec::new(), rivals: Vec::new(), diff: 1 });
        g.award_race(2, 4);
        assert!(g.last_reward.0 > 0 && g.last_reward.1 > 0);
        g.series = None;
        // (a delivery on Hard with a third of the damage: the manager's two thirds and the
        // event's whole - no reputation; the Hollywood event counts it)
        g.event_diff = Some(2);
        let base = race_money(g.rep, class, 2, 10.0);
        let (rep, unl) = (g.rep, g.mc4_unlocking());
        assert_eq!(g.mc4_mission_pay(Some(0.25)), ((0.75 * base) as i64 + race_money(g.rep, class, 3, 10.0) as i64, 0));
        assert_eq!((g.rep, g.mc4_unlocking(), g.mc4_event_completions("hw_delivery", 2)), (rep, unl, 1));
        // (a payback outside its missions is five legs: 10, 15, 20, 25, 30 % of the class A
        // money in steps of 20; the fifth gives five events' experience to both systems, then
        // the event's own to the Unlocking system)
        g.event_diff = Some(1);
        let top = race_money(g.rep, 0, 1, 20.0);
        for k in 0..4 {
            g.award_mission(true, None);
            assert_eq!(g.last_reward, (((0.10 + 0.05 * k as f32) * top).round() as i64, 0), "leg {k}");
            assert!(!g.career.payback_done);
        }
        assert_eq!((g.career.payback_wins, g.mc4_event_completions("be_payback", 1), g.rep, g.mc4_unlocking()), (4, 0, rep, unl));
        let one = dynamic_exp(g.rep, 1);
        g.award_mission(true, None);
        let five = ((5.0 * one * 0.1) as i64) * 10;
        assert_eq!(g.last_reward, ((0.30 * top).round() as i64, five));
        assert_eq!((g.rep, g.mc4_unlocking()), (rep + five, unl + five));
        assert_eq!((g.career.payback_wins, g.career.payback_done, g.mc4_event_completions("be_payback", 1)), (0, true, 1));
        // (a lost leg starts the payback over)
        g.award_mission(true, None);
        assert_eq!(g.career.payback_wins, 1);
        g.award_mission(false, None);
        assert_eq!(g.career.payback_wins, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 15b: the upgrade percentage of a set-up (F-76051) and the run to the line's
    /// thirty seconds.
    #[test]
    fn upgrade_percent_and_the_run_to_the_line() {
        // (twelve parts, the nitrous - place 6 - not among them)
        assert_eq!(upgrade_percent(&[2; 13], false), 100);
        assert_eq!(upgrade_percent(&[1; 13], false), 50);
        assert_eq!(upgrade_percent(&[0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0], false), 0);
        // (`GH_04_1_vp_chv_camaross_69`: exhaust and tires at stage 1 - 2 of 24)
        assert_eq!(upgrade_percent(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], false), 8);
        // (a motorcycle has five: the 100 % save's Ninja, 2 in places 0, 4, 5, 6, 8, 12, is at 100)
        assert_eq!(upgrade_percent(&[2, 0, 0, 0, 2, 2, 2, 0, 2, 0, 0, 0, 2], true), 100);
        assert_eq!(upgrade_percent(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], true), 20);

        // (the player first: over at once, first)
        let mut c = BmtClose::default();
        assert_eq!(c.step(300.0, &[200.0], 24.0, 0.1), BmtStep::Running);
        assert_eq!(c.step(10.0, &[10.0], 24.0, 0.1), BmtStep::Over(1));
        // (an opponent first: thirty seconds; the player in after ten is second)
        let mut c = BmtClose::default();
        assert_eq!(c.step(300.0, &[20.0], 24.0, 0.1), BmtStep::FirstIn);
        for _ in 0..100 {
            assert_eq!(c.step(200.0, &[5.0], 24.0, 0.1), BmtStep::Running);
        }
        assert_eq!(c.step(20.0, &[5.0], 24.0, 0.1), BmtStep::Over(2));
        // (two of three opponents in before him: fourth is not possible, third it is)
        let mut c = BmtClose::default();
        assert_eq!(c.step(300.0, &[20.0, 90.0, 400.0], 24.0, 0.1), BmtStep::FirstIn);
        assert_eq!(c.step(200.0, &[5.0, 10.0, 350.0], 24.0, 0.1), BmtStep::Running);
        assert_eq!(c.step(20.0, &[5.0, 10.0, 300.0], 24.0, 0.1), BmtStep::Over(3));
        // (the time runs out: his rank on the road)
        let mut c = BmtClose::default();
        assert_eq!(c.step(900.0, &[20.0, 700.0, 950.0], 24.0, 0.1), BmtStep::FirstIn);
        let mut last = BmtStep::Running;
        let mut ticks = 0;
        while last == BmtStep::Running {
            last = c.step(900.0, &[5.0, 700.0, 950.0], 24.0, 0.5);
            ticks += 1;
        }
        assert_eq!((last, ticks), (BmtStep::Over(3), 60));
    }

    /// Step 15b: the racers' set-ups, the event panels, the payback's run, the showroom lock.
    #[test]
    fn event_panels_payback_run_and_showroom() {
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s15b") else { return };
        if std::path::Path::new(&data_dir()).join("resources/vehicle/configs").is_dir() {
            // (most hookmen drive stock cars; a few set-ups carry upgrades)
            assert_eq!(g.mc4_setup_percent("vp_dsn_280z_75", "GH_23_0_vp_dsn_280z_75"), 0);
            assert_eq!(g.mc4_setup_percent("vp_dge_challenger_70", "GH_05_1_vp_dge_challenger_70"), 50);
            assert_eq!(g.mc4_setup_percent("vp_chv_camaross_69", "GH_04_1_vp_chv_camaross_69"), 8);
            assert_eq!(g.mc4_setup_percent("vp_pon_transam_79", "pyback_gn_vp_pon_transam_79"), 100);
            assert_eq!(g.mc4_setup_percent("vp_dsn_280z_75", "no_such_setup"), 0);
            if let (Some(stock), Some(tuned)) = (g.samples.value("vp_dge_challenger_70", 0.0), g.samples.value("vp_dge_challenger_70", 0.5)) {
                assert!(tuned > stock);
                assert_eq!(g.mc4_difficulty_of_setup("vp_dge_challenger_70", "GH_05_1_vp_dge_challenger_70"), crate::rewards::difficulty(tuned, g.player_perf()));
            }
            assert_eq!(g.mc4_difficulty_of_setup("vp_dge_challenger_70", ""), g.difficulty_of("vp_dge_challenger_70"));
        }
        // (the stock Scirocco: nothing fitted)
        assert_eq!(g.mc4_upgrade_share(g.current_car), 0.0);

        // the tournament's panel: the DUB Camaro on Hard, one and a half times the race money else
        let class = crate::rewards::perf_class(g.player_perf());
        let p = g.mc4_event_panel("hw_tournament1", 2);
        assert_eq!((p.title.as_str(), p.desc.as_str(), p.reward_label.as_str(), p.count_label.as_str()), ("hw_tournament1", "tournament_desc", "REWARD_TOURNAMENT_LABEL", "RACES_TOTAL_LABEL"));
        assert_eq!((p.reward_car.as_deref(), p.reward_money), (Some("vp_chv_camaro_06_DUB"), None));
        assert_eq!(p.count, g.career.event_races("hw_tournament1").len());
        let p = g.mc4_event_panel("hw_tournament1", 1);
        assert_eq!((p.reward_car, p.reward_money), (None, Some((crate::rewards::race_money(g.rep, class, 1, 10.0) * 1.5) as i64)));
        // the payback's: five to destroy, their shares in all = the class A money in steps of 20
        let p = g.mc4_event_panel("be_payback", 3);
        assert_eq!((p.title.as_str(), p.desc.as_str(), p.reward_label.as_str(), p.count_label.as_str(), p.count), ("PAYBACK_MENU_TITLE", "PAYBACK_MENU_DESC", "REWARD_PAYBACK_LABEL", "DESTROY_RACERS_LABEL", 5));
        assert_eq!(p.reward_money, Some(crate::rewards::race_money(g.rep, 0, 3, 20.0) as i64));

        // the payback's run: accepted at its panel, a won leg goes on with the nearest client
        // this run has not had; the fifth ends it; a lost leg ends it
        for (k, x) in [("a", 10.0f32), ("b", 500.0), ("c", 100.0)] {
            g.race_defs.push(crate::races::RaceDef { id: format!("beaches/payback/test_{k}"), race_type: "Payback".into(), beacon: glam::Vec3::new(x, 0.0, 0.0), ..Default::default() });
        }
        let n0 = g.race_defs.len() - 3;
        g.current_race = Some(n0 + 1);
        g.hangout_diff = 2;
        assert!(g.mc4_event_panel_accept("be_payback"));
        assert_eq!((g.event_diff, g.career.payback_run, g.career.panel_ok), (Some(2), true, true));
        assert!(!g.mc4_payback_chain(glam::Vec3::ZERO));
        g.award_mission(true, None);
        assert_eq!((g.career.payback_wins, g.career.payback_next), (1, true));
        assert!(g.mc4_payback_chain(glam::Vec3::ZERO));
        assert_eq!(g.current_race, Some(n0));
        g.award_mission(true, None);
        assert!(g.mc4_payback_chain(glam::Vec3::ZERO));
        assert_eq!(g.current_race, Some(n0 + 2));
        // (all three had: the list starts over, nearest first)
        g.award_mission(true, None);
        assert!(g.mc4_payback_chain(glam::Vec3::ZERO));
        assert_eq!(g.current_race, Some(n0));
        g.award_mission(true, None);
        assert!(g.mc4_payback_chain(glam::Vec3::ZERO));
        g.award_mission(true, None);
        assert_eq!((g.career.payback_run, g.career.payback_done, g.mc4_event_completions("be_payback", 2)), (false, true, 1));
        assert!(!g.mc4_payback_chain(glam::Vec3::ZERO));
        assert!(g.mc4_event_panel_accept("be_payback"));
        g.award_mission(true, None);
        g.award_mission(false, None);
        assert_eq!((g.career.payback_run, g.career.payback_wins), (false, 0));
        assert!(!g.mc4_payback_chain(glam::Vec3::ZERO));

        // the tournament's difficulty goes with the event
        g.hangout_diff = 2;
        assert!(g.mc4_event_panel_accept("hw_tournament1"));
        assert_eq!((g.career.stats.get("tour_diff:hw_tournament1").copied(), g.event_diff), (Some(2), Some(2)));
        assert_eq!(g.career.tournament_of(""), None);

        // the showroom: the DUB Camaro is locked until its tournament is won on Hard
        assert_eq!(g.mc4_car_need("vp_chv_camaro_06_dub", "B0"), i64::MAX);
        assert_eq!(g.mc4_car_need("vp_chv_camaro_06", "B0"), g.mc4_group_need("B0"));
        g.mc4_event_complete("hw_tournament1", 2, 1.5);
        assert_eq!(g.mc4_car_need("vp_chv_camaro_06_dub", "B0"), g.mc4_group_need("B0"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 16: the part lists' rows by the unlocked flag, the "new" marks, the rewards
    /// screen's list, the garage's ability meter.
    #[test]
    fn body_part_types_and_new_marks() {
        use crate::vehicle_model::Slot;
        // (a locked list: three rows open of more than three, else the stock row alone)
        assert_eq!((0..6).map(|i| part_row_open(true, i, 6)).collect::<Vec<_>>(), [true, true, true, false, false, false]);
        assert_eq!((0..3).map(|i| part_row_open(true, i, 3)).collect::<Vec<_>>(), [true, false, false]);
        assert!((0..6).all(|i| part_row_open(false, i, 6)));
        assert_eq!((slot_part_type(Slot::BumperF), slot_part_type(Slot::Body), slot_part_type(Slot::Paint), slot_part_type(Slot::Wheels)), (Some("FrontBumper"), Some("WideBody"), None, None));
        assert_eq!((section_part_type("side_exhaust"), section_part_type("post_gauge"), section_part_type("scissor_door"), section_part_type("tires")), (Some("Exhaust"), Some("PostGauge"), None, None));
        // (the ability's meter: five per point on the level it stands on)
        assert_eq!([0, 10, 19, 20, 30, 39, 40, 83].map(|a| ability_meter(a, 40)), [0, 50, 95, 0, 50, 95, 100, 100]);

        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s16") else { return };
        // (a new career: nothing unlocked for the Tuner - three front bumpers of six, no more)
        assert!(g.mc4_part_type_locked(Some("FrontBumper")) && g.mc4_part_type_locked(Some("Roof")) && !g.mc4_part_type_locked(None));
        assert_eq!((g.mc4_part_row_open(Some("FrontBumper"), 2, 6), g.mc4_part_row_open(Some("FrontBumper"), 3, 6), g.mc4_part_row_open(Some("FrontBumper"), 1, 3), g.mc4_part_row_open(None, 5, 6)), (true, false, false, true));
        assert!(!g.mc4_part_new(Some("FrontBumper")));
        assert_eq!(g.mc4_ability_panel(0), (1, 0));
        for _ in 0..6 {
            g.award_race(1, 4);
            assert!(g.career.unlocked_now.is_empty());
        }
        // (the ninth race of the class: FrontBumper and Skirts - listed for the rewards screen,
        // unlocked, marked new)
        g.award_race(4, 4);
        assert_eq!(g.career.unlocked_now, vec!["FrontBumper".to_string(), "Skirts".to_string()]);
        assert!(g.reward_panel);
        assert!(!g.mc4_part_type_locked(Some("FrontBumper")) && g.mc4_part_row_open(Some("frontbumper"), 5, 6) && g.mc4_part_type_locked(Some("Spoiler")));
        assert!(g.mc4_part_new(Some("FrontBumper")) && g.mc4_part_new(Some("Skirts")) && !g.mc4_part_new(Some("Spoiler")));
        assert_eq!((g.mc4_part_type_label("FrontBumper"), g.mc4_part_type_label("Skirts"), g.mc4_part_type_label("Headlight")), ("FRONT BUMPER".to_string(), "SIDESKIRTS".to_string(), "HEADLIGHTS".to_string()));
        // (opening the list takes the mark off that type; the next race lists nothing)
        g.mc4_part_list_opened(Some("FrontBumper"));
        assert!(!g.mc4_part_new(Some("FrontBumper")) && g.mc4_part_new(Some("Skirts")));
        g.award_race(1, 4);
        assert!(g.career.unlocked_now.is_empty());
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!(b["career"]["body_parts_new"]["Tuner"], serde_json::json!(["Skirts"]));
        assert_eq!(b["career"]["body_parts_unlocked"]["Tuner"], serde_json::json!(["FrontBumper", "Skirts"]));
        let _ = std::fs::remove_dir_all(&dir);

        // (the 100 % save: the R8's lists are open, but a type no threshold names never is;
        // Zone at 83 of 40 is three stars and a full meter)
        let Some((g, dir, _)) = career_game("full_import.json", "s16full") else { return };
        assert!(!g.mc4_part_type_locked(Some("FrontBumper")) && !g.mc4_part_type_locked(Some("WideBody")) && g.mc4_part_type_locked(Some("Roof")));
        assert_eq!((g.mc4_ability_panel(0), g.mc4_ability_panel(3)), ((3, 100), (3, 100)));
        // (flag games off line pay as any race: no factor but the ARCADE menu's)
        assert_eq!(g.mc4_race_factor(), 1.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `career_game` with the race table and the race definitions read.
    fn race_game(name: &str, tag: &str) -> Option<(Game, String, Value)> {
        let (mut g, dir, save) = career_game(name, tag)?;
        g.race_defs = crate::races::load(&data_dir());
        let t = std::fs::read_to_string(std::path::Path::new(&data_dir()).join("tune/race/0x0f1f4d8d")).ok()?;
        g.race_table = t.lines().filter_map(|l| { let mut it = l.split_whitespace(); (it.next() == Some("Id")).then(|| it.next().unwrap_or("").to_string()) }).collect();
        Some((g, dir, save))
    }

    fn names(v: &[Value]) -> Vec<String> {
        v.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
    }

    /// Step 17: the list of unlocked races by the original's rules - the start-up scripts'
    /// calls give the list of the new game sample entry for entry, the mission scripts'
    /// calls leave both samples as they are.
    #[test]
    fn unlocked_races_by_the_scripts() {
        let Some((g, dir, _)) = race_game("newgame_import.json", "s17ng") else { return };
        assert_eq!(g.race_table.len(), 227);
        // (the masks of the district call: every flag game; Hollywood's ordered and circuit
        // races - the test-only ones too; the deliveries; the paybacks)
        assert_eq!(g.mc4_district_races(6, 8, -1).len(), 79);
        assert_eq!(g.mc4_district_races(0, 2, 6).len(), 27);
        assert_eq!((g.mc4_district_races(0, 16, 64).len(), g.mc4_district_races(4, 16, 64).len(), g.mc4_district_races(6, 32, 128).len()), (15, 10, 14));
        assert_eq!(g.mc4_district_races(4, -1, -1).len(), 48);
        assert!(race_passes((0, 1, 2), 0, 2, 6) && !race_passes((0, 1, 5), 0, 2, 6) && !race_passes((1, 1, 2), 0, 2, 6) && race_passes((1, 1, 2), 6, -1, -1) && !race_passes((0, -1, 2), 0, 2, 6));
        // (the new game sample: the twelve, the flag games, South Central - 127)
        let sample = names(&g.mc4.as_ref().unwrap().career.races_unlocked);
        assert_eq!(sample.len(), 127);
        assert_eq!(g.mc4_start_races(), sample);
        // (its missions have unlocked nothing more; nor does the rule change the full save's 224)
        let mut m = g.mc4.clone().unwrap();
        assert!(!g.mc4_races_apply(&mut m));
        assert_eq!(names(&m.career.races_unlocked), sample);
        assert!(m.career.extra.get("races_counted").is_none());
        let _ = std::fs::remove_dir_all(&dir);
        let Some((g, dir, _)) = race_game("full_import.json", "s17full") else { return };
        let full = names(&g.mc4.as_ref().unwrap().career.races_unlocked);
        assert_eq!(full.len(), 224);
        let mut m = g.mc4.clone().unwrap();
        assert!(!g.mc4_races_apply(&mut m), "{:?}", names(&m.career.races_unlocked).iter().filter(|r| !full.contains(r)).collect::<Vec<_>>());
        // (every delivery, payback and the pink slip races the scripts name are in it)
        for r in g.mc4_district_races(0, 16, 64).iter().chain(g.mc4_district_races(0, 2, 6).iter()).chain(PAYBACK_RACES.map(|s| s.to_string()).iter()) {
            assert!(full.iter().any(|f| f.eq_ignore_ascii_case(r)), "{r}");
        }
        assert!(g.mc4_race_unlocked("Hollywood/Ordered/hw_ordered_jp_04") && g.mc4_race_unlocked("hills/ordered/hl_ordered_ts_01"));
        // (the race definitions carry the whole name: the ARCADE lists ask with it)
        assert!(g.race_defs.iter().all(|d| d.id.starts_with("game/Race/")));
        assert!(g.race_defs.iter().filter(|d| d.sub_type == "Ordered").all(|d| g.mc4_race_unlocked(&d.id)));
        assert_eq!(g.mc4_race_index("game/Race/Hollywood/Ordered/hw_ordered_jp_11"), Some(18));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 17: a career begun in the port keeps the list from its first race on; a career
    /// of the port that was under way, and a save without the block, list every race.
    #[test]
    fn unlocked_races_of_a_port_career() {
        let Some((mut g, dir, _)) = race_game("newgame_import.json", "s17port") else { return };
        // (a new career: no block yet)
        g.mc4 = None;
        g.fresh_game = true;
        assert!(g.mc4_race_unlocked("Hollywood/Ordered/hw_ordered_jp_11"));
        assert!(g.mc4_race_unlocked("SouthCentral/Blitz/sc_blitz_mb_01"));
        let flag = g.mc4_district_races(2, 8, -1)[0][10..].to_string();
        assert!(g.mc4_race_unlocked(&flag), "{flag}");
        assert!(!g.mc4_race_unlocked("Hollywood/Ordered/hw_ordered_jp_04") && !g.mc4_race_unlocked("Hollywood/Unordered/hw_unordered_jj_01"));
        // (a finished race is unlocked; the block gets the list of the start with it)
        g.mc4_race_finished("Hollywood/Ordered/hw_ordered_jp_04");
        g.mc4_race_finished("user/race_editor_1");
        g.mc4_race_finished("Hollywood/Ordered/hw_ordered_jp_04");
        let m = g.mc4.as_ref().unwrap();
        assert!(m.made_by_port() && races_counted(m));
        let list = names(&m.career.races_unlocked);
        assert_eq!(list.len(), 128);
        assert_eq!(list[..127], g.mc4_start_races()[..]);
        assert_eq!(list[127], "game/Race/Hollywood/Ordered/hw_ordered_jp_04");
        assert!(g.mc4_race_unlocked("Hollywood/Ordered/hw_ordered_jp_04") && !g.mc4_race_unlocked("Hollywood/Unordered/hw_unordered_jj_01"));
        // (it is in the save and comes back)
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!(b["career"]["races_unlocked"].as_array().unwrap().len(), 128);
        assert_eq!(b["career"]["races_counted"], true);
        let mut g2 = Game::new(g.car_dirs.clone(), 0);
        g2.race_defs = g.race_defs.clone();
        g2.race_table = g.race_table.clone();
        g2.load_save(&dir);
        assert!(!g2.fresh_game);
        assert!(g2.mc4_race_unlocked("Hollywood/Ordered/hw_ordered_jp_04") && !g2.mc4_race_unlocked("Hollywood/Unordered/hw_unordered_jj_01"));
        // (a block of the port from before the list was kept: every race, and the list says so)
        let mut m = Mc4::seeded(false);
        g2.fresh_game = false;
        assert!(g2.mc4_races_apply(&mut m));
        assert_eq!(m.career.races_unlocked.len(), 227);
        g2.mc4 = Some(Mc4::seeded(false));
        assert!(g2.mc4_race_unlocked("Hollywood/Unordered/hw_unordered_jj_01"));
        // (a save without the block: every race is listed, nothing is written by asking)
        g2.mc4 = None;
        assert!(g2.mc4_race_unlocked("Hollywood/Unordered/hw_unordered_jj_01") && g2.mc4_race_unlocked("Downtown/Ordered/dt_ordered_mb_04"));
        g2.mc4_races_sync();
        assert!(g2.mc4.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 17: the ring of the last twelve races (F-63023, F-98664) and what
    /// reads it (F-97924, F-18968).
    #[test]
    fn race_history_ring() {
        let mut c = Career::default();
        for k in 0..2 {
            c.add_race_to_history([18, 15][k], None);
        }
        // (the new game sample: two races, written last 2, head 0)
        assert_eq!((c.race_history.write_index, c.race_history.head, c.race_history.count), (2, 0, 2));
        assert_eq!((c.history_position(18), c.history_position(15), c.history_position(7)), (Some(0), Some(1), None));
        // (a race that is in it moves to the end; nothing else changes)
        c.add_race_to_history(18, None);
        assert_eq!(c.race_history.oldest_first.iter().map(|r| r.race_index).collect::<Vec<_>>(), vec![15, 18]);
        assert_eq!((c.race_history.write_index, c.race_history.head, c.race_history.count), (2, 0, 2));
        // (twelve fill it; the thirteenth pushes the oldest out and the head steps on)
        for k in 100..110 {
            c.add_race_to_history(k, None);
        }
        assert_eq!((c.race_history.write_index, c.race_history.head, c.race_history.count), (0, 0, 12));
        c.add_race_to_history(200, None);
        assert_eq!((c.race_history.write_index, c.race_history.head, c.race_history.count), (1, 1, 12));
        assert_eq!((c.history_position(15), c.history_position(18), c.history_position(200)), (None, Some(0), Some(11)));
        // (the full sample's state is one this rule reaches: written last = head, twelve)
        for k in 0..7 {
            c.add_race_to_history(300 + k, None);
        }
        assert_eq!((c.race_history.write_index, c.race_history.head, c.race_history.count), (8, 8, 12));
        // (the hookman's next race: the first not in the history, else the one run longest ago)
        assert_eq!(c.next_race_of(&[306, 9999, 300]), Some(9999));
        assert_eq!(c.next_race_of(&[306, 300, 303]), Some(300));
        assert_eq!(c.next_race_of(&[]), None);
        // (the list of unlocked races: once each, whatever the case, 250 at the most)
        assert!(c.unlock_race("game/Race/A/B/c") && !c.unlock_race("game/race/a/b/C") && c.race_is_unlocked("GAME/RACE/A/B/C"));
        for k in 0..300 {
            c.unlock_race(&format!("r{k}"));
        }
        assert_eq!(c.races_unlocked.len(), 250);
    }

    /// Step 17: a race that starts joins the history with its place in the race table - not
    /// a delivery, not a race of the editor -, and it is in the save.
    #[test]
    fn race_history_of_the_game() {
        let Some((mut g, dir, _)) = race_game("newgame_import.json", "s17hist") else { return };
        let at = |g: &Game, id: &str| g.race_defs.iter().position(|d| d.id.eq_ignore_ascii_case(&format!("game/Race/{id}"))).unwrap();
        assert_eq!(g.mc4_race_index("Hollywood/Ordered/hw_ordered_jp_11"), Some(18));
        assert_eq!(g.mc4.as_ref().unwrap().career.race_history.oldest_first.iter().map(|r| r.race_index).collect::<Vec<_>>(), vec![18, 15]);
        let k = at(&g, "Hollywood/Circuit/hw_circuit_jp_03");
        g.mc4_race_started(k);
        let k = at(&g, "Hollywood/Ordered/hw_ordered_jp_11");
        g.mc4_race_started(k);
        let d = g.race_defs.iter().position(|d| d.sub_type == "Delivery").unwrap();
        g.mc4_race_started(d);
        let h = &g.mc4.as_ref().unwrap().career.race_history;
        assert_eq!(h.oldest_first.iter().map(|r| r.race_index).collect::<Vec<_>>(), vec![15, 6, 18]);
        assert_eq!(h.oldest_first[1].race.as_deref(), Some("game/Race/Hollywood/Circuit/hw_circuit_jp_03"));
        assert_eq!((h.write_index, h.head, h.count), (3, 0, 3));
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!(b["career"]["race_history"]["oldest_first"].as_array().unwrap().len(), 3);
        assert_eq!(b["career"]["race_history"]["oldest_first"][2]["race_index"], 18);
        assert_eq!(b["career"]["race_history"]["write_index"], 3);
        // (a game without the block: the history begins, as the block does)
        g.mc4 = None;
        g.mc4_race_started(k);
        assert_eq!(g.mc4.as_ref().unwrap().career.race_history.count, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 16b: the pages of the rewards screen (F-36204) and what a race lists on
    /// them - the performance levels of the Unlocking system, the ability levels.
    #[test]
    fn reward_pages_of_the_form() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let kinds = |p: &Vec<RewardRow>| p.iter().map(|r| r.0).collect::<Vec<_>>();
        assert!(reward_pages(&[], &[], "UNLOCKED", "REWARDED").is_empty());
        // (UNLOCKED alone: a page of its own, the header with the number of all)
        let p = reward_pages(&[], &s(&["A", "B"]), "UNLOCKED", "REWARDED");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0], vec![(13, "UNLOCKED".to_string(), "2".to_string()), (19, "A".to_string(), String::new()), (19, "B".to_string(), String::new())]);
        let ten: Vec<String> = (0..10).map(|k| format!("U{k}")).collect();
        let p = reward_pages(&[], &ten, "UNLOCKED", "REWARDED");
        assert_eq!((p.len(), p[0].len(), p[0][0].2.as_str()), (1, 9, "10"));
        // (REWARDED first, on its page; six rows and no more: UNLOCKED follows on the next)
        let p = reward_pages(&s(&["Z", "Y"]), &s(&["A", "B"]), "UNLOCKED", "REWARDED");
        assert_eq!(p.len(), 2);
        assert_eq!((kinds(&p[0]), p[0][0].2.as_str()), (vec![17, 19, 19], "2"));
        assert_eq!((kinds(&p[1]), p[1][0].2.as_str()), (vec![13, 19, 19], "2"));
        // (more than six rows: a page is counted between them, with no rows, and the count
        // of items starts again)
        let p = reward_pages(&s(&["Z"]), &s(&["A", "B", "C", "D"]), "UNLOCKED", "REWARDED");
        assert_eq!(p.len(), 3);
        assert!(p[1].is_empty());
        assert_eq!(kinds(&p[2]), vec![13, 19, 19, 19, 19]);
        // (the count goes on from REWARDED's otherwise: 8 - 2 items when there were more)
        let p = reward_pages(&s(&["Z", "Y"]), &s(&["A"]), "UNLOCKED", "REWARDED");
        assert_eq!((p.len(), p[1].len()), (2, 2));

        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s17rew") else { return };
        // (the Unlocking system at 210: Unlocking_0, at 800, gives the level D1)
        assert_eq!(g.mc4_unlocking(), 210);
        g.mc4_rewards_begin();
        g.mc4_unlocking_add(g.rep, 500);
        assert!(g.career.perf_now.is_empty() && !g.mc4_rewards_listed());
        g.mc4_unlocking_add(g.rep, 100);
        assert_eq!(g.career.perf_now, vec!["D1".to_string()]);
        assert_eq!(g.mc4_perf_label("D1"), "GROUP 1 VEHICLES: PERFORMANCE LEVEL 1");
        assert_eq!(g.mc4_perf_label("C0"), "GROUP 2 VEHICLES");
        assert_eq!(g.mc4_perf_label("D9"), "!!Performance level D9!!");
        // (with the body part types of the class point after it: one UNLOCKED group)
        g.career.unlocked_now = s(&["FrontBumper", "Skirts"]);
        let p = g.mc4_reward_pages();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), vec!["UNLOCKED", "GROUP 1 VEHICLES: PERFORMANCE LEVEL 1", "FRONT BUMPER", "SIDESKIRTS"]);
        assert_eq!(p[0][0].2, "3");
        // (the first page is the reputation's and the money's; CONTINUE goes to the group's, then closes)
        assert!(g.mc4_reward_page(true).is_none());
        assert!(g.mc4_reward_continue(true));
        assert_eq!(g.mc4_reward_page(true).unwrap().len(), 4);
        assert!(!g.mc4_reward_continue(true));
        assert_eq!(g.career.reward_page, 0);
        // (without rows on the first page the group is the first)
        assert_eq!(g.mc4_reward_page(false).unwrap().len(), 4);
        assert!(!g.mc4_reward_continue(false));
        // (an ability level is a line of REWARDED)
        g.career.rewarded_now = s(&["ZONE LEVEL 2"]);
        let p = g.mc4_reward_pages();
        assert_eq!((p.len(), p[0][0].1.as_str(), p[0][0].2.as_str(), p[0][1].1.as_str()), (2, "REWARDED", "1", "ZONE LEVEL 2"));
        // (the next race starts the lists again; a race that crosses nothing lists nothing)
        g.award_race(1, 4);
        assert!(g.career.perf_now.is_empty() && g.career.rewarded_now.is_empty());
        let _ = std::fs::remove_dir_all(&dir);

        // (the ability fitted reaches a level: the text of the level, no message)
        let Some((mut g, dir, _)) = career_game("full_import.json", "s17abil") else { return };
        if g.mc4_abilities_unlocked() && !g.mc4_ability_none() {
            let k = g.car_x.ability.unwrap_or(g.ability_sel).min(3);
            let (sys, stat) = (ABILITY_SYSTEMS[k], ["gZON", "gAGR", "gROR", "gPUL"][k]);
            let max = g.career.exp_tables.system(sys).map_or(40, |d| d.max);
            let mut edge = None;
            for a in 0..max {
                g.career.ability_amounts.insert(sys.to_string(), a);
                let l0 = g.career.ability_level(sys, stat);
                g.career.ability_amounts.insert(sys.to_string(), a + 1);
                if g.career.ability_level(sys, stat) > l0 {
                    edge = Some((a, l0 + 1));
                    break;
                }
            }
            let (a, level) = edge.expect("an ability level");
            g.career.ability_amounts.insert(sys.to_string(), a);
            let shown = g.message.clone();
            g.award_race(1, 4);
            let key = format!("{}_LEVEL_{}", sys.to_ascii_uppercase(), level);
            assert_eq!(g.career.rewarded_now, vec![g.career.strings.get(&key).map(|s| s.to_uppercase()).unwrap()]);
            assert_eq!(g.message, shown);
            assert_eq!(g.mc4_reward_pages()[0][0], (17, "REWARDED".to_string(), "1".to_string()));
            assert!(g.reward_panel);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 17: an ARCADE race runs at the menu's difficulty, on the easy layout below Hard.
    #[test]
    fn arcade_difficulty_and_layout() {
        let Some((mut g, dir, _)) = game_on("newgame_import.json", "s17arc") else { return };
        assert_eq!((g.mc4_arcade_difficulty(), g.mc4_layout_easy()), (None, None));
        g.arcade_restore = Some((0, 3));
        for (opt, easy) in [(0, true), (1, true), (2, false)] {
            g.arcade_opts[1] = opt;
            assert_eq!((g.mc4_arcade_difficulty(), g.mc4_layout_easy()), (Some(opt), Some(easy)));
        }
        g.arcade_restore = None;
        // (a wager and an event from a panel: the same rule)
        g.wager = Some((500, 1));
        assert_eq!(g.mc4_layout_easy(), Some(true));
        g.wager = Some((500, 2));
        assert_eq!(g.mc4_layout_easy(), Some(false));
        g.event_diff = Some(2);
        assert_eq!(g.mc4_layout_easy(), Some(false));
        g.event_diff = Some(1);
        assert_eq!(g.mc4_layout_easy(), Some(true));
        g.event_diff = Some(0);
        assert_eq!(g.mc4_layout_easy(), Some(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 17b: the launchers' unlocks and the red light hookman's; the ARCADE menu's rows;
    /// the car page of the rewards screen.
    #[test]
    fn launchers_arcade_rows_and_the_car_page() {
        let Some((mut g, dir, _)) = race_game("newgame_import.json", "s17b") else { return };
        // (the tables name races of the race table, and launchers that are listed)
        for (_, races) in LAUNCHERS {
            for r in races.iter() {
                assert!(g.race_table.iter().any(|t| t.eq_ignore_ascii_case(r)), "{r}");
            }
        }
        assert!(MISSION_LAUNCHERS.iter().all(|(_, l)| LAUNCHERS.iter().any(|(n, _)| n == l)));
        // (the red light racer of Hollywood met: his id is marked, the four landmark races unlocked)
        assert!(!g.mc4_race_unlocked("Hollywood/Blitz/hw_blitz_mb_01"));
        g.mc4_redlight_seen("Hollywood");
        let m = g.mc4.as_ref().unwrap();
        assert_eq!(m.career.races_unlocked.len(), 131);
        assert!(m.career.alt_hookmen.iter().any(|a| a.uid == 350 && a.visible));
        assert!(g.mc4_race_unlocked("Hollywood/Blitz/hw_blitz_mb_04") && !g.mc4_race_unlocked("Beaches/Blitz/be_blitz_mb_01"));
        g.mc4_redlight_seen("Hollywood");
        g.mc4_redlight_seen("SouthCentral");
        assert_eq!(g.mc4.as_ref().unwrap().career.races_unlocked.len(), 131);
        // (ARCADE: an ordered race has six rows, a circuit eight; the lists of the original)
        let at = |g: &Game, id: &str| g.race_defs.iter().position(|d| d.id.eq_ignore_ascii_case(&format!("game/Race/{id}"))).unwrap();
        let ord = at(&g, "Hollywood/Ordered/hw_ordered_jp_11");
        let o = g.mc4_arcade_rows(Some(ord));
        let allowed = g.race_defs[ord].opponents;
        assert_eq!(o.iter().map(|r| (r.slot, r.choices.len(), r.default)).collect::<Vec<_>>(), vec![(0, allowed, allowed.min(3) - 1), (1, 4, 1), (2, 3, 0), (3, 3, 2), (4, 5, 0), (5, 5, 0)]);
        assert_eq!(o[1].choices, vec!["EASIEST", "MEDIUM", "HARD", "HARDEST"]);
        assert_eq!(o[4].choices, vec!["DYNAMIC", "CLEAR", "PARTLY CLOUDY", "CLOUDY", "RAINY"]);
        assert_eq!(o[5].choices, vec!["DYNAMIC", "MIDNIGHT", "DAWN", "NOON", "DUSK"]);
        let k = at(&g, "Hollywood/Circuit/hw_circuit_jp_03");
        let c = g.mc4_arcade_rows(Some(k));
        assert_eq!(c.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), vec!["NUMBER OF OPPONENTS", "LAST MAN OUT", "DIFFICULTY", "NUMBER OF LAPS", "POWER UPS", "TRAFFIC", "WEATHER", "TIME OF DAY"]);
        assert_eq!((c[3].slot, c[3].choices.len(), c[3].default), (6, 5, g.race_defs[k].laps as usize - 1));
        // (the laps reach the race from the menu only)
        let def = g.race_defs[k].clone();
        assert_eq!(g.mc4_arcade_laps(&def), None);
        g.arcade_restore = Some((k, 3));
        g.arcade_x[0] = 4;
        assert_eq!(g.mc4_arcade_laps(&def), Some(5));
        let odef = g.race_defs[ord].clone();
        assert_eq!(g.mc4_arcade_laps(&odef), None);
        g.arcade_restore = None;
        // (a car given: its page comes before the groups, with the string of the car)
        g.mc4_rewards_begin();
        assert!(!g.mc4_rewards_listed());
        g.career.give_car("timetrial_vp_mit_eclipse_99");
        assert_eq!(g.career.cars_now, vec!["timetrial_vp_mit_eclipse_99".to_string()]);
        g.career.unlocked_now = vec!["FrontBumper".to_string()];
        let p = g.mc4_reward_pages();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], vec![(14, "1999 ECLIPSE GSX".to_string(), String::new())]);
        assert_eq!(p[1][0].0, 13);
        assert_eq!(g.mc4_car_reward_label("no_such_car"), "!!no_such_car!!");
        g.mc4_rewards_begin();
        assert!(g.career.cars_now.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 18: the time trials - targets, best times, the rows of the panel, the end of a run.
    #[test]
    fn time_trials() {
        // (the first row of Hollywood in the 100 % save: 130293 from a best time of 130881)
        assert_eq!(tt_evaluate(130881, 125000), Some(130293));
        assert_eq!((tt_evaluate(120000, 125000), tt_evaluate(0, 125000)), (Some(119985), None));
        assert!(tt_beaten(130299, 130293) && tt_beaten(130293, 130293) && !tt_beaten(130300, 130293));
        assert_eq!((tt_clock(130293), tt_clock(59999), tt_clock(0)), ("02:10.29".to_string(), "00:59.99".to_string(), "00:00.00".to_string()));
        std::env::remove_var("MCLA_TT_PICK");
        let Some((g, dir, _)) = race_game("newgame_import.json", "s18ng") else { return };
        // (a new game: nothing saved - the first targets of the scripts answer; the panel has its first row only)
        assert!(!g.mc4_tt_unlocked(0) && !g.mc4_tt_open(0));
        assert_eq!((g.mc4_tt_target(0, 0), g.mc4_tt_target(4, 3), g.mc4_tt_best(0, 0)), (145000, 135000, None));
        let rows = g.mc4_tt_rows(0);
        assert_eq!(rows[0], TimeTrialRow { car: "1999 ECLIPSE GSX".into(), target: "02:25.00".into(), best: "--:--.--".into(), enabled: true });
        assert!(rows[1..].iter().all(|r| !r.enabled && r.target.is_empty()));
        assert_eq!(g.mc4_tt_of("game/Race/Hollywood/Circuit/hw_circuit_mb_01"), None);
        let mut m = g.mc4.clone().unwrap();
        assert!(!g.mc4_tt_apply_block(&mut m));
        let _ = std::fs::remove_dir_all(&dir);

        let Some((mut g, dir, _)) = race_game("full_import.json", "s18full") else { return };
        let mut m = g.mc4.clone().unwrap();
        assert!(!g.mc4_tt_apply_block(&mut m));
        assert!((0..5).all(|d| g.mc4_tt_unlocked(d) && g.mc4_tt_open(d)));
        assert_eq!((g.mc4_tt_target(0, 0), g.mc4_tt_target(0, 1), g.mc4_tt_best(0, 0), g.mc4_tt_best(4, 0), g.mc4_tt_best(4, 1)), (130293, 150000, Some(130287), Some(161967), None));
        let rows = g.mc4_tt_rows(0);
        assert!(rows.iter().all(|r| r.enabled));
        assert_eq!((rows[0].target.as_str(), rows[0].best.as_str(), rows[3].car.as_str()), ("02:10.29", "02:10.28", "2006 GALLARDO SPYDER"));
        // (the race is the time trial of the district - not when it is started from ARCADE)
        let id = "game/Race/Hollywood/Circuit/hw_circuit_mb_01";
        assert_eq!(g.mc4_tt_of(id), Some(0));
        g.arcade_restore = Some((0, 3));
        assert_eq!(g.mc4_tt_of(id), None);
        g.arcade_restore = None;
        assert_eq!(g.mc4_tt_run(id), Some(("vp_mit_eclipse_99".to_string(), "timetrial_vp_mit_eclipse_99".to_string(), 130.293)));
        g.career.tt_pick = 2;
        assert_eq!(g.mc4_tt_run(id).unwrap().0, "vp_frd_mustang_06");
        g.career.tt_pick = 0;
        let k = g.race_defs.iter().position(|d| d.id.eq_ignore_ascii_case(id)).unwrap();
        let mut def = g.race_defs[k].clone();
        g.mc4_tt_apply(&mut def);
        assert_eq!((def.laps, def.goal_time), (1, 130.293));
        g.current_race = Some(k);
        assert_eq!(g.mc4_fixed_difficulty(), Some(1));
        // (the start badge's line, GetRaceInfo's event kind 2 - `cap/tt_07`)
        assert_eq!(g.mc4_goal_badge().as_deref(), Some("TARGET TIME: 02:10.29"));
        // (a run above the target: no money, the best time stays, the panel says so)
        let money = g.money;
        assert_eq!(g.mc4_tt_end(id, Some(131.0)), Some(false));
        assert_eq!(g.mission_results[0], (format!("{}TIME", TT_PANEL), vec!["TARGET TIME: 02:10.29".to_string(), "YOUR TIME: 02:11.00".to_string()], false));
        g.award_race(1, 1);
        assert_eq!((g.money, g.mc4_tt_best(0, 0), g.last_reward), (money, Some(130287), (0, 0)));
        // (a beaten time: the best time, the money of the event at Medium, no reputation)
        let rep = g.rep;
        assert_eq!(g.mc4_tt_end(id, Some(129.5)), Some(true));
        g.award_race(1, 1);
        assert_eq!((g.mc4_tt_best(0, 0), g.rep, g.career.tt_last), (Some(129500), rep, Some(true)));
        assert!(g.last_reward.0 > 0 && g.last_reward.1 == 0, "{:?}", g.last_reward);
        assert_eq!(g.mc4_tt_end("game/Race/Hollywood/Ordered/hw_ordered_jp_11", Some(100.0)), None);
        assert_eq!(g.career.tt_last, None);
        // (the best time is in the save)
        g.write_save(&dir);
        let t = std::fs::read_to_string(std::path::Path::new(&dir).join("mcla_save.json")).unwrap();
        let v: Value = serde_json::from_str(&t).unwrap();
        assert_eq!(v["career"]["ostats"]["k:TB/0"], 129500.0);
        assert_eq!(v["mc4"]["career"]["time_trials"][0]["target_ms"][0], 130293);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 18b: the tightening when a mission is put at READY_TO_START, the counters; step
    /// 19: the district locks of the slot system.
    #[test]
    fn tightening_and_district_locks() {
        let Some((mut g, dir, _)) = race_game("full_import.json", "s19full") else { return };
        // (the second mission of Hollywood is put at READY_TO_START: row 0 is tightened by the
        // best time, 130287 over the bound 125000 - once)
        assert_eq!(g.mc4_tt_target(0, 0), 130293);
        g.career.ready_now.push("TIMETRIAL_HOLLYWOODB".to_string());
        g.career.ready_now.push("SOME_OTHER_MISSION".to_string());
        g.mc4_tt_sync();
        assert_eq!(g.mc4_tt_target(0, 0), 130287 - (130287 - 125000) / 10);
        assert!(g.career.ready_now.is_empty());
        g.mc4_tt_sync();
        assert_eq!((g.mc4_tt_target(0, 0), g.mc4_tt_target(0, 1), g.mc4_tt_target(1, 0)), (129759, 150000, 226712));
        // (the 100 % save: every district open, and the rule leaves it so)
        assert!(DISTRICTS.iter().all(|d| g.mc4_district_open(d)));
        let mut m = g.mc4.clone().unwrap();
        assert!(!g.mc4_slots_apply(&mut m));
        // (a run starts: the hookman is visited)
        g.career.ostats.remove("tTTA/250");
        let k = g.race_defs.iter().position(|d| d.id.eq_ignore_ascii_case("game/Race/Hollywood/Circuit/hw_circuit_mb_01")).unwrap();
        g.mc4_race_started(k);
        assert_eq!(g.career.ostats.get("tTTA/250"), Some(&1.0));
        let _ = std::fs::remove_dir_all(&dir);

        let Some((mut g, dir, _)) = race_game("newgame_import.json", "s19ng") else { return };
        // (a new game: Hollywood alone; the missions have opened nothing yet)
        assert_eq!(DISTRICTS.iter().map(|d| g.mc4_district_open(d)).collect::<Vec<_>>(), vec![true, false, false, false, false]);
        assert!(g.mc4_district_open("All") && g.mc4_district_open(""));
        let mut m = g.mc4.clone().unwrap();
        assert!(!g.mc4_slots_apply(&mut m));
        // (a block of the port: a new career gets the new game's locks - every district locked,
        // every slot hidden: F-18978, F-57269 -, a career under way none; the import was
        // taken after RACE_GH_02 was won, which opens Hollywood)
        g.fresh_game = true;
        let mut m = Mc4::seeded(true);
        assert!(g.mc4_slots_apply(&mut m));
        assert_eq!(m.career.cruising_slots, g.mc4.as_ref().unwrap().career.cruising_slots);
        assert!(g.mc4_open_evaluates(0) && g.mc4_open_evaluates(3));
        let rows = |g: &Game| DISTRICTS.iter().map(|d| g.mc4_district_open(d)).collect::<Vec<_>>();
        let saved = g.mc4.take();
        assert_eq!(rows(&g), vec![false; 5]);
        g.mc4 = saved;
        // (before RACE_GH_02 and RACE_SH_01 are won: Hollywood locked and hidden as well)
        g.career.new_game();
        assert!(!g.career.state_at_least("RACE_GH_02", 5) && !g.career.state_at_least("RACE_SH_01", 5));
        let mut m = Mc4::seeded(true);
        g.mc4_slots_apply(&mut m);
        assert!(m.career.cruising_slots.iter().all(|s| s.district_locked && s.slots_hidden == vec![true; 3]));
        assert!(!g.mc4_open_evaluates(0));
        g.mc4 = None;
        assert!(!g.mc4_district_open("Hollywood") && !g.mc4_district_open("Downtown"));
        g.fresh_game = false;
        assert!(g.mc4_district_open("Downtown"));
        let mut m = Mc4::seeded(false);
        assert!(g.mc4_slots_apply(&mut m));
        assert!(m.career.cruising_slots.iter().all(|s| !s.district_locked && s.slots_hidden == vec![false; 3]));
        // (every mission of the table is a mission of the career)
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// N-HK-1: the flashing icon - the man the current objective of his mission points at.
    #[test]
    fn the_flashing_icon_follows_the_objective() {
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "pulse") else { return };
        g.career.new_game();
        // (a new career: RACE_GH_06 open, its objective 0 is TREVOR's - he flashes)
        assert!(g.career.mission_active("RACE_GH_06"));
        assert_eq!(g.career.pulse_objective("RACE_GH_06"), Some(0), "state {:?}", g.career.mission_no("RACE_GH_06").map(|i| g.career.state_id_of(i)));
        assert_eq!(target_objective("RACE_GH_06", "GH_06"), 0);
        // (a mission not under way: nothing)
        assert_eq!(g.career.pulse_objective("RACE_GH_02"), None);
        assert_eq!((target_objective("DISTRICT_CHAMPS", "GH_20"), target_objective("DISTRICT_CHAMPS", "GH_11"), target_objective("DISTRICT_CHAMPS", "GH_10"), target_objective("BESTOF_SH_05", "SH_05")), (0, 1, 2, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 19b: the performance bands, the mod value and the flash test.
    #[test]
    fn hookman_setups_and_the_flash_test() {
        // (F-44143: the steps -0.15, 0.025, 0.15, 0.3, 0.45)
        assert_eq!([-0.2f32, -0.15, 0.0, 0.025, 0.14, 0.15, 0.29, 0.3, 0.44, 0.45].map(gap_difficulty), [-1, 0, 0, 1, 1, 2, 2, 3, 3, 5]);
        assert_eq!(diff_window(0.5, 2), (0.65, 0.8));
        assert_eq!(diff_window(0.5, 3), (0.8, 0.95));
        assert_eq!(diff_window(0.5, 0), (0.35, 0.525));
        // (a vehicle from 0.4 over 0.6 to 0.9 against a player at 0.5: Hard is 0.65..0.8, all
        // of it within reach - the middle, 0.725, is 50 + (0.125 / 0.3) x 50 = 70.8 -> 71)
        let s = [0.4, 0.6, 0.9];
        assert!(setup_can_reach(s, 0.5, 2));
        assert_eq!(setup_mod_value(s, 0.5, 2), 71);
        // (the original's AROUND HOLLYWOOD on MEDIUM against a fully upgraded Saleen S281, 0.790155:
        // its opponents measured at 98 / 78 / 82 - docs/reference/ai/upgrades.md)
        let m = [[0.580801, 0.707273, 0.822529], [0.701087, 0.803725, 0.888737], [0.710337, 0.798889, 0.861136]].map(|s| setup_mod_value(s, 0.790155, 1));
        assert_eq!(m, [98, 78, 82]);
        assert!((setup_perf(s, 71) - 0.726).abs() < 1e-3);
        assert_eq!(gap_difficulty(setup_perf(s, 71) - 0.5), 2);
        // (a weak vehicle cannot be Hard: its band is cut at its best, the mod value is 100)
        let w = [0.3, 0.4, 0.5];
        assert!(!setup_can_reach(w, 0.5, 2));
        assert_eq!(setup_mod_value(w, 0.5, 2), 100);
        // (a strong one cannot be Easy: stock)
        assert!(!setup_can_reach([0.9, 1.0, 1.1], 0.5, 0));
        assert_eq!(setup_mod_value([0.9, 1.0, 1.1], 0.5, 0), 0);
        assert_eq!(setup_perf(s, 0), 0.4);
        assert_eq!(setup_perf(s, 50), 0.6);
        assert_eq!(setup_perf(s, 100), 0.9);
        // (Racer_CanFlashHookman: 30 m, 25 of speed, 0.2 ahead)
        assert!(can_flash(30.0, 0.0, 0.21) && can_flash(5.0, -25.0, 1.0));
        // (step 19d: 0.2 itself is ahead - the routine fails the test with `ble` on c . dir
        // against -0.2, F-96972 A-35936)
        assert!(!can_flash(30.1, 0.0, 1.0) && !can_flash(10.0, 25.5, 1.0) && can_flash(10.0, 0.0, 0.2) && !can_flash(10.0, 0.0, 0.19) && !can_flash(10.0, 0.0, -1.0));
        // (the three races a local hookman keeps: his first, then those never run in the
        // order of the array, then the ones run longest ago)
        let hist = |r: usize| match r { 3 => Some(5), 4 => Some(1), 6 => Some(0), _ => None };
        assert_eq!(local_hookman_filter(&[1, 2, 3, 4, 5], 9, 3, &hist), vec![9, 1, 2]);
        assert_eq!(local_hookman_filter(&[3, 4, 1, 6], 3, 3, &hist), vec![3, 1, 6]);
        assert_eq!(local_hookman_filter(&[3, 4, 6], 7, 3, &hist), vec![7, 6, 4]);
        assert_eq!(local_hookman_filter(&[3], 3, 3, &hist), vec![3]);
    }

    /// Step 19b: the slot system by the routines of the original.
    #[test]
    fn cruising_slot_system() {
        let mut s = SlotSystem::load(&data_dir());
        if s.slots.iter().all(|d| d.is_empty()) {
            return;
        }
        // (the file: five districts, three slots each with the script of its loop, one
        // threshold - Easy, Medium, Hard)
        assert_eq!(s.slots.iter().map(|d| d.len()).collect::<Vec<_>>(), vec![3; 5]);
        assert_eq!((s.slots[0][0].script.as_str(), s.slots[0][0].default_id, s.slots[4][2].script.as_str()), ("game/CruisingHookmen/hw_slot2", 101, "sc/CruisingHookmen/sc_slot4"));
        assert!(s.thresholds.iter().all(|t| t.len() == 1 && t[0] == (0.0, vec![0, 1, 2])));
        assert_eq!(s.characters.iter().map(|c| c.len()).collect::<Vec<_>>(), vec![7, 7, 6, 7, 8]);
        assert_eq!(s.characters[2].iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec!["GH_06", "GH_07", "GH_12", "GH_13", "GH_14", "GH_17"]);
        assert_eq!(s.threshold_index(0, 0.5), 0);
        let mut car = |name: &str, d: i32| Some((format!("vp_car_{d}"), format!("{name}_{d}_vp_car"), 10 * d));
        // (a locked district is not evaluated)
        s.locked = [false, true, true, true, true];
        for d in 0..5 {
            s.evaluate(d, 0.3, &mut car);
        }
        assert!(s.slots[1].iter().all(|x| x.hookman.is_none() && x.priority == PRIO_FREE));
        // (Hollywood: one of each difficulty, three different characters of its list, priority 5)
        let h: Vec<SlotHookman> = s.slots[0].iter().map(|x| x.hookman.clone().unwrap()).collect();
        assert_eq!(h.iter().map(|x| x.difficulty).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert!(h[0].name != h[1].name && h[1].name != h[2].name && h[0].name != h[2].name);
        assert!(h.iter().all(|x| s.characters[0].iter().any(|c| c.0 == x.name && c.1) && x.core == CORE_GENERIC && x.mission.is_empty()));
        assert!(s.slots[0].iter().all(|x| x.priority == PRIO_GENERIC));
        assert_eq!(s.characters[0].iter().filter(|c| c.1).count(), 3);
        // (a second evaluation changes nothing)
        let before = s.slots[0].clone();
        s.evaluate(0, 0.3, &mut car);
        assert_eq!(s.slots[0], before);
        // (a mission asks for a slot, priority 1, its man Hard: no slot is free, so the
        // generic hookman of the same difficulty gives way - the third slot)
        let man = |name: &str, diff: i32, d: usize, mission: &str| SlotHookman { name: name.into(), config: format!("{name}_1_vp_x"), car: "vp_x".into(), core: "core".into(), mod_value: 50, difficulty: diff, district: d, mission: mission.into() };
        assert!(s.request(man("SH_02", 2, 0, "BEAT_SH_02_DCLASS"), PRIO_MISSION));
        assert_eq!((s.slots[0][2].hookman.as_ref().unwrap().name.as_str(), s.slots[0][2].priority), ("SH_02", 1));
        assert!(!s.characters[0].iter().any(|c| c.0 == h[2].name && c.1));
        // (the evaluation finds Hard answered for by him)
        s.evaluate(0, 0.3, &mut car);
        assert_eq!(s.slots[0][2].hookman.as_ref().unwrap().name, "SH_02");
        // (a second mission man of a difficulty nobody of priority 5 has - Impossible -
        // takes the last slot with a priority number above his: the second)
        assert!(s.request(man("SH_03", 3, 0, "BEAT_SH_03"), PRIO_MISSION));
        assert_eq!(s.slots[0][1].hookman.as_ref().unwrap().name, "SH_03");
        // (a third takes the first; a fourth finds none and has to wait)
        assert!(s.request(man("GH_02", 1, 0, "RACE_GH_02"), PRIO_MISSION));
        assert!(!s.request(man("GH_06", 1, 0, "RACE_GH_06"), PRIO_MISSION));
        assert!(s.slots[0].iter().all(|x| x.priority == 1));
        // (the garage clears the districts: the men of priority 1 stay)
        s.clear_all();
        assert!(s.slots[0].iter().all(|x| x.hookman.is_some()));
        // (a mission is over: its slot is given back and the district evaluated - the free
        // slot gets the first difficulty nobody answers for: Easy)
        assert_eq!(s.remove_mission("BEAT_SH_03"), Some(0));
        assert_eq!(s.remove_mission("BEAT_SH_03"), None);
        assert_eq!(s.slots[0][1].priority, PRIO_FREE);
        s.evaluate(0, 0.3, &mut car);
        let g = s.slots[0][1].hookman.clone().unwrap();
        assert_eq!((g.difficulty, s.slots[0][1].priority, g.mission.as_str()), (0, PRIO_GENERIC, ""));
        // (a mission man goes into a locked district as well - the first free slot; a man
        // of the same name in a slot of priority 5 would be replaced where he is)
        assert!(s.request(man("SH_01", 2, 3, "FIND_SH_01"), PRIO_MISSION));
        assert_eq!(s.slots[3][0].hookman.as_ref().unwrap().mission, "FIND_SH_01");
        s.locked[2] = false;
        s.evaluate(2, 0.3, &mut car);
        let who = s.slots[2][1].hookman.as_ref().unwrap().name.clone();
        assert!(s.request(man(&who, 3, 2, "SOME_MISSION"), PRIO_MISSION));
        assert_eq!((s.slots[2][1].hookman.as_ref().unwrap().mission.as_str(), s.slots[2][1].priority), ("SOME_MISSION", 1));
        // (hide and unlock)
        s.hide_district(2, true);
        assert!(s.slots[2].iter().all(|x| x.hidden));
        s.unlock_slot(2, &who);
        assert_eq!(s.slots[2].iter().map(|x| x.hidden).collect::<Vec<_>>(), vec![true, false, true]);
        // (the characters: all in use -> the marks are cleared)
        for c in s.characters[4].iter_mut() {
            c.1 = true;
        }
        assert!(s.random_character(4).is_some());
        assert!(s.characters[4].iter().all(|c| !c.1));
        // (the table of the missions: 63 requests, all of priority 1; the first mission's
        // man is asked for at Easy, the second's at Medium, the others at Hard)
        assert_eq!(MISSION_HOOKMEN.len(), 63);
        assert!(MISSION_HOOKMEN.iter().all(|r| r.4 == 1 && r.1 < 5));
        assert_eq!(MISSION_HOOKMEN.iter().find(|r| r.0 == "RACE_GH_06").map(|r| (r.1, r.2, r.3, r.5, r.6)), Some((0, "GH_06", "GH_06_0_vp_dsn_280z_75", 0, "Game/Career/Story/RACE_GH_06_CORE")));
        assert_eq!(MISSION_HOOKMEN.iter().find(|r| r.0 == "RACE_GH_02").map(|r| r.5), Some(1));
        assert_eq!(MISSION_HOOKMEN.iter().filter(|r| r.0 == "DISTRICT_CHAMPS").map(|r| r.1).collect::<Vec<_>>(), vec![3, 2, 1]);
    }

    /// Step 19b: the hookmen's cars and races in a game; the time trial man's panel; the
    /// tournament's man.
    #[test]
    fn hookmen_cars_races_and_panels() {
        std::env::remove_var("MCLA_TT_PICK");
        let Some((mut g, dir, _)) = race_game("full_import.json", "s19hfull") else { return };
        g.samples = crate::rewards::Samples::load(&data_dir());
        g.rivals = crate::rivals::Rivals::load(&data_dir());
        let perf = g.player_perf();
        // (a character's car for a difficulty: one of his, no motorcycle, with the mod value
        // of the band - and the difficulty it then has is the one asked whenever a car of his
        // can reach it)
        for d in 0..3 {
            let (car, config, m) = g.mc4_character_car("GH_04", d).unwrap();
            assert!(config.to_ascii_lowercase().starts_with("gh_04_") && config.to_ascii_lowercase().ends_with(&car.to_ascii_lowercase()) && (0..=100).contains(&m), "{car} {config} {m}");
            let s = g.mc4_samples_of(&car).unwrap();
            if setup_can_reach(s, perf, d) {
                assert_eq!(gap_difficulty(setup_perf(s, m) - perf), d, "{car} {m}");
            }
        }
        assert!(g.mc4_character_car("NOBODY", 1).is_none());
        // (a mission's man: his set-up's vehicle, a difficulty of 1..3)
        let row = MISSION_HOOKMEN.iter().find(|r| r.0 == "BEAT_GH_04_MUSCLE").unwrap();
        let h = g.mc4_mission_hookman(row);
        assert_eq!((h.name.as_str(), h.car.as_str(), h.district, h.mission.as_str()), ("GH_04", "vp_chv_camaross_69", 0, "BEAT_GH_04_MUSCLE"));
        assert!((1..=3).contains(&h.difficulty) && (0..=100).contains(&h.mod_value));
        let sc = g.mc4_mission_hookman(MISSION_HOOKMEN.iter().find(|r| r.0 == "BEAT_GH_26_SUV").unwrap());
        assert_eq!(sc.car, "vpd_lr_hse_08");
        // (the slot system with the game's cars: every district of the 100 % save gets its three)
        let mut s = SlotSystem::load(&data_dir());
        for d in 0..5 {
            s.evaluate(d, perf, &mut |n, k| g.mc4_character_car(n, k));
            assert_eq!(s.slots[d].iter().filter_map(|x| x.hookman.as_ref()).map(|h| h.difficulty).collect::<Vec<_>>(), vec![0, 1, 2], "district {d}");
        }
        // (the local hookman's array: the unlocked ordered and circuit races, none test-only,
        // none a flag game or a delivery; the district argument is "all" with no RACES_IN
        // mission under way)
        assert_eq!(g.mc4_local_district(), 6);
        let arr = g.mc4_race_array(6, 2, 6);
        assert!(arr.len() > 40 && arr.windows(2).all(|w| w[0] < w[1]));
        assert!(arr.iter().all(|&t| { let r = g.mc4_race_record(&g.race_table[t]); r.1 == 1 && (r.2 == 1 || r.2 == 2) }));
        assert!(g.mc4_race_array(0, 2, 6).len() < arr.len());
        // (a hookman at Hollywood's centre facing north: his race is one of Hollywood, more
        // than 400 m off, and not in the history while such a race exists; the three he
        // keeps begin with it and are different races)
        let (pos, fwd) = (glam::Vec3::new(0.0, 10.0, -500.0), glam::Vec3::NEG_Z);
        let first = g.mc4_bmt_race(&arr, 0, pos, fwd, 400.0, 7).unwrap();
        let (_, p) = g.mc4_table_race(first).unwrap();
        assert_eq!(g.mc4_race_record(&g.race_table[first]).0, 0);
        assert!(((p.x - pos.x).powi(2) + (p.z - pos.z).powi(2)).sqrt() > 400.0);
        let fresh: Vec<usize> = arr.iter().copied().filter(|&t| g.mc4_race_record(&g.race_table[t]).0 == 0 && g.mc4_history_at(t).is_none() && g.mc4_table_race(t).map_or(false, |(_, q)| ((q.x - pos.x).powi(2) + (q.z - pos.z).powi(2)).sqrt() > 400.0)).collect();
        assert!(!fresh.is_empty() && fresh.contains(&first));
        // (of those it is the one most straight ahead)
        let ahead = |t: usize| fwd.dot((g.mc4_table_race(t).unwrap().1 - pos).normalize());
        assert!(fresh.iter().all(|&t| ahead(t) <= ahead(first) + 1e-6));
        // (facing the other way he picks another)
        assert_ne!(g.mc4_bmt_race(&arr, 0, pos, -fwd, 400.0, 7), Some(first));
        let kept = g.mc4_local_races(pos, fwd, 0, 7);
        assert_eq!((kept.len(), kept[0]), (3, first));
        assert!(kept[0] != kept[1] && kept[1] != kept[2] && kept[0] != kept[2]);
        // (a race that was just run is no longer his first choice)
        g.mc4.as_mut().unwrap().career.add_race_to_history(first as i64, None);
        assert_ne!(g.mc4_bmt_race(&arr, 0, pos, fwd, 400.0, 7), Some(first));
        // (the time trial man's panel in the 100 % save: every row can be taken)
        assert_eq!(Game::mc4_tt_panel_district("timetrial_dt"), Some(3));
        assert_eq!(Game::mc4_tt_panel_district("hw_tournament1"), None);
        for row in 0..4 {
            g.current_race = None;
            assert!(g.mc4_tt_panel_accept("timetrial_hw", row));
            assert_eq!(g.career.tt_pick, row);
            let i = g.current_race.unwrap();
            assert!(g.race_defs[i].id.eq_ignore_ascii_case(TIME_TRIALS[0].race));
            assert_eq!(g.mc4_tt_run(&g.race_defs[i].id.clone()).map(|r| r.1), Some(TIME_TRIALS[0].cars[row].to_string()));
        }
        // (the tournament's man is out in the 100 % save)
        assert_eq!(g.mc4_tournament_man(0).as_deref(), Some("hw_tournament1"));
        let _ = std::fs::remove_dir_all(&dir);

        // (a new game: the panel has its first row only; no tournament man yet; a slot is
        // hidden while its district is locked)
        let Some((mut g, dir, _)) = race_game("newgame_import.json", "s19hng") else { return };
        g.career.tt_pick = 0;
        assert!(!g.mc4_tt_panel_accept("timetrial_hw", 1) && g.current_race.is_none());
        assert!(g.mc4_tt_panel_accept("timetrial_hw", 0) && g.current_race.is_some());
        assert_eq!(g.mc4_tournament_man(0), None);
        assert_eq!((g.mc4_slot_hidden(0, 0), g.mc4_slot_hidden(1, 2)), (false, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 19b: the district of a place by the zone map.
    #[test]
    fn district_of_a_place() {
        let Some(m) = DistrictMap::load(&data_dir()) else { return };
        // (the garages, the Bonaventure, the lookout in the hills)
        assert_eq!(m.district(glam::Vec3::new(824.3, 16.75, -791.7)), 0);
        assert_eq!(m.district(glam::Vec3::new(-3093.0, -25.0, 185.0)), 1);
        assert_eq!(m.district(glam::Vec3::new(131.3, 99.6, -1768.2)), 2);
        assert_eq!(m.district(glam::Vec3::new(1607.0, -3.0, -238.0)), 3);
        assert_eq!(m.district(glam::Vec3::new(-69.0, -20.0, 1337.0)), 4);
        // (a place off the map is taken at its edge)
        assert_eq!(m.district(glam::Vec3::new(-9000.0, 0.0, 185.0)), 1);
        // (most races of the table start in the district their record names - 157 of 182; the others are South Central landmark races and flag games placed elsewhere and a
        // dozen that start across a border)
        let defs = crate::races::load(&data_dir());
        let known: Vec<(usize, usize)> = defs.iter().filter(|d| d.beacon.length_squared() > 1.0).filter_map(|d| DISTRICTS.iter().position(|n| n.eq_ignore_ascii_case(&d.district)).map(|k| (k, m.district(d.beacon)))).collect();
        let same = known.iter().filter(|(a, b)| a == b).count();
        let bad: Vec<String> = defs.iter().filter(|d| d.beacon.length_squared() > 1.0).filter_map(|d| DISTRICTS.iter().position(|n| n.eq_ignore_ascii_case(&d.district)).filter(|k| *k != m.district(d.beacon)).map(|k| format!("{}:{}>{} ({:.0},{:.0})", d.name, k, m.district(d.beacon), d.beacon.x, d.beacon.z))).collect();
        assert!(known.len() > 150 && same * 100 >= known.len() * 80, "{} of {}: {:?}", same, known.len(), bad);
        assert_eq!(district_of_place(&data_dir(), glam::Vec3::new(824.3, 16.75, -791.7)), Some(0));
    }
    /// Step 19c: the ARCADE options record and what ACCEPT does; POWER UPS' Custom rows;
    /// LAST MAN OUT; a set-up's parts; the time trial's clock and panel; the payback leg.
    #[test]
    fn arcade_options_time_trial_forms_and_the_payback_leg() {
        // (the record: the defaults of F-87205 around the rows' fields)
        let all = [true; 11];
        let r = event_options(false, 0, 2, 0, all);
        assert_eq!((r[0], r[1], r[4], r[5], r[6]), (0xFF, 0xE0, 0x80, 10, 0xC0));
        assert_eq!((&r[8..12], &r[12..16], &r[16..20], r[20], r[21]), (&[0u8, 9, 0x27, 0xC0][..], &[0u8, 0, 0, 2][..], &[0u8; 4][..], 0, 0));
        // (NITRO is bit 7 of byte 0, PULSE bit 0; MIRROR, AGRO, RANDOM bits 7..5 of byte 1)
        let mut one = [false; 11];
        one[0] = true;
        assert_eq!(event_options(true, 2, 0, 4, one)[..2], [0x80, 0x00]);
        one = [false; 11];
        one[7] = true;
        one[9] = true;
        let r = event_options(true, 2, 0, 4, one);
        assert_eq!((r[0], r[1], r[15], r[19], r[20], r[21]), (0x01, 0x40, 0, 4, 1, 1));
        // (ACCEPT: the density scale, the clock, the weather - Foggy is weather 3, Stormy 2)
        assert_eq!(arcade_env(2, 0, 0), ArcadeEnv { traffic: 1.0, hour: None, weather: None });
        assert_eq!(arcade_env(1, 3, 4), ArcadeEnv { traffic: 0.5, hour: Some(17.4), weather: Some(3) });
        assert_eq!(arcade_env(0, 4, 1), ArcadeEnv { traffic: 0.0, hour: Some(0.0), weather: Some(2) });
        assert_eq!((arcade_env(2, 1, 2).hour, arcade_env(2, 2, 3).hour, arcade_env(2, 2, 3).weather), (Some(6.0), Some(12.0), Some(1)));
        // (LAST MAN OUT: four racers; nothing before the leader has a lap behind him)
        let mut l = Lmo::default();
        assert_eq!(l.step(5_000, &[(2, 0), (1, 0), (3, 0), (4, 0)], 0), None);
        // (the leader over the line, the last not: ten seconds)
        assert_eq!(l.step(60_000, &[(2, 0), (1, 1), (3, 0), (4, 0)], 0), Some(LmoEvent::Clock(70_000)));
        assert_eq!(l.step(69_000, &[(2, 1), (1, 1), (3, 1), (4, 0)], 0), None);
        // (the clock up: the racer of the last rank goes - not the one who was last when it was set)
        assert_eq!(l.step(70_001, &[(2, 1), (1, 1), (4, 1), (3, 0)], 0), Some(LmoEvent::Out(2)));
        assert_eq!((l.timer, l.out, l.eliminated.clone()), (-1, 1, vec![2]));
        // (one a lap: the leader's lap is the number out - nothing until his next lap)
        assert_eq!(l.step(71_000, &[(2, 1), (1, 1), (4, 1), (3, 0)], 0), None);
        assert_eq!(l.step(120_000, &[(3, 1), (1, 2), (4, 1), (2, 1)], 0), Some(LmoEvent::Clock(130_000)));
        // (the player last: racer 0)
        assert_eq!(l.step(130_500, &[(3, 1), (1, 2), (4, 1), (2, 2)], 0), Some(LmoEvent::Out(0)));
        // (a racer who finished counts as out: the leader's laps equal it - no clock)
        let mut l = Lmo::default();
        assert_eq!(l.step(1_000, &[(2, 0), (1, 1)], 1), None);
        // (a leg is won at the difficulty's share of the client's damage)
        assert_eq!([0, 1, 2, 3, 9].map(payback_threshold), [0.5, 0.75, 0.87, 1.0, 1.0]);
        // (a set-up's parts: the Hollywood time trial's Eclipse)
        let cfgs = std::path::Path::new(&data_dir()).join("resources/vehicle/configs");
        if let Ok(b) = std::fs::read(cfgs.join("timetrial_vp_mit_eclipse_99.xcc")) {
            let (cfg, rim, inches) = setup_parts(&b).unwrap();
            assert_eq!((cfg.bumper_r, cfg.bumper_f, cfg.skirts, cfg.spoiler, cfg.hood, cfg.exhaust), (3, 2, 2, 5, 0, 0));
            assert_eq!((cfg.extra.get("headlight"), cfg.extra.get("taillight"), cfg.extra.len()), (Some(&2), Some(&2), 2));
            assert_eq!((rim, inches), (39, 19));
            assert!(setup_parts(&b[..100]).is_none());
        }
        let Some((mut g, dir, _)) = race_game("full_import.json", "s19a") else { return };
        // (POWER UPS' Custom: eleven rows under it, all ON; DEFAULT's value 1)
        let at = |g: &Game, id: &str| g.race_defs.iter().position(|d| d.id.eq_ignore_ascii_case(&format!("game/Race/{id}"))).unwrap();
        let k = at(&g, "Hollywood/Circuit/hw_circuit_jp_03");
        assert_eq!(g.mc4_arcade_rows(Some(k)).len(), 8);
        g.mc4_arcade_set(2, 2);
        let rows = g.mc4_arcade_rows(Some(k));
        assert_eq!(rows.len(), 19);
        assert_eq!(rows[4].label, "POWER UPS");
        assert_eq!(rows[5..16].iter().map(|r| r.label.trim_start_matches('\u{b1}').to_string()).collect::<Vec<_>>(), vec!["NITRO", "ICE", "REVERSE STEERING", "STEALTH", "DISRUPTER", "SHIELD", "STOP", "PULSE", "MIRROR", "AGRO", "RANDOM"]);
        assert!(rows[5..16].iter().enumerate().all(|(i, r)| r.slot == ARCADE_CUSTOM_SLOT + i && r.choices == vec!["OFF", "ON"] && r.default == 1 && g.mc4_arcade_value(r.slot) == 1));
        assert_eq!(rows[16].label, "TRAFFIC");
        g.mc4_arcade_set(ARCADE_CUSTOM_SLOT + 1, 0);
        g.mc4_arcade_set(ARCADE_CUSTOM_SLOT + 10, 0);
        assert_eq!((g.mc4_arcade_value(ARCADE_CUSTOM_SLOT + 1), g.mc4_arcade_value(ARCADE_CUSTOM_SLOT)), (0, 1));
        let rec = g.mc4_arcade_record();
        assert_eq!((rec[0], rec[1], rec[21]), (0xBF, 0xC0, 1));
        // (nothing of it without the menu; with it: traffic, clock, weather, the laps of LAST MAN OUT)
        assert_eq!((g.mc4_arcade_env(), g.mc4_lmo_on()), (None, false));
        g.current_race = Some(k);
        g.arcade_restore = Some((k, g.race_defs[k].opponents));
        g.race_defs[k].opponents = 3;
        g.mc4_arcade_set(3, 1);
        g.mc4_arcade_set(4, 4);
        g.mc4_arcade_set(5, 3);
        assert_eq!(g.mc4_arcade_env(), Some(ArcadeEnv { traffic: 0.5, hour: Some(12.0), weather: Some(2) }));
        let def = g.race_defs[k].clone();
        g.mc4_arcade_set(6, 4);
        assert_eq!((g.mc4_lmo_on(), g.mc4_arcade_laps(&def)), (false, Some(5)));
        g.mc4_arcade_set(7, 1);
        assert_eq!((g.mc4_lmo_on(), g.mc4_arcade_laps(&def)), (true, Some(3)));
        assert_eq!(g.mc4_hud_total(4), 4);
        g.opt_x.arcade.lmo = Some(Lmo { timer: -1, out: 1, eliminated: vec![2], last_ms: 0 });
        assert_eq!(g.mc4_hud_total(4), 3);
        // (an ordered race has no such row: not LAST MAN OUT whatever the widget holds)
        let ord = at(&g, "Hollywood/Ordered/hw_ordered_jp_11");
        g.current_race = Some(ord);
        assert!(!g.mc4_lmo_on());
        g.arcade_restore = None;
        g.opt_x.arcade.lmo = None;
        // (the test hook)
        // (the time trial's panel wears the form's mark; the payback's HUD figures need a leg)
        let id = "game/Race/Hollywood/Circuit/hw_circuit_mb_01";
        assert_eq!(g.mc4_tt_end(id, Some(200.0)), Some(false));
        assert!(g.mission_results[0].0.starts_with(TT_PANEL) && g.mission_results[0].0.ends_with("TIME"));
        assert_eq!(g.mission_results[0].1.len(), 2);
        assert_eq!((g.mc4_hud_clock(), g.mc4_payback_hud()), (None, None));
        // (the run to the line's thirty seconds on the race clock: 12.5 s left, the pie from 10 s)
        assert!(g.mc4_bmt_hud().is_none());
        g.opt_x.arcade.bmt_left.set(Some(12.5));
        let h = g.mc4_bmt_hud().unwrap();
        assert_eq!((h.time_left, h.clock_count, h.elapsed, h.total), (Some(12.5), Some((1.25, false)), 17.5, 1));
        g.opt_x.arcade.bmt_left.set(None);
        // (the car page's pictures: by the texture's name, whatever its case)
        if cfgs.is_dir() {
            assert!(g.mc4_reward_picture("timetrial_vp_mit_eclipse_99").map_or(false, |p| p.is_file()));
            assert!(g.mc4_reward_picture("SC_TOURNAMENT_vpd_lr_sport_08").is_some());
            assert!(g.mc4_reward_picture("10_vp_dge_challenger_70").is_none());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 19d: the payback's tables and its chase's bookkeeping, the factors and the meter's
    /// table, a set-up's look and dress, the local hookman's meeting and NEXT RACE, form 56,
    /// the counting down clock's sounds, the flash test's boundary.
    #[test]
    fn payback_chase_meeting_banner_clock_and_setups() {
        // (PaybackCore 0x848: the r-th free entry)
        let mut used = [false; 12];
        assert_eq!((payback_pick(&used, 0, 0), payback_pick(&used, 0, 5), payback_pick(&used, 0, 11)), (0, 5, 11));
        used[0] = true;
        used[5] = true;
        assert_eq!((payback_pick(&used, 2, 0), payback_pick(&used, 2, 4), payback_pick(&used, 2, 9)), (1, 6, 11));
        // (the tables of the payback's data: +3388 the player's factor, +3368 the meter's mark)
        assert_eq!((PAYBACK_INFLICT, PAYBACK_CLIENT_INFLICT), ([3.0; 4], (0.65, 0.55)));
        assert_eq!(PAYBACK_METER_MAX, [0, 1, 2, 3].map(payback_threshold));
        // (twelve clients without a repeat, then the table again with the last one in it; five
        // drivers the same way)
        let mut g = Game::new(Vec::new(), 0);
        g.career.payback_run = true;
        let mut cars = std::collections::HashSet::new();
        let mut drivers = Vec::new();
        for k in 0..12 {
            let (car, cfg, drv) = g.mc4_payback_pick();
            assert!(PAYBACK_CLIENTS.contains(&cfg.as_str()) && car == cfg.trim_start_matches("pyback_"));
            assert!(cars.insert(cfg));
            assert!(PAYBACK_DRIVERS.contains(&drv.as_str()));
            if k < 5 {
                drivers.push(drv);
            }
        }
        drivers.sort();
        drivers.dedup();
        assert_eq!((drivers.len(), g.opt_x.arcade.d.cars_n), (5, 1));
        // (the clients are stock whatever their set-up says; the chasers are not)
        if let Some((rg, _, _)) = race_game("full_import.json", "s19d") {
            if let Some(d) = rg.mc4_dress_of("vp_chv_impala_96", "pyback_vp_chv_impala_96") {
                assert_eq!(d.perf, None);
            }
            if let Some(d) = rg.mc4_dress_of("vp_pon_transam_79", "pyback_gn_vp_pon_transam_79") {
                assert_eq!((d.perf, d.inches, d.tire.as_str(), d.brake.as_deref()), (Some(1.0), 16, "tir_car_bfg_rta_ol", Some("brk_car_brembo_zinc")));
            }
        }
        // (a leg in which the chasers were lost: no win, no money, the run goes on)
        g.opt_x.arcade.d.lost_leg = true;
        g.last_reward = (5, 5);
        assert!(g.mc4_payback_take_lost());
        assert_eq!((g.career.payback_next, g.last_reward, g.opt_x.arcade.d.next_call), (true, (0, 0), true));
        assert!(!g.mc4_payback_take_lost());
        // (paybackisvisible: 1, then 2 once the time is up)
        assert_eq!(g.mc4_payback_vis(), 1);
        g.opt_x.arcade.pb = Some(PaybackLeg { phase: 2, ..Default::default() });
        assert_eq!(g.mc4_payback_vis(), 2);
        // (a set-up's look: the payback's Grand National - black, gloss; the front bumper in
        // grey of its own, the hood in the body's colour; spokes painted, the lip brushed)
        if let Ok(b) = std::fs::read(std::path::Path::new(&data_dir()).join("resources/vehicle/configs/pyback_plr_vp_buk_grandnational_87.xcc")) {
            let s = setup_look(&b).unwrap();
            assert!((s.color[0] - 0.012).abs() < 0.001 && s.color[3] == 1.0);
            assert_eq!((s.look.paint_type, s.tire.as_str(), s.brake.as_deref()), (2, "tir_car_bfg_kdw_blk", Some("brk_car_base")));
            // (areas after the body: doors, hood, front bumper, rear bumper, skirts)
            assert_eq!(s.look.area_a.map(|a| a[3]), [1.0, 1.0, 1.0, 1.0, 1.0]);
            assert!((s.look.area_a[2][0] - to_linear([0.349, 0.0, 0.0])[0]).abs() < 0.002);
            // (doors in Matte: drawn with MATTE_B and the falloff; w = type + 8 x fade)
            assert_eq!(s.look.area_b[0][3], 0.0 + 8.0);
            assert_eq!(s.look.rim_finish.map(|r| r[3]), [1.0, 3.0, 1.0]);
            assert!(setup_look(&b[..100]).is_none());
        }
        // (the meeting: the first time the tutorial; police near - the run is skipped)
        let mut g = Game::new(Vec::new(), 0);
        let man = Some(("GH_10".to_string(), "vp_lam_gallardo_06".to_string(), "GH_10_3_vp_lam_gallardo_06".to_string()));
        assert!(!g.mc4_meet(man.clone(), 7, &[3, 4, 5], false, true));
        assert_eq!(g.career.ostats.get("tTBM/0"), Some(&1.0));
        // (step 23: form 102 queued in the slow motion)
        assert!(g.opt_x.s23.queue.iter().any(|r| r.form == 102 && r.slow && r.speech == "TUTORIAL_BEAT_ME_THERE") == !g.fresh_game);
        assert_eq!(g.opt_x.arcade.d.say.last().map(|s| s.as_str()), Some("GREETING_BEAT_ME_THERE"));
        assert_eq!((g.opt_x.arcade.d.race_man.clone(), g.opt_x.arcade.d.man_race, g.opt_x.arcade.d.next_race), (man.clone(), Some(7), true));
        g.opt_x.arcade.d.say.clear();
        assert!(g.mc4_meet(man.clone(), 7, &[3, 4, 5], true, true));
        assert_eq!(g.opt_x.arcade.d.say, vec!["GREETING_MEET_ME_THERE_COPS".to_string()]);
        // (a mission's own man: no NEXT RACE)
        assert!(!g.mc4_meet(man.clone(), 7, &[], false, false));
        assert_eq!((g.opt_x.arcade.d.race_man.clone(), g.mc4_next_race_on()), (None, false));
        // (NEXT RACE: the first of his three not in the history; the second takes the row away)
        if let Some((mut rg, _, _)) = race_game("full_import.json", "s19d2") {
            let k = |n: &str| rg.race_table.iter().position(|t| t.to_ascii_lowercase().ends_with(n)).unwrap();
            let three = [k("hw_circuit_jp_03"), k("hw_circuit_gs_01"), k("dt_circuit_mb_01")];
            let first = rg.mc4_table_race(three[0]).unwrap().0;
            rg.hookman_race = false;
            rg.current_race = Some(first);
            rg.mc4_meet(man.clone(), first, &three, false, true);
            assert!(rg.mc4_next_race_on());
            let hist = |rg: &Game, t: usize| rg.mc4.as_ref().map_or(false, |m| m.career.history_position(t as i64).is_some());
            let want = three.iter().copied().find(|t| !hist(&rg, *t)).unwrap_or(three[0]);
            let next = rg.mc4_next_race_take().unwrap();
            assert_eq!((Some(next), rg.current_race), (rg.mc4_table_race(want).map(|r| r.0), Some(next)));
            assert_eq!(rg.opt_x.arcade.d.say.last().map(|s| s.as_str()), Some("GREETING_MEET_ME_THERE_EVENT"));
            assert!(rg.mc4_next_race_on());
            assert!(rg.mc4_next_race_take().is_some());
            assert!(!rg.mc4_next_race_on());
        }
        // (form 56: template_Menu, iconID 4, the text by bComplete, 3.5 s)
        let mut g = Game::new(Vec::new(), 0);
        assert!(g.mc4_banner_page(0.1).is_none());
        g.opt_x.arcade.d.banner = Some((3.5, true));
        let p = g.mc4_banner_page(1.0).unwrap();
        assert_eq!((p.icon, p.title.as_str(), p.rows.len(), p.select, p.prompts.len()), (4, "NEW TIME POSTED", 0, 1, 1));
        g.opt_x.arcade.d.banner = Some((0.5, false));
        assert_eq!(g.mc4_banner_page(1.0).unwrap().title, "TIME TRIAL FAILED");
        assert!(g.mc4_banner_page(1.0).is_none() && g.opt_x.arcade.d.banner.is_none());
        // (a series' and a tournament's: icons 3 and 5, SERIES WON / TOURNAMENT LOST - F-94174)
        g.opt_x.arcade.d.banner = Some((3.5, true));
        g.opt_x.arcade.d.banner_kind = 3;
        let p = g.mc4_banner_page(0.1).unwrap();
        assert_eq!((p.icon, p.title.as_str()), (3, "SERIES WON"));
        g.opt_x.arcade.d.banner = Some((3.5, false));
        g.opt_x.arcade.d.banner_kind = 4;
        let p = g.mc4_banner_page(0.1).unwrap();
        assert_eq!((p.icon, p.title.as_str()), (5, "TOURNAMENT LOST"));
        // (a series is decided by the first to three wins, a rival's too)
        g.series = Some(crate::game::Series { wins: vec![("YOU".into(), 2), ("RIVAL".into(), 2)], ..Default::default() });
        assert!(!g.series_decided());
        g.series.as_mut().unwrap().wins[1].1 = 3;
        assert!(g.series_decided());
        // (the clock's sounds: the loop under 20 s, a beep a second from 10 s, the speech once)
        let mut g = Game::new(Vec::new(), 0);
        let steps: [(Option<i64>, bool, usize, usize); 8] = [(Some(21_000), false, 0, 0), (Some(19_900), true, 0, 0), (Some(10_500), true, 0, 0), (Some(9_990), true, 1, 1), (Some(9_000), true, 1, 1), (Some(8_999), true, 2, 1), (Some(8_999), false, 2, 1), (None, false, 2, 1)];
        for (left, tick, beeps, said) in steps {
            g.mc4_clock_sounds(left);
            let d = &g.opt_x.arcade.d;
            assert_eq!((d.tick_loop, d.sounds.len(), d.say.len()), (tick, beeps, said), "{left:?}");
        }
        assert_eq!((g.opt_x.arcade.d.sounds[0], g.opt_x.arcade.d.say[0].as_str()), ("ui/menus/beep_fin", "PLAYER_TIME_TRIAL_TIMES_UP"));
        // (the clock run out: no more beeps, the loop off)
        g.mc4_clock_sounds(Some(500));
        g.mc4_clock_sounds(Some(-10));
        g.mc4_clock_sounds(Some(-20));
        assert!(!g.opt_x.arcade.d.tick_loop);
        // (the flash test: ahead by 0.2 or more is ahead - F-96972's `ble`)
        assert!(can_flash(30.0, 25.0, 0.2) && !can_flash(30.01, 0.0, 1.0) && !can_flash(10.0, 25.5, 1.0) && !can_flash(10.0, 0.0, 0.19));
    }

    /// Step 20: the samples' mission records come back into the port - the log's order and
    /// the completed list, the hidden bytes, the baselines - and an untouched save writes them
    /// back as they were read (with the career loaded, which the cycle test above has not).
    #[test]
    fn mission_records_of_the_samples() {
        for (name, done_n, open) in [("full_import.json", 130usize, 10usize), ("newgame_import.json", 2, 2)] {
            let Some((mut g, dir, save)) = career_game(name, "s20rec") else { return };
            let c = &save["mc4"]["career"];
            let names = |v: &Value| v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<String>>();
            let done: Vec<String> = g.career.done.iter().map(|&i| g.career.mission_id(i).to_string()).collect();
            let log: Vec<String> = g.career.log_order().iter().map(|&i| g.career.mission_id(i).to_string()).collect();
            assert_eq!(done, names(&c["completed_order"]), "{name}");
            assert_eq!(done.len(), done_n);
            assert_eq!(log, names(&c["active_order"]), "{name}");
            assert_eq!(log.len(), open);
            // (the COMPLETED tab lists them in the order of completion; the hidden ones not)
            let tab = g.career.completed_views();
            let shown: Vec<String> = done.iter().filter(|id| tab.iter().any(|v| v.id == **id)).cloned().collect();
            assert_eq!(tab.iter().map(|v| v.id.clone()).collect::<Vec<_>>(), shown);
            assert!(!tab.is_empty());
            g.write_save(&dir);
            let b = saved_block(&dir);
            for key in ["missions", "active_order", "completed_order", "scripts", "addon"] {
                same(&c[key], &b["career"][key], &format!("{name}: {key}"));
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Step 20: baselines. The new game sample's REP_ON_HW_STRIP armed at 260 reputation asks
    /// for 400 more (not 400 in all); the mission log puts the count after the objective.
    #[test]
    fn experience_baseline_and_the_count() {
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s20exp") else { return };
        let i = g.career.mission_no("REP_ON_HW_STRIP").unwrap();
        assert_eq!(g.career.ext[i].obj_base[0], Some(260));
        let line = |g: &Game| g.career.active().into_iter().find(|v| v.id == "REP_ON_HW_STRIP").map(|v| (v.objectives[0].cur, v.objectives[0].count.clone()));
        assert_eq!(line(&g), Some((0, Some("(0/400)".to_string()))));
        g.rep = 600;
        g.career.event(crate::career::Ev::Rep(600));
        assert_eq!(line(&g), Some((340, Some("(340/400)".to_string()))));
        assert!(g.career.mission_active("REP_ON_HW_STRIP"), "400 in all is not enough");
        // (GitHub #10: the first hangout waits for this objective - its reward mission
        // HANGOUT_SERIES_HOLLYWOOD starts Game/Hangouts/hw_local_hangout1_setup)
        assert!(!g.mc4_hangout_open("hangout/cut_hangout_01") && !g.mc4_hangout_open("hangout/cut_hangout_04"));
        g.rep = 660;
        g.career.event(crate::career::Ev::Rep(660));
        assert!(g.career.mission_complete("REP_ON_HW_STRIP"));
        // (the reward waits for its call at READY_TO_START; its own script - the set-up's
        // start - runs when the call has put it in the log, F-58560)
        assert!(g.career.mission_active("HANGOUT_SERIES_HOLLYWOOD") && !g.mc4_hangout_open("hangout/cut_hangout_01"));
        g.career.call_delivered("HANGOUT_SERIES_HOLLYWOOD");
        assert!(g.mc4_hangout_open("hangout/cut_hangout_01"));
        assert!(!g.mc4_hangout_open("hangout/cut_hangout_04"), "the wager hangout comes later (HANGOUT_SERIES_HOLLYWOODC)");
        // (it joined the completed list at its end; the record says so)
        assert_eq!(g.career.done.last().map(|&k| g.career.mission_id(k)), Some("REP_ON_HW_STRIP"));
        g.write_save(&dir);
        let b = saved_block(&dir);
        let rec = b["career"]["missions"].as_array().unwrap().iter().find(|r| r["name"] == "REP_ON_HW_STRIP").unwrap().clone();
        assert_eq!((rec["state_id"].as_u64(), rec["completed_slot"].as_i64(), rec["active_slot"].is_null()), (Some(5), Some(2), true));
        assert_eq!((rec["objectives"][0]["satisfied"].as_bool(), rec["objectives"][0]["conditions"][0]["start_amount"].as_i64()), (Some(true), Some(260)));
        assert_eq!(b["career"]["completed_order"][2], "REP_ON_HW_STRIP");
        assert!(!b["career"]["active_order"].as_array().unwrap().iter().any(|x| x == "REP_ON_HW_STRIP"));
        assert_eq!(b["career"]["active_order"][0], "GARAGE_HOLLYWOOD");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 20: a statistic's objective counts from the value the save holds as its baseline;
    /// without one (a save of the port from before) from 0, as the port did; a Purse statistic
    /// as it stands.
    #[test]
    fn stat_baselines() {
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s20stat") else { return };
        let mut s = g.career.save();
        let mut tuner = crate::career::MState::default();
        tuner.stage = crate::career::Stage::Active;
        s.missions.insert("TUNER_REP".to_string(), tuner.clone());
        s.missions.insert("EARN_1MILLION".to_string(), tuner);
        s.order.push("TUNER_REP".to_string());
        s.order.push("EARN_1MILLION".to_string());
        s.stats.insert("kTUW".to_string(), 4);
        s.stats.insert("tPUR".to_string(), 600_000);
        g.career.restore(&s);
        let t = g.career.mission_no("TUNER_REP").unwrap();
        let e = g.career.mission_no("EARN_1MILLION").unwrap();
        let cur = |g: &Game, id: &str| g.career.active().into_iter().find(|v| v.id == id).map(|v| (v.objectives[0].cur, v.objectives[0].count.clone()));
        g.career.event(crate::career::Ev::Rep(g.rep));
        assert_eq!(cur(&g, "TUNER_REP"), Some((4, Some("(4/6)".to_string()))), "no baseline: counted from 0");
        g.career.restore(&s);
        g.career.ext_from_save(t, &[false], &[Some(1)], &[]);
        g.career.event(crate::career::Ev::Rep(g.rep));
        assert_eq!(cur(&g, "TUNER_REP"), Some((3, Some("(3/6)".to_string()))));
        assert_eq!(cur(&g, "EARN_1MILLION"), Some((600_000, Some("($600,000/$1,000,000)".to_string()))));
        g.career.ext_from_save(e, &[false], &[Some(500_000)], &[]);
        g.career.event(crate::career::Ev::Rep(g.rep));
        assert_eq!(cur(&g, "EARN_1MILLION").map(|x| x.0), Some(600_000), "a Purse statistic as it stands");
        // (the record: start 1, the statistic now)
        g.write_save(&dir);
        let b = saved_block(&dir);
        let rec = b["career"]["missions"].as_array().unwrap().iter().find(|r| r["name"] == "TUNER_REP").unwrap().clone();
        assert_eq!((rec["state_id"].as_u64(), rec["objectives"][0]["hidden"].as_bool()), (Some(3), Some(false)));
        assert_eq!((rec["objectives"][0]["conditions"][0]["start_value"].as_i64(), rec["objectives"][0]["conditions"][0]["value"].as_i64()), (Some(1), Some(4)));
        // (and back: the baseline is the record's)
        let mut g2 = Game::new(g.car_dirs.clone(), 0);
        g2.data_dir = data_dir();
        g2.career = crate::career::Career::load(&data_dir());
        g2.load_save(&dir);
        assert_eq!(g2.career.ext[t].obj_base[0], Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Career queue item 6: every titled mission has a start in the data (the tables' rewards, another mission's,
    /// a Mission prerequisite, or the start-up script's RACE_GH_06) - none is left for the port to place
    /// (career2 `orphans/notes.md`: 141 of 142 rewarded).
    #[test]
    fn every_mission_has_a_start() {
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            return;
        }
        let c = crate::career::Career::load(&data_dir());
        assert!(c.unstarted().is_empty(), "{:?}", c.unstarted());
    }

    /// A tournament's standings after each race (form 49: this race's points 7 / 4 / 2 / 1 and the
    /// total, decided only with its last race) and RESTART TOURNAMENT (every point at nought, its
    /// first race again).
    #[test]
    fn tournament_standings_and_restart() {
        use crate::career::{Career, MState, SaveState, Stage};
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            return;
        }
        let mut c = Career::load(&data_dir());
        let mut s = SaveState::default();
        s.missions.insert("RACE_GH_06".into(), MState { stage: Stage::Complete, ..Default::default() });
        s.missions.insert("TOURNAMENT_BEACHES".into(), MState { stage: Stage::Active, ..Default::default() });
        c.restore(&s);
        let title = c.title_of("TOURNAMENT_BEACHES").unwrap_or_default();
        let races: Vec<String> = c.mission_races().into_iter().filter(|(_, t)| *t == title).map(|(r, _)| r).collect();
        assert!(races.len() >= 2, "{races:?}");
        let order = |me: usize| -> Vec<String> {
            let mut o = vec!["A".to_string(), "B".to_string(), "C".to_string()];
            o.insert(me, "YOU".to_string());
            o
        };
        c.tournament_race(&format!("game/race/{}", races[0]), &order(1)).unwrap();
        let t = c.tour_table.clone().unwrap();
        assert_eq!((t.rows[0].clone(), t.rows[1].clone(), t.decided, t.last), (("A".to_string(), 7, 7), ("YOU".to_string(), 4, 4), None, false));
        c.tournament_race(&format!("game/race/{}", races[1]), &order(0)).unwrap();
        let t = c.tour_table.clone().unwrap();
        assert_eq!(t.rows[0], ("YOU".to_string(), 7, 11));
        assert_eq!(t.last, races.len() == 2);
        assert_eq!(c.tournament_restart(&format!("game/race/{}", races[1])), Some(races[0].clone()));
        assert_eq!(c.mission_races().into_iter().filter(|(_, t)| *t == title).count(), races.len());
    }

    /// CareerSetup_SC (`sc/career/0x70e862b0`): a mission at CHECKING_PREREQUISITES whose
    /// missions are complete goes to READY_TO_START; one whose missions are not stays.
    #[test]
    fn sc_setup_starts_waiting_missions() {
        use crate::career::{Career, MState, SaveState, Stage};
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            return;
        }
        let mut c = Career::load(&data_dir());
        let st = |stage| MState { stage, ..Default::default() };
        let mut s = SaveState::default();
        for (id, stage) in [("RACE_GH_06", Stage::Complete), ("GARAGE_HOLLYWOOD", Stage::Complete), ("GARAGE_SC", Stage::Armed), ("DELIVERY_SC", Stage::Armed), ("LOWRIDER_REP", Stage::Active)] {
            s.missions.insert(id.into(), st(stage));
        }
        c.restore(&s);
        assert_eq!(c.sc_setup(), vec!["GARAGE_SC".to_string()]);
        let stage = |c: &Career, id: &str| c.mission_no(id).map(|i| c.stage_of(i));
        assert_eq!(stage(&c, "GARAGE_SC"), Some(Stage::Active));
        // (DELIVERY is not complete)
        assert_eq!(stage(&c, "DELIVERY_SC"), Some(Stage::Armed));
        assert!(c.sc_setup().is_empty());
    }

    /// The first career race (RACE_GH_06) runs at Easy: its core script sets the man's
    /// difficulty to 0 (`0x40a56c35` 0x1b7), and the recomp paid $100 for second of two - the
    /// curve's Easy money (250 x 0.8 x 1/2); BEAT_SH_05's core sets 1.
    #[test]
    fn story_men_difficulty_from_their_cores() {
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            return;
        }
        let mut g = Game::new(Vec::new(), 0);
        g.data_dir = data_dir();
        g.career = crate::career::Career::load(&data_dir());
        g.fresh_game = true;
        g.career.new_game();
        g.career.call_delivered("RACE_GH_06");
        assert_eq!(g.career.hookman_mission(), Some("RACE_GH_06"));
        assert_eq!(g.mc4_hookman_diff(), Some(0));
        assert_eq!(g.mc4_story_difficulty("BEAT_SH_05"), Some(1));
        assert_eq!(g.mc4_story_difficulty("FIND_SH_05"), Some(1));
        // (RACE_GH_06's record takes the easy layout whatever its difficulty; RACE_GH_02's man is
        // wrapped again by CruisingHookmanCore - never Easy - and FIND_SH_01's too)
        g.hookman_race = true;
        assert_eq!(g.mc4_layout_easy(), Some(true));
        g.samples = crate::rewards::Samples::load(&data_dir());
        assert!(g.mc4_story_difficulty("RACE_GH_02").is_some_and(|d| d >= 1));
        assert!(g.mc4_story_difficulty("FIND_SH_01").is_some_and(|d| d >= 1));
        let (money, rep) = crate::rewards::race_reward(0, g.player_perf(), 0, 2, 2, 1.0);
        assert_eq!((money, rep), (100, 50));
    }

    /// Step 20: a career begun in the port writes the records and the scripts the original's
    /// new game has after its first two races: the new game sample's, entry for entry.
    #[test]
    fn a_new_career_writes_the_new_game_records() {
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            return;
        }
        let Some(sample) = fixture("newgame_import.json") else { return };
        let dir = std::env::temp_dir().join(format!("mcla_mc4_s20new_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.to_string_lossy().to_string();
        let mut g = Game::new(Vec::new(), 0);
        g.data_dir = data_dir();
        g.career = crate::career::Career::load(&data_dir());
        g.fresh_game = true;
        g.career.new_game();
        use crate::career::Ev;
        for (race, rep, call) in [("game/Race/Hollywood/Ordered/hw_ordered_jp_11", 100, "RACE_GH_06"), ("game/Race/Hollywood/Ordered/hw_ordered_ds_02", 260, "RACE_GH_02")] {
            // (READY_TO_START until the mission's call has been made: step 20b)
            let k = g.career.mission_no(call).unwrap();
            assert_eq!((g.career.state_id_of(k), g.career.log_order().contains(&k)), (2, false), "{call}");
            g.career.call_delivered(call);
            assert_eq!((g.career.state_id_of(k), g.career.log_order().contains(&k)), (3, true), "{call}");
            g.career.event(Ev::Race { district: "Hollywood", place: 1, hookman: true, class: "TUNER", race, beat_goal: false });
            g.rep = rep;
            g.career.event(Ev::Rep(rep));
        }
        let ids = |v: &[usize]| v.iter().map(|&i| g.career.mission_id(i).to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&g.career.done), vec!["RACE_GH_06", "RACE_GH_02"]);
        assert_eq!(ids(g.career.log_order()), vec!["GARAGE_HOLLYWOOD", "REP_ON_HW_STRIP"]);
        g.write_save(&dir);
        let b = saved_block(&dir);
        let want = &sample["mc4"]["career"];
        let names = |v: &Value| v.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names(&b["career"]["scripts"]), names(&want["scripts"]));
        same(&want["scripts"], &b["career"]["scripts"], "scripts");
        same(&want["active_order"], &b["career"]["active_order"], "active order");
        same(&want["completed_order"], &b["career"]["completed_order"], "completed order");
        // (every record as the original's: names, indices, states, slots, requirements)
        same(&want["missions"], &b["career"]["missions"], "missions");
        // (step 21: the lot - five empty slots, the counter at 0)
        same(&want["pink_slips"], &b["career"]["pink_slips"], "pink slips");
        assert_eq!(b["career"]["addon"]["present"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 20: the hidden bytes. A story mission's second objective comes out once the
    /// hookman is met and goes back when the series is lost; after a load the always-hidden
    /// byte is the XML's again. The scripts a mission's script starts, once each.
    #[test]
    fn hidden_objectives_and_mission_scripts() {
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s20hid") else { return };
        let mut s = g.career.save();
        let mut st = crate::career::MState::default();
        st.stage = crate::career::Stage::Active;
        s.missions.insert("CITY_CHAMP".to_string(), st);
        s.order.push("CITY_CHAMP".to_string());
        g.career.restore(&s);
        let i = g.career.mission_no("CITY_CHAMP").unwrap();
        let shown = |g: &Game| g.career.active().into_iter().find(|v| v.id == "CITY_CHAMP").map(|v| v.objectives.len()).unwrap_or(0);
        assert_eq!(shown(&g), 1);
        assert_eq!((g.career.ext[i].obj_hidden.clone(), g.career.ext[i].obj_always.clone()), (vec![false, true], vec![false, true]));
        // (a won race before the meeting does nothing; the meeting is at the hookman's place,
        // 20 m, not with the police on the tail - CITY_CHAMP_CORE, step 20b)
        let races = g.career.event_races("CITY_CHAMP");
        let race = format!("game/race/{}", races.first().cloned().unwrap_or_default());
        g.career.event(crate::career::Ev::Race { district: "Downtown", place: 1, hookman: true, class: "TUNER", race: &race, beat_goal: false });
        assert!(!races.is_empty());
        assert_eq!((shown(&g), g.career.save().missions["CITY_CHAMP"].progress.clone()), (1, vec![0, 0]));
        assert!(g.career.story_meet_at([1658.652, -1.997, 224.921 + 25.0], false).is_none());
        assert!(g.career.story_meet_at([1658.652, -1.997, 224.921 + 5.0], true).is_none());
        assert_eq!(g.career.story_meet_at([1658.652, -1.997, 224.921 + 5.0], false).as_deref(), Some("CITY_CHAMP"));
        g.career.event(crate::career::Ev::Race { district: "Downtown", place: 1, hookman: true, class: "TUNER", race: &race, beat_goal: false });
        {
            assert!(g.career.mission_active("CITY_CHAMP") && !g.career.ext[i].obj_always[1]);
            assert_eq!(shown(&g), 2);
            assert!(!g.career.ext[i].obj_hidden[1]);
            // (the rival takes three: the first step again)
            for _ in 0..3 {
                g.career.series_race(&race, "RIVAL");
            }
            assert_eq!((shown(&g), g.career.ext[i].obj_always[1], g.career.ext[i].obj_hidden[1]), (1, true, true));
        }
        // (the scripts: GARAGE_HOLLYWOOD's starts the garage once, though its script runs again)
        let mut c = Career::default();
        assert!(mission_starts(&mut c, "GARAGE_HOLLYWOOD", true));
        assert!(!mission_starts(&mut c, "INSTALL_ABILITY", true));
        assert!(mission_starts(&mut c, "REDLIGHT_GH_01", true));
        assert!(mission_starts(&mut c, "GARAGE_SC", true));
        assert!(mission_starts(&mut c, "TIMETRIAL_SCC", false));
        let n: Vec<(&str, u32, u32)> = c.scripts.iter().map(|s| (s.name.as_str(), s.stack_size, s.saved_bytes)).collect();
        assert_eq!(n[0], ("Game/Garages/hw_garage_setup", 1500, 4));
        assert_eq!(n[1], ("Game/CruisingHookmen/hw_redlight_hk1", 3800, 4));
        assert_eq!(n.len(), 6);
        assert_eq!(c.addon.scripts.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["sc/Garages/sc_garage_setup", "sc/CruisingHookmen/sc_redlight_hk1", "sc/CruisingHookmen/freeway_hookman7", "sc/CruisingHookmen/freeway_hookman8", "sc/CruisingHookmen/sc_time_trial_hk1"]);
        assert_eq!(script_record("Game/Flow/BootDebug", 1500).saved_bytes, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 20: the career's list of scripts is read - a set-up script running when the save
    /// was made is started again, and a launcher's races are unlocked as it starts (step 17b).
    #[test]
    fn the_script_list_is_read() {
        let Some((g0, dir, mut save)) = career_game("newgame_import.json", "s20list") else { return };
        let race = "game/Race/Beaches/Ordered/be_ordered_jj_03";
        assert!(!g0.mc4_race_unlocked(race));
        drop(g0);
        save["mc4"]["career"]["scripts"].as_array_mut().unwrap().push(serde_json::json!({"name": "Game/Hangouts/be_local_hangout1_setup", "stack_size": 1500, "saved_bytes": 4, "saved_words": [0]}));
        std::fs::write(std::path::Path::new(&dir).join("mcla_save.json"), serde_json::to_string(&save).unwrap()).unwrap();
        let mut g = Game::new(Vec::new(), 0);
        g.data_dir = data_dir();
        g.career = crate::career::Career::load(&data_dir());
        g.load_save(&dir);
        g.mc4_races_sync();
        assert!(g.mc4_race_unlocked(race));
        // (and the list goes back as it was read, the new entry where it stood)
        g.write_save(&dir);
        same(&save["mc4"]["career"]["scripts"], &saved_block(&dir)["career"]["scripts"], "scripts");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 20: a save without the block: the port's lists as before (the completed ones in the
    /// lists' order, nothing else being known), and at the first save the records and the
    /// scripts its missions stand for.
    #[test]
    fn a_save_without_the_block_gets_records() {
        let Some((g0, dir, mut save)) = career_game("newgame_import.json", "s20nb") else { return };
        drop(g0);
        let want = save["mc4"]["career"].clone();
        save.as_object_mut().unwrap().remove("mc4");
        std::fs::write(std::path::Path::new(&dir).join("mcla_save.json"), serde_json::to_string(&save).unwrap()).unwrap();
        let mut g = Game::new(Vec::new(), 0);
        g.data_dir = data_dir();
        g.career = crate::career::Career::load(&data_dir());
        g.load_save(&dir);
        assert!(g.mc4.is_none());
        let ids = |g: &Game, v: &[usize]| v.iter().map(|&i| g.career.mission_id(i).to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&g, &g.career.done), vec!["RACE_GH_02", "RACE_GH_06"]);
        // (no baseline: THE STRIP counts the reputation from 0, as the port did)
        assert_eq!(g.career.active().into_iter().find(|v| v.id == "REP_ON_HW_STRIP").map(|v| v.objectives[0].cur), Some(260));
        g.write_save(&dir);
        let b = saved_block(&dir);
        let names = |v: &Value| { let mut n: Vec<String> = v.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect(); n.sort(); n };
        assert_eq!(names(&b["career"]["scripts"]), names(&want["scripts"]));
        assert_eq!(names(&b["career"]["missions"]), names(&want["missions"]));
        let state = |v: &Value, id: &str| v["missions"].as_array().unwrap().iter().find(|r| r["name"] == id).map(|r| (r["state_id"].as_u64(), r["active_slot"].as_i64(), r["index"].as_i64()));
        for id in ["RACE_GH_06", "GARAGE_HOLLYWOOD", "REP_ON_HW_STRIP", "global", "TOURNAMENT_SC"] {
            assert_eq!(state(&b["career"], id), state(&want, id), "{id}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 20b: READY_TO_START is a state of its own - out of the log until the call, saved as
    /// 2 and back as 2 (the call again), then 3; TUNER_REP's call opens MUSCLE_REP; a
    /// bCareerScript ON_ENTER script is in the list while its mission waits.
    #[test]
    fn ready_to_start_saved_and_resumed() {
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s20bready") else { return };
        let mut s = g.career.save();
        s.missions.remove("TUNER_REP");
        g.career.restore(&s);
        // (TUNER_REP handed out at READY_TO_START: its call comes, the log has not got it)
        g.career.give_mission("TUNER_REP", "READYTOSTART");
        let t = g.career.mission_no("TUNER_REP").unwrap();
        assert_eq!(g.career.state_id_of(t), 2);
        assert!(!g.career.active().iter().any(|v| v.id == "TUNER_REP"));
        assert!(g.career.news.iter().any(|n| matches!(n, crate::career::News::Started { id, .. } if id == "TUNER_REP")));
        g.write_save(&dir);
        let rec = |dir: &str, id: &str| saved_block(dir)["career"]["missions"].as_array().unwrap().iter().find(|r| r["name"] == id).cloned().unwrap();
        let r = rec(&dir, "TUNER_REP");
        assert_eq!((r["state_id"].as_u64(), r["active_slot"].is_null(), r["objectives"][0]["hidden"].as_bool()), (Some(2), true, Some(false)));
        // (read back: READY_TO_START again, the call again)
        let mut g2 = Game::new(Vec::new(), 0);
        g2.data_dir = data_dir();
        g2.career = crate::career::Career::load(&data_dir());
        g2.load_save(&dir);
        assert_eq!(g2.career.state_id_of(t), 2);
        assert!(g2.career.news.iter().any(|n| matches!(n, crate::career::News::Started { id, .. } if id == "TUNER_REP")));
        g2.career.call_delivered("TUNER_REP");
        let m = g2.career.mission_no("MUSCLE_REP").unwrap();
        assert_eq!((g2.career.state_id_of(t), g2.career.state_id_of(m)), (3, 3));
        assert!(g2.career.active().iter().any(|v| v.id == "TUNER_REP") && g2.career.active().iter().any(|v| v.id == "MUSCLE_REP"));
        g2.write_save(&dir);
        let r = rec(&dir, "TUNER_REP");
        assert_eq!((r["state_id"].as_u64(), r["active_slot"].is_null()), (Some(3), false));
        // (PAYBACK_GH_18's ON_ENTER is a career script: in the list while it waits)
        g2.career.give_mission("PAYBACK_GH_18", "READYTOSTART");
        g2.write_save(&dir);
        let names = |dir: &str| saved_block(dir)["career"]["scripts"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert!(names(&dir).iter().any(|n| n == "Game/Career/Events/PAYBACK_GH_18_READY_TO_START_ON_ENTER"));
        g2.career.call_delivered("PAYBACK_GH_18");
        g2.write_save(&dir);
        assert!(!names(&dir).iter().any(|n| n.contains("PAYBACK_GH_18_READY")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 20b: k%AG as the original posts it; the money form by language; a met statistic's
    /// value frozen at the meeting moment.
    #[test]
    fn percent_money_and_frozen_values() {
        for (name, want) in [("full_import.json", 100), ("newgame_import.json", 1)] {
            let Some((g, dir, save)) = career_game(name, "s20bpct") else { return };
            assert_eq!(g.career.percent_complete(), want);
            assert_eq!(g.career.stats.get("k%AG").copied(), Some(want));
            assert_eq!(save["career"]["ostats"]["k%AG/0"].as_f64(), Some(want as f64));
            let _ = std::fs::remove_dir_all(&dir);
        }
        use crate::career::money_text_in;
        assert_eq!([money_text_in(30000, 0), money_text_in(30000, 1), money_text_in(1000000, 2), money_text_in(5000, 3), money_text_in(5000, 4), money_text_in(999, 0)], ["$30,000", "30.000$", "1|000|000$", "$5.000", "5.000$", "$999"]);
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s20bfrz") else { return };
        let mut s = g.career.save();
        let mut st = crate::career::MState::default();
        st.stage = crate::career::Stage::Active;
        s.missions.insert("TUNER_REP".to_string(), st);
        s.order.push("TUNER_REP".to_string());
        s.stats.insert("kTUW".to_string(), 7);
        g.career.restore(&s);
        let t = g.career.mission_no("TUNER_REP").unwrap();
        g.career.ext_from_save(t, &[false], &[Some(1)], &[]);
        g.career.event(crate::career::Ev::Rep(g.rep));
        assert!(g.career.mission_complete("TUNER_REP"));
        assert_eq!(g.career.ext[t].obj_last[0], Some(7));
        g.career.stats.insert("kTUW".to_string(), 9);
        g.write_save(&dir);
        let b = saved_block(&dir);
        let r = b["career"]["missions"].as_array().unwrap().iter().find(|r| r["name"] == "TUNER_REP").unwrap().clone();
        assert_eq!((r["objectives"][0]["conditions"][0]["value"].as_i64(), r["objectives"][0]["conditions"][0]["start_value"].as_i64()), (Some(7), Some(1)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 20b: the markers from the running scripts; a hangout left goes to the list's end.
    #[test]
    fn markers_from_the_running_scripts() {
        let Some((g, dir, _)) = career_game("newgame_import.json", "s20bmk") else { return };
        assert!(g.mc4_setup_shown("hw_garage_setup") && !g.mc4_setup_shown("hw_local_hangout1_setup") && !g.mc4_setup_shown("hw_redlight_hk1"));
        assert!(!g.mc4_hangout_open("hangout/cut_hangout_01"));
        assert_eq!(hangout_key_of_scene("x/sc_cut_hangout_02"), Some("sc_wager_hangout1"));
        assert_eq!(g.mc4_district_code(glam::Vec3::new(860.0, 17.0, -810.0)), "hw");
        let _ = std::fs::remove_dir_all(&dir);
        let Some((mut g, dir, _)) = career_game("full_import.json", "s20bmk2") else { return };
        assert!(g.mc4_hangout_open("hangout/cut_hangout_01") && g.mc4_setup_shown("dt_redlight_hk1") && g.mc4_setup_shown("be_garage_setup"));
        g.mc4_hangout_left("hw_local_hangout1");
        // (step 21b: the restarter waits at its point, 25 m for this one; the hangout is closed
        // meanwhile; within the distance the set-up starts again, at the list's end)
        assert!(!g.mc4_hangout_open("hangout/cut_hangout_01"));
        g.mc4_hangout_restart_check([727.875, 7.438, -279.156 + 30.0]);
        assert!(!g.mc4_hangout_open("hangout/cut_hangout_01"));
        g.mc4_hangout_restart_check([727.875, 7.438, -279.156 + 20.0]);
        assert!(g.mc4_hangout_open("hangout/cut_hangout_01"));
        let l = g.mc4.as_ref().unwrap().career.scripts.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(l.last().map(|s| s.as_str()), Some("Game/Hangouts/hw_local_hangout1_setup"));
        assert_eq!(l.len(), 35);
        // (the delivery's restarter starts Game/Delivery/hw_delivery_setup: that name, at the end)
        g.mc4_hangout_left("hw_delivery");
        g.mc4_hangout_restart_check([824.34, 16.75, -791.74]);
        let l = g.mc4.as_ref().unwrap().career.scripts.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(l.last().map(|s| s.as_str()), Some("Game/Delivery/hw_delivery_setup"));
        assert_eq!((l.len(), l.iter().filter(|n| n.ends_with("hw_delivery_setup")).count()), (35, 1));
        // (South Central's wager restarter waits at Hollywood's wager point, as on the disc)
        g.mc4_hangout_left("sc_wager_hangout1");
        g.mc4_hangout_restart_check([-140.0, 7.0, -470.595 + 3.0]);
        let a = g.mc4.as_ref().unwrap().career.addon.scripts.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(a.last().map(|s| s.as_str()), Some("sc/Hangouts/sc_wager_hangout1_setup"));
        // (a save of the port without the block: not known, the port's rules)
        let mut g2 = Game::new(Vec::new(), 0);
        g2.career = crate::career::Career::load(&data_dir());
        g2.career.journal_complete = false;
        assert!(g2.mc4_running_scripts().is_none() && g2.mc4_setup_shown("hw_local_hangout1_setup"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 21: the pink slip lot - five slots, the newest first, the oldest lost; a removed
    /// slot closes up; the id counter.
    #[test]
    fn pink_slip_lot_rules() {
        let mut p = PinkSlips::default();
        p.uid_counter = 16;
        let car = |n: &str| Car { TypeName: n.to_string(), vehicle: n.to_string(), ..Default::default() };
        let ids: Vec<i32> = ["a", "b", "c", "d", "e", "f"].iter().map(|n| p.add(1, &car(n))).collect();
        assert_eq!(ids, vec![17, 18, 19, 20, 21, 22]);
        assert_eq!(p.held().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["f", "e", "d", "c", "b"]);
        assert_eq!(p.remove(19).map(|s| s.name), Some("c".to_string()));
        assert_eq!(p.held().iter().map(|s| (s.slot, s.name.as_str())).collect::<Vec<_>>(), vec![(0, "f"), (1, "e"), (2, "d"), (3, "b")]);
        assert!(p.slots[4].empty && p.slots[4].uid == -1 && p.slots[4].kind == -1);
        assert!(p.remove(99).is_none());
        p.uid_counter = i32::MAX as u32;
        assert_eq!(p.next_uid(), 0);
        assert_eq!(setup_vehicle("SC_pinkslip_vpd_pon_gto_66"), "vpd_pon_gto_66");
        assert_eq!(pink_record(&p.slots[0]).map(|c| c.vehicle), Some("f".to_string()));
    }

    /// Step 21: a pink slip race at the Hollywood wager hangout - its own car while
    /// HANGOUT_PINKSLIP_HOLLYWOOD is open; won: the car and the mission; lost: the stake held
    /// in the lot, raced for again and won back with its record.
    #[test]
    fn pink_slip_races() {
        let Some((mut g, dir, _)) = career_game("full_import.json", "s21pink") else { return };
        assert!(g.mc4_pink_profiles("hw_wager_hangout1").is_empty(), "the 100 % save has won them all");
        let mut s = g.career.save();
        let mut st = crate::career::MState::default();
        st.stage = crate::career::Stage::Active;
        st.progress = vec![0];
        s.missions.insert("HANGOUT_PINKSLIP_HOLLYWOOD".to_string(), st);
        s.order.push("HANGOUT_PINKSLIP_HOLLYWOOD".to_string());
        g.career.restore(&s);
        let p = g.mc4_pink_profiles("hw_wager_hangout1");
        assert_eq!(p.iter().map(|x| (x.kind, x.car.as_str(), x.setup.as_str())).collect::<Vec<_>>(), vec![(0, "vp_pon_transam_79", "pinkslip_vp_pon_transam_79")]);
        assert!(g.mc4_pink_profiles("dt_wager_hangout1").is_empty() && g.mc4_pink_profiles("hw_local_hangout1").is_empty());
        let mine = g.mc4_car_name(g.current_car);
        let owned = g.owned.len();
        assert!(g.mc4_pink_start("hw_wager_hangout1", 0));
        assert_eq!(g.mc4_pink_rival().map(|r| (r.1, r.2)), Some(("vp_pon_transam_79".to_string(), "pinkslip_vp_pon_transam_79".to_string())));
        let lot = |g: &Game| g.mc4.as_ref().unwrap().career.pink_slips.held().iter().map(|s| (s.uid, s.kind, s.extra.get("vehicle_name").and_then(|v| v.as_str()).unwrap_or("").to_string())).collect::<Vec<_>>();
        assert_eq!(lot(&g), vec![(17, 1, mine.clone())]);
        assert_eq!(g.mc4_pink_end(true), Some("PINKSLIP_SUCCESS"));
        assert!(lot(&g).is_empty() && g.owned.len() == owned);
        assert!(g.career.mission_complete("HANGOUT_PINKSLIP_HOLLYWOOD"));
        assert!(g.career.news.iter().any(|n| matches!(n, crate::career::News::Car(c) if c == "vp_pon_transam_79")));
        assert_eq!(g.career.car_configs.get("vp_pon_transam_79").map(|s| s.as_str()), Some("pinkslip_vp_pon_transam_79"));
        // (a second race - the mission done, its own car gone: only the held cars; none yet)
        assert!(g.mc4_pink_profiles("hw_wager_hangout1").is_empty());
        g.career.restore(&s);
        assert!(g.mc4_pink_start("hw_wager_hangout1", 0));
        assert_eq!(g.mc4_pink_end(false), Some("PINKSLIP_FAILURE"));
        assert!(!g.owned.iter().any(|o| o.eq_ignore_ascii_case(&mine)) && g.owned.len() == owned - 1);
        assert_eq!(lot(&g), vec![(18, 1, mine.clone())]);
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!((b["career"]["pink_slips"]["uid_counter"].as_u64(), b["career"]["pink_slips"]["slots"][0]["type"].as_i64(), b["career"]["pink_slips"]["slots"][0]["uid"].as_i64()), (Some(18), Some(1), Some(18)));
        assert!(!b["garage"].as_array().unwrap().iter().any(|c| c["vehicle"].as_str() == Some(mine.as_str())));
        // (read back: the held car is offered at every wager hangout, after the hangout's own)
        let mut g2 = Game::new(g.car_dirs.clone(), 0);
        g2.data_dir = data_dir();
        g2.career = crate::career::Career::load(&data_dir());
        g2.load_save(&dir);
        g2.ensure_owned();
        g2.career.restore(&s);
        let p = g2.mc4_pink_profiles("be_wager_hangout1");
        assert_eq!(p.iter().map(|x| (x.kind, x.car.clone())).collect::<Vec<_>>(), vec![(1, mine.clone())]);
        let hw = g2.mc4_pink_profiles("hw_wager_hangout1");
        assert_eq!(hw.len(), 2);
        // (won back: the car and its record return, the lot is empty)
        let cur = g2.current_car;
        g2.current_car = (0..g2.car_dirs.len()).find(|&k| g2.is_owned(k) && k != cur).unwrap_or(cur);
        assert!(g2.mc4_pink_start("hw_wager_hangout1", 1));
        assert_eq!(g2.mc4_pink_end(true), Some("PINKSLIP_SUCCESS"));
        assert!(g2.owned.iter().any(|o| o.eq_ignore_ascii_case(&mine)));
        assert!(g2.mc4.as_ref().unwrap().car(&mine).is_some());
        assert!(g2.mc4.as_ref().unwrap().career.pink_slips.held().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 20: what a load restarts. A mission saved in READY_TO_START runs its ON_ENTER
    /// script again (`ready_now`); an open event's partial count is not carried over; the
    /// missions in the log start their scripts again (journal).
    #[test]
    fn a_load_restarts_the_missions() {
        let Some((g0, dir, mut save)) = career_game("full_import.json", "s20load") else { return };
        drop(g0);
        // (REPEAT_BESTOF_HILLS as if saved while its call was coming in; a partial series in the
        // port's own field)
        for r in save["mc4"]["career"]["missions"].as_array_mut().unwrap() {
            if r["name"] == "REPEAT_BESTOF_HILLS" {
                r["state_id"] = Value::from(2);
                r["state"] = Value::from("READY_TO_START");
            }
            // (its series won, the mission not yet stamped)
            if r["name"] == "REPEAT_BESTOF_DOWNTOWN" {
                r["objectives"][0]["satisfied"] = Value::from(true);
            }
        }
        if let Some(m) = save["career"]["missions"].get_mut("REPEAT_BESTOF_BEACHES") {
            m["progress"] = serde_json::json!([1]);
            m["pts"] = serde_json::json!({"YOU": 1});
            m["won"] = serde_json::json!(["beaches/ordered/be_ordered_jj_03"]);
        }
        std::fs::write(std::path::Path::new(&dir).join("mcla_save.json"), serde_json::to_string(&save).unwrap()).unwrap();
        let mut g = Game::new(Vec::new(), 0);
        g.data_dir = data_dir();
        g.career = crate::career::Career::load(&data_dir());
        g.load_save(&dir);
        assert!(g.career.ready_now.iter().any(|m| m == "REPEAT_BESTOF_HILLS"));
        let b = g.career.mission_no("REPEAT_BESTOF_BEACHES").unwrap();
        let st = g.career.save().missions["REPEAT_BESTOF_BEACHES"].clone();
        assert_eq!((st.progress.first().copied(), st.pts.len(), st.won.len()), (Some(0), 0, 0));
        assert!(g.career.journal.iter().any(|&(e, k)| e && k == b));
        assert!(g.career.journal.len() >= 10);
        // (a met String objective is done: the mission completes with the load's tally)
        assert!(g.career.mission_complete("REPEAT_BESTOF_DOWNTOWN"));
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// Step 22: VINYL PACKAGES' rows (the vehicle's own preset, the disc's, the stored slots),
    /// the boxes' YES, STORE VINYL PACKAGES written to the block and read back.
    #[test]
    fn vinyl_package_rows_and_store() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
        let p = |x: &str| PkgRow::Preset(x.to_string());
        let names = s(&["preset_apexi", "preset_bboy", "sc_preset_Gold"]);
        assert_eq!(vinyl_package_rows("vp_dge_challenger_70", &names, &[true, false, true], CONTENT_MASK_START[1]), vec![p("vp_dge_challenger_70"), p("preset_apexi"), p("preset_bboy"), p("sc_preset_Gold"), PkgRow::User(0), PkgRow::User(2)]);
        // (without content bit 4 a car gets no South Central presets; a bike's list is its own)
        assert_eq!(vinyl_package_rows("vp_aud_r8_08", &names, &[false; 3], 0), vec![p("preset_apexi"), p("preset_bboy")]);
        assert_eq!(vinyl_package_rows("vp_duc_999r_06", &s(&["bikepreset4"]), &[false, true, false], CONTENT_MASK_START[1]), vec![p("vp_duc_999r_06"), p("bikepreset4"), PkgRow::User(1)]);
        assert_eq!(CONTENT_MASK_START[1] & CONTENT_SC_PRESETS, CONTENT_SC_PRESETS);

        // the 100 % save: its three stored packages close the R8's list
        let Some((mut g, dir, save)) = game_on("full_import.json", "vinylpkg") else { return };
        let have_data = std::path::Path::new(&data_dir()).join("tune/garage").is_dir();
        if have_data {
            g.vinyl_presets = crate::vinyl::Presets::load(&data_dir());
        }
        g.vinyl_for = "vp_aud_r8_08".into();
        g.vinyl_list = vec!["custom".into(), "preset:preset_apexi".into(), "preset:preset_bboy".into()];
        assert_eq!(g.mc4_vinyl_used(), [true, true, true]);
        assert_eq!(g.mc4_vinyl_rows(), vec![p("preset_apexi"), p("preset_bboy"), PkgRow::User(0), PkgRow::User(1), PkgRow::User(2)]);
        // a preset put on (no popup movie in a test: the box's YES at once)
        g.mc4_vinyl_ask(VinylAsk::Load(p("preset_apexi")));
        assert_eq!(g.vinyls.get("vp_aud_r8_08").map(|s| s.as_str()), Some("preset:preset_apexi"));
        assert!(g.mc4_vinyl_has_layers() && g.car_x.vinyl_ask.is_none());
        // STORE VINYL PACKAGES -> PACKAGE 1: the preset's layers into slot 0
        g.car_x.vinyl_save = Some(0);
        assert!(g.mc4_vinyl_save_page(false, false, true, false));
        let apexi = g.vinyl_presets.livery("preset_apexi");
        assert_eq!(g.vinyls.get("vp_aud_r8_08").map(|s| s.as_str()), Some("custom"));
        // REMOVE ALL VINYLS: no layers, both rows closed again
        g.mc4_vinyl_ask(VinylAsk::RemoveAll);
        assert!(!g.mc4_vinyl_has_layers());
        // a stored package back on: the slot's layers
        g.mc4_vinyl_ask(VinylAsk::Load(PkgRow::User(0)));
        assert_eq!(serde_json::to_value(g.liveries.get("vp_aud_r8_08").unwrap()).unwrap(), serde_json::to_value(&apexi).unwrap());
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!(b["vinyl_packages"][0]["used"], true);
        assert_eq!(serde_json::from_value::<crate::vinyl::Livery>(b["vinyl_packages"][0]["livery"].clone()).unwrap().zones, apexi.zones);
        assert_eq!(b["vinyl_packages"][1], save["mc4"]["vinyl_packages"][1], "the other slots are kept");
        assert_eq!(b["vinyl_packages"][2], save["mc4"]["vinyl_packages"][2]);
        let _ = std::fs::remove_dir_all(&dir);

        // a new game has no stored package: the rows are the disc's; a store makes slot 2 alone used
        let Some((mut g, dir, _)) = game_on("newgame_import.json", "vinylpkg_ng") else { return };
        let car = g.mc4_car_name(g.current_car);
        g.vinyl_for = car.clone();
        g.vinyl_list = vec!["preset:preset_apexi".into()];
        assert_eq!(g.mc4_vinyl_rows(), vec![p("preset_apexi")]);
        let mut l = crate::vinyl::Livery::default();
        l.zones[0] = apexi.zones[0].clone();
        if l.is_empty() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        g.liveries.insert(car.clone(), l.clone());
        g.vinyls.insert(car.clone(), "custom".into());
        g.mc4_vinyl_ask(VinylAsk::Store(2));
        assert_eq!(g.mc4_vinyl_used(), [false, false, true]);
        assert_eq!(g.mc4_vinyl_rows(), vec![p("preset_apexi"), PkgRow::User(2)]);
        g.write_save(&dir);
        let cars = g.car_dirs.clone();
        let mut g2 = Game::new(cars, 0);
        g2.data_dir = dir.clone();
        g2.load_save(&dir);
        assert_eq!(g2.mc4_vinyl_used(), [false, false, true]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 21b: the call only for a mission that waits for it; the CORE scripts' scenes at a
    /// completion; REDLIGHT_GH_01 waits for the Hollywood garage; the GPS button follows
    /// RACE_GH_06's and RACE_GH_02's scripts.
    #[test]
    fn scripts_scenes_and_the_garage_mission() {
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            return;
        }
        use crate::career::{Ev, News};
        let mut g = Game::new(Vec::new(), 0);
        g.data_dir = data_dir();
        g.career = crate::career::Career::load(&data_dir());
        g.fresh_game = true;
        g.career.new_game();
        let started = |g: &Game| g.career.news.iter().filter_map(|n| if let News::Started { id, .. } = n { Some(id.clone()) } else { None }).collect::<Vec<_>>();
        let scenes = |g: &Game| g.career.news.iter().filter_map(|n| if let News::Scene(id) = n { Some(id.clone()) } else { None }).collect::<Vec<_>>();
        // RACE_GH_06's script switches the GPS button off as the career starts
        assert!(g.career.reached("RACE_GH_06"));
        assert!(!g.mc4_gps_enabled());
        g.career.call_delivered("RACE_GH_06");
        g.career.event(Ev::Race { district: "Hollywood", place: 1, hookman: true, class: "TUNER", race: "game/Race/Hollywood/Ordered/hw_ordered_jp_11", beat_goal: false });
        g.rep = 100;
        g.career.event(Ev::Rep(100));
        assert!(!g.mc4_gps_enabled(), "RACE_GH_02 waits for its call");
        // RACE_GH_02's call, the mission log's lesson open: still off; the lesson over: on
        g.career.news.clear();
        g.career.call_delivered("RACE_GH_02");
        g.career.stats.insert("tut:mission_log".into(), 1);
        assert!(!g.mc4_gps_enabled());
        g.career.stats.insert("tut:target".into(), 1);
        assert!(g.mc4_gps_enabled());
        // RACE_GH_02 complete: GARAGE_HOLLYWOOD and REP_ON_HW_STRIP (bAutoStart) have no call;
        // RACE_GH_02_CORE plays REP_ON_STRIP_GARAGE
        g.career.event(Ev::Race { district: "Hollywood", place: 1, hookman: true, class: "TUNER", race: "game/Race/Hollywood/Ordered/hw_ordered_ds_02", beat_goal: false });
        g.rep = 260;
        g.career.event(Ev::Rep(260));
        assert!(g.career.mission_complete("RACE_GH_02"));
        assert_eq!(started(&g), Vec::<String>::new(), "no call for a mission that passes READY_TO_START by");
        assert_eq!(scenes(&g), vec!["REP_ON_HW_STRIP".to_string()]);
        // the garage: GARAGE_HOLLYWOOD complete puts REDLIGHT_GH_01 at READY_TO_START (not in
        // the log); its event script at the garage's door plays the intro and moves it on
        g.career.event(Ev::Garage);
        let k = g.career.mission_no("REDLIGHT_GH_01").unwrap();
        assert!(g.career.mission_complete("GARAGE_HOLLYWOOD"));
        assert_eq!((g.career.state_id_of(k), g.career.log_order().contains(&k)), (2, false));
        assert_eq!(g.career.location_enter("BE_GARAGE_ENTER"), None);
        assert_eq!(g.career.location_enter("HW_GARAGE_ENTER").as_deref(), Some("GARAGE_HOLLYWOOD"));
        assert_eq!((g.career.state_id_of(k), g.career.log_order().contains(&k)), (3, true));
        assert_eq!(g.career.location_enter("HW_GARAGE_ENTER"), None, "once");
        assert!(crate::career::Career::scene_elsewhere("CITY_CHAMP") && !crate::career::Career::scene_elsewhere("RACE_GH_06"));
    }

    /// Step 21b: a story hookman's meeting plays his scene (the CORE's), the call does not.
    #[test]
    fn the_meeting_plays_the_scene() {
        let Some((mut g, _dir, _)) = game_on("full_import.json", "s21b_meet") else { return };
        if !std::path::Path::new(&data_dir()).join("tune/career").is_dir() {
            return;
        }
        g.data_dir = data_dir();
        g.career = crate::career::Career::load(&data_dir());
        g.career.new_game();
        g.career.give_mission("CITY_CHAMP", "CHECKINGOBJECTIVES");
        g.career.news.clear();
        let at = crate::career::STORY_POINTS.iter().find(|s| s.0 == "CITY_CHAMP").unwrap().1;
        assert_eq!(g.career.story_meet_at(at, false).as_deref(), Some("CITY_CHAMP"));
        assert!(g.career.news.iter().any(|n| matches!(n, crate::career::News::Scene(id) if id == "CITY_CHAMP")));
    }

    /// Step 21b: the pink slip race and man of the hangout's set-up, the stake out of the
    /// garage in a save written during the race, the vehicle back at the garage's end when
    /// won, a held car of an original save raced in its own record.
    #[test]
    fn pink_slip_race_man_and_mid_race_save() {
        let Some((mut g, dir, _)) = career_game("full_import.json", "s21bpink") else { return };
        let mut s = g.career.save();
        let mut st = crate::career::MState::default();
        st.stage = crate::career::Stage::Active;
        st.progress = vec![0];
        s.missions.insert("HANGOUT_PINKSLIP_HOLLYWOOD".to_string(), st);
        s.order.push("HANGOUT_PINKSLIP_HOLLYWOOD".to_string());
        g.career.restore(&s);
        let jp04 = g.mc4_race_index("game/Race/Hollywood/Ordered/hw_ordered_jp_04").and_then(|k| g.mc4_table_race(k)).map(|(i, _)| i);
        assert_eq!(PINK_SLIP_RACES[0].1, "game/Race/Hollywood/Ordered/hw_ordered_jp_04");
        assert_eq!(g.mc4_pink_race("hw_wager_hangout1"), jp04);
        let mine = g.mc4_car_name(g.current_car);
        assert!(g.mc4_pink_start("hw_wager_hangout1", 0));
        assert_eq!(g.mc4_pink_rival().map(|r| r.0), Some("GH_13".to_string()));
        // (a save during the race: the stake is not in the garage, the first vehicle in use)
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert!(!b["garage"].as_array().unwrap().iter().any(|c| c["vehicle"].as_str() == Some(mine.as_str())));
        let sd: Value = serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(&dir).join("mcla_save.json")).unwrap()).unwrap();
        assert!(!sd["owned"].as_array().unwrap().iter().any(|o| o.as_str().map_or(false, |o| o.eq_ignore_ascii_case(&mine))));
        assert!(g.owned.iter().any(|o| o.eq_ignore_ascii_case(&mine)), "the game itself still drives it");
        // (won: back, at the garage's end)
        assert_eq!(g.mc4_pink_end(true), Some("PINKSLIP_SUCCESS"));
        assert!(g.owned.last().map_or(false, |o| o.eq_ignore_ascii_case(&mine)));
        g.write_save(&dir);
        let b = saved_block(&dir);
        assert!(b["garage"].as_array().unwrap().iter().any(|c| c["vehicle"].as_str() == Some(mine.as_str())));
        // (an original save's held car: its record's bytes are the opponent's set-up)
        let rec = std::fs::read(std::path::Path::new(&data_dir()).join("resources/vehicle/configs/GH_13_2_vp_mit_evo_06.xcc")).ok();
        if let Some(rec) = rec {
            let hex: String = rec.iter().map(|b| format!("{:02x}", b)).collect();
            let m = g.mc4.as_mut().unwrap();
            let uid = m.career.pink_slips.add(1, &Car { TypeName: "vp_mit_evo_06".into(), vehicle: "vp_mit_evo_06".into(), ..Default::default() });
            let slot = m.career.pink_slips.slots.iter_mut().find(|x| x.uid == uid).unwrap();
            slot.extra.remove("car");
            slot.extra.insert("vehicle_record_hex".into(), Value::String(hex));
            slot.extra.insert("vehicle_name".into(), Value::String("vp_mit_evo_06".into()));
            let p = g.mc4_pink_profiles("be_wager_hangout1");
            let k = p.iter().position(|x| x.uid == uid).unwrap();
            assert!(g.mc4_pink_start("be_wager_hangout1", k));
            let r = g.mc4_pink_rival().unwrap();
            assert_eq!((r.0.as_str(), r.2.clone()), ("GH_07", format!("pinkslip:{uid}")));
            assert!(g.mc4_dress_of(&r.1, &r.2).is_some());
            g.mc4_pink_end(false);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// (probe, step 22) the fonts of the UI movies: what the money separator of the French
    /// form ('|', F-36073) is in each of them.
    #[test]
    #[ignore]
    fn s22_font_bar_probe() {
        fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            if let Ok(rd) = std::fs::read_dir(p) {
                for e in rd.flatten() {
                    let q = e.path();
                    if q.is_dir() { walk(&q, out) } else if q.extension().map_or(false, |x| x == "r27") { out.push(q) }
                }
            }
        }
        let mut files = Vec::new();
        walk(&std::path::Path::new(&data_dir()).join("resources/ui"), &mut files);
        for f in files {
            let Ok(res) = crate::assets::load_resource_file(&f) else { continue };
            let Ok(m) = mcla_formats::rage::swf::Movie::parse(&res) else { continue };
            for c in m.chars.iter().flatten() {
                if let mcla_formats::rage::swf::Character::Font(font) = c {
                    let g = |ch: char| font.glyph_index(ch).map(|i| (i, font.advances.get(i).copied().unwrap_or(-1.0), font.pages.first().and_then(|p| p.glyphs.get(i)).map(|gl| format!("{:?}", gl)).unwrap_or_default()));
                    println!("{} | {} glyphs {} | bar {:?} | space {:?} | dot {:?}", f.file_name().unwrap().to_string_lossy(), font.name, font.codes.len(), g('|'), g(' ').map(|x| (x.0, x.1)), g('.').map(|x| (x.0, x.1)));
                }
            }
        }
    }

    // ---- step 22b / 23 ----

    /// Step 23: an editor slot's checkpoints race with their own activation radius and height
    /// (Net_EditorRaceCreateCheckpoints), and a slot of another city is kept but not offered.
    #[test]
    fn editor_slots_radius_height_city() {
        assert!(editor_city_ok(3, 3) && editor_city_ok(0, 3) && editor_city_ok(0xCD, 3) && editor_city_ok(0, 0) && editor_city_ok(0xCD, 0));
        assert!(!editor_city_ok(1, 3) && !editor_city_ok(2, 3) && !editor_city_ok(0, 1) && !editor_city_ok(3, 0));
        let Some((mut g, dir, _)) = game_on("full_import.json", "s23editor") else { return };
        let m = g.mc4.as_mut().unwrap();
        assert_eq!(m.editor.races.len(), 1);
        let r = &mut m.editor.races[0];
        assert_eq!((r.count, r.city, r.checkpoints[1].activation_radius, r.checkpoints[1].activation_height), (30, 3, 25, 25));
        r.checkpoints[1].activation_radius = 12;
        r.checkpoints[1].activation_height = 7;
        let pts: Vec<glam::Vec3> = g.user_races[0].iter().map(|p| glam::Vec3::from(*p)).collect();
        let i = g.add_user_race(1, &pts).unwrap();
        let def = g.race_defs[i].clone();
        let specs = g.mc4_editor_specs(&def).unwrap();
        assert_eq!(specs.len(), 29);
        assert_eq!((specs[0].radius, specs[0].act_height, specs[0].height), (12.0, 7.0, 100.0));
        assert_eq!((specs[1].radius, specs[1].act_height), (25.0, 25.0));
        assert_eq!(specs[0].pos, pts[1]);
        // (a race of the game's own tables has none)
        let mut other = def.clone();
        other.id = "Hollywood/Ordered/hw_ordered_jp_04".into();
        assert!(g.mc4_editor_specs(&other).is_none());
        // (a slot of another city: kept in the block, not offered)
        g.mc4.as_mut().unwrap().editor.races[0].city = 1;
        assert!(!g.mc4_editor_listed(0));
        g.write_save(&dir);
        let mut h = Game::new(g.car_dirs.clone(), 0);
        h.data_dir = dir.clone();
        h.load_save(&dir);
        assert_eq!(h.user_races.len(), 1, "kept");
        assert!(!h.race_defs.iter().any(|d| d.id.starts_with("user/")), "not offered");
        h.write_save(&dir);
        let b = saved_block(&dir);
        assert_eq!(b["editor"]["races"][0]["city"], 1);
        assert_eq!(b["editor"]["races"][0]["checkpoints"][1]["activation_radius"], 12);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 23: the achievements' rules (F-56238, F-98556, the purse, the tournaments,
    /// the police, the editor) and their record in the statistics and the block.
    #[test]
    fn achievement_rules() {
        let mut g = Game::new(Vec::new(), 0);
        g.fresh_game = false;
        // (the race-end statistics: wins, the streak and its best, places)
        for _ in 0..4 {
            g.mc4_race_stats(1);
        }
        g.mc4_race_stats(2);
        assert_eq!((g.mc4_stat("kWIN", 0), g.mc4_stat("gRCP", 0), g.mc4_stat("kCWN", 0), g.mc4_stat("kCCW", 0), g.mc4_stat("k2ND", 0)), (4.0, 5.0, 0.0, 4.0, 1.0));
        for _ in 0..5 {
            g.mc4_race_stats(1);
        }
        assert!(g.mc4_achievements_check().contains(&"aw5C".to_string()));
        assert_eq!(g.career.ostats.get("aw5C/0"), Some(&1.0));
        assert!(g.mc4.as_ref().unwrap().achievements.contains(&"aw5C".to_string()));
        // (set once: not again)
        assert!(!g.mc4_award("aw5C"));
        // (equality: kWIN 180 awards, 181 would not have; kRLW past 15 never does)
        g.mc4_stat_set("kWIN", 0, 179.0);
        assert!(!g.mc4_achievements_check().contains(&"aW18".to_string()));
        g.mc4_race_stats(1);
        assert!(g.mc4_achievements_check().contains(&"aW18".to_string()));
        g.mc4_stat_set("kRLW", 0, 16.0);
        assert!(!g.mc4_achievements_check().contains(&"aRRR".to_string()));
        g.mc4_stat_set("gk2W", 0, 50.5);
        assert!(g.mc4_achievements_check().contains(&"a2WD".to_string()));
        // (the stats manager: the thresholds at its count 30)
        g.mc4_stat_set("kLOW", 0, 10.0);
        for _ in 0..31 {
            g.mc4_s23_tick(0.016, S23In::default());
        }
        assert!(g.mc4_awarded("aLOW"));
        // (the purse: an addition that reaches $1,000,000)
        g.money = 999_000;
        g.mc4_s23_tick(0.016, S23In::default());
        assert!(!g.mc4_awarded("aMIL"));
        g.money = 1_000_500;
        g.mc4_s23_tick(0.016, S23In::default());
        assert!(g.mc4_awarded("aMIL"));
        // (the four districts' tournaments)
        for d in 0..3 {
            g.mc4_tournament_won(d);
        }
        assert!(!g.mc4_awarded("aBTO"));
        g.mc4_tournament_won(3);
        assert!(g.mc4_awarded("aBTO"));
        // (the editor saved a race)
        g.user_races.push(vec![[0.0; 3], [1.0, 0.0, 1.0]]);
        g.mc4_s23_tick(0.016, S23In::default());
        assert!(g.mc4_awarded("aSRE"));
        // (the police: a ticket run from, the police lost)
        if g.career.loaded() {
            g.mc4_s23_tick(0.016, S23In { heat: 1, ..Default::default() });
            g.mc4_s23_tick(0.016, S23In { heat: 2, ..Default::default() });
            assert!(g.mc4_awarded("aRUN"));
            g.mc4_s23_tick(0.016, S23In { heat: 0, ..Default::default() });
            assert!(g.mc4_awarded("aEVA") && g.mc4_stat("gCPL", 0) == 1.0);
        }
        // (the tables: the Achievement rewards of the ability systems)
        let mut t = ExpTables::default();
        t.parse("System { Type Zone MaxAmount 40 Threshold { Id Zone_1 Percent 0.5 Reward { Type AbilityLevel AbilityId Zone Level 1 } } Threshold { Id Zone_2 Percent 1.0 Reward { Type AbilityLevel AbilityId Zone Level 2 } Reward { Type Achievement Hidden 1 GiveImmediately 0 Name aZON } } }");
        assert_eq!(t.system("Zone").unwrap().thresholds[1].achievements, vec!["aZON".to_string()]);
        // (the missions' own rewards on the disc are the table's)
        if let Ok(rd) = std::fs::read_dir(std::path::Path::new(&data_dir()).join("tune/career/missions/sc")) {
            let mut found = Vec::new();
            for p in rd.flatten().map(|e| e.path()).chain(std::iter::once(std::path::Path::new(&data_dir()).join("tune/career/missions/CITY_CHAMP.xml"))) {
                let Ok(x) = std::fs::read_to_string(&p) else { continue };
                if let Some(k) = x.find("mcRewardAchievement") {
                    let id = x.split("<szId content=\"ascii\">").nth(1).and_then(|s| s.split('<').next()).unwrap_or("").to_string();
                    let a = x[k..].split("<szStatName content=\"ascii\">").nth(1).and_then(|s| s.split('<').next()).unwrap_or("").to_string();
                    found.push((id, a));
                }
            }
            for (id, a) in &found {
                assert!(ACHIEVEMENT_MISSIONS.iter().any(|m| m.0 == id && m.1 == a), "{id} {a}");
            }
            assert_eq!(found.len(), 6);
        }
    }

    /// The race's end in slow motion (EndOfRaceSequence): 0.2 s of real time, then to a
    /// standstill in 0.5 s with sin(t pi / 2) ^ 0.4, the simulation held at the floor of 0.1.
    #[test]
    fn race_end_slow_motion() {
        let mut g = crate::game::Game::new(Vec::new(), 0);
        g.mode = crate::game::Mode::Results;
        g.race_slow_wait = Some(0.2);
        g.race_slow_tick(0.1);
        assert_eq!(g.mc4_time_scale(), 1.0);
        g.race_slow_tick(0.1);
        g.race_slow_tick(0.15);
        let e = (0.5f32 * std::f32::consts::FRAC_PI_2).sin().powf(0.4);
        assert!((g.mc4_time_scale() - (1.0 - e)).abs() < 1e-5, "{}", g.mc4_time_scale());
        g.race_slow_tick(0.25);
        assert_eq!(g.mc4_time_scale(), 0.1);
        g.race_slow_tick(5.0);
        assert_eq!(g.mc4_time_scale(), 0.1);
        // (leaving the race: back to 1)
        g.mode = crate::game::Mode::FreeRoam;
        g.race_slow_tick(0.016);
        assert_eq!(g.mc4_time_scale(), 1.0);
    }

    /// The race's end: the panels 0.2 s after the line, the game paused at 1.2 s (form 60's
    /// one second), held through the menu, running again once the race is left.
    #[test]
    fn race_end_pause() {
        let mut g = crate::game::Game::new(Vec::new(), 0);
        g.mode = crate::game::Mode::Results;
        g.race_slow_wait = Some(crate::game::Game::RACE_END_PANEL);
        g.race_end_t = Some(0.0);
        for _ in 0..11 {
            g.race_slow_tick(0.1);
        }
        assert_eq!(g.mc4_time_scale(), 0.1);
        g.race_slow_tick(0.1);
        assert_eq!(g.mc4_time_scale(), 0.0);
        g.race_slow_tick(10.0);
        assert_eq!(g.mc4_time_scale(), 0.0);
        g.mode = crate::game::Mode::FreeRoam;
        g.race_slow_tick(0.016);
        assert_eq!(g.race_end_t, None);
        assert_eq!(g.mc4_time_scale(), 1.0);
    }

    /// Step 23: the tutorial flags - shown once, the form in the slow motion, CONTINUE; the
    /// garage's, the hangout's, the police's and the hookman's; RACE_GH_02's lesson.
    #[test]
    fn sst_tutorial_steps() {
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "ssttut") else { return };
        g.fresh_game = false;
        g.opt_x.s23.sst_tut = SstTut::Charge;
        let mut step = |g: &mut Game, n: usize, level: f32, racing: bool| {
            for _ in 0..n {
                g.mc4_sst_tut_step(0.02, level, racing);
                g.opt_x.s23.slow.update(0.02);
            }
        };
        // (not before 0.3, nor out of the race)
        step(&mut g, 20, 0.3, true);
        step(&mut g, 20, 0.5, false);
        assert_eq!(g.opt_x.s23.sst_tut, SstTut::Charge);
        // (over 0.3: within the 100 ms poll the ease to pause, the box after 500 ms)
        step(&mut g, 5, 0.31, true);
        assert!(matches!(g.opt_x.s23.sst_tut, SstTut::Ease(_)));
        step(&mut g, 27, 0.31, true);
        assert_eq!(g.opt_x.s23.sst_tut, SstTut::Box78);
        assert_eq!(g.tutorial_box.as_ref().map(|b| b.0.as_str()), Some("tutorial_slipstream_title"));
        assert!(g.opt_x.arcade.d.say.contains(&"TUTORIAL_SST_INGAME".to_string()));
        assert_eq!(g.opt_x.s23.slow.global, 0.0);
        // (answered: the ease from pause; form 79 at a full charge, closed by the burn)
        g.tutorial_box = None;
        step(&mut g, 60, 0.99, true);
        assert_eq!(g.opt_x.s23.sst_tut, SstTut::Full);
        assert_eq!(g.opt_x.s23.slow.global, 1.0);
        step(&mut g, 5, 1.0, true);
        assert_eq!(g.opt_x.s23.sst_tut, SstTut::Box79);
        step(&mut g, 5, 0.95, false);
        assert_eq!(g.tutorial_box.as_ref().map(|b| b.0.as_str()), Some("tutorial_slipstream_use_title"));
        step(&mut g, 1, 0.89, false);
        assert_eq!(g.opt_x.s23.sst_tut, SstTut::Done);
        assert!(g.tutorial_box.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tutorial_flags_and_slow_motion() {
        // (F-82413: ease out to 0 in 0.5 s, sin(t pi / 2), back to 1)
        let mut s = SlowMo::default();
        s.ease(0.0);
        s.update(0.25);
        assert!((s.global - (1.0 - (0.5f32 * std::f32::consts::FRAC_PI_2).sin())).abs() < 1e-5);
        s.update(0.25);
        assert_eq!(s.global, 0.0);
        s.ease(1.0);
        s.update(0.1);
        assert!(s.global > 0.0 && s.global < 1.0);
        s.update(0.5);
        assert_eq!(s.global, 1.0);
        // (both flags: (1 - cos(pi t)) / 2; neither: t)
        let mut s = SlowMo::default();
        s.set(0.0, 1.0, true, true, 1.0);
        s.update(0.5);
        assert!((s.global - 0.5).abs() < 1e-5);
        assert_eq!(tutorial_keys(111).map(|k| k.0), Some("tutorial_hydraulics_title".to_string()));
        assert_eq!(tutorial_keys(102).map(|k| k.1), Some("tutorial_beat_me_there_desc".to_string()));
        let Some((mut g, dir, _)) = career_game("newgame_import.json", "s23tut") else { return };
        g.fresh_game = false;
        g.mode = crate::game::Mode::FreeRoam;
        // (the new game sample has seen tTBM, tTFB, tTFL only)
        assert!(g.mc4_tut_seen("tTBM") && !g.mc4_tut_seen("tTDE"));
        assert!(!g.mc4_tutorial("tTBM"));
        // (a delivery hangout's panel waits for its tutorial: form 100 in the slow motion)
        g.hangout_key = Some("hw_delivery".into());
        g.mode = crate::game::Mode::RaceSelect;
        g.mc4_s23_tick(0.016, S23In::default());
        // (step 23b: the box stands over the hangout's scene - the panel's mode stays, held)
        assert_eq!(g.mode, crate::game::Mode::RaceSelect);
        assert_eq!(g.opt_x.s23.hangout_hold.as_deref(), Some("hw_delivery"));
        assert!(g.mc4_tut_seen("tTDE") && g.mc4.as_ref().unwrap().tutorials_shown.contains(&"tTDE".to_string()));
        for _ in 0..40 {
            g.mc4_s23_tick(0.016, S23In::default());
        }
        let b = g.tutorial_box.clone().unwrap();
        assert_eq!(b.0, "tutorial_delivery_title");
        assert!(g.mc4_tut_continue(&b.0));
        assert_eq!(g.mc4_time_scale(), 0.0);
        assert!(g.opt_x.arcade.d.say.contains(&"TUTORIAL_DELIVERY_INGAME".to_string()));
        g.tutorial_box = None;
        g.mc4_s23_tick(0.016, S23In::default());
        assert_eq!(g.mode, crate::game::Mode::RaceSelect, "the panel comes up");
        for _ in 0..40 {
            g.mc4_s23_tick(0.016, S23In::default());
        }
        assert_eq!(g.mc4_time_scale(), 1.0);
        // (the second visit: nothing)
        g.mode = crate::game::Mode::FreeRoam;
        g.mc4_s23_tick(0.016, S23In::default());
        g.mode = crate::game::Mode::RaceSelect;
        g.mc4_s23_tick(0.016, S23In::default());
        assert_eq!(g.mode, crate::game::Mode::RaceSelect);
        g.hangout_key = None;
        g.mode = crate::game::Mode::FreeRoam;
        // (out of the garage with hydraulics: form 111)
        g.hydraulics = true;
        g.mode = crate::game::Mode::Garage;
        g.mc4_s23_tick(0.016, S23In::default());
        g.mode = crate::game::Mode::FreeRoam;
        g.mc4_s23_tick(0.016, S23In::default());
        assert!(g.mc4_tut_seen("tTHY"));
        for _ in 0..40 {
            g.mc4_s23_tick(0.016, S23In::default());
        }
        assert_eq!(g.tutorial_box.as_ref().map(|b| b.0.as_str()), Some("tutorial_hydraulics_title"));
        g.tutorial_box = None;
        for _ in 0..40 {
            g.mc4_s23_tick(0.016, S23In::default());
        }
        // (the first pursuit: 103; the second: 104 - no slow motion)
        g.mc4_s23_tick(0.016, S23In { heat: 2, ..Default::default() });
        assert!(g.mc4_tut_seen("tTPP") && !g.mc4_tut_seen("tTPL"));
        g.mc4_s23_tick(0.016, S23In { heat: 2, ..Default::default() });
        assert_eq!(g.tutorial_box.as_ref().map(|b| b.0.as_str()), Some("tutorial_police_pursuit_title"));
        assert_eq!(g.mc4_time_scale(), 1.0);
        g.tutorial_box = None;
        g.mc4_s23_tick(0.016, S23In { heat: 0, ..Default::default() });
        g.mc4_s23_tick(0.016, S23In { heat: 2, ..Default::default() });
        assert!(g.mc4_tut_seen("tTPL"));
        g.mc4_s23_tick(0.016, S23In { heat: 0, ..Default::default() });
        g.tutorial_box = None;
        g.mc4_s23_tick(0.016, S23In::default());
        g.tutorial_box = None;
        // (a hookman in reach: the new game has seen tTFL and tTFB - nothing)
        g.mc4_s23_tick(0.016, S23In { near: true, ..Default::default() });
        assert!(g.tutorial_box.is_none());
        g.mc4_stat_set("tTFL", 0, 0.0);
        g.mc4_stat_set("tTFB", 0, 0.0);
        g.mc4_s23_tick(0.016, S23In { near: false, ..Default::default() });
        g.mc4_s23_tick(0.016, S23In { near: true, ..Default::default() });
        assert_eq!(g.tutorial_box.as_ref().map(|b| b.0.as_str()), Some("tutorial_challenge_title"));
        assert!(!g.mc4_tut_continue("tutorial_challenge_title"));
        g.mc4_s23_tick(0.016, S23In { near: true, flashed: true, ..Default::default() });
        assert!(g.tutorial_box.is_none() && g.mc4_tut_seen("tTFL") && !g.mc4_tut_seen("tTFB"));
        g.mc4_s23_tick(0.016, S23In { near: false, ..Default::default() });
        g.mc4_s23_tick(0.016, S23In { near: true, ..Default::default() });
        assert_eq!(g.tutorial_box.as_ref().map(|b| b.0.as_str()), Some("tutorial_flash_title"));
        g.mc4_s23_tick(0.016, S23In { near: false, ..Default::default() });
        assert!(g.tutorial_box.is_none() && !g.mc4_tut_seen("tTFB"), "out of reach: closed, not seen");
        // (RACE_GH_02's lesson: the ease to pause with box 76, the speeches, back after box 77)
        g.tutorial_box = Some(("tutorial_mission_log_title".into(), "tutorial_mission_log_desc".into(), 0.3, 600.0));
        g.career.stats.insert("tut:mission_log".into(), 1);
        g.mc4_s23_tick(0.016, S23In::default());
        assert_eq!(g.opt_x.s23.lesson, 1);
        assert!(g.tutorial_box.as_ref().unwrap().2 >= 0.5);
        g.tutorial_box = None;
        g.mlog_guided = true;
        g.mc4_s23_tick(0.016, S23In::default());
        assert!(g.opt_x.arcade.d.say.contains(&"TUTORIAL_MISSION_LOG_GPS".to_string()));
        for _ in 0..40 {
            g.mc4_s23_tick(0.016, S23In::default());
        }
        assert_eq!(g.mc4_time_scale(), 0.0);
        g.mlog_guided = false;
        g.tutorial_box = Some(("tutorial_target_title".into(), "tutorial_target_desc".into(), 0.5, 600.0));
        g.mc4_s23_tick(0.016, S23In::default());
        assert!(g.opt_x.arcade.d.say.contains(&"TUTORIAL_TARGET_INGAME".to_string()));
        g.tutorial_box = None;
        g.career.stats.insert("tut:target".into(), 1);
        for _ in 0..40 {
            g.mc4_s23_tick(0.016, S23In::default());
        }
        assert_eq!((g.opt_x.s23.lesson, g.mc4_time_scale()), (4, 1.0));
        // (the flags and the block's list are saved)
        g.write_save(&dir);
        let b = saved_block(&dir);
        for f in ["tTDE", "tTHY", "tTPP", "tTPL", "tTFL"] {
            assert!(b["tutorials_shown"].as_array().unwrap().iter().any(|x| x == f), "{f}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 22b: a vehicle the port staked is kept as the record's bytes (`car_to_xcc`): what
    /// its opponent races in reads back as the record.
    #[test]
    fn held_car_record_bytes() {
        for (v, h) in [(0.0f32, 0u16), (1.0, 0x3c00), (-2.0, 0xc000), (0.5, 0x3800), (65504.0, 0x7bff), (1e-7, 0x0002)] {
            assert_eq!(f32_to_half(v), h, "{v}");
        }
        let Ok(base) = std::fs::read(std::path::Path::new(&data_dir()).join("resources/vehicle/configs/vp_aud_r8_07.xcc")) else { return };
        let mut c = Car::from_xcc(&base, "vp_aud_r8_07", "vp_aud_r8_07", 0).unwrap();
        // (the file it was made from comes back: the same bytes but for the floats the record
        // keeps to six places)
        let same = car_to_xcc(&c, &base, None).unwrap();
        for k in (0..8176).step_by(4) {
            if same[k..k + 4] != base[k..k + 4] {
                let (a, b) = (f32::from_be_bytes([same[k], same[k + 1], same[k + 2], same[k + 3]]), f32::from_be_bytes([base[k], base[k + 1], base[k + 2], base[k + 3]]));
                assert!((a - b).abs() <= 1e-6 * b.abs().max(1.0), "word at {k:#x}: {a} {b}");
            }
        }
        c.visual.parts.insert("hood".into(), 3);
        c.visual.parts.insert("spoiler".into(), 2);
        c.visual.paint[0].ColorA = fv(&[0.8, 0.1, 0.05]);
        c.visual.paint[2].set_paint_type(1);
        c.visual.RimMdlIdx = 40;
        c.visual.TireTexIdx = 5;
        c.set_ability(Some(2));
        c.performance.stages.insert("engine".into(), 2);
        let mut liv = crate::vinyl::Livery::default();
        liv.zones[1].push(crate::vinyl::Layer { shape: 1003, scale: [2.0, -1.5], pos: [0.25, -3.0], rot: 90.0, skew: 0.0, color: [10, 20, 30, 200], blend: crate::vinyl::Blend::NegativeCutout });
        let b = car_to_xcc(&c, &base, Some(&liv)).unwrap();
        let back = Car::from_xcc(&b, "vp_aud_r8_07", "vp_aud_r8_07", 0).unwrap();
        assert_eq!((back.part("hood"), back.part("spoiler"), back.visual.RimMdlIdx, back.visual.TireTexIdx, back.ability_idx()), (3, 2, 40, 5, Some(2)));
        assert_eq!(back.performance.stages.get("engine"), Some(&2));
        assert_eq!(back.visual.paint[2].paint_type(), Some(1));
        let (cfg, rim, _) = setup_parts(&b).unwrap();
        assert_eq!((cfg.hood, cfg.spoiler, rim), (3, 2, 40));
        let look = setup_look(&b).unwrap();
        assert!((look.color[0] - 0.8).abs() < 1e-6 && (look.color[1] - 0.1).abs() < 1e-6);
        let l2 = crate::vinyl::Livery::from_xcc(&b);
        assert_eq!(l2.zones[1], liv.zones[1]);
        assert!(l2.zones[0].is_empty() && l2.layer_count() == 1);
        // (in the lot: the port's stake carries the bytes, its opponent races them)
        let Some((mut g, dir, _)) = career_game("full_import.json", "s23held") else { return };
        let mine = g.mc4_car_name(g.current_car);
        let mut s = g.career.save();
        let mut st = crate::career::MState::default();
        st.stage = crate::career::Stage::Active;
        st.progress = vec![0];
        s.missions.insert("HANGOUT_PINKSLIP_HOLLYWOOD".to_string(), st);
        s.order.push("HANGOUT_PINKSLIP_HOLLYWOOD".to_string());
        g.career.restore(&s);
        assert!(g.mc4_pink_start("hw_wager_hangout1", 0));
        assert_eq!(g.mc4_pink_end(false), Some("PINKSLIP_FAILURE"));
        let held = g.mc4.as_ref().unwrap().career.pink_slips.held()[0].clone();
        assert!(held.extra.contains_key("vehicle_record_hex"));
        let uid = held.uid;
        let bytes = g.mc4_held_bytes(uid).unwrap();
        let rec = pink_record(&held).unwrap();
        assert_eq!(Car::from_xcc(&bytes, &mine, &mine, 0).unwrap().visual.parts, rec.visual.parts);
        assert!(g.mc4_pink_start("hw_wager_hangout1", 0));
        // (the hangout's own car first, then the held one: pick 1)
        let _ = g.mc4_pink_end(false);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 22b: the pink slip's confirmation box, FIND_SH_02C's money box at his place,
    /// Downtown's scripted pink slip and the hangout's event every visit.
    #[test]
    fn pink_box_find02c_and_downtown() {
        let Some((mut g, dir, _)) = career_game("full_import.json", "s23boxes") else { return };
        g.fresh_game = false;
        g.mode = crate::game::Mode::FreeRoam;
        let mut s = g.career.save();
        for id in ["HANGOUT_PINKSLIP_HOLLYWOOD", "FIND_SH_02C", "HANGOUT_PINKSLIP_DOWNTOWN"] {
            let mut st = crate::career::MState::default();
            st.stage = crate::career::Stage::Active;
            st.progress = vec![0];
            s.missions.insert(id.to_string(), st);
            s.order.push(id.to_string());
        }
        g.career.restore(&s);
        // (ACCEPT asks first; NO: nothing staked)
        assert!(!g.mc4_pink_ask("hw_wager_hangout1", 0));
        assert!(g.message_box.is_some());
        g.message_box = None;
        g.mc4_s23_tick(0.016, S23In::default());
        assert!(g.opt_x.s23.pink_ask.is_none() && !g.mc4_pink_on());
        // (YES stakes and starts)
        assert!(!g.mc4_pink_ask("hw_wager_hangout1", 0));
        let def = g.message_box.as_ref().unwrap().0.clone();
        assert_eq!((def.title.as_str(), def.description.as_str()), ("hw_wager_hangout1", "pinkslip_confirmation"));
        assert!(matches!(g.message_box.as_ref().unwrap().2, crate::game::MenuAction::StartRace));
        g.message_box = None;
        g.mc4_s23_box_yes();
        assert!(g.mc4_pink_on() && g.hangout_key.is_none());
        let _ = g.mc4_pink_end(true);
        // (FIND_SH_02C: away - nothing; at his place without the money - the restriction)
        g.money = 500_000;
        g.mc4_find02c_meet([0.0, 0.0, 0.0], false);
        assert!(g.message_box.is_none());
        g.mc4_find02c_meet([FIND_SH_02C_AT[0] + 5.0, FIND_SH_02C_AT[1], FIND_SH_02C_AT[2]], false);
        assert_eq!(g.message_box.as_ref().unwrap().0.title, "FIND_SH_02C_MONEY_RESTRICTION_TITLE");
        g.message_box = None;
        g.mc4_s23_tick(0.016, S23In::default());
        // (he waits until the player has been 30 m away)
        g.money = 2_500_000;
        g.mc4_find02c_meet([FIND_SH_02C_AT[0] + 5.0, FIND_SH_02C_AT[1], FIND_SH_02C_AT[2]], false);
        assert!(g.message_box.is_none());
        g.mc4_find02c_meet([FIND_SH_02C_AT[0] + 31.0, FIND_SH_02C_AT[1], FIND_SH_02C_AT[2]], false);
        g.mc4_find02c_meet([FIND_SH_02C_AT[0] + 5.0, FIND_SH_02C_AT[1], FIND_SH_02C_AT[2]], true);
        assert!(g.message_box.is_none(), "not while chased");
        g.mc4_find02c_meet([FIND_SH_02C_AT[0] + 5.0, FIND_SH_02C_AT[1], FIND_SH_02C_AT[2]], false);
        assert_eq!(g.message_box.as_ref().unwrap().0.title, "FIND_SH_02C_SPEND_MONEY_TITLE");
        g.message_box = None;
        g.mc4_s23_box_yes();
        g.career.stats.remove("tPAR");
        g.career.ostats.remove("tPAR/0");
        g.mc4_s23_tick(0.016, S23In::default());
        assert_eq!(g.money, 1_500_000);
        assert_eq!(g.career.stats.get("tPAR"), Some(&1));
        assert!(g.career.news.iter().any(|n| matches!(n, crate::career::News::Scene(id) if id == "FIND_SH_02C")));
        assert!(g.career.mission_complete("FIND_SH_02C"));
        // (Downtown: the event at every visit, the race after the scene, GH_18 in the 350Z)
        assert_eq!(g.career.location_enter("DT_WAGER_HANGOUT1_ENTER").as_deref(), Some("HANGOUT_PINKSLIP_DOWNTOWN"));
        assert!(g.career.location_enter("DT_WAGER_HANGOUT1_ENTER").is_none(), "once a visit");
        assert!(g.mc4_dt_pink_busy());
        // (the race's definition, as the game reads it from the race files)
        if !g.race_defs.iter().any(|d| d.id.ends_with("hw_ordered_jp_11")) {
            g.race_defs.push(crate::races::RaceDef { id: "Hollywood/Ordered/hw_ordered_jp_11".into(), beacon: glam::Vec3::new(10.0, 0.0, 10.0), ..Default::default() });
        }
        for _ in 0..61 {
            g.mc4_s23_tick(0.016, S23In::default());
        }
        assert!(g.mc4_s23_take_start());
        assert_eq!(g.current_race.map(|i| g.race_defs[i].id.ends_with("hw_ordered_jp_11")), Some(true));
        assert_eq!(g.mc4_pink_rival(), Some(("GH_18".to_string(), "vp_nsn_350z_06".to_string(), "pinkslip_vp_nsn_350z_06".to_string())));
        let n = g.opt_x.s23.dt_pink.unwrap();
        assert!((400..=600).contains(&n));
        assert_eq!(g.mc4_pink_end(true), None);
        assert!(g.career.mission_complete("HANGOUT_PINKSLIP_DOWNTOWN"));
        assert!(g.career.news.iter().any(|x| matches!(x, crate::career::News::Car(c) if c == "vp_nsn_350z_06")));
        g.mc4_hangout_left("dt_wager_hangout1");
        assert!(g.career.visit_events.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Step 23b: the race goals (F-96807), the driving statistics (odometer, two wheels,
    /// wheelie, jumps), the test drive's table field and the photo album's aFAB.
    #[test]
    fn s23b_goals_drive_stats_album() {
        use crate::mc4b::S23Drive;
        let mut g = Game::new(Vec::new(), 0);
        g.fresh_game = false;
        g.mode = crate::game::Mode::FreeRoam;
        // (two LA races and one of South Central in the city table; a delivery does not count)
        let def = |id: &str, sub: &str, district: &str| crate::races::RaceDef { id: id.into(), sub_type: sub.into(), district: district.into(), goal_time: 100.0, ..Default::default() };
        g.race_defs = vec![def("game/Race/Hollywood/Ordered/a", "Ordered", "Hollywood"), def("game/Race/Beaches/Circuit/b", "Circuit", "Beaches"), def("game/Race/SouthCentral/Ordered/c", "Ordered", "SouthCentral"), def("game/Race/Hollywood/Delivery/d", "Delivery", "Hollywood")];
        g.race_table = g.race_defs.iter().map(|d| d.id.clone()).collect();
        // (only a race started from GOAL ATTACK records them: F-98556 A-77758)
        g.current_race = Some(0);
        g.mc4_race_goals(1, Some(50.0), 0.0);
        assert_eq!(g.mc4_stat("rWIN", 0), 0.0);
        g.goal_attack = true;
        // won but not under the goal time: rWIN / rMDG only - the list's mask 8 + 4
        g.mc4_race_goals(1, Some(100.0), 0.1);
        assert_eq!(g.mc4_goal_mask(0), 12);
        assert_eq!(g.mc4_goal_lines(0)[2], "FINISH THIS RACE IN UNDER 01:40.00");
        assert_eq!((g.mc4_stat("rWIN", 0), g.mc4_stat("rBTT", 0), g.mc4_stat("rMDG", 0), g.mc4_stat("rCGL", 0)), (1.0, 0.0, 1.0, 0.0));
        // a quarter of MaxDamage is too much
        g.mc4_race_goals(1, Some(99.0), 0.25);
        assert!(!g.mc4_awarded("aAOR"));
        // all three: rCGL, aAOR; aAAR waits for the other LA race
        g.mc4_race_goals(1, Some(99.0), 0.2);
        assert_eq!(g.mc4_stat("rCGL", 0), 1.0);
        assert!(g.mc4_awarded("aAOR") && !g.mc4_awarded("aAAR"));
        assert_eq!(g.mc4_goal_mask(0), 15);
        // (the badge's line: TARGET TIME)
        assert_eq!(g.mc4_goal_badge().as_deref(), Some("TARGET TIME: 01:40.00"));
        g.current_race = Some(1);
        g.mc4_race_goals(1, Some(50.0), 0.0);
        assert!(g.mc4_awarded("aAAR") && !g.mc4_awarded("aARC"));
        g.current_race = Some(2);
        g.mc4_race_goals(1, Some(50.0), 0.0);
        assert!(g.mc4_awarded("aARC"));
        // the odometer in miles (2.237 mph per m/s), two wheels, a jump
        let d = |two: bool, air: bool, x: f32| S23Drive { present: true, vel_xz: 20.0, speed: 20.0, two, wheelie: false, airborne: air, pos: glam::Vec3::new(x, 0.0, 0.0), placed: true };
        g.mc4b_tick(1.0, d(true, false, 0.0));
        let mile = 20.0 * crate::mc4b::MPH * crate::mc4b::PER_HOUR;
        assert!((g.mc4_stat("gkDS", 0) - mile as f64).abs() < 1e-5 && (g.mc4_stat("gk2W", 0) - mile as f64).abs() < 1e-5);
        // (a wheel down takes the take-off point; the first frame is a set-down)
        g.mc4b_tick(0.1, d(false, false, 0.0));
        g.mc4b_tick(0.5, d(false, true, 5.0));
        g.mc4b_tick(0.5, d(false, true, 10.0));
        g.mc4b_tick(0.1, d(false, false, 12.0));
        assert!((g.mc4_stat("gsJT", 0) - 1.0).abs() < 1e-5 && (g.mc4_stat("gmJD", 0) - 12.0).abs() < 1e-4);
        // (the frame after the landing only takes the new take-off point; a shorter jump keeps the longest)
        g.mc4b_tick(0.1, d(false, false, 13.0));
        g.mc4b_tick(0.2, d(false, true, 15.0));
        g.mc4b_tick(0.1, d(false, false, 16.0));
        assert!((g.mc4_stat("gsJT", 0) - 1.2).abs() < 1e-5 && (g.mc4_stat("gmJD", 0) - 12.0).abs() < 1e-4);
        // (a move the driving cannot make - a warp, the grid - is no jump: no air, no take-off point)
        g.mc4b_tick(0.1, d(false, true, 900.0));
        g.mc4b_tick(0.1, d(false, false, 901.0));
        assert!((g.mc4_stat("gsJT", 0) - 1.2).abs() < 1e-5 && (g.mc4_stat("gmJD", 0) - 12.0).abs() < 1e-4);
        // nothing counts in a menu
        g.mode = crate::game::Mode::Paused;
        let before = g.mc4_stat("gkDS", 0);
        g.mc4b_tick(1.0, d(false, false, 0.0));
        assert_eq!(g.mc4_stat("gkDS", 0), before);
        // the photo album: sixteen slots, aFAB as the sixteenth is first filled, tPTS each picture
        let dir = std::env::temp_dir().join(format!("mcla_s23b_album_{}", std::process::id()));
        let data = dir.join("cache");
        std::fs::create_dir_all(&data).unwrap();
        g.data_dir = data.to_string_lossy().to_string();
        for k in 0..16 {
            let p = g.mc4b_photo_path();
            assert!(p.ends_with(&format!("slot_{k:02}.png")));
            assert_eq!(g.mc4_awarded("aFAB"), k == 15);
            std::fs::write(&p, b"png").unwrap();
        }
        assert!(g.mc4b_photo_path().ends_with("slot_00.png"));
        assert_eq!(g.mc4_stat("tPTS", 0), 17.0);
        let _ = std::fs::remove_dir_all(&dir);
        // the vehicle table +48 (ContentDownloadFlags, else ContentFlags) for aTDV
        assert_eq!(crate::mc4b::race_content("SouthCentral"), 4);
        let data_dir = "D:/MCLA/extracted/cache";
        if std::path::Path::new(data_dir).join("tune/vehicle/0xe94bde63").exists() {
            assert_eq!(crate::mc4b::content_flags(data_dir, "vp_aud_r8_08"), 3);
            assert_eq!(crate::mc4b::content_flags(data_dir, "vpd_chv_chevelle_70"), 60);
            assert_eq!(crate::mc4b::content_flags(data_dir, "vp_se_dge_plc_06"), 257);
            assert_eq!(crate::mc4b::content_flags(data_dir, "vp_vw_golf_83"), 0);
        }
    }

    /// Step 23b: the hangouts' scenes - the set-ups' cars, the pink slip choice's camera (its
    /// goal and the 30 ms steps of fn 0x4d52), the tutorials of the man in reach and the
    /// gas station, the audio's slow-down state, the payback's place.
    #[test]
    fn s23b_hangout_scene_and_tutorials() {
        use crate::hangout_scene::*;
        let s = setup_of("hw_wager_hangout1").unwrap();
        assert_eq!(s.cars.len(), 2);
        assert_eq!(s.cars[0].0, "GH_13_2_vp_mit_evo_06");
        assert_eq!((s.cam_base, s.pink.len(), s.location), (20, 4, 503));
        assert!(setup_of("hw_local_hangout1").unwrap().cars.iter().any(|c| c.0 == "GH_01_1_vp_chv_impala_96"));
        // UILogic_GetCameraPos(20) = PauseCamera21Position
        assert_eq!(camera_pos(20), Some(glam::Vec3::new(0.05, 0.625, 5.4)));
        let (eye, look) = pink_goal(s, 1).unwrap();
        let spot = glam::Vec3::from(s.pink[1].0);
        assert!((look - (spot + glam::Vec3::Y * 0.5)).length() < 1e-5);
        assert!((eye - (spot + glam::Vec3::new(-0.1, 0.775 + 0.5 + 0.1, 5.875))).length() < 1e-4);
        // the steps: the speed grows by 0.004 up to 0.3 when far, then slows by 0.9 and never
        // passes the goal
        let (mut cur, mut v) = (glam::Vec3::ZERO, 0.0f32);
        step_toward(&mut cur, glam::Vec3::X * 10.0, &mut v);
        assert!((v - 0.004).abs() < 1e-7 && (cur.x - 0.004).abs() < 1e-6);
        for _ in 0..400 {
            step_toward(&mut cur, glam::Vec3::X * 10.0, &mut v);
        }
        assert!(v <= 0.3 && cur.x <= 10.0 && (cur.x - 10.0).abs() < 1e-3);
        let mut c = PinkChoice::start(s, 0).unwrap();
        c.tick(s, 3, 0.029);
        assert_eq!(c.eye, pink_goal(s, 0).unwrap().0);
        c.tick(s, 3, 0.002);
        assert!(c.eye != pink_goal(s, 0).unwrap().0);
        // the tutorials: the man in reach by his type (tTTT for a time trial man), the station
        let mut g = Game::new(Vec::new(), 0);
        g.fresh_game = false;
        g.mode = crate::game::Mode::FreeRoam;
        g.mc4_near_tick(true, false, 3);
        assert!(g.mc4_tut_seen("tTTT") && g.opt_x.s23.queue.first().map_or(false, |r| r.form == 96 && r.slow));
        g.mc4_near_tick(true, false, 3);
        assert_eq!(g.opt_x.s23.queue.iter().filter(|r| r.flag == "tTTT").count(), 1);
        g.tutorial_box = None;
        g.mc4_station_tick(true);
        assert!(g.mc4_tut_seen("tTGS") && g.tutorial_box.as_ref().map_or(false, |b| b.0 == "tutorial_gas_station_title"));
        g.mc4_station_tick(false);
        assert!(g.tutorial_box.is_none());
        // the audio's slow-down state goes with a slow tutorial under way
        g.opt_x.s23.run = Some(g.opt_x.s23.queue.remove(0));
        assert!(!g.mc4b_time_slow());
        g.opt_x.s23.run.as_mut().unwrap().phase = 1;
        assert!(g.mc4b_time_slow());
        // the payback's place: its panel's first leg is the payback race nearest
        g.race_defs = vec![
            crate::races::RaceDef { id: "game/Race/Beaches/Payback/far".into(), race_type: "Payback".into(), beacon: glam::Vec3::ZERO, ..Default::default() },
            crate::races::RaceDef { id: "game/Race/Beaches/Payback/near".into(), race_type: "Payback".into(), beacon: glam::Vec3::from(PAYBACK_AT), ..Default::default() },
        ];
        g.mc4_payback_place_panel(glam::Vec3::from(PAYBACK_AT));
        assert_eq!((g.current_race, g.hangout_key.as_deref(), g.mode, g.hangout_diff), (Some(1), Some("be_payback"), crate::game::Mode::RaceSelect, 1));
    }

    // ---- verify-save stage 1 (docs/reference/save/verify_save_stage1.md): the original's own code (work/verify-save/stage1.py)
    // run on random cases against the music order container and generator, the difficulty / set-up functions and the time of day.
    fn save_fixture(name: &str) -> String {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/verify-save");
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    #[test]
    #[ignore = "reads the original's reference output (work/verify-save/stage1_original.txt)"]
    fn stage1_original_music_orders_and_difficulty() {
        let cases = save_fixture("cases1.txt");
        let want = save_fixture("stage1_original.txt");
        let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let hx = |v: &[usize]| v.iter().map(|x| format!("{:x}", x)).collect::<Vec<_>>().join(" ");
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        assert_eq!(cases.lines().count(), want.lines().count());
        for (c, w) in cases.lines().zip(want.lines()) {
            let f: Vec<&str> = c.split(' ').collect();
            let op = f[0];
            let arr_at = |k: usize| f[k..].iter().map(|x| h(x) as usize).collect::<Vec<usize>>();
            let got = match op {
                "R" => {
                    let mut m = MusicLists { rng: (h(f[1]), h(f[2])), ..Default::default() };
                    let v = m.rand(h(f[3]) as usize, h(f[4]) as usize);
                    format!("R {:x} {:x} {:x}", v, m.rng.0, m.rng.1)
                }
                "S" => {
                    let (n, cap) = (h(f[1]) as usize, h(f[2]) as usize);
                    let mut m = MusicLists { rng: (h(f[4]), h(f[5])), ..Default::default() };
                    m.order[0] = MusicOrder { arr: arr_at(6), count: n, pos: h(f[3]) as usize };
                    m.shuffle_order(0);
                    format!("S {:x} {:x} {:x} 1 {} {:x} {:x}", n, cap, m.order[0].pos, hx(&m.order[0].arr), m.rng.0, m.rng.1)
                }
                "Z" => {
                    let cap = h(f[1]) as usize;
                    let mut m = MusicLists::default();
                    m.order[0] = MusicOrder { arr: arr_at(2), count: cap, pos: 0 };
                    m.set_shuffle(false);
                    format!("Z {:x} {:x} 0 1 {}", cap, cap, hx(&m.order[0].arr)).trim_end().to_string()
                }
                "N" | "P" => {
                    let (n, cap, pos) = (h(f[1]) as usize, h(f[2]) as usize, h(f[3]) as usize);
                    let mut m = MusicLists::default();
                    m.order[0] = MusicOrder { arr: arr_at(4), count: n, pos };
                    m.step(op == "N");
                    format!("{} {:x} {:x} {:x} 1 {} {:x}", op, n, cap, m.order[0].pos, hx(&m.order[0].arr), m.cur)
                }
                "F" => {
                    let (n, cap, pos) = (h(f[1]) as usize, h(f[2]) as usize, h(f[3]) as usize);
                    let mut m = MusicLists::default();
                    m.order[0] = MusicOrder { arr: arr_at(5), count: n, pos };
                    m.play_index(h(f[4]) as usize);
                    format!("F {:x} {:x} {:x} 1 {}", n, cap, m.order[0].pos, hx(&m.order[0].arr))
                }
                "C" => {
                    let cap = h(f[2]) as usize;
                    let n = if h(f[6]) == u32::MAX { cap } else { h(f[6]) as usize };
                    let mut m = MusicLists { rng: (h(f[4]), h(f[5])), ..Default::default() };
                    m.order[0] = MusicOrder { arr: arr_at(7), count: h(f[1]) as usize, pos: h(f[3]) as usize };
                    m.resize(0, n);
                    format!("C {:x} {:x} {:x} 80 {} {:x} {:x}", n, cap, m.order[0].pos, hx(&m.order[0].arr), m.rng.0, m.rng.1)
                }
                "W" => {
                    let n = h(f[1]) as usize;
                    let mut m = MusicLists { rng: (h(f[3]), h(f[4])), ..Default::default() };
                    m.order[0].arr = (0..n).collect();
                    m.resize(0, n);
                    format!("W {:x} {:x} 80 3000 {} {:x} {:x}", n, n, hx(&m.order[0].arr), m.rng.0, m.rng.1)
                }
                "G" => format!("G {}", gap_difficulty(f32::from_bits(h(f[1])))),
                "D" | "M" | "Q" => {
                    let (perf, d, m) = (f32::from_bits(h(f[1])), h(f[2]) as i32, h(f[3]) as i32);
                    let mut s = [0f32; 3];
                    for e in 0..3 {
                        let (pc, v) = (h(f[4 + 2 * e]), f32::from_bits(h(f[5 + 2 * e])));
                        s[(pc / 50) as usize] = v;
                    }
                    match op {
                        "D" => format!("D {}", setup_can_reach(s, perf, d) as i32),
                        "M" => {
                            let v = setup_mod_value(s, perf, d);
                            if w.trim() == "M -2147483648" && v != i32::MIN {
                                // two neighbouring samples equal: the original divides 0 by 0 and converts the NaN (0x80000000); the port returns the segment's start
                                assert!(s[0] == s[1] || s[1] == s[2], "case {c}");
                                *counts.entry("M-equal-samples-differ".to_string()).or_default() += 1;
                                continue;
                            }
                            format!("M {}", v)
                        }
                        _ => format!("Q {:x}", setup_perf(s, m).to_bits()),
                    }
                }
                "T" => {
                    let t = h(f[1]) as usize;
                    match arcade_env(0, 0, t).hour {
                        Some(x) => format!("T {:x}", x.to_bits()),
                        None => "T 7fc01234".to_string(), // the original leaves f1 as it was (the caller's value)
                    }
                }
                _ => panic!("op {op}"),
            };
            assert_eq!(got.split_whitespace().collect::<Vec<_>>(), w.split_whitespace().collect::<Vec<_>>(), "case {c}");
            *counts.entry(op.to_string()).or_default() += 1;
        }
        println!("stage1 cases per op: {counts:?}");
    }
    /// F-63809 (LocalOptions defaults): the original's constructor run on a zeroed 1340-byte block (work/verify-save/stage2.py;
    /// the platform language query is stubbed to 0) against `Options::default()` at the offsets of options__options.md.
    #[test]
    #[ignore = "reads the original's reference output (work/verify-save/stage2_original.txt)"]
    fn stage2_original_option_defaults() {
        let hex = save_fixture("stage2_original.txt");
        let orig: Vec<u8> = (0..1340).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap()).collect();
        let o = Options::default();
        let mut mine = vec![0u8; 1340];
        let mut put = |off: usize, b: &[u8]| mine[off..off + b.len()].copy_from_slice(b);
        put(0, &(o.control_config_pad as u16).to_be_bytes());
        put(2, &(o.control_config_wheel as u16).to_be_bytes());
        put(1188, &(o.vibration as u32).to_be_bytes());
        put(1192, &o.cinematics.to_be_bytes());
        put(1204, &o.gps_arrow.to_be_bytes());
        put(1208, &o.camera.to_be_bytes());
        put(1224, &(o.steering_sensitivity.0 as f32).to_be_bytes());
        put(1228, &[o.convertible_top_down as u8]);
        put(1238, &(o.subtitles as u16).to_be_bytes());
        for (off, v) in [(1240, &o.volume_effects), (1244, &o.volume_music), (1248, &o.volume_voice), (1252, &o.volume_player_vehicle), (1256, &o.volume_cutscene)] {
            put(off, &(v.0 as f32).to_be_bytes());
        }
        put(1316, &o.online_character.to_be_bytes());
        put(1320, &o.language.to_be_bytes());
        put(1324, &[o.airride_pattern]);
        put(1325, &[o.music_shuffle as u8]);
        put(1328, &o.music_genre.to_be_bytes());
        put(1332, &(o.time_of_day_hours.0 as f32).to_be_bytes());
        let fields: [(usize, usize); 20] = [(0, 2), (2, 2), (1188, 4), (1192, 4), (1204, 4), (1208, 4), (1224, 4), (1228, 1), (1238, 2), (1240, 4), (1244, 4), (1248, 4), (1252, 4), (1256, 4), (1316, 4), (1320, 4), (1324, 1), (1325, 1), (1328, 4), (1332, 4)];
        for (off, len) in fields {
            assert_eq!(orig[off..off + len], mine[off..off + len], "offset {off}");
        }
        // what the original sets that the port has no field for (kept as unknown bytes by the importer)
        let mut covered = vec![false; 1340];
        for (off, len) in fields {
            covered[off..off + len].iter_mut().for_each(|c| *c = true);
        }
        let extra: Vec<usize> = (0..1340).filter(|i| !covered[*i] && orig[*i] != 0).collect();
        println!("original default bytes without a port field (offsets): {extra:?}");
        assert_eq!(extra, vec![1203, 1219, 1232, 1233, 1234, 1235, 1336]);
    }

    /// F-34172 (an editor slot is offered in the city): the original on all 65,536 (slot city byte, current city) pairs
    /// (work/verify-save/stage3.py) against `editor_city_ok`.
    #[test]
    #[ignore = "reads the original's reference output (work/verify-save/stage3_original.txt)"]
    fn stage3_original_editor_city_ok() {
        let want = save_fixture("stage3_original.txt");
        let mut n = 0;
        for line in want.lines() {
            let v: Vec<u32> = line.split(' ').map(|x| x.parse().unwrap()).collect();
            assert_eq!(editor_city_ok(v[0], v[1]) as u32, v[2], "city {} current {}", v[0], v[1]);
            n += 1;
        }
        assert_eq!(n, 65536);
    }
}
    /// The five real saves (the 100 % save, a new game, a trace save, the PS3 one, Xenia's current one), imported by
    /// `tools/import_mc4_save.py` onto an empty port save (`imp`) and onto a copy of a real one (`imp2`) (the folders
    /// under `MCLA_ROUNDTRIP_FIXTURES`, default `D:/MCLA/work/save-audit`), loaded by the game with its career tables and
    /// written: every difference between the file the importer wrote and the file the game writes is listed in
    /// `<folder>/roundtrip_<set>_<name>.txt`; the block `mc4` must come back as it went in. Skipped without the fixtures
    /// or the data folder.
    #[test]
    fn real_saves_round_trip() {
        let root = std::env::var("MCLA_ROUNDTRIP_FIXTURES").unwrap_or_else(|_| "D:/MCLA/work/save-audit".to_string());
        for set in ["imp", "imp2"] {
            std::env::set_var("MCLA_SAVE_FIXTURES", format!("{root}/{set}"));
            for name in ["full_v15", "newgame_v14", "ps3_RAGE_dec", "recomp_trace_v14", "xenia_current_v14"] {
                if let Some(save) = fixture(&format!("{name}.json")) {
                    // (the game reads the importer's file: an unreadable one is silently a new game)
                    if let Err(e) = serde_json::from_value::<crate::game::SaveData>(save) {
                        panic!("{set}/{name}: the game cannot read the importer's file: {e}");
                    }
                }
                let Some((mut g, dir, save)) = career_game(&format!("{name}.json"), "rt") else {
                    eprintln!("(no fixture {set}/{name}: skipped)");
                    continue;
                };
                g.write_save(&dir);
                let t = std::fs::read_to_string(std::path::Path::new(&dir).join("mcla_save.json")).unwrap();
                let after: Value = serde_json::from_str(&t).unwrap();
                let mut diff = Vec::new();
                lost(&save, &after, "save", &mut diff);
                lost(&after, &save, "save(added)", &mut diff);
                let _ = std::fs::write(std::path::Path::new(&root).join(format!("roundtrip_{set}_{name}.txt")), diff.join("
"));
                let block: Vec<&String> = diff.iter().filter(|d| d.starts_with("save/mc4") || d.starts_with("save(added)/mc4")).collect();
                assert!(block.is_empty(), "{set}/{name}: {:#?}", &block[..block.len().min(40)]);
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }
    /// A key the types do not name, put into every kind of object of the block of the 100 % save, comes back where it
    /// was (the block is never narrowed to the keys the port knows).
    #[test]
    fn unknown_keys_come_back() {
        let root = std::env::var("MCLA_ROUNDTRIP_FIXTURES").unwrap_or_else(|_| "D:/MCLA/work/save-audit".to_string());
        let Some(mut save) = std::fs::read_to_string(format!("{root}/imp2/full_v15.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
            eprintln!("(no fixture: skipped)");
            return;
        };
        let spots = ["", "/options", "/garage/0", "/garage/0/visual", "/garage/0/performance", "/vehicle_records/0", "/career", "/career/missions/0",
            "/career/missions/0/objectives/0", "/career/experience", "/career/events/0", "/editor", "/editor/races/0", "/vinyl_packages/0", "/career/pink_slips"];
        let mut put = Vec::new();
        for sp in spots {
            if let Some(Value::Object(o)) = save["mc4"].pointer_mut(sp) {
                o.insert("zz_unknown".to_string(), Value::from(4242));
                put.push(sp);
            }
        }
        assert!(put.len() >= 12, "the spots: {put:?}");
        let dir = std::env::temp_dir().join(format!("mcla_unk_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("unk.json"), serde_json::to_string(&save).unwrap()).unwrap();
        std::env::set_var("MCLA_SAVE_FIXTURES", dir.to_string_lossy().to_string());
        let (mut g, d, _) = career_game("unk.json", "unk").unwrap();
        g.write_save(&d);
        let after = saved_block(&d);
        let mut gone = Vec::new();
        for sp in &put {
            if after.pointer(&format!("{sp}/zz_unknown")) != Some(&Value::from(4242)) {
                gone.push(*sp);
            }
        }
        assert!(gone.is_empty(), "unknown keys lost at {gone:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&d);
    }
    /// The purse is held within +-2,140,000,000 and mirrored into the statistic `tPUR` (F-71075, F-38145); an escape
    /// from the police does not touch it; the driving flush (F-15445) counts `vkDT` / `gvTS` / `ghTP` / `khTC` / `ghTC`.
    #[test]
    fn purse_cap_and_driving_flush() {
        let root = std::env::var("MCLA_ROUNDTRIP_FIXTURES").unwrap_or_else(|_| "D:/MCLA/work/save-audit".to_string());
        std::env::set_var("MCLA_SAVE_FIXTURES", format!("{root}/imp2"));
        let Some((mut g, dir, _)) = career_game("full_v15.json", "purse") else { return };
        g.money = 3_000_000_000;
        g.mc4_s23_tick(0.016, S23In::default());
        assert_eq!((g.money, g.career.stats.get("tPUR").copied()), (PURSE_CAP, Some(PURSE_CAP)));
        g.money = -3_000_000_000;
        g.mc4_s23_tick(0.016, S23In::default());
        assert_eq!(g.money, -PURSE_CAP);
        g.money = 12_345;
        g.career.event(crate::career::Ev::EscapedPolice);
        g.mc4_s23_tick(0.016, S23In::default());
        assert_eq!(g.career.stats.get("tPUR").copied(), Some(12_345), "an escape is not money");
        // (the flush: one second at 20 m/s in free roam)
        g.fresh_game = false;
        g.mode = crate::game::Mode::FreeRoam;
        let d = crate::mc4b::S23Drive { present: true, vel_xz: 20.0, speed: 20.0, ..Default::default() };
        let (p0, c0) = (g.mc4_stat("ghTP", 0), g.mc4_stat("khTC", 0));
        g.mc4b_tick(1.0, d);
        let hours = 1.0 / 3600.0;
        assert!((g.mc4_stat("ghTP", 0) - p0 - hours).abs() < 1e-6 && (g.mc4_stat("khTC", 0) - c0 - hours).abs() < 1e-6);
        assert!((g.mc4_stat("gvTS", 0) - 20.0 * 2.237).abs() < 1e-3 || g.mc4_stat("gvTS", 0) > 20.0 * 2.237);
        let car = g.car_dirs.get(g.current_car).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap();
        assert!(g.career.ostats.get(&format!("vkDT/{car}")).copied().unwrap_or(0.0) > 0.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
